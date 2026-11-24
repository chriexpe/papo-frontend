//! Draft writes and queueing, in order: one lane per channel, so a write
//! waits for the channel's lane and is dropped when a newer one has landed
//! ([`Lane`]); what one write stores ([`Stored`]); the SDK calls that save
//! a draft and queue a message; the wall clock the SDK stamps them with.
//! The composer decides what to write (`shell/composer.rs`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use papo_sdk::{
    DraftBody, MessageTarget, OperationKind, OutgoingAttachment, OutgoingError, OutgoingMessage,
    Session, UnixMicros, Uuid,
};
use tokio::sync::Mutex;

use super::Shell;
use crate::sdk_bridge::Sdk;
use crate::timeline::TimelineKey;

/// One lane of draft writes per channel, shared with the bridge's jobs.
#[derive(Clone, Default)]
pub(crate) struct DraftWrites(Arc<StdMutex<HashMap<TimelineKey, Arc<Mutex<Lane>>>>>);

impl DraftWrites {
    /// The lane of `key`'s writes. The map's lock is held only to find it.
    pub(super) fn lane(&self, key: TimelineKey) -> Arc<Mutex<Lane>> {
        let mut lanes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(lanes.entry(key).or_default())
    }

    /// Forgets the lanes of accounts no longer saved.
    pub(super) fn retain_accounts(&self, keep: impl Fn(papo_sdk::AccountId) -> bool) {
        let mut lanes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        lanes.retain(|key, _| keep(key.account));
    }
}

/// One channel's write order: the order of the last write that landed.
/// Held across a write, so a check and its write are one step.
#[derive(Debug, Default)]
pub(super) struct Lane {
    landed: u64,
}

impl Lane {
    /// Whether a write with order `seq` may still land: nothing newer has.
    pub(super) fn admits(&self, seq: u64) -> bool {
        self.landed < seq
    }

    pub(super) fn landed(&mut self, seq: u64) {
        self.landed = self.landed.max(seq);
    }
}

/// Saves `draft` as `key`'s stored draft in its lane, unless a newer write
/// (`seq`) landed first; a failure comes back to the composer.
pub(super) fn save_effect(
    sdk: &Sdk<Shell>,
    writes: &DraftWrites,
    key: TimelineKey,
    draft: Stored,
    seq: u64,
) {
    let lane = writes.lane(key);
    let updates = sdk.updates();
    sdk.finish(move |client| async move {
        let mut lane = lane.lock().await;
        if !lane.admits(seq) {
            return;
        }
        let saved = match client.session(key.account) {
            Ok(session) => save_draft(&session, key.channel, draft).await,
            Err(error) => Err(error.into()),
        };
        match saved {
            Ok(()) => lane.landed(seq),
            Err(error) => {
                log::warn!("a draft was not saved: {error}");
                updates.send(move |shell| shell.composer.save_failed(key, seq));
            }
        }
    });
}

/// What one draft write stores: the text (its wire form), the people it
/// mentions, the message it replies to and the staged files; empty clears
/// the draft.
#[derive(Default)]
pub(super) struct Stored {
    pub(super) text: String,
    pub(super) mentions: Vec<Uuid>,
    pub(super) reply_to: Option<Uuid>,
    pub(super) attachments: Vec<OutgoingAttachment>,
}

/// Stores `draft` as the channel's draft.
pub(super) async fn save_draft(
    session: &Session,
    channel: Uuid,
    draft: Stored,
) -> Result<(), OutgoingError> {
    let body = DraftBody {
        message: OutgoingMessage {
            content: draft.text,
            reply_to: draft.reply_to.map(MessageTarget::Remote),
            attachments: draft.attachments,
            ..OutgoingMessage::default()
        },
        mentions: draft.mentions,
    };
    session
        .outgoing()
        .save_draft(channel, body, now(), None)
        .await
        .map(|_revision| ())
}

/// Queues `message` with identity `local`. "Mention when replying" is not
/// sent: the backend does not store it yet (the switch is a mock).
pub(super) async fn queue_message(
    session: &Session,
    channel: Uuid,
    local: Uuid,
    message: OutgoingMessage,
) -> Result<(), OutgoingError> {
    session
        .outgoing()
        .enqueue(channel, local, OperationKind::Message(message), now())
        .await
}

/// The wall clock, as the SDK stamps drafts and intents (never to order
/// them).
pub(super) fn now() -> UnixMicros {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_micros()).unwrap_or(i64::MAX)
        });
    UnixMicros(micros)
}

#[cfg(test)]
mod tests {
    use super::Lane;

    /// A lane that has seen the writes `landed`, in that order.
    fn after(landed: &[u64]) -> Lane {
        let mut lane = Lane::default();
        for &seq in landed {
            lane.landed(seq);
        }
        lane
    }

    #[test]
    fn a_save_older_than_a_landed_send_is_dropped() {
        assert!(!after(&[5]).admits(4));
    }

    #[test]
    fn a_newer_save_before_the_send_s_clear_skips_the_clear() {
        assert!(
            !after(&[6]).admits(5),
            "the clear would erase what was typed since"
        );
    }

    #[test]
    fn a_failed_send_lets_the_older_save_land() {
        // The send never marked the lane.
        assert!(after(&[3]).admits(4));
    }

    #[test]
    fn landing_out_of_order_keeps_the_newest() {
        let lane = after(&[7, 6]);
        assert!(!lane.admits(7));
        assert!(lane.admits(8));
    }
}
