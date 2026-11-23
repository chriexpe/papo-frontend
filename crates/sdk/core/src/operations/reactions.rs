//! Bounded ephemeral reaction taps coalesce before durable admission; Session 9 owns all sends and outcomes.

#[cfg(test)]
mod tests;

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, oneshot};

use crate::{
    ClientError, Generation, MessageTarget, OperationKind, OutgoingError, Session, UnixMicros, Uuid,
};

const COMMAND_CAPACITY: usize = 64;

/// One unambiguous reaction identity. Custom emoji and unicode cannot be supplied together.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ReactionEmoji {
    /// Server-owned custom emoji identity.
    Custom(Uuid),
    /// Unicode sequence; the outgoing/backend limit is sixteen Unicode scalar values.
    Unicode(String),
}

/// Caller-owned desired membership, with an identity for a possible durable operation.
#[derive(Clone)]
pub struct ReactionIntent {
    /// Channel or DM containing the message.
    pub channel: Uuid,
    /// Existing message or an earlier durable local create.
    pub message: MessageTarget,
    /// The canonical custom or unicode identity.
    pub emoji: ReactionEmoji,
    /// Desired membership, never a toggle inferred from stale cached totals.
    pub add: bool,
    /// Caller-generated unique local identity; reuse is never interpreted as retry.
    pub local: Uuid,
    /// Caller-provided Unix microseconds, never used for ordering or matching.
    pub at: UnixMicros,
}

/// A FULL-acknowledged admission. Observe `Session::outgoing` for Sent/Failed/Unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReactionAdmission {
    /// This request's final desired membership was durably queued under this identity.
    Queued(Uuid),
    /// A later adjacent pending request superseded this tap; that identity is durably queued.
    CoalescedInto(Uuid),
}

/// Sanitized ephemeral-admission failure; durable outgoing errors preserve their typed cause.
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReactionError {
    /// The unicode identity violates the established outgoing request contract.
    #[error("invalid reaction input")]
    InvalidInput,
    /// The bounded ephemeral command channel is full; no operation was admitted for this call.
    #[error("reaction command capacity exceeded")]
    Busy,
    /// The account or command worker ended.
    #[error("reaction account closed")]
    Closed,
    /// Reconnect replaced this pending command's connection generation before admission.
    #[error("reaction generation superseded")]
    Superseded,
    /// Durable enqueue did not acknowledge success. No automatic retry is performed.
    #[error("reaction durable admission failed: {0}")]
    Outgoing(Arc<OutgoingError>),
}

/// Clones share one bounded account command lane, never an independent HTTP sender.
#[derive(Clone)]
pub struct ReactionService {
    session: Session,
}

#[derive(Default)]
pub(crate) struct ReactionQueue {
    sender: Mutex<Option<mpsc::Sender<Command>>>,
}
struct Command {
    generation: Generation,
    intent: ReactionIntent,
    reply: oneshot::Sender<Result<ReactionAdmission, ReactionError>>,
}

impl Session {
    /// Returns a reaction coalescer using this account's existing durable outgoing queue.
    #[must_use]
    pub fn reactions(&self) -> ReactionService {
        ReactionService {
            session: self.clone(),
        }
    }
}

impl ReactionService {
    /// Coalesces adjacent not-yet-admitted commands sharing message/emoji/generation.
    /// A bounded drain of currently pending commands replaces clock-based debounce.
    ///
    /// Only a completed FULL enqueue returns `Queued` or `CoalescedInto`, never Sent.
    /// Canceled callers are skipped before admission. Once admission starts, dropping a caller
    /// cannot undo an immutable operation; inspect the account outgoing image to reconcile it.
    /// Different commands remain ordered; already queued intents are never rewritten or retried.
    /// # Errors
    /// Returns invalid input, bounded capacity, account/generation closure, or typed outgoing failure.
    pub async fn set(&self, intent: ReactionIntent) -> Result<ReactionAdmission, ReactionError> {
        if let ReactionEmoji::Unicode(unicode) = &intent.emoji
            && (unicode.is_empty() || unicode.chars().count() > 16)
        {
            return Err(ReactionError::InvalidInput);
        }
        let generation = self.session.generation();
        self.session
            .inner
            .epochs
            .validate(generation)
            .map_err(|error| lifecycle(&error))?;
        let (reply, received) = oneshot::channel();
        let command = Command {
            generation,
            intent,
            reply,
        };
        self.sender()?
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ReactionError::Busy,
                mpsc::error::TrySendError::Closed(_) => ReactionError::Closed,
            })?;
        received.await.map_err(|_| ReactionError::Closed)?
    }

    fn sender(&self) -> Result<mpsc::Sender<Command>, ReactionError> {
        let mut sender = self
            .session
            .inner
            .reaction_commands
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.session.is_closed() {
            return Err(ReactionError::Closed);
        }
        if let Some(sender) = &*sender {
            return Ok(sender.clone());
        }
        let (tx, rx) = mpsc::channel(COMMAND_CAPACITY);
        *sender = Some(tx.clone());
        let session = self.session.clone();
        let token = session.inner.tasks.token();
        self.session.client.inner.runtime.clone().spawn(async move {
            let _token = token;
            run(&session, rx).await;
        });
        Ok(tx)
    }
}

fn lifecycle(error: &ClientError) -> ReactionError {
    match error {
        ClientError::Superseded => ReactionError::Superseded,
        _ => ReactionError::Closed,
    }
}

async fn run(session: &Session, mut receiver: mpsc::Receiver<Command>) {
    loop {
        let first = tokio::select! {
            biased;
            () = session.inner.root.cancelled() => break,
            command = receiver.recv() => match command { Some(command) => command, None => break },
        };
        let mut pending = VecDeque::with_capacity(COMMAND_CAPACITY);
        pending.push_back(first);
        while pending.len() < COMMAND_CAPACITY {
            let Ok(command) = receiver.try_recv() else {
                break;
            };
            pending.push_back(command);
        }
        while let Some(group) = adjacent_group(&mut pending) {
            admit(session, group).await;
        }
    }
}

fn same_key(left: &Command, right: &Command) -> bool {
    left.generation == right.generation
        && left.intent.channel == right.intent.channel
        && left.intent.message == right.intent.message
        && left.intent.emoji == right.intent.emoji
}

fn adjacent_group(pending: &mut VecDeque<Command>) -> Option<Vec<Command>> {
    let first = pending.pop_front()?;
    let mut group = vec![first];
    while pending
        .front()
        .is_some_and(|next| same_key(&group[0], next))
    {
        if let Some(command) = pending.pop_front() {
            group.push(command);
        }
    }
    Some(group)
}

async fn admit(session: &Session, mut group: Vec<Command>) {
    group.retain(|command| !command.reply.is_closed());
    let Some(final_command) = group.last() else {
        return;
    };
    let generation = final_command.generation;
    let local = final_command.intent.local;
    let result = match session.inner.epochs.validate(generation) {
        Err(error) => Err(lifecycle(&error)),
        Ok(()) => enqueue(session, &final_command.intent, generation).await,
    };
    let last = group.len() - 1;
    for (index, command) in group.into_iter().enumerate() {
        let response = result.clone().map(|()| {
            if index == last {
                ReactionAdmission::Queued(local)
            } else {
                ReactionAdmission::CoalescedInto(local)
            }
        });
        let _ = command.reply.send(response);
    }
}

async fn enqueue(
    session: &Session,
    intent: &ReactionIntent,
    generation: Generation,
) -> Result<(), ReactionError> {
    let (emoji_id, unicode) = match &intent.emoji {
        ReactionEmoji::Custom(id) => (Some(*id), None),
        ReactionEmoji::Unicode(value) => (None, Some(value.clone())),
    };
    let kind = OperationKind::Reaction {
        target: intent.message,
        emoji_id,
        unicode,
        add: intent.add,
    };
    session
        .outgoing()
        .enqueue_in_generation(intent.channel, intent.local, kind, intent.at, generation)
        .await
        .map_err(|error| ReactionError::Outgoing(Arc::new(error)))
}
