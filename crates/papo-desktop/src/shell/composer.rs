//! The shell's side of the composer: carries out its effects against the
//! SDK (reading and saving drafts, queueing a send, reporting typing), and
//! the field's handlers.
//!
//! Draft writes are ordered by the composer's `seq`: each takes the shared
//! write lock and is dropped when a newer one for its channel has landed,
//! since the bridge runs jobs side by side. Saves and sends go through
//! `Sdk::finish`, so the exit fence lets them land.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use papo_sdk::{DraftBody, OperationKind, OutgoingError, OutgoingMessage, UnixMicros, Uuid};
use rune::Tx;
use tokio::sync::Mutex;

use super::Shell;
use crate::composer::{Effect, Sent};
use crate::sdk_bridge::Sdk;
use crate::timeline::TimelineKey;

/// The last draft write landed for each channel, by the composer's `seq`.
pub(crate) type DraftWrites = Arc<Mutex<HashMap<TimelineKey, u64>>>;

impl Shell {
    /// The field's text changed.
    pub(crate) fn composer_edited(&mut self, tx: &mut Tx) {
        let effects = self.composer.edited(tx.clock());
        self.run_composer(effects, Some(tx));
    }

    /// Send, or the Enter that sends: the text leaves the field and is
    /// queued.
    pub(crate) fn composer_send(&mut self, tx: &mut Tx) {
        match self.composer.send(Uuid::new_v4()) {
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
    /// and the poll ask for one, and both have it.
    fn run_composer(&mut self, effects: Vec<Effect>, mut tx: Option<&mut Tx>) {
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
                let text = match client.session(key.account) {
                    Ok(session) => load_draft(&session, key.channel).await,
                    Err(error) => {
                        log::warn!("a draft was not read: {error}");
                        String::new()
                    }
                };
                updates.send(move |shell| shell.composer.loaded(key, text));
            });
        }
        Effect::Save { key, text, seq } => {
            let writes = Arc::clone(writes);
            sdk.finish(move |client| async move {
                // Held across the write: it orders this channel's drafts.
                let mut written = writes.lock().await;
                if written.get(&key).is_some_and(|&last| last > seq) {
                    return;
                }
                let Ok(session) = client.session(key.account) else {
                    return;
                };
                match save_draft(&session, key.channel, text).await {
                    Ok(()) => {
                        written.insert(key, seq);
                    }
                    Err(error) => log::warn!("a draft was not saved: {error}"),
                }
            });
        }
        Effect::Send {
            key,
            text,
            local,
            seq,
        } => {
            let writes = Arc::clone(writes);
            let updates = sdk.updates();
            sdk.finish(move |client| async move {
                let mut written = writes.lock().await;
                let queued = match client.session(key.account) {
                    Ok(session) => queue_message(&session, key.channel, local, &text)
                        .await
                        .map(|()| session),
                    Err(error) => Err(error.into()),
                };
                let session = match queued {
                    Ok(session) => session,
                    Err(error) => {
                        log::warn!("a message was not queued: {error}");
                        drop(written);
                        updates.send(move |shell| {
                            let effects = shell.composer.send_failed(key, text);
                            shell.run_composer(effects, None);
                            true
                        });
                        return;
                    }
                };
                // The queue holds the message now; the stored draft still
                // holds its text, unless something newer was saved since.
                if written.get(&key).is_some_and(|&last| last > seq) {
                    return;
                }
                match save_draft(&session, key.channel, String::new()).await {
                    Ok(()) => {
                        written.insert(key, seq);
                    }
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

/// The channel's stored draft text; empty when there is none or it could
/// not be read.
async fn load_draft(session: &papo_sdk::Session, channel: Uuid) -> String {
    match session.outgoing().draft(channel).await {
        Ok(draft) => draft
            .map(|draft| draft.body.message.content)
            .unwrap_or_default(),
        Err(error) => {
            log::warn!("a draft was not read: {error}");
            String::new()
        }
    }
}

/// Stores `text` as the channel's draft; empty clears it.
async fn save_draft(
    session: &papo_sdk::Session,
    channel: Uuid,
    text: String,
) -> Result<(), OutgoingError> {
    let body = DraftBody {
        message: OutgoingMessage {
            content: text,
            ..OutgoingMessage::default()
        },
        mentions: Vec::new(),
    };
    session
        .outgoing()
        .save_draft(channel, body, now(), None)
        .await
        .map(|_revision| ())
}

/// Queues `text` as a new message with identity `local`.
async fn queue_message(
    session: &papo_sdk::Session,
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
