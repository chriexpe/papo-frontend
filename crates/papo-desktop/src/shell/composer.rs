//! The shell's side of the composer: carries out its effects against the
//! SDK (reading and saving drafts, queueing a send, reporting typing), and
//! the field's handlers.
//!
//! The bridge runs jobs side by side, so draft writes are ordered by the
//! composer's `seq`, one lane per channel: a write waits for the channel's
//! lane and is dropped when a newer one has landed ([`Lane`]). Saves and
//! sends go through `Sdk::finish`, so the exit fence lets them land; what
//! fails comes back to the composer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use papo_sdk::{
    DraftBody, OperationKind, OutgoingError, OutgoingMessage, Session, UnixMicros, Uuid,
};
use papo_sdk_ui::message_content::{DecodedComposerMentions, decode_composer_mentions};
use rune::Tx;
use tokio::sync::Mutex;

use super::Shell;
use crate::composer::{Effect, Sent};
use crate::sdk_bridge::{Sdk, Updates};
use crate::timeline::TimelineKey;

/// One lane of draft writes per channel, shared with the bridge's jobs.
#[derive(Clone, Default)]
pub(crate) struct DraftWrites(Arc<StdMutex<HashMap<TimelineKey, Arc<Mutex<Lane>>>>>);

impl DraftWrites {
    /// The lane of `key`'s writes. The map's lock is held only to find it.
    fn lane(&self, key: TimelineKey) -> Arc<Mutex<Lane>> {
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
struct Lane {
    landed: u64,
}

impl Lane {
    /// Whether a write with order `seq` may still land: nothing newer has.
    fn admits(&self, seq: u64) -> bool {
        self.landed < seq
    }

    fn landed(&mut self, seq: u64) {
        self.landed = self.landed.max(seq);
    }
}

impl Shell {
    /// The field's text changed.
    pub(crate) fn composer_edited(&mut self, tx: &mut Tx) {
        let effects = self.composer.edited(tx.clock());
        self.run_composer(effects, Some(tx));
        self.refresh_picker();
    }

    /// Send, or the Enter that sends: the text leaves the field and is
    /// queued.
    pub(crate) fn composer_send(&mut self, tx: &mut Tx) {
        let local = (self.local_ids)();
        match self.composer.send(local) {
            Sent::Queued(effect) => self.run_composer(vec![effect], Some(tx)),
            Sent::Nothing | Sent::TooLong => {}
        }
    }

    /// The Enter that adds a line, when "Enter to send" is off.
    pub(crate) fn composer_new_line(&mut self, tx: &mut Tx) {
        self.composer.field_mut().insert("\n");
        self.composer_edited(tx);
    }

    /// Shows the composer of the channel the navigation shows, saving the
    /// one left.
    pub(super) fn sync_composer(&mut self) {
        let shown = self.wanted_timeline();
        let effects = self.composer.show(shown);
        self.run_composer(effects, None);
    }

    /// The window's poll: saves the drafts whose pause is over. Returns
    /// whether the view changed (it never does).
    pub(super) fn poll_composer(&mut self, clock_s: f64, tx: &mut Tx) -> bool {
        let effects = self.composer.poll(clock_s);
        self.run_composer(effects, Some(tx));
        false
    }

    /// The window hides or closes: every unsaved draft is saved now.
    pub(super) fn flush_composer(&mut self) {
        let effects = self.composer.flush();
        self.run_composer(effects, None);
    }

    /// Carries out the composer's effects. A wake needs `tx`; only edits
    /// and the poll ask for one, and both have it. Without an SDK (before
    /// it opens; headless tests) nothing reaches the bridge: no channel
    /// shows before the SDK lists one.
    pub(super) fn run_composer(&mut self, effects: Vec<Effect>, mut tx: Option<&mut Tx>) {
        for effect in effects {
            match effect {
                Effect::WakeAfter(after_s) => {
                    if let Some(tx) = tx.as_deref_mut() {
                        tx.wake_after(Duration::from_secs_f64(after_s.max(0.0)));
                    }
                }
                effect => {
                    if let Some(sdk) = &self.sdk {
                        run_effect(sdk, &self.draft_writes, effect);
                    }
                }
            }
        }
    }
}

/// Sends one SDK effect to the bridge.
fn run_effect(sdk: &Sdk<Shell>, writes: &DraftWrites, effect: Effect) {
    match effect {
        Effect::Load(key) => {
            let updates = sdk.updates();
            sdk.run(move |client| async move {
                let read = match client.session(key.account) {
                    Ok(session) => session.outgoing().draft(key.channel).await.map(|draft| {
                        let wire = draft.map(|draft| draft.body.message.content);
                        decode(Some(&session), &wire.unwrap_or_default())
                    }),
                    Err(error) => Err(error.into()),
                };
                match read {
                    Ok(decoded) => {
                        updates.send(move |shell| shell.composer.loaded(key, decoded));
                    }
                    Err(error) => {
                        log::warn!("a draft was not read: {error}");
                        updates.send(move |shell| shell.composer.load_failed(key));
                    }
                }
            });
        }
        Effect::Save {
            key,
            text,
            mentions,
            seq,
        } => {
            let lane = writes.lane(key);
            let updates = sdk.updates();
            sdk.finish(move |client| async move {
                let mut lane = lane.lock().await;
                if !lane.admits(seq) {
                    return;
                }
                let saved = match client.session(key.account) {
                    Ok(session) => save_draft(&session, key.channel, text, mentions).await,
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
        Effect::Send {
            key,
            text,
            local,
            seq,
        } => {
            let lane = writes.lane(key);
            let updates = sdk.updates();
            sdk.finish(move |client| async move {
                // Held from the queueing to the clear: a newer save waits,
                // and lands after the clear.
                let mut lane = lane.lock().await;
                let session = match client.session(key.account) {
                    Ok(session) => session,
                    Err(error) => {
                        return send_failed(&updates, key, decode(None, &text), &error.into());
                    }
                };
                if let Err(error) = queue_message(&session, key.channel, local, &text).await {
                    return send_failed(&updates, key, decode(Some(&session), &text), &error);
                }
                // The queue holds the message; the stored draft may still
                // hold its text, unless something newer was saved since.
                if !lane.admits(seq) {
                    return;
                }
                match save_draft(&session, key.channel, String::new(), Vec::new()).await {
                    Ok(()) => lane.landed(seq),
                    Err(error) => log::warn!("a sent draft was not cleared: {error}"),
                }
            });
        }
        Effect::ReportTyping(key) => sdk.run(move |client| async move {
            let reported = client
                .session(key.account)
                .and_then(|session| session.report_typing(key.channel));
            // Best effort: offline, nobody hears it anyway.
            if let Err(error) = reported {
                log::debug!("typing was not reported: {error}");
            }
        }),
        Effect::WakeAfter(_) => {}
    }
}

/// The SDK did not queue `text`: it goes back in the field. A send refused
/// as the window closes cannot come back; its text survives only as far as
/// the stored draft held it (saved up to a second before).
fn send_failed(
    updates: &Updates<Shell>,
    key: TimelineKey,
    decoded: DecodedComposerMentions,
    error: &OutgoingError,
) {
    log::warn!("a message was not queued: {error}");
    updates.send(move |shell| {
        let effects = shell.composer.send_failed(key, decoded);
        shell.run_composer(effects, None);
        true
    });
}

/// `wire` (a draft or message as stored) as the field shows it: each
/// mention token the person's name, from `session`'s roster. A person the
/// account does not know keeps the token.
fn decode(session: Option<&Session>, wire: &str) -> DecodedComposerMentions {
    let snapshot = session.map(|session| session.subscribe_resources().borrow().clone());
    decode_composer_mentions(wire, |id| {
        let user = snapshot.as_deref()?.users.get(id)?;
        user.nickname.as_deref().or(user.username.as_deref())
    })
}

/// Stores `text` (its wire form) as the channel's draft, with the people it
/// mentions; empty clears it.
async fn save_draft(
    session: &Session,
    channel: Uuid,
    text: String,
    mentions: Vec<Uuid>,
) -> Result<(), OutgoingError> {
    let body = DraftBody {
        message: OutgoingMessage {
            content: text,
            ..OutgoingMessage::default()
        },
        mentions,
    };
    session
        .outgoing()
        .save_draft(channel, body, now(), None)
        .await
        .map(|_revision| ())
}

/// Queues `text` as a new message with identity `local`.
async fn queue_message(
    session: &Session,
    channel: Uuid,
    local: Uuid,
    text: &str,
) -> Result<(), OutgoingError> {
    let message = OutgoingMessage {
        content: text.to_owned(),
        ..OutgoingMessage::default()
    };
    session
        .outgoing()
        .enqueue(channel, local, OperationKind::Message(message), now())
        .await
}

/// The wall clock, as the SDK stamps drafts and intents (never to order
/// them).
fn now() -> UnixMicros {
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
