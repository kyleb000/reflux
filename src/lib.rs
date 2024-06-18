#![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
#![feature(unboxed_closures)]

use std::ops::{Coroutine, CoroutineState};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use crossbeam_channel::{Receiver, Sender, unbounded};


/// Receives data from an external source and broadcasts the data via channels.
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
///      }), stop_flag.clone());
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
    /// Creates a new `Inlet` object.
    /// 
    /// # Parameters
    /// - `inlet_fn` - A coroutine that reads and returns data from an external source.
    /// The use of the `add_routine!` macro is necessary when passing in an `inlet_fn`.
    /// - `stop_sig` - A flag to signal the `Inlet` object to terminate
    /// 
    /// # Returns
    /// A `Inlet` object and a `Receiver` to receive data from the `inlet_fn`
    pub fn new<F, C, T>(inlet_fn: F, stop_sig: Arc<AtomicBool>) -> (Self, Receiver<T>)
        where 
            F: Fn() -> C + Send + 'static,
            C: Coroutine<()> + Send + 'static + Unpin,
            T: Send + 'static + From<<C as Coroutine<()>>::Yield> {
        let (tx, rx) = unbounded();
        let inlet_thr = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
                let mut routine = inlet_fn();
                loop {
                    match Pin::new(&mut routine).resume(()) {
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
/// use reflux::Outlet;
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::Receiver;
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
/// use std::thread::sleep;
/// 
/// let stop_flag = Arc::new(AtomicBool::new(false));
/// let (test_tx, test_rx) = unbounded();
/// let (outlet, out_send)= Outlet::new(move |test: String| {
/// test_tx.send(test).unwrap();
/// }, stop_flag.clone());
/// 
/// out_send.send("Hello, world".to_string()).unwrap();
/// 
/// let data_recv = test_rx.recv().unwrap();
/// stop_flag.store(true, Ordering::Relaxed);
/// outlet.join().unwrap();
/// 
/// assert_eq!(data_recv, "Hello, world".to_string())
/// ```
pub struct Outlet {
    outlet_fn: JoinHandle<()>
}

impl Outlet {
    /// Creates a new `Outlet` object.
    ///
    /// # Parameters
    /// - `outlet_fn` - A function that receives data from a `Reflux` network and sends it to an
    /// external sink.
    /// - `receiver` - A `Receiver` channel object from which to receive data.
    /// - `stop_sig` - A flag to signal the `Inlet` object to terminate
    ///
    /// # Returns
    /// A `Outlet` object
    pub fn new<T, F>(outlet_fn: F, stop_sig: Arc<AtomicBool>) -> (Self, Sender<T>)
        where
            T: Send + 'static,
            F: Fn(T) + Send + 'static {
        let (sender, receiver) = unbounded();
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
    
    /// Waits for the `Outlet` object to finish execution
    pub fn join(self) -> thread::Result<()> {
        self.outlet_fn.join()?;
        Ok(())
    }
}

/// An object that received data from a provided `Receiver`, and broadcasts the data to all
/// subscribers.
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
///  #![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
///  #![feature(unboxed_closures)]
/// use reflux::{Inlet, Outlet, Broadcast};
///  use std::sync::Arc;
///  use std::sync::atomic::{AtomicBool, Ordering};
///  use crossbeam_channel::Receiver;
///  use reflux::add_routine;
///  use crossbeam_channel::unbounded;
/// use std::time::Duration;
/// use std::thread::sleep;
/// let stop_flag = Arc::new(AtomicBool::new(false));
/// 
/// let test_inlet: (Inlet, Receiver<String>) = Inlet::new(add_routine!(#[coroutine] || {
///             sleep(Duration::from_secs(1));
///             yield "hello".to_string()
///         }), stop_flag.clone());
/// 
/// let (test_outlet1_sink, test_outlet1_source) = unbounded();
/// let (test_outlet2_sink, test_outlet2_source) = unbounded();
/// 
/// let (test_outlet1, test1_tx) = Outlet::new(move |example: String| {
/// test_outlet1_sink.send(format!("1: {example}")).unwrap()
/// }, stop_flag.clone());
/// 
/// let (test_outlet2, test2_tx) = Outlet::new(move |example: String| {
/// test_outlet2_sink.send(format!("2: {example}")).unwrap()
/// }, stop_flag.clone());
/// 
/// let mut broadcaster = Broadcast::new(test_inlet.1, stop_flag.clone());
/// broadcaster.subscribe(test1_tx);
/// broadcaster.subscribe(test2_tx);
/// 
/// let data1 = test_outlet1_source.recv().unwrap();
/// let data2 = test_outlet2_source.recv().unwrap();
/// 
/// stop_flag.store(true, Ordering::Relaxed);
/// 
/// test_outlet1.join().unwrap();
/// test_outlet2.join().unwrap();
/// test_inlet.0.join().unwrap();
/// broadcaster.join().unwrap();
/// 
/// 
/// assert_eq!(data1, "1: hello".to_string());
/// assert_eq!(data2, "2: hello".to_string());
/// ```
pub struct Broadcast<T> {
    subscribers: Arc<Mutex<Vec<Sender<T>>>>,
    _broadcaster: JoinHandle<()>,
}

impl<T> Broadcast<T> where T: Clone + Send + 'static {
    pub fn new(source: Receiver<T>, stop_sig: Arc<AtomicBool>) -> Self {
        let subscribers = Arc::new(Mutex::new(Vec::<Sender<T>>::new()));
        
        let thr_subscribers = subscribers.clone();
        let broadcaster = thread::spawn(move || {
            while !stop_sig.load(Ordering::Relaxed) {
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
            _broadcaster: broadcaster
        }
    }
    
    pub fn subscribe(&mut self, subscriber: Sender<T>) {
        let mut subscribers_lock = self.subscribers.lock().unwrap();
        subscribers_lock.push(subscriber)
    }

    pub fn join(self) -> thread::Result<()> {
        self._broadcaster.join()?;
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
    use std::fs;
    use std::thread::sleep;
    use std::time::Duration;
    use super::*;
    
    #[test]
    fn inlet_works() {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (inlet, inlet_chan): (Inlet, Receiver<String>) = Inlet::new(
            add_routine!(#[coroutine] |_: ()| {
                yield "Hello, world".to_string()
            }), stop_flag.clone());
        
        
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
        }), stop_flag.clone());

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
        
        let data1 = test_outlet1_source.recv().unwrap();
        let data2 = test_outlet2_source.recv().unwrap();
        
        stop_flag.store(true, Ordering::Relaxed);

        test_outlet1.join().unwrap();
        test_outlet2.join().unwrap();
        test_inlet.0.join().unwrap();
        broadcaster.join().unwrap();
        
        
        assert_eq!(data1, "1: hello".to_string());
        assert_eq!(data2, "2: hello".to_string());
    }
}
