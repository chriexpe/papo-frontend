//! How a watch or a job changes the UI state: through the bridge's [`Post`],
//! in the order sent.

use std::sync::Arc;

use rune::Tx;

use super::Post;

/// A watch's way to change the UI state: each change runs on the UI thread,
/// in the order sent.
pub(crate) struct Updates<S> {
    post: Arc<dyn Post<S>>,
}

impl<S> Updates<S> {
    pub(super) fn new(post: Arc<dyn Post<S>>) -> Updates<S> {
        Updates { post }
    }
}

impl<S: 'static> Updates<S> {
    /// Queues `change`, which returns whether it changed what the view reads.
    pub(crate) fn send(&self, change: impl FnOnce(&mut S) -> bool + Send + 'static) {
        self.post.post(Box::new(move |state, _tx| change(state)));
    }

    /// [`Updates::send`] for a change that also asks Rune for something,
    /// such as a scroll.
    pub(crate) fn send_tx(&self, change: impl FnOnce(&mut S, &mut Tx) -> bool + Send + 'static) {
        self.post.post(Box::new(change));
    }
}

impl<S> Clone for Updates<S> {
    fn clone(&self) -> Self {
        Updates {
            post: Arc::clone(&self.post),
        }
    }
}
