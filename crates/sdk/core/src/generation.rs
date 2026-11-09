//! In-memory connection epochs and cancellation; durable credential epochs belong to storage.

use crate::ClientError;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio_util::sync::CancellationToken;

/// Connection/request epoch, unique within one Client lifetime, never a credential key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(pub(crate) u64);

impl Generation {
    /// Returns the diagnostic epoch number, without account or credential data.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A result tagged at dispatch, not at delivery.
#[derive(Clone)]
pub struct Generated<T> {
    /// Epoch that requested this result.
    pub generation: Generation,
    /// Result payload; consumers must validate the epoch before applying it.
    pub value: T,
}

pub(crate) struct Epochs {
    next: Arc<AtomicU64>,
    root: CancellationToken,
    current: Mutex<Epoch>,
}

#[derive(Clone)]
pub(crate) struct Epoch {
    pub generation: Generation,
    pub cancel: CancellationToken,
}

impl Epochs {
    pub fn new(next: Arc<AtomicU64>, root: CancellationToken) -> Self {
        let current = Epoch {
            generation: Generation(next.fetch_add(1, Ordering::Relaxed)),
            cancel: root.child_token(),
        };
        Self {
            next,
            root,
            current: Mutex::new(current),
        }
    }

    pub fn current(&self) -> Epoch {
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn advance(&self) -> Result<Epoch, ClientError> {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.root.is_cancelled() {
            return Err(ClientError::Closed);
        }
        current.cancel.cancel();
        let generation = self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| ClientError::Closed)?;
        *current = Epoch {
            generation: Generation(generation),
            cancel: self.root.child_token(),
        };
        Ok(current.clone())
    }

    pub fn validate(&self, generation: Generation) -> Result<(), ClientError> {
        let current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.root.is_cancelled() {
            Err(ClientError::Closed)
        } else if current.generation != generation || current.cancel.is_cancelled() {
            Err(ClientError::Superseded)
        } else {
            Ok(())
        }
    }

    /// The check and publication are indivisible with respect to epoch replacement.
    pub fn apply<T>(
        &self,
        generation: Generation,
        apply: impl FnOnce() -> T,
    ) -> Result<T, ClientError> {
        let current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.root.is_cancelled() {
            return Err(ClientError::Closed);
        }
        if current.generation != generation || current.cancel.is_cancelled() {
            return Err(ClientError::Superseded);
        }
        Ok(apply())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_drops_old_events_and_rest_results() {
        let epochs = Epochs::new(Arc::new(AtomicU64::new(1)), CancellationToken::new());
        let old = epochs.current();
        let new = epochs.advance().unwrap();
        assert!(old.cancel.is_cancelled());
        assert!(matches!(
            epochs.apply(old.generation, || panic!("stale result applied")),
            Err(ClientError::Superseded)
        ));
        assert_eq!(epochs.apply(new.generation, || 7).unwrap(), 7);
    }
    #[test]
    fn removal_rejects_current_generation_and_new_admission() {
        let root = CancellationToken::new();
        let epochs = Epochs::new(Arc::new(AtomicU64::new(1)), root.clone());
        let old = epochs.current();
        root.cancel();
        assert!(matches!(
            epochs.validate(old.generation),
            Err(ClientError::Closed)
        ));
        assert!(matches!(epochs.advance(), Err(ClientError::Closed)));
    }
}
