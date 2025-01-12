use std::thread::sleep;
use std::time::Duration;
use super::*;

#[test]
fn extractor_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (extractor, inlet_chan): (Extractor, Receiver<String>) = Extractor::new(
        add_routine!(#[coroutine] |_: ()| {
                yield "Hello, world".to_string()
            }), stop_flag.clone(), None, (), 0);

    let data = inlet_chan.recv().unwrap();
    stop_flag.store(true, Ordering::Relaxed);
    extractor.join().unwrap();

    assert_eq!(data, "Hello, world".to_string())
}

#[test]
fn loader_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (test_tx, test_rx) = util::get_channel(0);
    let (loader, data_tx) = Loader::new(move |test: String| {
        test_tx.send(test).unwrap();
    }, None, stop_flag.clone(), 0);

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
        }), stop_flag.clone(), None, (), 0);

    let (test_outlet1_sink, test_outlet1_source) = util::get_channel(0);
    let (test_outlet2_sink, test_outlet2_source) = util::get_channel(0);

    let (test_outlet1, test1_tx) = Loader::new(move |example: String| {
        test_outlet1_sink.send(format!("1: {example}")).unwrap()
    }, None, stop_flag.clone(), 0);

    let (test_outlet2, test2_tx) = Loader::new(move |example: String| {
        test_outlet2_sink.send(format!("2: {example}")).unwrap()
    }, None, stop_flag.clone(), 0);

    let mut broadcaster = Broadcast::new(test_inlet.1, None, stop_flag.clone(), 0);
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

    let (tx, rx) = util::get_channel(0);

    let mut router= Router::new(rx, None, stop_flag.clone());

    let (in1, out1) = util::get_channel(0);
    let (in2, out2) = util::get_channel(0);

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
    
    let input_data = vec![
        "hello world".to_string(),
        "goodbye world".to_string(),
        "hello there".to_string()
    ];
    
    let expected_data = vec![
        "hello world".to_string(),
        "hello there".to_string()
    ];

    let stop_flag = Arc::new(AtomicBool::new(false));

    let (tx, rx) = util::get_channel(0);

    let (filter, filter_sink) = Filter::new(fun, rx, None, stop_flag.clone(), 0);

    tx.send(input_data).unwrap();
    let data = filter_sink.recv().unwrap();

    assert_eq!(data, expected_data);

    stop_flag.store(true, Ordering::Relaxed);
    filter.join().unwrap()
}

#[test]
fn transformer_works() {
    let flag = Arc::new(AtomicBool::new(false));
    let array = vec![1, 3, 5, 7, 9];

    let routine = add_routine!(#[coroutine] |input: Arc<Mutex<Cell<TransformerContext<Vec<i32>, i32>>>>| {
        let context = input.lock().unwrap().take();

        let multiplier = context.globals;
        let data = context.data.unwrap().clone();

        let mut result = Vec::new();
        for item in data.into_iter() {
            result.push(item * multiplier)
        }

        #[cfg(debug_assertions)]
        effect_debug!(vec!["Function completed".to_string()]);

        terminate!(Some(result));
    });

    let (transformer,
        input_sink,
        output_source,
        effect) = Transformer::new(routine,  None, flag.clone(), 2, 100);


    input_sink.send(array).unwrap();
    #[cfg(debug_assertions)]
    (|| {
        match effect.recv().unwrap() {
            TransformerEffectType::Debug(val) => {
                assert_eq!(val, vec!["Function completed".to_string()])
            },
            _ => {panic!("Invalid variant");}
        }

    })();

    let result: Vec<i32> = output_source.recv().unwrap();
    assert_eq!(result, vec![2, 6, 10, 14, 18]);
    flag.store(true, Ordering::Relaxed);
    transformer.join().unwrap()
}

#[test]
fn funnel_works() {
    let stop_flag = Arc::new(AtomicBool::new(false));

    let (mut funnel, funnel_out) = Funnel::new(None, stop_flag.clone(), 0);

    let (rx1, tx1) = util::get_channel(0);
    let (rx2, tx2) = util::get_channel(0);
    let (rx3, tx3) = util::get_channel(0);

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
    let (mut messenger, messenger_sender) = Messenger::new(None, stop_flag.clone(), 0);

    let (tx1, rx1) = util::get_channel(0);
    let (tx2, rx2) = util::get_channel(0);
    let (tx3, rx3) = util::get_channel(0);

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

    let (src_tx, src_rx) = util::get_channel(0);

    let (accumulator, accumulate_chan) = Accumulator::new(1000, None, stop_flag.clone(), src_rx, 0);

    src_tx.send("hello").unwrap();
    src_tx.send("there").unwrap();
    src_tx.send("world").unwrap();

    let result = accumulate_chan.recv().unwrap();

    assert_eq!(result, vec!["hello", "there", "world"]);

    stop_flag.store(true, Ordering::Relaxed);
    accumulator.join().unwrap()
}

#[test]
fn transformer_emerg_effect() {
    let flag = Arc::new(AtomicBool::new(false));

    let routine = add_routine!(#[coroutine] |_: Arc<Mutex<Cell<TransformerContext<Vec<()>, ()>>>>| {

        effect_emerg!(vec!["Emergency effect".to_string()]);

        effect_debug!(vec!["Invalid effect".to_string()]);
        terminate!(None::<Vec<()>>);
    });

    let (transformer,
        input_sink,
        _o,
        effect) = Transformer::new(routine,  None, flag.clone(), (), 100);


    input_sink.send(vec![()]).unwrap();

    match effect.recv().unwrap() {
        TransformerEffectType::Emergency(val) => {
            assert_eq!(val, vec!["Emergency effect".to_string()])
        },
        _ => panic!("Invalid variant")
    };

    match effect.try_recv(){
        Err(val ) => {
            assert!(val.is_empty(), "{}", true)
        },
        _ => {panic!("Transformer did not abort")}
    }

    flag.store(true, Ordering::Relaxed);
    transformer.join().unwrap()
}

#[test]
fn transformer_crit_effect() {
    let flag = Arc::new(AtomicBool::new(false));

    let routine = add_routine!(#[coroutine] |_: Arc<Mutex<Cell<TransformerContext<Vec<()>, ()>>>>| {

        effect_crit!(vec!["Emergency effect".to_string()]);

        effect_debug!(vec!["Invalid effect".to_string()]);
        terminate!(None::<Vec<()>>);
    });

    let (transformer,
        input_sink,
        _o,
        effect) = Transformer::new(routine,  None, flag.clone(), (), 100);


    input_sink.send(vec![()]).unwrap();

    match effect.recv().unwrap() {
        TransformerEffectType::Critical(val) => {
            assert_eq!(val, vec!["Emergency effect".to_string()])
        },
        _ => panic!("Invalid variant")
    };

    match effect.try_recv(){
        Err(val ) => {
            assert!(val.is_empty(), "{}", true)
        },
        _ => {panic!("Transformer did not abort")}
    }

    flag.store(true, Ordering::Relaxed);
    transformer.join().unwrap()
}

#[test]
fn transformer_effect() {
    let flag = Arc::new(AtomicBool::new(false));

    let routine = add_routine!(#[coroutine] |_: Arc<Mutex<Cell<TransformerContext<Vec<()>, ()>>>>| {

        effect_alert!(vec!["This is an alert".to_string()]);
        effect_error!(vec!["This is an error".to_string()]);
        effect_info!(vec!["This is an info".to_string()]);
        effect_warn!(vec!["This is a warning".to_string()]);
        effect_notice!(vec!["This is a notice".to_string()]);
        effect_debug!(vec!["This is a debug".to_string()]);
        terminate!(None::<Vec<()>>);
    });

    let (transformer,
        input_sink,
        _o,
        effect) = Transformer::new(routine,  None, flag.clone(), (), 100);


    input_sink.send(vec![()]).unwrap();

    match effect.recv().unwrap() {
        TransformerEffectType::Alert(val) => {
            assert_eq!(val, vec!["This is an alert".to_string()])
        },
        _ => panic!("Invalid variant")
    };

    match effect.recv().unwrap() {
        TransformerEffectType::Error(val) => {
            assert_eq!(val, vec!["This is an error".to_string()])
        },
        _ => panic!("Invalid variant")
    };

    match effect.recv().unwrap() {
        TransformerEffectType::Info(val) => {
            assert_eq!(val, vec!["This is an info".to_string()])
        },
        _ => panic!("Invalid variant")
    };

    match effect.recv().unwrap() {
        TransformerEffectType::Warning(val) => {
            assert_eq!(val, vec!["This is a warning".to_string()])
        },
        _ => panic!("Invalid variant")
    };

    match effect.recv().unwrap() {
        TransformerEffectType::Notice(val) => {
            assert_eq!(val, vec!["This is a notice".to_string()])
        },
        _ => panic!("Invalid variant")
    };

    match effect.recv().unwrap() {
        TransformerEffectType::Debug(val) => {
            assert_eq!(val, vec!["This is a debug".to_string()])
        },
        _ => panic!("Invalid variant")
    };

    flag.store(true, Ordering::Relaxed);
    transformer.join().unwrap()
}
