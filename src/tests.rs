use std::thread::sleep;
use std::time::Duration;
use super::*;

#[test]
fn extractor_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (extractor, inlet_chan): (Extractor, Receiver<String>) = Extractor::new(
        add_routine!(#[coroutine] |_: ()| {
                yield "Hello, world".to_string()
            }), stop_flag.clone(), None, (), None);

    let data = inlet_chan.recv().unwrap();
    stop_flag.store(true, Ordering::Relaxed);
    extractor.join().unwrap();

    assert_eq!(data, "Hello, world".to_string())
}

#[test]
fn loader_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (test_tx, test_rx) = unbounded();
    let (loader, data_tx) = Loader::new(move |test: String| {
        test_tx.send(test).unwrap();
    }, None, stop_flag.clone(), None);

    data_tx.send("Hello, world".to_string()).unwrap();

    let data_recv = test_rx.recv().unwrap();
    stop_flag.store(true, Ordering::Relaxed);
    loader.join().unwrap();

    assert_eq!(data_recv, "Hello, world".to_string())
}

#[test]
fn broadcast_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));

    let test_inlet: (Extractor, Receiver<String>) = Extractor::new(add_routine!(#[coroutine] || {
            sleep(Duration::from_secs(1));
            yield "hello".to_string()
        }), stop_flag.clone(), None, (), None);

    let (test_outlet1_sink, test_outlet1_source) = unbounded();
    let (test_outlet2_sink, test_outlet2_source) = unbounded();

    let (test_outlet1, test1_tx) = Loader::new(move |example: String| {
        test_outlet1_sink.send(format!("1: {example}")).unwrap()
    }, None, stop_flag.clone(), None);

    let (test_outlet2, test2_tx) = Loader::new(move |example: String| {
        test_outlet2_sink.send(format!("2: {example}")).unwrap()
    }, None, stop_flag.clone(), None);

    let mut broadcaster = Broadcast::new(test_inlet.1, None, stop_flag.clone(), None);
    broadcaster.subscribe(test1_tx);
    broadcaster.subscribe(test2_tx);

    let chan1 = broadcaster.channel();
    let chan2 = broadcaster.channel();

    let data1 = test_outlet1_source.recv().unwrap();
    let data2 = test_outlet2_source.recv().unwrap();
    let data3 = chan1.recv().unwrap();
    let data4 = chan2.recv().unwrap();

    stop_flag.store(true, Ordering::Relaxed);

    test_outlet1.join().unwrap();
    test_outlet2.join().unwrap();
    test_inlet.0.join().unwrap();
    broadcaster.join().unwrap();

    assert_eq!(data1, "1: hello".to_string());
    assert_eq!(data2, "2: hello".to_string());
    assert_eq!(data3, "hello".to_string());
    assert_eq!(data4, "hello".to_string());
}

#[test]
fn router_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));

    let (tx, rx) = unbounded();

    let mut router= Router::new(rx, None, stop_flag.clone());

    let (in1, out1) = unbounded();
    let (in2, out2) = unbounded();

    router.subscribe(in1);
    router.subscribe(in2);

    tx.send("hello".to_string()).unwrap();
    tx.send("there".to_string()).unwrap();
    tx.send("beautiful".to_string()).unwrap();
    tx.send("world".to_string()).unwrap();

    let out1_res = out1.recv().unwrap();
    let out2_res = out2.recv().unwrap();
    let out3_res = out1.recv().unwrap();
    let out4_res = out2.recv().unwrap();

    assert_eq!(out1_res, "hello".to_string());
    assert_eq!(out2_res, "there".to_string());
    assert_eq!(out3_res, "beautiful".to_string());
    assert_eq!(out4_res, "world".to_string());
}

#[test]
fn filter_works() {
    let fun = |data: &String| -> bool {
        data.contains("hello")
    };

    let stop_flag = Arc::new(AtomicBool::new(false));

    let (tx, rx) = unbounded();

    let (filter, filter_sink) = Filter::new(fun, rx, None, stop_flag.clone(), None);

    tx.send("hello world".to_string()).unwrap();
    let data = filter_sink.recv().unwrap();

    assert_eq!(data, "hello world");

    tx.send("goodbye world".to_string()).unwrap();
    let res = filter_sink.recv_timeout(Duration::from_secs(1));
    assert!(res.is_err());

    tx.send("hello there".to_string()).unwrap();
    let data = filter_sink.recv().unwrap();

    assert_eq!(data, "hello there");

    stop_flag.store(true, Ordering::Relaxed);
    filter.join().unwrap()
}

#[test]
fn transformer_works() {
    #[derive(Clone, Default)]
    struct InnerContext {
        inc_val: i32
    }

    let ctx = InnerContext {
        inc_val: 1
    };

    let stop_flag = Arc::new(AtomicBool::new(false));
    let (transformer, input, output, _): (Transformer<Vec<i32>, i32, String>, Sender<Vec<i32>>, Receiver<i32>, Receiver<String>) = Transformer::new(
        add_routine!(#[coroutine] |input: Arc<Mutex<Cell<TransformerContext<Vec<i32>, InnerContext>>>>| {

                let data = {
                    input.lock().unwrap().take()
                };

                let value = data.data.unwrap();
                let inc = data.globals.inc_val;
                for x in value {
                    yield TransformerResult::Transformed(x + inc);
                }
                yield TransformerResult::Completed(None)
            }), None, stop_flag.clone(), ctx, None);

    input.send(vec![1, 3, 5, 7, 9, 11]).unwrap();

    let mut result = 0;

    while let Ok(val) = output.recv_timeout(Duration::from_millis(50)) {
        result += val;
    }

    stop_flag.store(true, Ordering::Relaxed);
    transformer.join().unwrap();

    assert_eq!(result, 42)
}

#[test]
fn funnel_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));

    let (mut funnel, funnel_out) = Funnel::new(None, stop_flag.clone(), None);

    let (rx1, tx1) = unbounded();
    let (rx2, tx2) = unbounded();
    let (rx3, tx3) = unbounded();

    funnel.add_source(tx1);
    funnel.add_source(tx2);
    funnel.add_source(tx3);

    rx1.send("hello".to_string()).unwrap();
    rx2.send("beautiful".to_string()).unwrap();
    rx3.send("world".to_string()).unwrap();

    let str1 = funnel_out.recv().unwrap();
    let str2 = funnel_out.recv().unwrap();
    let str3 = funnel_out.recv().unwrap();

    assert_eq!(str1, "hello");
    assert_eq!(str2, "beautiful");
    assert_eq!(str3, "world");

    stop_flag.store(true, Ordering::Relaxed);

    funnel.join().unwrap()
}

#[test]
fn messenger_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (mut messenger, messenger_sender) = Messenger::new(None, stop_flag.clone(), None);

    let (tx1, rx1) = unbounded();
    let (tx2, rx2) = unbounded();
    let (tx3, rx3) = unbounded();

    messenger.subscribe("hello", tx1);
    messenger.subscribe("beautiful", tx3);
    messenger.subscribe("world", tx2);

    messenger_sender.send(Message{
        id: "beautiful",
        message: 330
    }).unwrap();

    messenger_sender.send(Message{
        id: "world",
        message: 5
    }).unwrap();

    messenger_sender.send(Message{
        id: "hello",
        message: 66
    }).unwrap();

    let res1 = rx1.recv().unwrap();
    let res2 = rx2.recv().unwrap();
    let res3 = rx3.recv().unwrap();

    assert_eq!(res1, 66);
    assert_eq!(res2, 5);
    assert_eq!(res3, 330);

    stop_flag.store(true, Ordering::Relaxed);
    messenger.join().unwrap()
}

#[test]
fn accumulator_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));

    let (src_tx, src_rx) = unbounded();

    let (accumulator, accumulate_chan) = Accumulator::new(1000, None, stop_flag.clone(), src_rx, None);

    src_tx.send("hello").unwrap();
    src_tx.send("there").unwrap();
    src_tx.send("world").unwrap();

    let result = accumulate_chan.recv().unwrap();

    assert_eq!(result, vec!["hello", "there", "world"]);

    stop_flag.store(true, Ordering::Relaxed);
    accumulator.join().unwrap()
}

#[test]
fn transformer_01_works() {
    #[derive(Clone, Default)]
    struct InnerContext {
        inc_val: i32
    }

    let ctx = InnerContext {
        inc_val: 1
    };

    let stop_flag = Arc::new(AtomicBool::new(false));
    let (transformer, input, output, _): (Transformer<i32, i32, String>, Sender<i32>, Receiver<i32>, Receiver<String>) = Transformer::new(
        add_routine!(#[coroutine] |input: Arc<Mutex<Cell<TransformerContext<i32, InnerContext>>>>| {

                let data = {
                    input.lock().unwrap().take()
                };

                let value = data.data.unwrap();
                let inc = data.globals.inc_val;

                if value < 42 {
                    yield TransformerResult::NeedsMoreWork(value + inc)
                } else {
                    yield TransformerResult::Completed(Some(value))
                }

            }), None, stop_flag.clone(), ctx, None);

    input.send(0).unwrap();

    let mut result = 0;

    while let Ok(val) = output.recv_timeout(Duration::from_millis(50)) {
        result += val;
    }

    stop_flag.store(true, Ordering::Relaxed);
    transformer.join().unwrap();

    assert_eq!(result, 42)
}