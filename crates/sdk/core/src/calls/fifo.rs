//! Bounded ordered FIFO of typed voice commands feeding `AccountSocket` next to coalesced typing.

use crate::Generation;
use papo_api::gateway::Command;
use tokio::sync::mpsc;

use super::model::CallAttemptId;

/// One typed voice command queued for transmission on the account's owning WebSocket.
#[derive(Clone)]
pub(crate) struct QueuedVoiceCommand {
    /// Connection generation that owns this command; older generations are dropped before send.
    pub generation: Generation,
    /// Call attempt that emitted this command.
    pub attempt: CallAttemptId,
    /// Wire command to serialize and send.
    pub command: Command,
    /// True only for teardown `voice_leave` once `voice_join` was queued for this attempt.
    pub allow_after_end: bool,
}

/// Outcome of attempting to enqueue a voice command onto the bounded FIFO.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnqueueError {
    /// The bounded FIFO is saturated; caller must fail the active call (`CommandOverflow`).
    Full,
    /// No active socket receiver is attached.
    Closed,
}

/// Sender half of the bounded voice command FIFO held by the session's call coordinator.
#[derive(Clone)]
pub(crate) struct VoiceCommandSender {
    tx: mpsc::Sender<QueuedVoiceCommand>,
}

/// Receiver half attached to one `AccountSocket` connection.
pub(crate) struct VoiceCommandReceiver {
    rx: mpsc::Receiver<QueuedVoiceCommand>,
}

/// Creates a bounded FIFO with the requested capacity (minimum 1).
#[must_use]
pub(crate) fn channel(capacity: usize) -> (VoiceCommandSender, VoiceCommandReceiver) {
    let (tx, rx) = mpsc::channel(capacity.max(1));
    (VoiceCommandSender { tx }, VoiceCommandReceiver { rx })
}

impl VoiceCommandSender {
    /// Non-blocking enqueue; never stalls a caller or Tokio worker, returning `Full` on overflow.
    pub(crate) fn try_send(&self, item: QueuedVoiceCommand) -> Result<(), EnqueueError> {
        self.tx.try_send(item).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => EnqueueError::Full,
            mpsc::error::TrySendError::Closed(_) => EnqueueError::Closed,
        })
    }
}

impl VoiceCommandReceiver {
    /// Receives the next queued voice command in strict FIFO order.
    pub(crate) async fn recv(&mut self) -> Option<QueuedVoiceCommand> {
        self.rx.recv().await
    }

    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Option<QueuedVoiceCommand> {
        self.rx.try_recv().ok()
    }
}
