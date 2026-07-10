use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

struct State<T> {
    value: Option<T>,
    closed: bool,
    waker: Option<std::task::Waker>,
}

pub(crate) struct Sender<T> {
    state: Arc<Mutex<State<T>>>,
}

impl<T> Sender<T> {
    pub(crate) fn send(&self, value: T) -> bool {
        let waker = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.closed {
                return false;
            }
            state.value = Some(value);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        true
    }

    pub(crate) fn close(&self) {
        let waker = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

pub(crate) struct Receiver<T> {
    state: Arc<Mutex<State<T>>>,
}

impl<T> Receiver<T> {
    pub(crate) fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(value) = state.value.take() {
            return Poll::Ready(Some(value));
        }
        if state.closed {
            return Poll::Ready(None);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        state.waker = None;
    }
}

pub(crate) fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let state = Arc::new(Mutex::new(State {
        value: None,
        closed: false,
        waker: None,
    }));
    (
        Sender {
            state: Arc::clone(&state),
        },
        Receiver { state },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::task::noop_waker_ref;

    fn poll<T>(receiver: &mut Receiver<T>) -> Poll<Option<T>> {
        let mut context = Context::from_waker(noop_waker_ref());
        receiver.poll_next(&mut context)
    }

    #[test]
    fn replaces_an_unread_value_with_the_latest() {
        let (sender, mut receiver) = channel();
        assert!(sender.send(1));
        assert!(sender.send(2));

        assert_eq!(poll(&mut receiver), Poll::Ready(Some(2)));
        assert_eq!(poll(&mut receiver), Poll::Pending);
    }

    #[test]
    fn closes_and_detects_a_dropped_receiver() {
        let (sender, mut receiver) = channel::<u32>();
        sender.close();
        assert_eq!(poll(&mut receiver), Poll::Ready(None));

        let (sender, receiver) = channel::<u32>();
        drop(receiver);
        assert!(!sender.send(1));
    }
}
