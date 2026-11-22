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
    /// Reconstructs an epoch tag from a raw SDK-issued value across FFI/intent boundaries.
    ///
    /// This tag is never an authorization capability; `Epochs::validate` and active windows
    /// reject stale or fabricated values.
    #[must_use]
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Construct a deterministic epoch for isolated projection tests, never session admission.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub const fn for_test(value: u64) -> Self {
        Self(value)
    }

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

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub(crate) enum EpochError {
    #[error("client or session closed")]
    Closed,
    #[error("session generation superseded")]
    Superseded,
}

impl From<EpochError> for ClientError {
    fn from(error: EpochError) -> Self {
        match error {
            EpochError::Closed => Self::Closed,
            EpochError::Superseded => Self::Superseded,
        }
    }
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

    #[cfg(test)]
    pub fn advance(&self) -> Result<Epoch, ClientError> {
        self.replace(None, |_| {})
    }

    /// Replace the current epoch and publish its synchronous reset under the same fence.
    /// A driver's retry may only replace the failed epoch; an explicit reconnect starts anew.
    pub fn replace(
        &self,
        expected: Option<Generation>,
        publish: impl FnOnce(Generation),
    ) -> Result<Epoch, ClientError> {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.root.is_cancelled() {
            return Err(ClientError::Closed);
        }
        if expected.is_some_and(|generation| generation != current.generation) {
            return Err(ClientError::Superseded);
        }
        let generation = self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| ClientError::Closed)?;
        current.cancel.cancel();
        *current = Epoch {
            generation: Generation(generation),
            cancel: self.root.child_token(),
        };
        publish(current.generation);
        Ok(current.clone())
    }

    pub fn validate_epoch(&self, generation: Generation) -> Result<(), EpochError> {
        let current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.root.is_cancelled() {
            Err(EpochError::Closed)
        } else if current.generation != generation || current.cancel.is_cancelled() {
            Err(EpochError::Superseded)
        } else {
            Ok(())
        }
    }

    pub fn validate(&self, generation: Generation) -> Result<(), ClientError> {
        self.validate_epoch(generation).map_err(Into::into)
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
    fn from_raw_round_trips_epoch_number() {
        assert_eq!(Generation::from_raw(42).get(), 42);
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
