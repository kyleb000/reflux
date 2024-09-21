use crossbeam_channel::{bounded, unbounded, Receiver, Sender};

pub fn get_channel<T>(data_limit: Option<usize>) -> (Sender<T>, Receiver<T>) {
    match data_limit { 
        Some(limit) => bounded(limit),
        None => unbounded()
    }
}
