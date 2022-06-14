use std::cell::Cell;
use std::io::{Error, ErrorKind};
use std::sync::{Mutex, Arc};
use std::{thread, io};
use std::time::Duration;
use std::thread::JoinHandle;

use crossbeam_channel::{Sender, Receiver, RecvError};

pub struct RefluxComputeNode<I, O> {
    computers: Vec<(JoinHandle<()>, Sender<I>)>,
    receiver: Receiver<I>,
    collector: Sender<I>,
    run: Arc<Mutex<Cell<bool>>>,
    drainer: Option<Sender<O>>,
}

impl<I: 'static + Send, O: 'static + Send> RefluxComputeNode<I, O> {
    /**
     * Create a new Compute Node instance.
     */
    pub fn new() -> Self {        
        let (tx, rx) = crossbeam_channel::unbounded();
        RefluxComputeNode{
            computers: Vec::new(),
            receiver: rx,
            collector: tx,
            run: Arc::new(Mutex::new(Cell::new(true))),
            drainer: None,
        }
    }

    /**
     * Set a computation function that accepts an input, processes it and produces an output.
     */
    pub fn set_computers<F, S>(&mut self, n_sinks: usize, sink_fn: F, extern_state: S) -> ()
    where F: Fn(I, Sender<I>, io::Result<Sender<O>>, S) -> io::Result<()> + 'static + Copy + Send,
          S: 'static + Clone + Send
    {
        for _ in 0..n_sinks {
            let (tx, rx) = crossbeam_channel::unbounded();
            let own_sender = self.collector.clone();
            let state = extern_state.clone();
            let drainer = self.drainer.clone();
            let run_lock = self.run.clone();
            self.computers.push((thread::spawn(move || {
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
    pub fn collector(&self) -> Sender<I> {
        self.collector.clone()
    }

    /**
     * Set a destination for the computed result.
     */
    pub fn set_drain(&mut self, drainer: Sender<O>) {
        self.drainer = Some(drainer);
    }

    /**
     * Run the computer, providing a function to indicate when to terminate the computer.
     */
    pub fn run<F>(&mut self, timeout: F) -> Result<(), RecvError>
    where F: Fn(Sender<(u64, bool)>) -> ()
    {
        loop {
            for i in 0..self.computers.len() {
                let (tx, rx) = crossbeam_channel::unbounded();
                timeout(tx);
                let (tm, retry) = rx.recv()?;
                let val = self.receiver.recv_timeout(Duration::from_millis(tm));
                if val.is_ok() {
                    let _ = self.computers.get(i).unwrap().1.send(val.unwrap());
                } else if !retry{
                    self.run.lock().unwrap().set(false);
                    return Ok(());
                }
            }
        }
    }
}