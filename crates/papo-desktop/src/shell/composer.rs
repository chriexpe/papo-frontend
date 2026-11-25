//! The shell's side of the composer: carries out its effects against the
//! SDK (reading and saving drafts, queueing a send, reporting typing), and
//! the field's handlers.
//!
//! The bridge runs jobs side by side, so draft writes are ordered by the
//! composer's `seq`, one lane per channel: a write waits for the channel's
//! lane and is dropped when a newer one has landed (`draft_writes`). Saves and
//! sends go through `Sdk::finish`, so the exit fence lets them land; what
//! fails comes back to the composer.

use std::time::Duration;

use papo_sdk::{MessageTarget, OutgoingError, OutgoingMessage, Session, Uuid};
use papo_sdk_ui::display_name;
use papo_sdk_ui::message_content::{DecodedComposerMentions, decode_composer_mentions};
use rune::Tx;

use super::Shell;
use super::draft_writes::{DraftWrites, Stored, queue_message, save_draft, save_effect};
use super::staged_files::{pictures, restored_files};
use crate::composer::tray::Tile;
use crate::composer::{Effect, Sent};
use crate::sdk_bridge::{Sdk, Updates};
use crate::timeline::TimelineKey;

impl Shell {
    /// The field's text changed.
    pub(crate) fn composer_edited(&mut self, tx: &mut Tx) {
        // Typing is work outside a header panel.
        self.worked_outside();
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
            Sent::Edited { edit, save } => {
                let effects = std::iter::once(edit).chain(save).collect();
                self.run_composer(effects, Some(tx));
            }
            Sent::DeleteInstead { key, id } => self.ask_delete(key, id),
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
        // A confirmation is about the channel it was asked in.
        if self.deleting.is_some_and(|ask| Some(ask.key) != shown) {
            self.deleting = None;
        }
        let effects = self.composer.show(shown);
        self.run_composer(effects, None);
        // What was drawn of another channel's tray means nothing here.
        let ids = self
            .composer
            .shown()
            .map(|(_, draft)| draft.tray.ids())
            .unwrap_or_default();
        self.tray_ui.retain(&ids);
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
                        let mention = self.conversations.settings().mention_when_replying;
                        run_effect(sdk, &self.draft_writes, effect, mention);
                    }
                }
            }
        }
    }
}

/// Sends one SDK effect to the bridge. `mention`: where a reply's @ switch
/// starts, for a reply that comes back.
fn run_effect(sdk: &Sdk<Shell>, writes: &DraftWrites, effect: Effect, mention: bool) {
    match effect {
        Effect::Load(key) => load_draft(sdk, key, mention),
        Effect::Save {
            key,
            text,
            mentions,
            reply_to,
            tiles,
            seq,
        } => {
            let draft = Stored {
                text,
                mentions,
                reply_to,
                attachments: tiles.iter().filter_map(Tile::stored).collect(),
            };
            save_effect(sdk, writes, key, draft, seq);
        }
        Effect::Send {
            key,
            text,
            reply_to,
            tiles,
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
                        let failed = Failed {
                            decoded: decode(None, &text),
                            reply_to,
                            mention,
                            tiles,
                        };
                        return send_failed(&updates, key, failed, &error.into());
                    }
                };
                let attachments = tiles.iter().map(Tile::outgoing).collect();
                let message = OutgoingMessage {
                    content: text.clone(),
                    reply_to: reply_to.map(MessageTarget::Remote),
                    attachments,
                    ..OutgoingMessage::default()
                };
                let queued = queue_message(&session, key.channel, local, message).await;
                if let Err(error) = queued {
                    let failed = Failed {
                        decoded: decode(Some(&session), &text),
                        reply_to,
                        mention,
                        tiles,
                    };
                    return send_failed(&updates, key, failed, &error);
                }
                // The queue holds the message; the stored draft may still
                // hold its text, unless something newer was saved since.
                if !lane.admits(seq) {
                    return;
                }
                match save_draft(&session, key.channel, Stored::default()).await {
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
        Effect::Edit {
            key,
            id,
            text,
            local,
        } => super::replies::queue_edit(sdk, key, id, text, local),
        Effect::WakeAfter(_) => {}
    }
}

/// Reads `key`'s stored draft and its reply, and puts them in the field.
/// `mention`: where a restored reply's @ switch starts.
fn load_draft(sdk: &Sdk<Shell>, key: TimelineKey, mention: bool) {
    let updates = sdk.updates();
    sdk.run(move |client| async move {
        let read = match client.session(key.account) {
            Ok(session) => match session.outgoing().draft(key.channel).await {
                Ok(draft) => {
                    let (wire, reply_to, attachments) =
                        draft.map_or((String::new(), None, Vec::new()), |draft| {
                            let message = draft.body.message;
                            (message.content, message.reply_to, message.attachments)
                        });
                    // A reply is to a message the server knows; a local
                    // one never reaches a draft.
                    let reply_to = reply_to.and_then(|target| match target {
                        MessageTarget::Remote(id) => Some(id),
                        MessageTarget::Local(_) => None,
                    });
                    let files = restored_files(&session, attachments).await;
                    Ok((decode(Some(&session), &wire), reply_to, files))
                }
                Err(error) => Err(error),
            },
            Err(error) => Err(error.into()),
        };
        match read {
            Ok((decoded, reply_to, files)) => {
                updates.send(move |shell| {
                    let tiles: Vec<Tile> = files
                        .into_iter()
                        .map(|file| shell.restored_tile(file))
                        .collect();
                    let pictures = pictures(&tiles);
                    let loaded = shell
                        .composer
                        .loaded(key, decoded, reply_to, mention, tiles);
                    // Files attached while it loaded are saved with it now.
                    let effects = shell.composer.save_unsaved(key);
                    shell.run_composer(effects, None);
                    shell.read_thumbnails(key, pictures);
                    shell.resolve_replies();
                    loaded
                });
            }
            Err(error) => {
                log::warn!("a draft was not read: {error}");
                updates.send(move |shell| shell.composer.load_failed(key));
            }
        }
    });
}

/// A message the SDK did not queue, to put back in the field and tray.
struct Failed {
    decoded: DecodedComposerMentions,
    reply_to: Option<Uuid>,
    /// Where the @ switch starts if its reply comes back.
    mention: bool,
    tiles: Vec<Tile>,
}

/// The SDK did not queue `text`: it goes back in the field. A send refused
/// as the window closes cannot come back; its text survives only as far as
/// the stored draft held it (saved up to a second before).
fn send_failed(updates: &Updates<Shell>, key: TimelineKey, failed: Failed, error: &OutgoingError) {
    log::warn!("a message was not queued: {error}");
    let Failed {
        decoded,
        reply_to,
        mention,
        tiles,
    } = failed;
    updates.send(move |shell| {
        // Their watch ended when Send took them: a copy still running is
        // followed again (one already over answers at once).
        shell.watch_returned(key, &tiles);
        let effects = shell
            .composer
            .send_failed(key, decoded, reply_to, mention, tiles);
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
        display_name(user.nickname.as_deref(), user.username.as_deref())
    })
}
