use crossbeam_channel::{bounded, unbounded, Receiver, Sender};

pub fn get_channel<T>(data_limit: usize) -> (Sender<T>, Receiver<T>) {
    match data_limit {
        0 => unbounded(),
        _ => bounded(data_limit)
    }
}
