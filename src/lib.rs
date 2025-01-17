#![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
#![feature(unboxed_closures)]


#[cfg(test)]
mod tests;
mod util;

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::{Coroutine, CoroutineState};
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::thread::{JoinHandle, sleep};
use std::time::{Duration, Instant};
use crossbeam_channel::{Receiver, Sender};


/// An enum returned from a coroutine sent to a `Transformer` object.
/// 
/// The following variants work as follows:
/// - `Transformed` - Data has been mutated and can be sent along a `Reflux` network.
/// - `NeedsMoreWork` - Data has been processed, but is not ready to be sent through the network.
/// - `Error` - An error occurred when processing data.
#[derive(Debug, Clone)]
pub enum TransformerResult<O, T, E>  {
    Output(Vec<T>),
    Terminate(Option<Vec<T>>),
    Consume(Vec<O>),
    Effect(TransformerEffectType<E>),
}

#[derive(Debug, Clone)]
pub enum TransformerEffectType<E> {
    Emergency(Vec<E>),
    Alert(Vec<E>),
    Critical(Vec<E>),
    Error(Vec<E>),
    Warning(Vec<E>),
    Notice(Vec<E>),
    Info(Vec<E>),
    Debug(Vec<E>),
}

/// Receives data from an external source and sends the data through a channel.
///
/// Using an `Extractor` yields the following benefits:
/// - Abstracts away the use of channels. You only need to write a coroutine that takes a parameter
/// and yields a result.
/// - The coroutine does not have to be aware of termination signals, or joining threads. This
/// functionality is handled by `Extractor`.
/// - Easy integration with other `Reflux` modules.
///
/// # Example
/// ```
///  #![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
///  #![feature(unboxed_closures)]
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::Receiver;
///  use reflux::{add_routine, FnContext};
///  use reflux::Extractor;
///  let stop_flag = Arc::new(AtomicBool::new(false));
///
///  let extract_fn = |ctx: FnContext<(), String>| {
///      let data_vec = ctx.data.lock().unwrap();
///      data_vec.set(vec![String::from("Hello"), String::from("world")]);
///  };
///
///  let (extractor, inlet_chan): (Extractor, Receiver<Vec<String>>) = Extractor::new(
///  extract_fn, stop_flag.clone(), None, (), 100, 100);
///
///  let data = inlet_chan.recv().unwrap();
///  stop_flag.store(true, Ordering::Relaxed);
///  extractor.join().unwrap();
///
///  assert_eq!(data, vec![String::from("Hello"), String::from("world")]);
/// ```
pub struct Extractor {
    extract_fn: JoinHandle<()>,
    fn_thr: JoinHandle<()>,
}

impl Extractor {
    /// Creates a new `Extractor` object.
    /// 
    /// # Parameters
    /// - `extract_fn` - A coroutine that reads and returns data from an external source.
    /// The use of the `add_routine!` macro is necessary when passing in an `extract_fn`.
    /// - `pause_sig` - A flag to signal the `Extractor` object to pause execution.
    /// - `stop_sig` - A flag to signal the `Extractor` object to terminate execution.
    /// 
    /// # Returns
    ///  - A `Extractor` object.
    ///  - A`Receiver` channel to receive data from the `extract_fn`.
    pub fn new<F, T, I>( extract_fn: F,
                            stop_sig: Arc<AtomicBool>,
                            pause_sig: Option<Arc<AtomicBool>>,
                            init_data: I,
                            interval_ms: u64,
                            data_limit: usize) -> (Self, Receiver<Vec<T>>)
        where 
            F: Fn(FnContext<I, T>) -> () + Send + Sync + 'static,
            I: Send + 'static + Clone,
            T: Send + 'static + Clone, {
        let (tx, rx) : (Sender<Vec<T>>, Receiver<Vec<T>>) = util::get_channel(data_limit);

        let extract_ctx = FnContext {
            init_data,
            data: Arc::new(Mutex::new(Cell::new(Vec::new()))),
            stop_sig: stop_sig.clone(),
            pause_sig: pause_sig.clone()
        };

        let thr_ctx = extract_ctx.data.clone();
        let extract_pause = pause_sig.clone();
        let extract_stop = stop_sig.clone();

        let extract_thr = thread::spawn(move || {
            while !extract_stop.load(Ordering::Relaxed) {
                if let Some(sig) = extract_pause.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(interval_ms));
                        continue
                    }
                }

                if let Ok (guard) = thr_ctx.try_lock() {
                    let data = guard.take();
                    if data.len() > 0 {
                        tx.send(data).unwrap();
                    }
                }

                sleep(Duration::from_millis(50));
            }
        });

        let fn_thr = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Some(sig) = pause_sig.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(interval_ms));
                        continue
                    }
                }
                extract_fn(extract_ctx.clone());
            }
        });
        let s = Self {
            extract_fn: extract_thr,
            fn_thr,
        };

        (s, rx)
    }

    /// Waits for the `Extractor` object to finish execution.
    pub fn join(self) -> thread::Result<()> {
        self.extract_fn.join()?;
        self.fn_thr.join()?;
        Ok(())
    }
}

/// An object that receives data from a `Reflux` network and sends the data to some external sink.
/// 
/// Using an `Loader` yields the following benefits:
/// - Abstracts away the use of channels. You only need to receive a parameter and send it to an
/// external sink.
/// - The function does not have to be aware of termination signals, or joining threads. This
/// functionality is handled by `Loader`.
/// - Easy integration with other `Reflux` modules.
/// 
/// # Example
/// ```
///  use reflux::{FnContext, Loader};
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::{bounded, Receiver};
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
///  use std::thread::sleep;
///
///  let stop_flag = Arc::new(AtomicBool::new(false));
///  let (test_tx, test_rx) = bounded(1);
///  let (loader, data_tx) = Loader::new(move |test: FnContext<(), String>| {
///      let mut data = test.data.lock().unwrap();
///      if data.get_mut().len() > 0 {
///          test_tx.send(data.take()).unwrap();
///      }
///  }, (), None, stop_flag.clone(), 50, 50);
///
///  data_tx.send(vec!["Hello".to_string(), "world".to_string()]).unwrap();
///
///  let data_recv = test_rx.recv().unwrap();
///  stop_flag.store(true, Ordering::Relaxed);
///  loader.join().unwrap();
///
///  assert_eq!(data_recv, vec!["Hello".to_string(), "world".to_string()]);
/// ```
pub struct Loader {
    loader_fn: JoinHandle<()>,
    worker_fn: JoinHandle<()>
}

impl Loader {
    /// Creates a new `Loader` object.
    ///
    /// # Parameters
    /// - `loader_fn` - A function that receives data from a `Reflux` network and sends it to an
    /// external sink.
    /// - `receiver` - A `Receiver` channel object from which to receive data.
    /// - `pause_sig` - A flag to signal the `Loader` object to pause execution.
    /// - `stop_sig` - A flag to signal the `Loader` object to terminate execution.
    ///
    /// # Returns
    /// - A `Loader` object.
    /// - A `Sender` channel to send data out to.
    pub fn new<T, I, F>(mut loader_fn: F,
                        init_data: I,
                        pause_sig: Option<Arc<AtomicBool>>,
                        stop_sig: Arc<AtomicBool>,
                        interval_ms: u64,
                        data_limit: usize) -> (Self, Sender<Vec<T>>)
        where
            T: Send + 'static + Clone,
            I: Send + 'static + Clone,
            F: FnMut(FnContext<I, T>) + Send + 'static {

        let (sender, receiver) : (Sender<Vec<T>>, Receiver<Vec<T>>) = util::get_channel(data_limit);

        let ctx = FnContext {
            init_data,
            data: Arc::new(Mutex::new(Cell::new(Vec::new()))),
            stop_sig: stop_sig.clone(),
            pause_sig: pause_sig.clone()
        };

        let thr_ctx = ctx.data.clone();

        let thr_stop_sig = stop_sig.clone();
        let thr_pause_sig = pause_sig.clone();

        let loader_thr = thread::spawn(move || {
            while !thr_stop_sig.load(Ordering::Relaxed) {
                if let Some(sig) = thr_pause_sig.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(50));
                        continue
                    }
                }
                if let Ok(data) = receiver.recv_timeout(Duration::from_millis(10)) {
                    if data.len() > 0 {
                        thr_ctx.lock().unwrap().set(data);
                    }
                }
            }
        });

        let worker_thr = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Some(sig) = pause_sig.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(interval_ms));
                        continue
                    }
                }
                loader_fn(ctx.clone());
            }
        });



        (Self {
            loader_fn: loader_thr,
            worker_fn: worker_thr,
        }, sender)
    }
    
    /// Waits for the `Loader` object to finish execution.
    pub fn join(self) -> thread::Result<()> {
        self.loader_fn.join()?;
        self.worker_fn.join()?;
        Ok(())
    }
}

/// An object that received data from a provided `Receiver`, and broadcasts the data to all
/// subscribers.
///
/// Using a `Broadcast` yields the following benefits:
/// - Send received data to multiple endpoint. A `Broadcast` object can be used to build a web
/// of objects.
///
/// # Example
/// ```
///  #![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
///  #![feature(unboxed_closures)]
///  use reflux::{Extractor, Loader, Broadcast};
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::{bounded, Receiver};
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
///  use std::time::Duration;
///  use std::thread::sleep;
///  let stop_flag = Arc::new(AtomicBool::new(false));
///
/// let stop_flag = Arc::new(AtomicBool::new(false));
///
/// let (input_tx, input_rx) = bounded(0);
/// let (output1_tx, output1_rx) = bounded(0);
/// let (output2_tx, output2_rx) = bounded(0);
///
///
/// let mut broadcast = Broadcast::new(
///     input_rx,
///     None,
///     stop_flag.clone(),
///     100
/// );
///
/// broadcast.subscribe(output1_tx);
/// broadcast.subscribe(output2_tx);
///
/// input_tx.send(vec!["Hello".to_string(), "World".to_string()]).unwrap();
///
///assert_eq!(output1_rx.recv().unwrap(), vec!["Hello".to_string(), "World".to_string()]);
///assert_eq!(output2_rx.recv().unwrap(), vec!["Hello".to_string(), "World".to_string()]);
/// stop_flag.store(true, Ordering::Relaxed);
/// broadcast.join().unwrap();
/// ```
pub struct Broadcast<T> {
    subscribers: Arc<Mutex<Vec<Sender<Vec<T>>>>>,
    _broadcaster: JoinHandle<()>,
    data_limit: usize,
}

impl<T> Broadcast<T> where T: Clone + Send + 'static {
    /// Creates a new `Broadcast` object.
    ///
    /// # Parameters
    /// - `source` - The source of data that needs to be broadcast.
    /// - `pause_sig` - A flag to signal the `Broadcast` object to pause execution.
    /// - `stop_sig` - A flag to signal the `Broadcast` object to terminate execution.
    ///
    /// # Returns
    /// A `Broadcast` object.
    pub fn new( source: Receiver<Vec<T>>,
                pause_sig: Option<Arc<AtomicBool>>,
                stop_sig: Arc<AtomicBool>,
                data_limit: usize) -> Self {
        let subscribers = Arc::new(Mutex::new(Vec::<Sender<Vec<T>>>::new()));
        
        let thr_subscribers = subscribers.clone();
        let broadcaster = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Some(sig) = pause_sig.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(50));
                        continue
                    }
                }
                if let Ok(data) = source.recv_timeout(Duration::from_millis(10)) {
                    let subscribers_lock = thr_subscribers.lock().unwrap();
                    for subscriber in subscribers_lock.iter() {
                        subscriber.send(data.clone()).unwrap();
                    };           
                }
            }
        });
        Self {
            subscribers,
            _broadcaster: broadcaster,
            data_limit
        }
    }

    /// Add a subscriber to the `Broadcast`
    ///
    /// # Parameters
    ///  - `subscriber` - A `Sender` with which to send data.
    pub fn subscribe(&mut self, subscriber: Sender<Vec<T>>) {
        let mut subscribers_lock = self.subscribers.lock().unwrap();
        subscribers_lock.push(subscriber)
    }
    
    /// Create a subscription for an external object to use to receive data from the `Broadcast`
    /// 
    /// # Returns
    /// - A `Receiver` channel from which to receive data.
    pub fn channel(&mut self) -> Receiver<Vec<T>> {
        let (tx, rx) = util::get_channel(self.data_limit);
        let mut subscribers_lock = self.subscribers.lock().unwrap();
        subscribers_lock.push(tx);
        rx
    }

    /// Waits for the `Broadcast` object to finish execution.
    pub fn join(self) -> thread::Result<()> {
        self._broadcaster.join()
    }
}


/// An object that received data from a provided `Receiver`, and broadcasts the data to subscribers
/// using a Round Robin algorithm.
///
/// Using a `Router` yields the following benefits:
/// - Distribute data among multiple receivers, thus ensuring an even distribution of workload.
///
/// # Example
/// ```
///  #![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
///  #![feature(unboxed_closures)]
///  use reflux::Router;
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::Receiver;
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
///  use std::time::Duration;
///  use std::thread::sleep;
///  let stop_flag = Arc::new(AtomicBool::new(false));
///  let stop_flag = Arc::new(AtomicBool::new(false));
///         
///  let (tx, rx) = unbounded();
///         
///  let mut router = Router::new(rx, None, stop_flag.clone());
///         
///  let (in1, out1) = unbounded();
///  let (in2, out2) = unbounded();
///         
///  router.subscribe(in1);
///  router.subscribe(in2);
///         
///  tx.send("hello".to_string()).unwrap();
///  tx.send("there".to_string()).unwrap();
///  tx.send("beautiful".to_string()).unwrap();
///  tx.send("world".to_string()).unwrap();
///         
///  let out1_res = out1.recv().unwrap();
///  let out2_res = out2.recv().unwrap();
///  let out3_res = out1.recv().unwrap();
///  let out4_res = out2.recv().unwrap();
///         
///  assert_eq!(out1_res, "hello".to_string());
///  assert_eq!(out2_res, "there".to_string());
///  assert_eq!(out3_res, "beautiful".to_string());
///  assert_eq!(out4_res, "world".to_string());
/// ```
pub struct Router <T>  {
    subscribers: Arc<Mutex<Vec<Sender<T>>>>,
    _dispatcher: JoinHandle<()>,
}


impl <T> Router<T> where T: Send + 'static {
    /// Creates a new `Router` object.
    ///
    /// # Parameters
    /// - `source` - The source of data that needs to be routed.
    /// - `pause_sig` - A flag to signal the `Router` object to pause execution.
    /// - `stop_sig` - A flag to signal the `Router` object to terminate execution.
    ///
    /// # Returns
    /// A `Router` object.
    pub fn new(source: Receiver<T>,
               pause_sig: Option<Arc<AtomicBool>>,
               stop_sig: Arc<AtomicBool>) -> Self {
        let subscribers = Arc::new(Mutex::new(Vec::<Sender<T>>::new()));

        let thr_subscribers = subscribers.clone();
        let dispatcher = thread::spawn(move || {
            let mut pointer = 0;
            while !stop_sig.load(Ordering::Relaxed) {
                if let Some(sig) = pause_sig.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(50));
                        continue
                    }
                }
                if let Ok(data) = source.recv_timeout(Duration::from_millis(10)) {
                    let subscribers_lock = thr_subscribers.lock().unwrap();
                    subscribers_lock.get(pointer).unwrap().send(data).unwrap();
                    pointer = (pointer + 1) % subscribers_lock.len();
                }
            }
        });
        Self {
            subscribers,
            _dispatcher: dispatcher
        }
    }

    /// Add a subscriber to the `Router`.
    /// 
    /// # Parameters
    ///  - `subscriber` - A `Sender` with which to send data to.
    pub fn subscribe(&mut self, subscriber: Sender<T>) {
        let mut subscribers_lock = self.subscribers.lock().unwrap();
        subscribers_lock.push(subscriber)
    }

    /// Waits for the `Router` object to finish execution.
    pub fn join(self) -> thread::Result<()> {
        self._dispatcher.join()
    }
}

pub struct Message<ID, M> where M: 'static + Send, ID: Hash + Eq {
    pub id: ID,
    pub message: M
}


pub struct Messenger<ID, S> where ID: Hash + Eq + Send + 'static, S : 'static + Send {
    subscribers: Arc<Mutex<HashMap<ID, Sender<S>>>>,
    worker: JoinHandle<()>,
}


impl <ID, S> Messenger<ID, S>  where ID: Eq + Hash + Send + 'static, S: 'static + Send {
    pub fn new( pause_sig: Option<Arc<AtomicBool>>,
                stop_sig: Arc<AtomicBool>,
                data_limit: usize) -> (Self, Sender<Message<ID, S>>) {
        let (tx, rx)= util::get_channel::<Message<ID, S>>(data_limit);
        let subscribers = Arc::new(Mutex::new(HashMap::<ID, Sender<S>>::new()));
        let thr_subscribers = subscribers.clone();
        let worker = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Some(sig) = pause_sig.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(50));
                        continue
                    }
                }
                if let Ok(msg) = rx.recv_timeout(Duration::from_millis(10)) {
                    let subs = thr_subscribers.lock().unwrap();
                    if let Some(sub) = subs.get(&msg.id) {
                        let _ = sub.send(msg.message);
                    }
                }
            }
        });
        (
            Self {
                worker,
                subscribers
            },
            tx
        )
    }

    pub fn subscribe(&mut self, id: ID, subscriber: Sender<S>) {
        let mut subscribers_lock = self.subscribers.lock().unwrap();
        subscribers_lock.insert(id, subscriber);
    }

    pub fn join(self) -> thread::Result<()> {
        self.worker.join()
    }
}


/// An object that receives data from multiple subscriber and channels the data to a single output.
/// 
/// Using a `Funnel` object yields the following benefits:
///  - Consolidates data from multiple sources into a single data stream.
/// 
/// # Example
/// ```
///  #![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
///  #![feature(unboxed_closures)]
///  use reflux::Funnel;
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::Receiver;
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
///  use std::time::Duration;
///  use std::thread::sleep;
///  let stop_flag = Arc::new(AtomicBool::new(false));
///  let stop_flag = Arc::new(AtomicBool::new(false));
///  let stop_flag = Arc::new(AtomicBool::new(false));
///         
///  let (mut funnel, funnel_out) = Funnel::new(None, stop_flag.clone(), 0);
///         
///  let (rx1, tx1) = unbounded();
///  let (rx2, tx2) = unbounded();
///  let (rx3, tx3) = unbounded();
///         
///  funnel.add_source(tx1);
///  funnel.add_source(tx2);
///  funnel.add_source(tx3);
///         
///  rx1.send("hello".to_string()).unwrap();
///  rx2.send("beautiful".to_string()).unwrap();
///  rx3.send("world".to_string()).unwrap();
///         
///  let str1 = funnel_out.recv().unwrap();
///  let str2 = funnel_out.recv().unwrap();
///  let str3 = funnel_out.recv().unwrap();
///         
///  assert_eq!(str1, "hello");
///  assert_eq!(str2, "beautiful");
///  assert_eq!(str3, "world");
///         
///  stop_flag.store(true, Ordering::Relaxed);
///         
///  funnel.join().unwrap()
/// ```
pub struct Funnel<D> {
    _funnel_fn: JoinHandle<()>,
    source_sink: Sender<Receiver<D>>,
    //receivers: Arc<RwLock<Vec<Receiver<D>>>>
}

impl<D> Funnel<D> 
where D: Send + 'static {
    /// Creates a new `Funnel` object
    /// 
    /// # Parameters
    /// - `pause_sig` - A flag to signal the `Funnel` object to pause execution.
    /// - `stop_sig` - A flag to signal the `Funnel` object to terminate execution.
    /// 
    /// # Returns
    ///  - A `Funnel` object.
    ///  - A `Receiver` channel for `Funnel` output.
    pub fn new( pause_sig: Option<Arc<AtomicBool>>,
                stop_sig: Arc<AtomicBool>,
                data_limit: usize) -> (Self, Receiver<D>) {
        let (recv_sink, recv_source) = util::get_channel::<Receiver<D>>(data_limit);
        
        let thr_sink = recv_sink.clone();
        
        
        let receivers:Arc<RwLock<Vec<Receiver<D>>>> = Arc::new(RwLock::new(Vec::new()));
        let (tx, rx) = util::get_channel(data_limit);
        
        //let worker_receivers = receivers.clone();
        let funnel_worker = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Some(sig) = pause_sig.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(50));
                        continue
                    }
                }
                
                if let Ok(receiver) = recv_source.recv_timeout(Duration::from_millis(10)) {
                    if let Ok(data) = receiver.recv_timeout(Duration::from_millis(10)) {
                        tx.send(data).unwrap()
                    }
                    thr_sink.send(receiver).unwrap();
                }
                
                // for receiver in worker_receivers.read().unwrap().iter() {
                //     if let Ok(data) = receiver.recv_timeout(Duration::from_millis(10)) {
                //         tx.send(data).unwrap()
                //     }
                // }
            }
        });
        (
            Self {
                _funnel_fn: funnel_worker,
                source_sink: recv_sink
            }, rx
        )
    }

    /// Add a data source to the `Funnel`
    ///
    /// # Parameters
    ///  - `source` - A `Receiver` channel from which data are received
    pub fn add_source(&mut self, source: Receiver<D>) {
        self.source_sink.send(source).unwrap();
    }

    /// Waits for the `Funnel` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self._funnel_fn.join()
    }
}


pub struct Accumulator<D> {
    _funnel_fn: JoinHandle<()>,
    __phantom_data: PhantomData<D>
}

impl<D> Accumulator<D>
where D: Send + 'static {
    pub fn new( max_wait: u64,
                pause_sig: Option<Arc<AtomicBool>>,
                stop_sig: Arc<AtomicBool>,
                source: Receiver<D>,
                data_limit: usize) -> (Self, Receiver<Vec<D>>) {
        let (a_tx, a_rx) = util::get_channel(data_limit);

        let accumulator = thread::spawn(move || {
            let mut accumulate_data = Cell::new(Vec::new());

            while !stop_sig.load(Ordering::Relaxed) {
                if let Some(ref pause) = pause_sig {
                    if pause.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(50));
                        continue
                    }
                }

                let now = Instant::now();
                let future = now + Duration::from_millis(max_wait);

                while Instant::now() < future {
                    if let Ok(data) = source.recv_timeout(Duration::from_millis(50)) {
                        accumulate_data.get_mut().push(data);
                    }
                }

                if !accumulate_data.get_mut().is_empty() {
                    a_tx.send(accumulate_data.take()).unwrap();
                }
            }
        });
        (
            Self {
                _funnel_fn: accumulator,
                __phantom_data: Default::default(),
            },
            a_rx
        )
    }

    pub fn join(self) -> thread::Result<()> {
        self._funnel_fn.join()
    }
}


#[derive(Copy, Clone, Default)]
pub struct TransformerContext<D, G> {
    pub globals: G,
    pub data: Option<D>
}

#[derive(Clone, Default)]
pub struct FnContext<I, D> {
    pub init_data: I,
    pub data: Arc<Mutex<Cell<Vec<D>>>>,
    pub stop_sig: Arc<AtomicBool>,
    pub pause_sig: Option<Arc<AtomicBool>>,
}


/// An object that enables data mutation. After processing data a `Transformer` can yield three variants:
///  - `TransformerResult::Transformed` - The data has been processed and can move along the rest of the `Reflux` network.
///  - `TransformerResult::NeedsMoreWork` - The data still needs additional processing. The data is sent back into the `Transformer`.
///  - `TransformerResult::Error` - There was an error in processing the data. This error is simply logged to `stderr`.
///
/// Using a `Transformer` yields the following benefits:
///  - Mutation of data in a `Reflux` network.
///
/// # Example
/// ```
///  #![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
///  #![feature(unboxed_closures)]
///  use reflux::{Transformer, TransformerContext, TransformerResult, TransformerEffectType};
///  use std::sync::{Arc, Mutex};
///  use std::cell::Cell;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::{Receiver, Sender};
///  use reflux::{add_routine, terminate, effect_debug};
///  use crossbeam_channel::unbounded;
///  use std::time::Duration;
///  use std::thread::sleep;
///
///  let flag = Arc::new(AtomicBool::new(false));
///     let array = vec![1, 3, 5, 7, 9];
/// 
///     let routine = add_routine!(#[coroutine] |input: Arc<Mutex<Cell<TransformerContext<Vec<i32>, i32>>>>| {
///         let context = input.lock().unwrap().take();
/// 
///         let multiplier = context.globals;
///         let data = context.data.unwrap().clone();
/// 
///         let mut result = Vec::new();
///         for item in data.into_iter() {
///             result.push(item * multiplier)
///         }
/// 
///         #[cfg(debug_assertions)]
///         effect_debug!(vec!["Function completed".to_string()]);
/// 
///         terminate!(Some(result));
///     });
/// 
///     let (transformer,
///         input_sink,
///         output_source,
///         effect) = Transformer::new(routine,  None, flag.clone(), 2, 100);
/// 
/// 
///     input_sink.send(array).unwrap();
///     #[cfg(debug_assertions)]
///     (|| {
///         match effect.recv().unwrap() {
///             TransformerEffectType::Debug(val) => {
///                 assert_eq!(val, vec!["Function completed".to_string()])
///             },
///             _ => {panic!("Invalid variant");}
///         }
/// 
///     })();
/// 
///     let result: Vec<i32> = output_source.recv().unwrap();
///     assert_eq!(result, vec![2, 6, 10, 14, 18]);
///     flag.store(true, Ordering::Relaxed);
///     transformer.join().unwrap()
/// ```
pub struct Transformer<I, O, E> {
    _run_thr: JoinHandle<()>,
    _i: PhantomData<I>,
    _o: PhantomData<O>,
    _e: PhantomData<TransformerEffectType<E>>,
}

impl<I, O, E> Transformer<I, O, E> {
    /// Creates a new `Transformer` object.
    ///
    /// # Parameters
    /// - `transform_fn` - A coroutine that will transform data.
    ///     The use of the `add_routine!` macro is necessary when passing in a coroutine.
    /// - `pause_sig` - A flag to signal the `Transformer` object to pause execution.
    /// - `stop_sig` - A flag to signal the `Transformer` object to terminate execution.
    /// - `context` - An object of immutable values for the `transformer_fn` to use during computation.
    ///
    /// # Returns
    /// A `Transformer` object.
    pub fn new<Ctx, F, C>(transform_fn: F,
                          pause_sig: Option<Arc<AtomicBool>>,
                          stop_sig: Arc<AtomicBool>,
                          context: Ctx,
                          data_limit: usize) -> (Self, Sender<Vec<I>>, Receiver<Vec<O>>, Receiver<TransformerEffectType<E>>)
    where
        F: Fn() -> C + Send + 'static,
        C: Coroutine<Arc<Mutex<Cell<TransformerContext<Vec<I>, Ctx>>>>> + Send + 'static + Unpin,
        Ctx: Send + 'static + Clone,
        I: Send + 'static,
        O: Send + 'static,
        E: Send + 'static,
        TransformerResult<I, O, E>: Send + 'static + From<<C as Coroutine<Arc<Mutex<Cell<TransformerContext<Vec<I>, Ctx>>>>>>::Yield>,
    {
        let (in_tx, in_rx) = util::get_channel(data_limit);
        let (out_tx, out_rx) = util::get_channel(data_limit);
        let (err_tx, err_rx) = util::get_channel(data_limit);

        let tx2 = in_tx.clone();
        let new_ctx = context;
        let run_thr = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Some(sig) = pause_sig.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(50));
                        continue
                    }
                }
                let mut routine = transform_fn();
                let mut is_running = false;
                let mut coro_completed = false;
                loop {
                    let in_data = match in_rx.recv_timeout(Duration::from_millis(10)) {
                        Ok(val) => Some(val),
                        Err(_) => {
                            if !is_running {
                                break
                            } else {
                                None
                            }
                        }
                    };

                    if coro_completed {
                        routine = transform_fn();
                        coro_completed = false;
                    }

                    let coro_context = Arc::new(Mutex::new(Cell::new(TransformerContext {
                        globals: new_ctx.clone(),
                        data: in_data
                    })));

                    loop {
                        let loop_context = coro_context.clone();
                        match Pin::new(&mut routine).resume(loop_context) {
                            CoroutineState::Yielded(res) => {
                                is_running = true;
                                let r: TransformerResult<I, O, E> = res.into();
                                match r {
                                    TransformerResult::Output(val) => {
                                        out_tx.send(val).unwrap();
                                    }
                                    TransformerResult::Terminate(val) => {
                                        is_running = false;
                                        coro_completed = true;
                                        if let Some(data) = val {
                                            out_tx.send(data).unwrap();
                                        }
                                    }
                                    TransformerResult::Consume(val) => {
                                        tx2.send(val).unwrap()
                                    }
                                    TransformerResult::Effect(effect) => {
                                        match effect {
                                            TransformerEffectType::Critical(err) => {
                                                coro_completed = true;
                                                is_running = false;
                                                err_tx.send(TransformerEffectType::Critical(err)).unwrap()
                                            },
                                            TransformerEffectType::Emergency(err) => {
                                                coro_completed = true;
                                                is_running = false;
                                                err_tx.send(TransformerEffectType::Emergency(err)).unwrap()
                                            },
                                            other => {
                                                err_tx.send(other).unwrap();
                                            }
                                        }

                                    }
                                }
                                break
                            }
                            _ => {
                                if is_running {
                                    routine = transform_fn()
                                }
                            }
                        }
                    }
                }
            }
        });
        (
            Self {
                _run_thr: run_thr,
                _i: Default::default(),
                _o: Default::default(),
                _e: Default::default(),
            },
            in_tx,
            out_rx,
            err_rx,
        )
    }

    /// Waits for the `Transformer` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self._run_thr.join()
    }
}


/// An object that uses a predicate function to determine whether some data can be passed through a network node
///
/// Using a `Filter` yields the following benefits:
/// - Conditionally allow data to flow through a `Reflux` network
///
/// # Example
/// ```
///  #![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
///  #![feature(unboxed_closures)]
/// use reflux::Filter;
/// use std::sync::Arc;
/// use std::sync::atomic::{AtomicBool, Ordering};
/// use crossbeam_channel::{bounded, Receiver};
/// use reflux::add_routine;
/// use crossbeam_channel::unbounded;
/// use std::time::Duration;
/// use std::thread::sleep;
/// let stop_flag = Arc::new(AtomicBool::new(false));
/// let fun = |data: &String| -> bool {
///     data.contains("hello")
/// };
///     
/// let input_data = vec![
///     "hello world".to_string(),
///     "goodbye world".to_string(),
///     "hello there".to_string()
/// ];
///  
/// let expected_data = vec![
///     "hello world".to_string(),
///     "hello there".to_string()
/// ];
///  
/// let (tx, rx) = bounded(0);
///
/// let (filter, filter_sink) = Filter::new(fun, rx, None, stop_flag.clone(), 0);
///
/// tx.send(input_data).unwrap();
/// let data = filter_sink.recv().unwrap();
///
/// assert_eq!(data, expected_data);
///
/// stop_flag.store(true, Ordering::Relaxed);
/// filter.join().unwrap()
/// ```
pub struct Filter {
    _filter_thr: JoinHandle<()>,
}

impl Filter {
    /// Creates a new `Filter` object.
    ///
    /// # Parameters
    /// - `filter` - A function that takes a reference, determines if the data meets some condition and returns a boolean.
    /// - `source` - A `Receiver` channel object from which to receive data.
    /// - `pause_sig` - A flag to signal the `Filter` object to pause execution.
    /// - `stop_sig` - A flag to signal the `Filter` object to terminate execution.
    ///
    /// # Returns
    ///  - A `Filter` object.
    ///  - A `Receiver` channel.
    pub fn new<T, F>(filter_fn: F,
                     source: Receiver<Vec<T>>,
                     pause_sig: Option<Arc<AtomicBool>>,
                     stop_sig: Arc<AtomicBool>,
                     data_limit: usize) -> (Self, Receiver<Vec<T>>)
    where
        T: Send + 'static,
        F: Fn(&T) -> bool + Send + 'static {
        let (sender, receiver) = util::get_channel(data_limit);
        let filter = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Some(sig) = pause_sig.as_ref() {
                    if sig.load(Ordering::Relaxed) {
                        sleep(Duration::from_millis(50));
                        continue
                    }
                }
                let mut result = Vec::new();
                if let Ok(data) = source.recv_timeout(Duration::from_millis(10)) {
                    for item in data {
                        if filter_fn(&item) {
                            result.push(item);
                        }
                    }
                    sender.send(result).unwrap()
                }
            }
        });
        (Self {
            _filter_thr: filter
        }, receiver)
    }

    /// Waits for the `Filter` object to finish execution.
    pub fn join(self) -> thread::Result<()> {
        self._filter_thr.join()
    }
}


/// A simple macro to create a function that returns a coroutine.
#[macro_export]
macro_rules! add_routine {
    ($a: expr) => {
        move || {
            return $a
        }
    };
}


#[macro_export]
macro_rules! terminate {
    ($a: expr) => {
        yield TransformerResult::Terminate($a);
        return;
    };
}

#[macro_export]
macro_rules! output {
    ($a: expr) => {
        yield TransformerResult::Output($a);
    };
}

#[macro_export]
macro_rules! consume {
    ($a: expr) => {
        yield TransformerResult::Consume($a);
    };
}

#[macro_export]
macro_rules! effect_emerg {
    ($a: expr) => {
        yield TransformerResult::Effect(TransformerEffectType::Emergency($a));
        return;
    };
}

#[macro_export]
macro_rules! effect_alert {
    ($a: expr) => {
        yield TransformerResult::Effect(TransformerEffectType::Alert($a));
    };
}

#[macro_export]
macro_rules! effect_crit {
    ($a: expr) => {
        yield TransformerResult::Effect(TransformerEffectType::Critical($a));
        return;
    };
}

#[macro_export]
macro_rules! effect_error {
    ($a: expr) => {
        yield TransformerResult::Effect(TransformerEffectType::Error($a));
    };
}

#[macro_export]
macro_rules! effect_warn {
    ($a: expr) => {
        yield TransformerResult::Effect(TransformerEffectType::Warning($a));
    };
}

#[macro_export]
macro_rules! effect_notice {
    ($a: expr) => {
        yield TransformerResult::Effect(TransformerEffectType::Notice($a));
    };
}

#[macro_export]
macro_rules! effect_info {
    ($a: expr) => {
        yield TransformerResult::Effect(TransformerEffectType::Info($a));
    };
}

#[macro_export]
macro_rules! effect_debug {
    ($a: expr) => {
        #[cfg(debug_assertions)]
        yield TransformerResult::Effect(TransformerEffectType::Debug($a));
    };
}
