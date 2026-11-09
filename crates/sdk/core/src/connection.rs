//! Per-account connection state and retry policy; the API gateway never reconnects itself.

use crate::{ClientError, Generation};
use std::{
    hash::{BuildHasher, Hasher},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::time::Instant;

/// Observable transport state, independent of authentication and cache reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionState {
    /// Establishing a single socket.
    Connecting,
    /// A socket is established; this does not assert cache freshness.
    Online,
    /// Retry will not start before this monotonic deadline.
    Backoff {
        /// Earliest retry time on Tokio's monotonic clock.
        retry_at: Instant,
    },
    /// No socket or retry is active.
    Offline,
}

/// A source of bounded jitter; tests inject constants/scripts, never randomness.
pub trait Jitter: Send + Sync {
    /// Returns a duration in `0..=upper`; values outside the range are clamped.
    fn sample(&self, upper: Duration) -> Duration;
}

/// Runtime jitter using a randomly seeded standard-library hasher, without a new dependency.
pub struct RuntimeJitter {
    seed: std::hash::RandomState,
    counter: AtomicU64,
}
impl Default for RuntimeJitter {
    fn default() -> Self {
        Self {
            seed: std::hash::RandomState::new(),
            counter: AtomicU64::new(0),
        }
    }
}
impl Jitter for RuntimeJitter {
    fn sample(&self, upper: Duration) -> Duration {
        let mut hasher = self.seed.build_hasher();
        hasher.write_u64(self.counter.fetch_add(1, Ordering::Relaxed));
        let upper_ns = u64::try_from(upper.as_nanos()).unwrap_or(u64::MAX);
        Duration::from_nanos(hasher.finish() % upper_ns.saturating_add(1).max(1))
    }
}

/// Capped exponential retry, with equal jitter in the upper half of each delay.
#[derive(Clone, Debug)]
pub struct Backoff {
    /// First retry's maximum delay; default one second.
    pub initial: Duration,
    /// Maximum delay at any attempt; default one minute.
    pub maximum: Duration,
}
impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            maximum: Duration::from_secs(60),
        }
    }
}
impl Backoff {
    pub(crate) fn validate(&self) -> Result<(), ClientError> {
        if self.initial.is_zero()
            || self.initial > self.maximum
            || self.maximum > Duration::from_hours(24)
        {
            Err(ClientError::Configuration)
        } else {
            Ok(())
        }
    }
    /// Returns the delay for a zero-based failure attempt, always bounded by `maximum`.
    #[must_use]
    pub fn delay(&self, attempt: u32, jitter: &dyn Jitter) -> Duration {
        let ceiling = self
            .initial
            .saturating_mul(1_u32.checked_shl(attempt).unwrap_or(u32::MAX))
            .min(self.maximum);
        let floor = ceiling / 2;
        floor
            + jitter
                .sample(ceiling.saturating_sub(floor))
                .min(ceiling.saturating_sub(floor))
    }
}

/// Generation-stamped connection status; watch channels may coalesce intermediate states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectionStatus {
    /// Epoch responsible for the status.
    pub generation: Generation,
    /// Current connection state.
    pub state: ConnectionState,
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixed(Duration);
    impl Jitter for Fixed {
        fn sample(&self, _: Duration) -> Duration {
            self.0
        }
    }
    #[test]
    fn retry_is_exponential_capped_and_jitter_is_injected() {
        let retry = Backoff::default();
        let lower: Vec<_> = (0..9)
            .map(|attempt| retry.delay(attempt, &Fixed(Duration::ZERO)).as_millis())
            .collect();
        assert_eq!(
            lower,
            [500, 1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000]
        );
        assert_eq!(
            retry.delay(u32::MAX, &Fixed(Duration::MAX)),
            Duration::from_secs(60)
        );
    }
}
