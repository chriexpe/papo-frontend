//! Someone else's profile card (`docs/design.md` §10), from the shell's
//! side: opening it beside what was clicked (an author's name, a reactor's
//! row) as an open menu's body, drawing it, and what its actions do:
//! Message opens or reopens a direct conversation, Copy ID, Block and
//! Unblock at once, and for moderators Ban, asked inside its row.
//!
//! Owns the card's content and actions; the overlay (its anchor, Escape, a
//! click outside, focus going back) is the menu's, the person and their
//! profile are the SDK's, and no picture is drawn yet (a later slice).

use papo_sdk::Uuid;
use papo_sdk::operations::social::DirectAccess;
use papo_sdk_ui::{DirectMessageId, requested_profile};
use rune::Tx;

use super::Shell;
use crate::menu::{CardView, OpenMenu, PersonAction, Placement};
use crate::notice::Refusal;
use crate::timeline::TimelineKey;

mod view;

use view::{BAN_CANCEL_KEY, BAN_KEY, COPY_ID_KEY, MESSAGE_KEY};
pub(super) use view::{Order, PersonFacts, body, header, sections};

impl Shell {
    /// Opens person `user`'s card, seen from timeline `key`, at
    /// `placement`; `trigger` gets the keyboard back when it closes. Asks
    /// for their profile when the account does not have it yet.
    pub(crate) fn open_card(
        &mut self,
        key: TimelineKey,
        user: Uuid,
        placement: Placement,
        trigger: String,
        tx: &mut Tx,
    ) {
        self.worked_outside();
        let mut menu = OpenMenu::with_entries(
            placement,
            trigger,
            Vec::new(),
            crate::menu::PROFILE_CARD_WIDTH_PT,
        );
        menu.card = Some(CardView {
            key,
            user,
            banning: false,
        });
        self.nav.menu = Some(menu);
        tx.focus(first_key(key, user), true);
        self.load_profile(key, user);
    }

    /// A reactor's row was pressed: the reactions list turns into their
    /// card, which Escape turns back.
    pub(crate) fn reactor_pressed(&mut self, user: Uuid, tx: &mut Tx) {
        let Some(menu) = self.nav.menu.as_mut() else {
            return;
        };
        let Some(view) = &menu.reactions else {
            return;
        };
        let key = view.key;
        menu.card = Some(CardView {
            key,
            user,
            banning: false,
        });
        tx.focus(first_key(key, user), true);
        self.load_profile(key, user);
    }

    /// Escape on a card the reactions list turned into: back to the list.
    /// Returns whether it was one.
    pub(super) fn card_back(&mut self, tx: &mut Tx) -> bool {
        let Some(menu) = self.nav.menu.as_mut() else {
            return false;
        };
        if menu.reactions.is_none() || menu.card.take().is_none() {
            return false;
        }
        tx.focus(crate::menu::MENU_KEY, true);
        true
    }

    /// Asks for person `user`'s profile when timeline `key`'s account does
    /// not have it yet; the SDK publishes it with the account's resources.
    pub(super) fn load_profile(&self, key: TimelineKey, user: Uuid) {
        let known = self
            .shown_timeline(key)
            .and_then(|open| open.resources.as_deref())
            .is_some_and(|resources| requested_profile(resources, user).is_some());
        if !known && let Some(sdk) = &self.sdk {
            sdk.run(move |client| async move {
                let Ok(session) = client.session(key.account) else {
                    return;
                };
                if let Err(error) = session.profiles(&[user]).await {
                    log::debug!("a profile was not read: {error}");
                }
            });
        }
    }

    /// Ban asks inside its row, or stops asking; the keyboard goes to
    /// Cancel, or back to Ban, so it never rests on a row that went.
    pub(super) fn card_ban_asked(&mut self, asking: bool, tx: &mut Tx) {
        if let Some(card) = self.nav.menu.as_mut().and_then(|menu| menu.card.as_mut()) {
            card.banning = asking;
            tx.focus(if asking { BAN_CANCEL_KEY } else { BAN_KEY }, true);
        }
    }

    /// Closes a card whose conversation is no longer shown (a server
    /// removed, a sign-out), which would draw nothing and still hold the
    /// keyboard as a menu. Returns whether the view changed.
    pub(super) fn poll_card(&mut self) -> bool {
        let Some(card) = self.nav.menu.as_ref().and_then(|menu| menu.card.as_ref()) else {
            return false;
        };
        if self.shown_timeline(card.key).is_some() {
            return false;
        }
        self.nav.menu = None;
        true
    }

    /// A card's action on person `user` of timeline `key`'s account; the
    /// card has closed.
    pub(super) fn person_action(
        &mut self,
        key: TimelineKey,
        user: Uuid,
        action: PersonAction,
        tx: &mut Tx,
    ) {
        match action {
            PersonAction::Message => self.message_person(key, user),
            PersonAction::CopyId => tx.copy(user.to_string()),
            PersonAction::Block => self.set_blocked(key, user, true),
            PersonAction::Unblock => self.set_blocked(key, user, false),
            PersonAction::Ban => self.ban(key, user),
            // Kept by `menu_picked`: the card stays for them.
            PersonAction::AskBan | PersonAction::CancelBan => {}
        }
    }

    /// Opens or reopens the direct conversation with `user`, and picks it
    /// once the SDK's list holds it.
    fn message_person(&mut self, key: TimelineKey, user: Uuid) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(key.account) else {
                return;
            };
            let operations = session.operations();
            let opened = operations
                .open_direct_conversation(user)
                .await
                .map(|receipt| Uuid::parse_str(&receipt.value.id).ok());
            let refusal = match opened {
                Ok(Some(channel)) => {
                    let id = DirectMessageId {
                        account: key.account,
                        channel,
                    };
                    updates.send_tx(move |shell, tx| {
                        shell.await_dm(id, tx);
                        true
                    });
                    return;
                }
                Ok(None) => Refusal::OpenDm,
                // A refusal reads as one only when the backend says so.
                Err(error) => {
                    log::warn!("a direct conversation was not opened: {error}");
                    match DirectAccess::proven_by(&error) {
                        Some(DirectAccess::Refused) => Refusal::DmRefused,
                        _ => Refusal::OpenDm,
                    }
                }
            };
            updates.send_tx(move |shell, tx| {
                shell.refuse(refusal, tx);
                true
            });
        });
    }

    pub(super) fn set_blocked(&mut self, key: TimelineKey, user: Uuid, blocked: bool) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let done = match client.session(key.account) {
                Ok(session) => {
                    let operations = session.operations();
                    let sent = if blocked {
                        operations.block_user(user).await
                    } else {
                        operations.unblock_user(user).await
                    };
                    sent.map(|_| ()).map_err(|error| error.to_string())
                }
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = done {
                log::warn!("a block was not changed: {error}");
                let refusal = if blocked {
                    Refusal::Block
                } else {
                    Refusal::Unblock
                };
                updates.send_tx(move |shell, tx| {
                    shell.refuse(refusal, tx);
                    true
                });
            }
        });
    }

    fn ban(&mut self, key: TimelineKey, user: Uuid) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let done = match client.session(key.account) {
                Ok(session) => session
                    .operations()
                    .set_banned(user, true)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = done {
                log::warn!("a ban did not happen: {error}");
                updates.send_tx(|shell, tx| {
                    shell.refuse(Refusal::Ban, tx);
                    true
                });
            }
        });
    }
}

/// The control a card opens on: Message, or Copy ID on your own card,
/// which has no Message.
fn first_key(key: TimelineKey, user: Uuid) -> &'static str {
    if key.account.user() == user {
        COPY_ID_KEY
    } else {
        MESSAGE_KEY
    }
}
