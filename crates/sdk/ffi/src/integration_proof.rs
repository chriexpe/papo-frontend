//! Test-only UniFFI translation and worker-thread callback delivery.
//! Source batches come from `papo-sdk-ui`; Kotlin owns all main-thread mutations.

use std::sync::Arc;

use papo_sdk_ui::integration_proof::{ProofRow, VectorDiff, batches};

/// Binding-safe row preserving stable identity across updates.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ProofItem {
    /// Stable identifier.
    pub id: String,
    /// Presentation text.
    pub title: String,
}

/// Binding-safe change records, applied by Kotlin in callback order.
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ProofChange {
    /// Insert a row at `index`.
    Insert {
        /// Zero-based insertion index.
        index: u32,
        /// Inserted item.
        value: ProofItem,
    },
    /// Replace an existing row at `index`.
    Set {
        /// Zero-based index of the replaced item.
        index: u32,
        /// Replacement item with the same identity.
        value: ProofItem,
    },
    /// Add a row to the end.
    PushBack {
        /// Item appended at the end.
        value: ProofItem,
    },
    /// Replace the entire list.
    Reset {
        /// Complete replacement snapshot.
        values: Vec<ProofItem>,
    },
}

impl From<ProofRow> for ProofItem {
    fn from(row: ProofRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
        }
    }
}

impl From<VectorDiff> for ProofChange {
    fn from(change: VectorDiff) -> Self {
        match change {
            VectorDiff::Insert { index, value } => Self::Insert {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                value: value.into(),
            },
            VectorDiff::Set { index, value } => Self::Set {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                value: value.into(),
            },
            VectorDiff::PushBack { value } => Self::PushBack {
                value: value.into(),
            },
            VectorDiff::Reset { values } => Self::Reset {
                values: values.into_iter().map(Into::into).collect(),
            },
        }
    }
}

/// Foreign callback. It is always invoked off the Android main thread.
#[uniffi::export(with_foreign)]
pub trait ProofListener: Send + Sync {
    /// Delivers one ordered batch; consumers post to their UI executor.
    fn on_batch(&self, changes: Vec<ProofChange>);
}

/// A native-thread producer for an actual generated UniFFI callback.
#[derive(uniffi::Object)]
pub struct ProofEmitter {
    listener: Arc<dyn ProofListener>,
}

#[uniffi::export]
impl ProofEmitter {
    /// Retains the foreign callback for the lifetime of the emitter.
    #[uniffi::constructor]
    pub fn new(listener: Arc<dyn ProofListener>) -> Arc<Self> {
        Arc::new(Self { listener })
    }

    /// Delivers snapshot, incremental batch and reset from a Rust worker.
    ///
    /// Joining the short-lived proof thread makes instrumentation deterministic;
    /// a production SDK subscriber will instead use the session's task group.
    pub fn emit_from_worker(&self) {
        let listener = Arc::clone(&self.listener);
        let worker = std::thread::spawn(move || {
            for changes in batches() {
                listener.on_batch(changes.into_iter().map(Into::into).collect());
            }
        });
        let _ = worker.join();
    }
}
