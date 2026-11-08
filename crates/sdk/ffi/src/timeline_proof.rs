//! Translates deterministic VectorDiff fixtures across UniFFI on a worker thread.
//! This opt-in proof owns no subscriptions, timeline policy or platform list state.

use std::thread;

use eyeball_im::VectorDiff;

/// Synthetic stable identity and edit revision; contains no message content.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ProofItem {
    /// Stable synthetic item identity, independent of its list index.
    pub id: u64,
    /// Edit revision, incremented without changing identity.
    pub revision: u32,
}

/// The `VectorDiff` operations exercised by the integration proof.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ProofDiff {
    /// Append one new item.
    PushBack {
        /// Item appended at the back.
        value: ProofItem,
    },
    /// Replace one edited item, retaining its stable id.
    Set {
        /// Zero-based index, checked to fit Kotlin Int.
        index: u32,
        /// Replacement item.
        value: ProofItem,
    },
    /// Insert one item before the given index.
    Insert {
        /// Zero-based index, checked to fit Kotlin Int.
        index: u32,
        /// Inserted item.
        value: ProofItem,
    },
    /// Replace all items; clients resolve the anchor by id afterward.
    Reset {
        /// New list in display order.
        values: Vec<ProofItem>,
    },
}

/// One ordered delivery, applied as a whole before resolving the scroll anchor.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ProofBatch {
    /// Resulting fixture version, increasing by one per delivery.
    pub version: u64,
    /// Operations applied in order before anchor resolution.
    pub diffs: Vec<ProofDiff>,
}

/// Diagnostic failures, never user-visible copy.
#[derive(Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[non_exhaustive]
pub enum ProofError {
    /// A Rust index exceeds the signed Kotlin list-index range.
    #[error("list index cannot be represented by Kotlin Int")]
    IndexOutOfRange,
    /// An operation outside the four-operation fixture was supplied.
    #[error("VectorDiff operation is outside this proof")]
    UnsupportedDiff,
    /// The operating system could not create the worker.
    #[error("could not start proof worker")]
    WorkerStart,
    /// Joining the worker detected a panic, including a callback panic.
    #[error("proof worker panicked")]
    WorkerPanicked,
}

/// Callbacks run serially on the Rust worker, never on the caller or main thread.
/// Consumers must enqueue work without waiting for the UI to apply it.
#[uniffi::export(callback_interface)]
pub trait ProofListener: Send + Sync {
    /// Snapshot precedes every batch; this fixture does not prove atomic subscription.
    fn on_snapshot(&self, version: u64, items: Vec<ProofItem>);
    /// Versions increase by one in this fixture. Preserve callback and diff order.
    fn on_batch(&self, batch: ProofBatch);
}

/// Sends the fixed proof fixture and joins the worker before returning.
///
/// Call off the UI thread. Success means every callback returned, not that posted
/// UI work finished. No production service or bounded subscription is implemented.
///
/// # Errors
/// Returns a typed error if worker creation, diff translation or worker join fails.
#[uniffi::export]
pub fn run_timeline_proof(listener: Box<dyn ProofListener>) -> Result<(), ProofError> {
    run_worker(move || {
        listener.on_snapshot(40, vec![item(30, 0), item(40, 0)]);
        for (version, diffs) in fixture_batches() {
            let diffs = diffs.into_iter().map(map_diff).collect::<Result<_, _>>()?;
            listener.on_batch(ProofBatch { version, diffs });
        }
        Ok(())
    })
}

/// A worker panic is an error even if some callbacks have already been delivered.
fn run_worker(
    work: impl FnOnce() -> Result<(), ProofError> + Send + 'static,
) -> Result<(), ProofError> {
    thread::Builder::new()
        .name("sdk-timeline-proof".into())
        .spawn(work)
        .map_err(|_| ProofError::WorkerStart)?
        .join()
        .map_err(|_| ProofError::WorkerPanicked)?
}

/// Checks both the FFI width and Kotlin's signed list-index range.
fn list_index(index: usize) -> Result<u32, ProofError> {
    let index = u32::try_from(index).map_err(|_| ProofError::IndexOutOfRange)?;
    i32::try_from(index).map_err(|_| ProofError::IndexOutOfRange)?;
    Ok(index)
}

/// Only the four operations required by this fixture cross this proof boundary.
fn map_diff(diff: VectorDiff<ProofItem>) -> Result<ProofDiff, ProofError> {
    Ok(match diff {
        VectorDiff::PushBack { value } => ProofDiff::PushBack { value },
        VectorDiff::Set { index, value } => ProofDiff::Set {
            index: list_index(index)?,
            value,
        },
        VectorDiff::Insert { index, value } => ProofDiff::Insert {
            index: list_index(index)?,
            value,
        },
        VectorDiff::Reset { values } => ProofDiff::Reset {
            values: values.into_iter().collect(),
        },
        _ => return Err(ProofError::UnsupportedDiff),
    })
}

fn item(id: u64, revision: u32) -> ProofItem {
    ProofItem { id, revision }
}

/// Front insertions are intentionally in reverse arrival order within each batch.
fn fixture_batches() -> Vec<(u64, Vec<VectorDiff<ProofItem>>)> {
    vec![
        (41, vec![VectorDiff::PushBack { value: item(50, 0) }]),
        (
            42,
            vec![VectorDiff::Set {
                index: 0,
                value: item(30, 1),
            }],
        ),
        (
            43,
            vec![
                VectorDiff::Insert {
                    index: 0,
                    value: item(20, 0),
                },
                VectorDiff::Insert {
                    index: 0,
                    value: item(10, 0),
                },
            ],
        ),
        (
            44,
            vec![VectorDiff::Insert {
                index: 0,
                value: item(5, 0),
            }],
        ),
        (
            45,
            vec![VectorDiff::Insert {
                index: 0,
                value: item(1, 0),
            }],
        ),
        (
            46,
            vec![VectorDiff::Reset {
                values: [item(20, 0), item(30, 2), item(40, 0), item(50, 0)]
                    .into_iter()
                    .collect(),
            }],
        ),
        (
            47,
            vec![VectorDiff::Reset {
                values: [item(50, 0)].into_iter().collect(),
            }],
        ),
    ]
}

#[cfg(test)]
mod tests;
