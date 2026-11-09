//! Closeable scalar state notifications; list projection and diff batching belong to SDK UI services.

use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// The owner alone holds a sender; service clones share the cell rather than cloning senders.
pub(crate) struct StateCell<T> {
    sender: Mutex<Option<watch::Sender<T>>>,
    receiver: watch::Receiver<T>,
}
impl<T: Clone> StateCell<T> {
    pub fn new(initial: T) -> Arc<Self> {
        let (sender, receiver) = watch::channel(initial);
        Arc::new(Self {
            sender: Mutex::new(Some(sender)),
            receiver,
        })
    }
    pub fn subscribe(&self) -> watch::Receiver<T> {
        self.receiver.clone()
    }
    pub fn borrow(&self) -> watch::Ref<'_, T> {
        self.receiver.borrow()
    }
    pub fn send_replace(&self, value: T) -> T {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(sender) = sender.as_ref() {
            sender.send_replace(value)
        } else {
            self.receiver.borrow().clone()
        }
    }
    pub fn send_if_modified(&self, change: impl FnOnce(&mut T) -> bool) {
        if let Some(sender) = self
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            sender.send_if_modified(change);
        }
    }
    pub fn is_closed(&self) -> bool {
        self.sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none()
    }
    pub fn close(&self) {
        self.sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}
