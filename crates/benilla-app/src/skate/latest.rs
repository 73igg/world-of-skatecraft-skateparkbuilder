//! One pending value, including collision requests and their completed replacements.

use std::sync::{Arc, Condvar, Mutex};

struct State<T> {
    value: Option<T>,
    closed: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}

pub(super) struct Sender<T>(Arc<Shared<T>>);
pub(super) struct Receiver<T>(Arc<Shared<T>>);

pub(super) fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            value: None,
            closed: false,
        }),
        ready: Condvar::new(),
    });
    (Sender(shared.clone()), Receiver(shared))
}

impl<T> Sender<T> {
    pub(super) fn send(&self, value: T) {
        self.0.state.lock().unwrap().value = Some(value);
        self.0.ready.notify_one();
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().closed = true;
        self.0.ready.notify_one();
    }
}

impl<T> Receiver<T> {
    pub(super) fn take(&self) -> Option<T> {
        self.0.state.lock().unwrap().value.take()
    }

    pub(super) fn recv(&self) -> Option<T> {
        let state = self.0.state.lock().unwrap();
        let mut state = self
            .0
            .ready
            .wait_while(state, |s| !s.closed && s.value.is_none())
            .unwrap();
        state.value.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_consumer_receives_only_the_latest_work() {
        let (sender, receiver) = channel();
        for i in 0..100_000 {
            sender.send(i);
        }
        assert_eq!(receiver.take(), Some(99_999));
        assert_eq!(receiver.take(), None);
    }

    #[test]
    fn sender_shutdown_wakes_a_blocked_worker() {
        let (sender, receiver) = channel::<usize>();
        let worker = std::thread::spawn(move || receiver.recv());
        drop(sender);
        assert_eq!(worker.join().unwrap(), None);
    }
}
