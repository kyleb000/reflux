#![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
#![feature(unboxed_closures)]

use std::marker::PhantomData;
use std::ops::{Coroutine, CoroutineState};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use crossbeam_channel::{Receiver, unbounded};


struct Inlet {
    inlet_fn: JoinHandle<()>,
}

impl Inlet {
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
                            tx.send(r).unwrap();
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
    pub fn join(self) -> thread::Result<()> {
        self.inlet_fn.join()?;
        Ok(())
    }
}


struct Outlet {
    outlet_fn: JoinHandle<()>
}

impl Outlet {
    pub fn new<T, F>(outlet_fn: F, receiver: Receiver<T>, stop_sig: Arc<AtomicBool>) -> Self
        where
            T: Send + 'static,
            F: Fn(T) + Send + 'static {
        let outlet = thread::spawn(move || {
            loop {
                while !stop_sig.load(Ordering::Relaxed) {
                    if let Ok(data) = receiver.recv_timeout(Duration::from_millis(10)) {
                        outlet_fn(data)
                    }
                }
            }
        });
        Self {
            outlet_fn: outlet
        }
    }
    
    pub fn join(self) -> thread::Result<()> {
        self.outlet_fn.join()?;
        Ok(())
    }
}




/// A simple macro to create a function that returns a coroutine.
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
        let (data_tx, data_rx) = unbounded();
        let outlet= Outlet::new(move |test: String| {
            test_tx.send(test).unwrap();
        }, data_rx, stop_flag.clone());
        
        data_tx.send("Hello, world".to_string()).unwrap();
        
        let data_recv = test_rx.recv().unwrap();
        stop_flag.store(true, Ordering::Relaxed);
        outlet.join().unwrap();
        
        assert_eq!(data_recv, "Hello, world".to_string())
    }

    // #[test]
    // fn it_works() {
    //     let node = ComputeNode::<String, String, usize>::new(
    //         add_routine!(
    //             #[coroutine] |path_str: String| {
    //                 let paths = fs::read_dir(&path_str).expect("fuck you");
    //                 for path in paths {
    //                     match path {
    //                         Ok(val) => {
    //                             yield format!("{}", val.path().display());
    //                         },
    //                         Err(_) => continue
    //                     }
    //                 }
    //             }),
    //         add_routine!(#[coroutine]|path: String| {
    //             match fs::metadata(&path) {
    //                 Ok(md) => {
    //                     if md.is_dir() {
    //                         yield Err(path);
    //                     } else {
    //                         let value = fs::read_to_string(path).unwrap().parse().ok().unwrap();
    //                         yield Ok(value);
    //                     }
    //                 },
    //                 Err(_) => ()
    //             }
    //         })
    //     );
    //     
    //     node.1.send("test/".to_string()).unwrap();
    // 
    //     let mut result = 0;
    //     
    //     loop {
    //         match node.2.recv_timeout(Duration::from_secs(1)) {
    //             Ok(value) => result += value,
    //             Err(val) => break
    //         }
    //     }
    //     node.3.store(true, Ordering::Relaxed);
    //     node.0.join().unwrap();
    // 
    //     assert_eq!(result, 122);
    // }
}
