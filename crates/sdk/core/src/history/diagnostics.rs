//! Payload-free history work accounting and idle-cache limits; retention decisions live in residency.

use std::sync::atomic::{AtomicU64, Ordering};

/// Per-account idle canonical-history limits. Active capabilities are protected separately.
/// Bytes count encoded entries and publication payloads, plus fixed index estimates; not RSS.
#[derive(Clone, Copy, Debug)]
pub struct HistoryBudget {
    /// Maximum retained idle channel models; zero disables warm idle retention.
    pub idle_channels: usize,
    /// Maximum estimated encoded bytes across idle channel models; zero retains none.
    pub idle_bytes: usize,
}
impl Default for HistoryBudget {
    fn default() -> Self {
        Self {
            idle_channels: 4,
            idle_bytes: 32 * 1024 * 1024,
        }
    }
}

/// Cumulative account history work, including channels already evicted. No content or IDs.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct HistoryWork {
    /// Entries examined to build or update raw publications.
    pub publication_entries: u64,
    /// Message payloads encoded for publication equality checks.
    pub publication_encodes: u64,
    /// Accepted raw publication count.
    pub publications: u64,
    /// Rows submitted to successful cache write transactions.
    pub persisted_rows: u64,
    /// Encoded row and metadata bytes submitted to successful cache transactions.
    pub persisted_bytes: u64,
    /// Cache model hydration count.
    pub cache_loads: u64,
    /// Idle channel models evicted without touching disk history.
    pub evictions: u64,
    /// Largest reconnect gateway journal observed.
    pub reconnect_journal_peak: u64,
    /// Reconnect generations refused because their gateway journal overflowed.
    pub reconnect_overflows: u64,
}

/// Account-local current residency plus cumulative work. Estimates exclude allocator overhead,
/// persistent-vector node sharing, consumer-retained old snapshots, media and native UI resources.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct HistoryDiagnostics {
    /// Models protected by a handle, watch receiver, or in-flight channel reference.
    pub pinned_channels: usize,
    /// Models eligible for the warm idle cache.
    pub idle_channels: usize,
    /// Estimated bytes owned by pinned histories.
    pub pinned_bytes: usize,
    /// Estimated bytes owned by idle histories.
    pub idle_bytes: usize,
    /// Cumulative work since this account session was created.
    pub work: HistoryWork,
}

#[derive(Default)]
pub(crate) struct Counters {
    pub version_high_water: AtomicU64,
    pub publication_entries: AtomicU64,
    pub publication_encodes: AtomicU64,
    pub publications: AtomicU64,
    pub persisted_rows: AtomicU64,
    pub persisted_bytes: AtomicU64,
    pub cache_loads: AtomicU64,
    pub evictions: AtomicU64,
    pub reconnect_journal_peak: AtomicU64,
    pub reconnect_overflows: AtomicU64,
}
impl Counters {
    pub fn snapshot(&self) -> HistoryWork {
        HistoryWork {
            publication_entries: self.publication_entries.load(Ordering::Relaxed),
            publication_encodes: self.publication_encodes.load(Ordering::Relaxed),
            publications: self.publications.load(Ordering::Relaxed),
            persisted_rows: self.persisted_rows.load(Ordering::Relaxed),
            persisted_bytes: self.persisted_bytes.load(Ordering::Relaxed),
            cache_loads: self.cache_loads.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            reconnect_journal_peak: self.reconnect_journal_peak.load(Ordering::Relaxed),
            reconnect_overflows: self.reconnect_overflows.load(Ordering::Relaxed),
        }
    }
}
