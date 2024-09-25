#![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
#![feature(unboxed_closures)]

mod util;

use std::fmt::Display;
use std::marker::PhantomData;
use std::ops::{Coroutine, CoroutineState};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use crossbeam_channel::{Receiver, Sender, unbounded};


/// An enum returned from a coroutine sent to a `Transformer` object.
/// 
/// The following variants work as follows:
/// - `Transformed` - Data has been mutated and can be sent along a `Reflux` network
/// - `NeedsMoreWork` - Data has been processed, but is not ready to be sent through the network.
/// - `Error` - An error occurred when processing data
pub enum TransformerResult<O, T, E> {
    Transformed(T),
    NeedsMoreWork(O),
    Error(E),
}

/// Receives data from an external source and sends the data through a channel.
///
/// Using an `Inlet` yields the following benefits:
/// - Abstracts away the use of channels. You only need to write a coroutine that takes a parameter
/// and yields a result.
/// - The coroutine does not have to be aware of termination signals, or joining threads. This
/// functionality is handled by `Inlet`.
/// - Easy integration with other `Reflux` modules.
///
/// # Example
/// ```
///  #![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
///  #![feature(unboxed_closures)]
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::Receiver;
///  use reflux::add_routine;
///  use reflux::Inlet;
///  let stop_flag = Arc::new(AtomicBool::new(false));
///  let (inlet, inlet_chan): (Inlet, Receiver<String>) = Inlet::new(
///      add_routine!(#[coroutine] |_: ()| {
///          yield "Hello, world".to_string()
///      }), stop_flag.clone(), ());
///
///  let data = inlet_chan.recv().unwrap();
///  stop_flag.store(true, Ordering::Relaxed);
///  inlet.join().unwrap();
///
///  assert_eq!(data, "Hello, world".to_string())
/// ```
pub struct Inlet {
    inlet_fn: JoinHandle<()>,
}

impl Inlet {
    /// Creates a new `Inlet` object with an unbounded internal channel.
    /// 
    /// # Parameters
    /// - `inlet_fn` - A coroutine that reads and returns data from an external source.
    /// The use of the `add_routine!` macro is necessary when passing in an `inlet_fn`.
    /// - `stop_sig` - A flag to signal the `Inlet` object to terminate
    /// 
    /// # Returns
    /// A `Inlet` object and a `Receiver` to receive data from the `inlet_fn`
    pub fn new<F, C, T, I>(inlet_fn: F, stop_sig: Arc<AtomicBool>, init_data: I) -> (Self, Receiver<T>)
        where 
            F: Fn() -> C + Send + 'static,
            I: Send + 'static + Clone,
            C: Coroutine<I> + Send + 'static + Unpin,
            T: Send + 'static + From<<C as Coroutine<I>>::Yield> {
        Self::new_bounded(inlet_fn, stop_sig, init_data, None)
    }

    /// Creates a new `Inlet` object with a bounded internal channel.
    ///
    /// # Parameters
    /// - `inlet_fn` - A coroutine that reads and returns data from an external source.
    /// The use of the `add_routine!` macro is necessary when passing in an `inlet_fn`.
    /// - `stop_sig` - A flag to signal the `Inlet` object to terminate
    /// - `data_limit` - An optional parameter to limit channel capacity.
    ///
    /// # Returns
    /// A `Inlet` object and a `Receiver` to receive data from the `inlet_fn`
    pub fn new_bounded<F, C, T, I>(inlet_fn: F,
                                   stop_sig: Arc<AtomicBool>,
                                   init_data: I,
                                   data_limit: Option<usize>) -> (Self, Receiver<T>)
        where
            F: Fn() -> C + Send + 'static,
            I: Send + 'static + Clone,
            C: Coroutine<I> + Send + 'static + Unpin,
            T: Send + 'static + From<<C as Coroutine<I>>::Yield> {
        let (tx, rx) = util::get_channel(data_limit);
        let inlet_thr = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                let mut routine = inlet_fn();
                loop {
                    match Pin::new(&mut routine).resume(init_data.clone()) {
                        CoroutineState::Yielded(res) => {
                            let r: T = res.into();
                            let _= tx.send(r);
                        }
                        _ => break
                    }
                }
            }
        });
        let s = Self {
            inlet_fn: inlet_thr,
        };

        (s, rx)
    }
    
    /// Waits for the `Inlet` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self.inlet_fn.join()?;
        Ok(())
    }
}

/// An object that receives data from a `Reflux` network and sends the data to some external sink.
/// 
/// Using an `Outlet` yields the following benefits:
/// - Abstracts away the use of channels. You only need to receive a parameter and send it to an
/// external sink
/// - The function does not have to be aware of termination signals, or joining threads. This
/// functionality is handled by `Outlet`.
/// - Easy integration with other `Reflux` modules.
/// 
/// # Example
/// ```
///  use reflux::Outlet;
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::Receiver;
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
///  use std::thread::sleep;
/// 
///  let stop_flag = Arc::new(AtomicBool::new(false));
///  let (test_tx, test_rx) = unbounded();
///  let (outlet, out_send)= Outlet::new(move |test: String| {
///      test_tx.send(test).unwrap();
///  }, stop_flag.clone());
/// 
///  out_send.send("Hello, world".to_string()).unwrap();
/// 
///  let data_recv = test_rx.recv().unwrap();
///  stop_flag.store(true, Ordering::Relaxed);
///  outlet.join().unwrap();
/// 
///  assert_eq!(data_recv, "Hello, world".to_string())
/// ```
pub struct Outlet {
    outlet_fn: JoinHandle<()>
}

impl Outlet {
    /// Creates a new `Outlet` object with an unbounded internal channel.
    ///
    /// # Parameters
    /// - `outlet_fn` - A function that receives data from a `Reflux` network and sends it to an
    /// external sink.
    /// - `receiver` - A `Receiver` channel object from which to receive data.
    /// - `stop_sig` - A flag to signal the `Inlet` object to terminate
    ///
    /// # Returns
    /// - An `Outlet` object
    /// - A `Sender` to send data out to.
    pub fn new<T, F>(outlet_fn: F, stop_sig: Arc<AtomicBool>) -> (Self, Sender<T>)
        where
            T: Send + 'static,
            F: FnMut(T) + Send + 'static {
        Self::new_bounded(outlet_fn, stop_sig, None)
    }
    
    /// Waits for the `Outlet` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self.outlet_fn.join()?;
        Ok(())
    }

    /// Creates a new `Outlet` object, with a bounded internal channel.
    ///
    /// # Parameters
    /// - `outlet_fn` - A function that receives data from a `Reflux` network and sends it to an
    /// external sink.
    /// - `receiver` - A `Receiver` channel object from which to receive data.
    /// - `stop_sig` - A flag to signal the `Inlet` object to terminate
    ///
    /// # Returns
    /// - An `Outlet` object
    /// - A `Sender` to send data out to.
    pub fn new_bounded<T, F>(mut outlet_fn: F, stop_sig: Arc<AtomicBool>, data_limit: Option<usize>) -> (Self, Sender<T>)
    where
        T: Send + 'static,
        F: FnMut(T) + Send + 'static {
        let (sender, receiver) = util::get_channel(data_limit);
        let outlet = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Ok(data) = receiver.recv_timeout(Duration::from_millis(10)) {
                    outlet_fn(data)
                }
            }
        });
        (Self {
            outlet_fn: outlet
        }, sender)
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
///  use reflux::{Inlet, Outlet, Broadcast};
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::Receiver;
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
///  use std::time::Duration;
///  use std::thread::sleep;
///  let stop_flag = Arc::new(AtomicBool::new(false));
/// 
///  let test_inlet: (Inlet, Receiver<String>) = Inlet::new(add_routine!(#[coroutine] || {
///             sleep(Duration::from_secs(1));
///             yield "hello".to_string()
///         }), stop_flag.clone(), ());
/// 
///  let (test_outlet1_sink, test_outlet1_source) = unbounded();
///  let (test_outlet2_sink, test_outlet2_source) = unbounded();
/// 
///  let (test_outlet1, test1_tx) = Outlet::new(move |example: String| {
///     test_outlet1_sink.send(format!("1: {example}")).unwrap()
///  }, stop_flag.clone());
/// 
///  let (test_outlet2, test2_tx) = Outlet::new(move |example: String| {
///      test_outlet2_sink.send(format!("2: {example}")).unwrap()
///  }, stop_flag.clone());
/// 
///  let mut broadcaster = Broadcast::new(test_inlet.1, stop_flag.clone());
///  broadcaster.subscribe(test1_tx);
///  broadcaster.subscribe(test2_tx);
/// 
///  let data1 = test_outlet1_source.recv().unwrap();
///  let data2 = test_outlet2_source.recv().unwrap();
/// 
///  stop_flag.store(true, Ordering::Relaxed);
/// 
///  test_outlet1.join().unwrap();
///  test_outlet2.join().unwrap();
///  test_inlet.0.join().unwrap();
///  broadcaster.join().unwrap();
/// 
/// 
///  assert_eq!(data1, "1: hello".to_string());
///  assert_eq!(data2, "2: hello".to_string());
/// ```
pub struct Broadcast<T> {
    subscribers: Arc<Mutex<Vec<Sender<T>>>>,
    _broadcaster: JoinHandle<()>,
}

impl<T> Broadcast<T> where T: Clone + Send + 'static {
    /// Creates a new `Broadcast` object.
    ///
    /// # Parameters
    /// - `source` - The source of data that needs to be broadcast 
    /// - `stop_sig` - A flag to signal the `Broadcast` object to terminate
    ///
    /// # Returns
    /// A `Broadcast` object
    pub fn new(source: Receiver<T>, stop_sig: Arc<AtomicBool>) -> Self {
        let subscribers = Arc::new(Mutex::new(Vec::<Sender<T>>::new()));
        
        let thr_subscribers = subscribers.clone();
        let broadcaster = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Ok(data) = source.recv_timeout(Duration::from_millis(10)) {
                    let subscribers_lock = thr_subscribers.lock().unwrap();
                    for subscriber in subscribers_lock.iter() {
                        let _= subscriber.send(data.clone());
                    };           
                }
            }
        });
        Self {
            subscribers,
            _broadcaster: broadcaster
        }
    }

    /// Add a subscriber to the `Broadcast`
    ///
    /// # Parameters
    ///  - `subscriber` - A `Sender` with which to send data to.
    pub fn subscribe(&mut self, subscriber: Sender<T>) {
        let mut subscribers_lock = self.subscribers.lock().unwrap();
        subscribers_lock.push(subscriber)
    }
    
    /// Create a subscription for an external object to use to receive data from the `Broadcast`.
    /// Note: Subscription is based on an unbounded channel.
    /// 
    /// # Returns
    /// - A `Receiver` object from which to receive data.
    pub fn channel(&mut self) -> Receiver<T> {
        Self::channel_bounded(self, None)
    }

    /// Create a subscription for an external object to use to receive data from the `Broadcast`.
    /// Note: Subscription is based on a bounded channel.
    ///
    /// # Returns
    /// - A `Receiver` object from which to receive data.
    pub fn channel_bounded(&mut self, data_limit: Option<usize>) -> Receiver<T> {
        let (tx, rx) = util::get_channel(data_limit);
        let mut subscribers_lock = self.subscribers.lock().unwrap();
        subscribers_lock.push(tx);
        rx
    }

    /// Waits for the `Broadcast` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self._broadcaster.join()?;
        Ok(())
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
///  let mut router= Router::new(rx, stop_flag.clone());
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
    /// - `source` - The source of data that needs to be routed 
    /// - `stop_sig` - A flag to signal the `Router` object to terminate
    ///
    /// # Returns
    /// A `Router` object
    pub fn new(source: Receiver<T>, stop_sig: Arc<AtomicBool>) -> Self {
        let subscribers = Arc::new(Mutex::new(Vec::<Sender<T>>::new()));

        let thr_subscribers = subscribers.clone();
        let dispatcher = thread::spawn(move || {
            let mut pointer = 0;
            while !stop_sig.load(Ordering::Relaxed) {
                if let Ok(data) = source.recv_timeout(Duration::from_millis(10)) {
                    let subscribers_lock = thr_subscribers.lock().unwrap();
                    let _= subscribers_lock.get(pointer).unwrap().send(data);
                    pointer = (pointer + 1) % subscribers_lock.len();
                }
            }
        });
        Self {
            subscribers,
            _dispatcher: dispatcher
        }
    }

    /// Add a subscriber to the `Router`
    /// 
    /// # Parameters
    ///  - `subscriber` - A `Sender` with which to send data to.
    pub fn subscribe(&mut self, subscriber: Sender<T>) {
        let mut subscribers_lock = self.subscribers.lock().unwrap();
        subscribers_lock.push(subscriber)
    }

    /// Waits for the `Router` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self._dispatcher.join()?;
        Ok(())
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
///  let (mut funnel, funnel_out) = Funnel::new(stop_flag.clone());
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
    receivers: Arc<Mutex<Vec<Receiver<D>>>>
}

impl<D> Funnel<D> 
where D: Send + 'static {
    /// Creates a new `Funnel` object with an unbounded internal channel.
    /// 
    /// # Parameters
    ///  - `stop_flag`: A flag to signal the termination of the `Funnel` instance.
    /// 
    /// # Returns
    ///  - A `Funnel` instance
    ///  - A `Receiver` channel for `Funnel` output
    pub fn new(stop_flag: Arc<AtomicBool>) -> (Self, Receiver<D>) {
        Self::new_bounded(stop_flag, None)
    }

    /// Creates a new `Funnel` object with a bounded internal channel.
    ///
    /// # Parameters
    ///  - `stop_flag`: A flag to signal the termination of the `Funnel` instance.
    ///  - `data_limit`: An optional size to limit channel capacity.
    ///
    /// # Returns
    ///  - A `Funnel` instance
    ///  - A `Receiver` channel for `Funnel` output
    pub fn new_bounded(stop_flag: Arc<AtomicBool>, data_limit: Option<usize>) -> (Self, Receiver<D>) {
        let receivers:Arc<Mutex<Vec<Receiver<D>>>> = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = util::get_channel(data_limit);

        let worker_receivers = receivers.clone();
        let funnel_worker = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                for receiver in worker_receivers.lock().unwrap().iter() {
                    if let Ok(data) = receiver.recv_timeout(Duration::from_millis(10)) {
                        let _= tx.send(data);
                    }
                }
            }
        });
        (
            Self {
                _funnel_fn: funnel_worker,
                receivers
            }, rx
        )
    }

    /// Add a data source to the `Funnel`
    ///
    /// # Parameters
    ///  - `source` - A `Receiver` from which data are received
    pub fn add_source(&mut self, source: Receiver<D>) {
        self.receivers.lock().unwrap().push(source)
    }

    /// Waits for the `Funnel` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self._funnel_fn.join()?;
        Ok(())
    }
}


pub struct TransformerContext<D, G> {
    pub globals: G,
    pub data: Option<D>
}


/// An object that enables data mutation. After processing data a `Transformer` can yield three variants:
///  - `TransformerResult::Transformed` - The data has been processed and can move along the rest of the `Reflux` network
///  - `TransformerResult::NeedsMoreWork` - The data still needs additional processing. The data is sent back into the `Transformer`
///  - `TransformerResult::Error` - There was an error in processing the data. This error is simply logged to `stderr`
///
/// Using a `Transformer` yields the following benefits:
///  - Mutation of data in a `Reflux` network.
///
/// # Example
/// ```
///  #![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
///  #![feature(unboxed_closures)]
///  use reflux::{Transformer, TransformerContext, TransformerResult};
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::{Receiver, Sender};
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
///  use std::time::Duration;
///  use std::thread::sleep;
/// 
///  #[derive(Clone)]
///  struct InnerContext {
///     inc_val: i32
///  }
///
///  let ctx = InnerContext {
///     inc_val: 1
///  };
///
///  let stop_flag = Arc::new(AtomicBool::new(false));
///
///  let (transformer, input, output): (Transformer<i32, String, String>, Sender<i32>, Receiver<String>) = Transformer::new(
///     add_routine!(#[coroutine] |input: TransformerContext<i32, InnerContext>| {
///         let mut data = input.data.unwrap();
///         while data < 5 {
///             data += input.globals.inc_val;
///             yield TransformerResult::NeedsMoreWork(data);
///         }
///     yield TransformerResult::Transformed(format!("The number is {data}"));
///  }), stop_flag.clone(), ctx);
///
///  input.send(0).unwrap();
///
///  let result = output.recv().unwrap();
///  stop_flag.store(true, Ordering::Relaxed);
///  transformer.join().unwrap();
///
///  assert_eq!(result, "The number is 5".to_string())
/// ```
pub struct Transformer<I, O, E> {
    _run_thr: JoinHandle<()>,
    _i: PhantomData<I>,
    _o: PhantomData<O>,
    _e: PhantomData<E>,
}

impl<I, O, E> Transformer<I, O, E> {
    /// Creates a new `Transformer` object with an unbounded internal channel.
    ///
    /// # Parameters
    /// - `transform_fn` - A coroutine that will transform data.
    ///     The use of the `add_routine!` macro is necessary when passing in a coroutine.
    /// - `stop_sig` - A flag to signal the `Router` object to terminate
    /// - `context` - An object of immutable values for the `transformer_fn` to use during computation
    ///
    /// # Returns
    /// A `Transformer` object
    pub fn new<Ctx, F, C>(transform_fn: F, stop_sig: Arc<AtomicBool>, context: Ctx) -> (Self, Sender<I>, Receiver<O>)
    where
        F: Fn() -> C + Send + 'static,
        C: Coroutine<TransformerContext<I, Ctx>> + Send + 'static + Unpin,
        Ctx: Send + 'static + Clone,
        I: Send + 'static,
        O: Send + 'static,
        E: Send + 'static + Display,
        TransformerResult<I, O, E>: Send + 'static + From<<C as Coroutine<TransformerContext<I, Ctx>>>::Yield>,
    {
        Self::new_bounded(transform_fn, stop_sig, context, None)
    }

    /// Creates a new `Transformer` object with a bounded internal channel.
    ///
    /// # Parameters
    /// - `transform_fn` - A coroutine that will transform data.
    ///     The use of the `add_routine!` macro is necessary when passing in a coroutine.
    /// - `stop_sig` - A flag to signal the `Router` object to terminate
    /// - `context` - An object of immutable values for the `transformer_fn` to use during computation
    ///
    /// # Returns
    /// A `Transformer` object
    pub fn new_bounded<Ctx, F, C>(transform_fn: F,
                                    stop_sig: Arc<AtomicBool>,
                                    context: Ctx,
                                    data_limit: Option<usize>) -> (Self, Sender<I>, Receiver<O>)
    where
        F: Fn() -> C + Send + 'static,
        C: Coroutine<TransformerContext<I, Ctx>> + Send + 'static + Unpin,
        Ctx: Send + 'static + Clone,
        I: Send + 'static,
        O: Send + 'static,
        E: Send + 'static + Display,
        TransformerResult<I, O, E>: Send + 'static + From<<C as Coroutine<TransformerContext<I, Ctx>>>::Yield>,
    {
        let (in_tx, in_rx) = util::get_channel(data_limit);
        let (out_tx, out_rx) = util::get_channel(data_limit);

        let tx2 = in_tx.clone();
        let new_ctx = context;
        let run_thr = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                let mut routine = transform_fn();
                loop {
                    let in_data = match in_rx.recv_timeout(Duration::from_millis(10)) {
                        Ok(val) => val,
                        Err(_) => break
                    };
                    let coro_context = TransformerContext {
                        globals: new_ctx.clone(),
                        data: Some(in_data)
                    };

                    match Pin::new(&mut routine).resume(coro_context) {
                        CoroutineState::Yielded(res) => {
                            let r: TransformerResult<I, O, E> = res.into();
                            match r {
                                TransformerResult::Transformed(val) => {
                                    let _= out_tx.send(val);
                                    break
                                }
                                TransformerResult::NeedsMoreWork(val) => {
                                    let _= tx2.send(val);
                                }
                                TransformerResult::Error(val) => {
                                    eprintln!("{val}");
                                    break
                                }
                            }
                        }
                        _ => break
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
            out_rx
        )
    }
    
    

    /// Waits for the `Transformer` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self._run_thr.join()?;
        Ok(())
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
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::Receiver;
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
/// use std::time::Duration;
/// use std::thread::sleep;
/// let stop_flag = Arc::new(AtomicBool::new(false));
/// let fun = |data: &String| -> bool {
///     data.contains("hello")
///  };
/// 
///  let stop_flag = Arc::new(AtomicBool::new(false));
///  let (tx, rx) = unbounded();
/// 
///  let (filter, filter_sink) = Filter::new(fun, rx, stop_flag.clone());
/// 
///  tx.send("hello world".to_string()).unwrap();
///  let data = filter_sink.recv().unwrap();
/// 
///  assert_eq!(data, "hello world");
/// 
///  tx.send("goodbye world".to_string()).unwrap();
///  let res = filter_sink.recv_timeout(Duration::from_secs(1));
///  assert!(res.is_err());
/// 
///  tx.send("hello there".to_string()).unwrap();
///  let data = filter_sink.recv().unwrap();
/// 
///  assert_eq!(data, "hello there");
///         
///  stop_flag.store(true, Ordering::Relaxed);
///  filter.join().unwrap()
/// ```
pub struct Filter {
    _filter_thr: JoinHandle<()>,
}

impl Filter {
    /// Creates a new `Filter` object with an unbounded internal channel.
    ///
    /// # Parameters
    /// - `filter` - A function that takes a reference, determines if the data meets some condition and returns a boolean.
    /// - `source` - A `Receiver` channel object from which to receive data.
    /// - `stop_sig` - A flag to signal the `Inlet` object to terminate
    ///
    /// # Returns
    ///  - A `Filter` object
    ///  - A `Receiver`
    pub fn new<T, F>(filter_fn: F, source: Receiver<T>, stop_sig: Arc<AtomicBool>) -> (Self, Receiver<T>)
    where
        T: Send + 'static,
        F: Fn(&T) -> bool + Send + 'static {
        Self::new_bounded(filter_fn, source, stop_sig, None)
    }

    /// Creates a new `Filter` object with a bounded internal channel.
    ///
    /// # Parameters
    /// - `filter` - A function that takes a reference, determines if the data meets some condition and returns a boolean.
    /// - `source` - A `Receiver` channel object from which to receive data.
    /// - `stop_sig` - A flag to signal the `Inlet` object to terminate
    ///
    /// # Returns
    ///  - A `Filter` object
    ///  - A `Receiver`
    pub fn new_bounded<T, F>(filter_fn: F,
                               source: Receiver<T>, 
                               stop_sig: Arc<AtomicBool>,
                               data_limit: Option<usize>) -> (Self, Receiver<T>)
    where
        T: Send + 'static,
        F: Fn(&T) -> bool + Send + 'static {
        let (sender, receiver) = util::get_channel(data_limit);
        let filter = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                if let Ok(data) = source.recv_timeout(Duration::from_millis(10)) {
                    if filter_fn(&data) {
                        let _= sender.send(data);
                    }
                }
            }
        });
        (Self {
            _filter_thr: filter
        }, receiver)
    }

    /// Waits for the `Filter` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self._filter_thr.join()?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use std::thread::sleep;
    use std::time::Duration;
    use super::*;
    
    #[test]
    fn inlet_works() {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (inlet, inlet_chan): (Inlet, Receiver<String>) = Inlet::new(
            add_routine!(#[coroutine] |_: ()| {
                yield "Hello, world".to_string()
            }), stop_flag.clone(), ());
        
        
        let data = inlet_chan.recv().unwrap();
        stop_flag.store(true, Ordering::Relaxed);
        inlet.join().unwrap();
        
        assert_eq!(data, "Hello, world".to_string())
    }

    #[test]
    fn outlet_works() {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (test_tx, test_rx) = unbounded();
        let (outlet, data_tx) = Outlet::new(move |test: String| {
            test_tx.send(test).unwrap();
        }, stop_flag.clone());

        data_tx.send("Hello, world".to_string()).unwrap();

        let data_recv = test_rx.recv().unwrap();
        stop_flag.store(true, Ordering::Relaxed);
        outlet.join().unwrap();

        assert_eq!(data_recv, "Hello, world".to_string())
    }
    
    #[test]
    fn broadcast_works() {
        let stop_flag = Arc::new(AtomicBool::new(false));
        
        let test_inlet: (Inlet, Receiver<String>) = Inlet::new(add_routine!(#[coroutine] || {
            sleep(Duration::from_secs(1));
            yield "hello".to_string()
        }), stop_flag.clone(), ());

        let (test_outlet1_sink, test_outlet1_source) = unbounded();
        let (test_outlet2_sink, test_outlet2_source) = unbounded();
        
        let (test_outlet1, test1_tx) = Outlet::new(move |example: String| {
            test_outlet1_sink.send(format!("1: {example}")).unwrap()
        }, stop_flag.clone());

        let (test_outlet2, test2_tx) = Outlet::new(move |example: String| {
            test_outlet2_sink.send(format!("2: {example}")).unwrap()
        }, stop_flag.clone());
        
        let mut broadcaster = Broadcast::new(test_inlet.1, stop_flag.clone());
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
        
        let mut router= Router::new(rx, stop_flag.clone());
        
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

        let (filter, filter_sink) = Filter::new(fun, rx, stop_flag.clone());

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
        #[derive(Clone)]
        struct InnerContext {
            inc_val: i32
        }

        let ctx = InnerContext {
            inc_val: 1
        };

        let stop_flag = Arc::new(AtomicBool::new(false));

        let (transformer, input, output): (Transformer<i32, String, String>, Sender<i32>, Receiver<String>) = Transformer::new(
            add_routine!(#[coroutine] |input: TransformerContext<i32, InnerContext>| {
                let mut data = input.data.unwrap();
                while data < 5 {
                    data += input.globals.inc_val;
                    yield TransformerResult::NeedsMoreWork(data);
                }
                yield TransformerResult::Transformed(format!("The number is {data}"));
            }), stop_flag.clone(), ctx);

        input.send(0).unwrap();

        let result = output.recv().unwrap();
        stop_flag.store(true, Ordering::Relaxed);
        transformer.join().unwrap();

        assert_eq!(result, "The number is 5".to_string())
    }
    
    #[test]
    fn funnel_works() {
        let stop_flag = Arc::new(AtomicBool::new(false));
        
        let (mut funnel, funnel_out) = Funnel::new(stop_flag.clone());
        
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
}
