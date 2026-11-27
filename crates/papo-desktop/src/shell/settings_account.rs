//! The shell's side of the account's pages (`docs/design.md` §8): a text
//! row edited and saved through the SDK's `change_profile` (one field,
//! merged over what the server holds), Presence through `update_status`,
//! Unblock, and Sign out (`logout`, then `remove_account`). One account call
//! runs at a time.
//!
//! The SDK never retries and publishes no optimistic profile: a row shows
//! the confirmed value, its control waits while the call is in flight, and
//! a failure keeps the old value with one `danger` line and Try again.
//! Owns these transitions; Change password's are `settings_password`'s, the
//! profile and the calls the SDK's.

use papo_sdk::AccountId;
use papo_sdk::operations::profile::{ProfileChange, UpdateStatusRequest};
use rune::{El, TextEdit, Tx, col};

use super::Shell;
use super::profile_card::{Order, PersonFacts, header, sections};
use crate::design::settings::TILE_OUTER_RADIUS;
use crate::design::{Look, SPACE_LG};
use crate::settings::rows::account::AccountFacts;
use crate::settings::text_row::field_key;
use crate::settings::{Presence, RowId, Target};
use crate::strings;

impl Shell {
    /// What the account's pages show: the panel's account, its profile,
    /// presence and blocked people.
    pub(crate) fn account_facts(&self) -> Option<AccountFacts> {
        let (server, account) = self.settings_subject()?;
        let panel = self.settings.as_ref();
        Some(AccountFacts {
            server: server.name.clone(),
            handle: account.username.clone(),
            own: server.own.clone().unwrap_or_default(),
            presence: self
                .people(account.id)
                .and_then(|people| people.presence_mark(account.id.user())),
            blocked: server.blocked.clone(),
            password_changed: panel.is_some_and(|panel| panel.password_changed),
            asking: panel.and_then(|panel| panel.asking),
            notification_policy: server.notification_policy,
        })
    }

    /// Profile's preview (§8: read-only, what others see): the account's
    /// card as others open it, without its actions.
    pub(crate) fn profile_preview(&self, facts: &AccountFacts, look: &Look) -> El {
        let own = &facts.own;
        let person = PersonFacts {
            name: papo_sdk_ui::display_name(own.nickname.as_deref(), Some(&facts.handle))
                .unwrap_or(&facts.handle)
                .to_owned(),
            username: Some(facts.handle.clone()),
            presence: facts.presence,
            status: own.status.clone(),
            roles: own.roles.clone(),
            about: own.about.clone(),
            since: own
                .since
                .as_deref()
                .and_then(|instant| self.local_time.full_date(instant)),
            server: facts.server.clone(),
            pictures: self
                .subject_account()
                .and_then(|account| self.profile_pictures.get(account, account.user()))
                .cloned()
                .unwrap_or_default(),
        };
        col()
            .key("settings-profile-preview")
            .w_full()
            .pb(look.space(SPACE_LG))
            .radius(look.radius(TILE_OUTER_RADIUS))
            // The banner runs to the card's rounded top.
            .clip_rounded(look.radius(TILE_OUTER_RADIUS))
            .fill(look.palette.surface_tile)
            .role("group")
            .label(strings::card_label(&person.name))
            .children(header(&person, true, look))
            .children(sections(&person, Order::Card, look))
    }

    /// The panel's account, once one is signed in.
    pub(super) fn subject_account(&self) -> Option<AccountId> {
        self.settings_subject().map(|(_, account)| account.id)
    }

    /// Profile opened: the account's profile is read back, so its text
    /// rows hold confirmed values and can be edited.
    pub(crate) fn read_own_profile(&mut self) {
        let (Some(account), Some(sdk)) = (self.subject_account(), self.sdk.clone()) else {
            return;
        };
        self.want_pictures(account, account.user());
        sdk.run(move |client| async move {
            let Ok(session) = client.session(account) else {
                return;
            };
            if let Err(error) = session.profiles(&[account.user()]).await {
                log::debug!("the account's profile was not read: {error}");
            }
        });
    }

    /// A text row's ✎: it turns into its field, holding the saved value.
    pub(crate) fn settings_edit(&mut self, row: RowId, tx: &mut Tx) {
        let own = self
            .account_facts()
            .map(|facts| facts.own)
            .unwrap_or_default();
        let value = match row {
            RowId::Nickname => own.nickname,
            RowId::Status => own.status,
            RowId::About => own.about,
            RowId::TypingPhrase => own.typing,
            _ => return,
        };
        if let Some(panel) = self
            .settings
            .as_mut()
            .filter(|panel| panel.waiting.is_none())
        {
            panel.editing = Some((row, TextEdit::new(value.unwrap_or_default())));
            tx.focus(field_key(row), true);
        }
    }

    /// The text row's field, for its editor.
    pub(crate) fn settings_text_mut(&mut self) -> &mut TextEdit {
        let Shell {
            settings,
            search_spare,
            ..
        } = self;
        if let Some((_, edit)) = settings.as_mut().and_then(|panel| panel.editing.as_mut()) {
            edit
        } else {
            *search_spare = TextEdit::new("");
            search_spare
        }
    }

    /// A text row's Cancel, or Escape in it: the row shows its saved value.
    pub(crate) fn settings_text_cancel(&mut self, tx: &mut Tx) {
        if let Some((row, _)) = self
            .settings
            .as_mut()
            .and_then(|panel| panel.editing.take())
        {
            tx.focus(row.key(), true);
        }
    }

    /// A text row's Save, or Enter in it: the field's text is saved, unless
    /// it is too long or unchanged.
    pub(crate) fn settings_text_save(&mut self, tx: &mut Tx) {
        let Some((row, edit)) = self
            .settings
            .as_ref()
            .and_then(|panel| panel.editing.as_ref())
        else {
            return;
        };
        let (row, text) = (*row, edit.text.trim().to_owned());
        let most = match row {
            RowId::Nickname => crate::settings::rows::account::NICKNAME_MOST,
            RowId::Status => crate::settings::rows::account::STATUS_MOST,
            RowId::About => crate::settings::rows::account::ABOUT_MOST,
            _ => crate::settings::rows::account::TYPING_MOST,
        };
        if text.chars().count() > most {
            return;
        }
        let own = self
            .account_facts()
            .map(|facts| facts.own)
            .unwrap_or_default();
        let saved = match row {
            RowId::Nickname => own.nickname,
            RowId::Status => own.status,
            RowId::About => own.about,
            _ => own.typing,
        };
        if saved.unwrap_or_default() == text {
            // Nothing changed: nothing to send.
            self.settings_text_cancel(tx);
            return;
        }
        let mut change = ProfileChange::default();
        match row {
            RowId::Nickname => change.nickname = Some(text),
            RowId::Status => change.status_message = Some(text),
            RowId::About => change.description = Some(text),
            RowId::TypingPhrase => change.typing = Some(text),
            _ => return,
        }
        if let Some(panel) = self.settings.as_mut() {
            panel.editing = None;
        }
        tx.focus(row.key(), true);
        self.save_profile(row, change);
    }

    /// Sends `change` for `row`, which waits until the SDK answers.
    pub(crate) fn save_profile(&mut self, row: RowId, change: ProfileChange) {
        let Some(account) = self.subject_account() else {
            return;
        };
        self.account_call(
            row,
            Target::Profile(change.clone()),
            move |client| async move {
                let session = client.session(account)?;
                session.operations().change_profile(change).await?;
                Ok(())
            },
        );
    }

    /// Presence's menu: the persisted presence is set.
    pub(crate) fn save_presence(&mut self, presence: Presence) {
        let Some(account) = self.subject_account() else {
            return;
        };
        let status = match presence {
            Presence::Available => None,
            Presence::Away => Some("away".to_owned()),
            Presence::Busy => Some("busy".to_owned()),
        };
        self.account_call(
            RowId::Presence,
            Target::Presence(presence),
            move |client| async move {
                let session = client.session(account)?;
                session
                    .operations()
                    .update_status(UpdateStatusRequest { status })
                    .await?;
                Ok(())
            },
        );
    }

    /// Runs `work` for `row`: the row waits, then shows the confirmed value
    /// or, failing, its danger line with Try again saving `target` again.
    pub(super) fn account_call<Work>(
        &mut self,
        row: RowId,
        target: Target,
        work: impl FnOnce(papo_sdk::ClientHandle) -> Work + Send + 'static,
    ) where
        Work: Future<Output = Result<(), AccountCallError>> + Send + 'static,
    {
        let (Some(sdk), Some(panel)) = (self.sdk.clone(), self.settings.as_mut()) else {
            return;
        };
        // One account call at a time: the next waits for this one's answer.
        if panel.waiting.is_some() {
            return;
        }
        panel.waiting = Some(row);
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let done = work(client).await;
            if let Err(error) = &done {
                log::warn!("an account setting was not saved: {error}");
            }
            updates.send(move |shell| {
                let Some(panel) = shell.settings.as_mut() else {
                    return false;
                };
                panel.waiting = None;
                panel.saved(row, target, done.is_ok());
                true
            });
        });
    }

    /// A blocked person's Unblock: at once.
    pub(crate) fn settings_unblock(&mut self, user: papo_sdk::Uuid, _tx: &mut Tx) {
        let Some((server, account)) = self.settings_subject() else {
            return;
        };
        if !server.blocked.iter().any(|person| person.id == user) {
            return;
        }
        let account = account.id;
        self.set_blocked(account, user, false);
    }

    /// Sign out, confirmed in its row: the account's session ends and the
    /// account leaves this device; the panel closes with it.
    pub(crate) fn sign_out_subject(&mut self) {
        let (Some(account), Some(sdk)) = (self.subject_account(), self.sdk.clone()) else {
            return;
        };
        self.close_settings();
        // Through the exit fence: quitting right after still signs out.
        sdk.finish(move |client| async move {
            if let Ok(session) = client.session(account)
                && let Err(error) = session.auth().logout().await
            {
                log::warn!("an account's sign-out was not confirmed: {error}");
            }
            if let Err(error) = client.remove_account(account).await {
                log::warn!("a signed-out account was not removed: {error}");
            }
        });
    }
}

/// Why an account call did not land.
#[derive(Debug)]
pub(crate) enum AccountCallError {
    /// The account's session could not be reached or refused.
    Session(papo_sdk::ClientError),
    /// The SDK refused the operation or its outcome is unknown.
    Operation(papo_sdk::operations::OperationError),
}

impl From<papo_sdk::ClientError> for AccountCallError {
    fn from(error: papo_sdk::ClientError) -> AccountCallError {
        AccountCallError::Session(error)
    }
}

impl From<papo_sdk::operations::OperationError> for AccountCallError {
    fn from(error: papo_sdk::operations::OperationError) -> AccountCallError {
        AccountCallError::Operation(error)
    }
}

impl std::fmt::Display for AccountCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountCallError::Session(error) => error.fmt(f),
            AccountCallError::Operation(error) => error.fmt(f),
        }
    }
}
