use std::cell::Cell;
use std::io::{Error, ErrorKind};
use std::sync::{Mutex, Arc};
use std::{thread, io};
use std::time::Duration;
use std::thread::JoinHandle;
use std::sync::mpsc::{self, Sender, Receiver, RecvError};

pub struct RefluxComputeNode<T, D> {
    transformers: Vec<(JoinHandle<()>, Sender<T>)>,
    receiver: Receiver<T>,
    sender: Sender<T>,
    run: Arc<Mutex<Cell<bool>>>,
    drainer: Option<Sender<D>>,
}

impl<T: 'static + Send, D: 'static + Send> RefluxComputeNode<T, D> {
    /**
     * Create a new Compute Node instance.
     */
    pub fn new() -> Self {        
        let (tx, rx) = mpsc::channel();
        RefluxComputeNode{
            transformers: Vec::new(),
            receiver: rx,
            sender: tx,
            run: Arc::new(Mutex::new(Cell::new(true))),
            drainer: None,
        }
    }

    /**
     * Set a computation function that accepts an input, processes it and produces an output.
     */
    pub fn set_computer<F, S>(&mut self, n_sinks: usize, sink_fn: F, extern_state: S) -> ()
    where F: Fn(T, Sender<T>, io::Result<Sender<D>>, S) -> io::Result<()> + 'static + Copy + Send,
          S: 'static + Clone + Send
    {
        for _ in 0..n_sinks {
            let (tx, rx) = mpsc::channel();
            let own_sender = self.sender.clone();
            let state = extern_state.clone();
            let drainer = self.drainer.clone();
            let run_lock = self.run.clone();
            self.transformers.push((thread::spawn(move || {
                while run_lock.lock().unwrap().get() {
                    loop {
                        let val = rx.recv();
                        if val.is_err() {
                            break;
                        }
                        let drainer_res = drainer.clone()
                            .ok_or(Error::new(ErrorKind::BrokenPipe, "No drain set"));

                        if let Err(e) = sink_fn(val.unwrap(), own_sender.clone(), drainer_res, state.clone()) {
                            println!("{}", e);
                            break;
                        };
                    }
                }
            }), tx))
        }
        
    }

    /**
     * Set a data source for the computer.
     */
    pub fn collector(&self) -> Sender<T> {
        self.sender.clone()
    }

    /**
     * Set a destination for the computed result.
     */
    pub fn set_drain(&mut self, drainer: Sender<D>) {
        self.drainer = Some(drainer);
    }

    /**
     * Run the computer, providing a function to indicate when to terminate the computer.
     */
    pub fn run<F>(&mut self, timeout: F) -> Result<(), RecvError>
    where F: Fn(Sender<(u64, bool)>) -> ()
    {
        loop {
            for i in 0..self.transformers.len() {
                let (tx, rx) = mpsc::channel();
                timeout(tx);
                let (tm, retry) = rx.recv()?;
                let val = self.receiver.recv_timeout(Duration::from_millis(tm));
                if val.is_ok() {
                    let _ = self.transformers.get(i).unwrap().1.send(val.unwrap());
                } else if !retry{
                    self.run.lock().unwrap().set(false);
                    return Ok(());
                }
            }
        }
    }
}