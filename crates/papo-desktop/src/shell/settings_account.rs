//! The shell's side of the account's pages (`docs/design.md` §8): a text
//! row edited and saved through the SDK's `change_profile` (one field,
//! merged over what the server holds), Presence through `update_status`,
//! Unblock, Change password (the account prepares its own reset, then sets
//! the new password), and Sign out (`logout`, then `remove_account`).
//!
//! The SDK never retries and publishes no optimistic profile: a row shows
//! the confirmed value, its control waits while the call is in flight, and
//! a failure keeps the old value with one `danger` line and Try again.
//! Owns these transitions; the profile and the calls are the SDK's.

use papo_sdk::operations::profile::{ProfileChange, UpdateStatusRequest};
use papo_sdk::{AccountId, ChangePasswordRequest, LoginRequest};
use rune::{El, TextEdit, Tx, col};

use super::Shell;
use super::profile_card::{Order, PersonFacts, header, sections};
use crate::design::settings::TILE_OUTER_RADIUS;
use crate::design::{Look, SPACE_LG};
use crate::settings::password_form::NEW_PASSWORD_KEY;
use crate::settings::rows::account::AccountFacts;
use crate::settings::text_row::field_key;
use crate::settings::{PageId, PasswordProblem, Presence, RowId, Target};
use crate::sign_in::password_rules_met;
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
        };
        col()
            .key("settings-profile-preview")
            .w_full()
            .pb(look.space(SPACE_LG))
            .radius(look.radius(TILE_OUTER_RADIUS))
            .fill(look.palette.surface_tile)
            .role("group")
            .label(strings::card_label(&person.name))
            .children(header(&person, true, look))
            .children(sections(&person, Order::Card, look))
    }

    /// The panel's account, once one is signed in.
    fn subject_account(&self) -> Option<AccountId> {
        self.settings_subject().map(|(_, account)| account.id)
    }

    /// Profile opened: the account's profile is read back, so its text
    /// rows hold confirmed values and can be edited.
    pub(crate) fn read_own_profile(&self) {
        let (Some(account), Some(sdk)) = (self.subject_account(), &self.sdk) else {
            return;
        };
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
        if let Some(panel) = self.settings.as_mut() {
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
                let session = client.session(account).map_err(|error| error.to_string())?;
                session
                    .operations()
                    .change_profile(change)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
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
                let session = client.session(account).map_err(|error| error.to_string())?;
                session
                    .operations()
                    .update_status(UpdateStatusRequest { status })
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            },
        );
    }

    /// Runs `work` for `row`: the row waits, then shows the confirmed value
    /// or, failing, its danger line with Try again saving `target` again.
    fn account_call<Work>(
        &mut self,
        row: RowId,
        target: Target,
        work: impl FnOnce(papo_sdk::ClientHandle) -> Work + Send + 'static,
    ) where
        Work: Future<Output = Result<(), String>> + Send + 'static,
    {
        let (Some(sdk), Some(panel)) = (self.sdk.clone(), self.settings.as_mut()) else {
            return;
        };
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
    pub(crate) fn settings_unblock(&mut self, index: usize, _tx: &mut Tx) {
        let Some((server, account)) = self.settings_subject() else {
            return;
        };
        let Some(person) = server.blocked.get(index) else {
            return;
        };
        let (account, user) = (account.id, person.id);
        self.set_blocked(account, user, false);
    }

    /// Change password's fields, for their editors.
    pub(crate) fn settings_new_password_mut(&mut self) -> &mut TextEdit {
        self.password_field(true)
    }

    pub(crate) fn settings_repeat_password_mut(&mut self) -> &mut TextEdit {
        self.password_field(false)
    }

    fn password_field(&mut self, new: bool) -> &mut TextEdit {
        let Shell {
            settings,
            search_spare,
            ..
        } = self;
        match settings.as_mut() {
            Some(panel) if new => &mut panel.password.new,
            Some(panel) => &mut panel.password.repeat,
            None => {
                *search_spare = TextEdit::new("");
                search_spare
            }
        }
    }

    /// Change password's "Stay signed in on this device".
    pub(crate) fn settings_password_stay(&mut self) {
        if let Some(panel) = self.settings.as_mut() {
            panel.password.stay = !panel.password.stay;
        }
    }

    /// Change password's Show or Hide.
    pub(crate) fn settings_password_reveal(&mut self) {
        if let Some(panel) = self.settings.as_mut() {
            panel.password.shown = !panel.password.shown;
        }
    }

    /// Change password's Save: §7's rules and the repeat are checked first,
    /// so a password the backend would refuse never flags the account for a
    /// reset; then the account prepares its own reset and sets it.
    pub(crate) fn settings_password_save(&mut self, tx: &mut Tx) {
        let account = self.subject_account();
        let username = self
            .settings_subject()
            .map(|(_, account)| account.username.clone());
        let sdk = self.sdk.clone();
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        if panel.waiting.is_some() {
            return;
        }
        let password = panel.password.new.text.clone();
        // Signing in again needs the username; without it this device signs
        // out with the rest.
        let stay = panel.password.stay.then_some(username).flatten();
        let problem = if !password_rules_met(&password) {
            Some(PasswordProblem::Weak)
        } else if password != panel.password.repeat.text {
            Some(PasswordProblem::Mismatch)
        } else {
            None
        };
        panel.password.problem = problem;
        if problem.is_some() {
            tx.focus(NEW_PASSWORD_KEY, true);
            return;
        }
        let (Some(account), Some(sdk)) = (account, sdk) else {
            return;
        };
        panel.waiting = Some(RowId::PasswordForm);
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let changed = change_password(&client, account, password, stay).await;
            if let Err(error) = &changed {
                log::warn!("the password was not changed: {error}");
            }
            updates.send_tx(move |shell, tx| {
                shell.password_answered(changed.is_ok(), tx);
                true
            });
        });
    }

    fn password_answered(&mut self, changed: bool, tx: &mut Tx) {
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        panel.waiting = None;
        if changed {
            panel.password_changed = true;
            panel.go(PageId::Privacy);
            tx.focus(RowId::Password.key(), true);
        } else {
            panel.saved(RowId::PasswordForm, Target::Password, false);
        }
    }

    /// Sign out, confirmed in its row: the account's session ends and the
    /// account leaves this device; the panel closes with it.
    pub(crate) fn sign_out_subject(&mut self) {
        let (Some(account), Some(sdk)) = (self.subject_account(), self.sdk.clone()) else {
            return;
        };
        self.close_settings();
        sdk.run(move |client| async move {
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

/// The account prepares its own reset (the backend flags it), then sets
/// `password`. The backend then signs every session of the account out,
/// this one too; with `stay` (its username) this device signs in again with
/// the new password, which replaces its session in place.
async fn change_password(
    client: &papo_sdk::ClientHandle,
    account: AccountId,
    password: String,
    stay: Option<String>,
) -> Result<(), String> {
    let session = client.session(account).map_err(|error| error.to_string())?;
    session
        .operations()
        .prepare_password_reset(account.user())
        .await
        .map_err(|error| error.to_string())?;
    session
        .auth()
        .change_password(ChangePasswordRequest {
            password: password.clone(),
        })
        .await
        .map_err(|error| error.to_string())?;
    let Some(username) = stay else {
        return Ok(());
    };
    // The password changed whatever happens next: a sign-in that fails
    // leaves this device to ask for it, as an ended session does.
    let signed_in = client
        .auth(account.server())
        .map_err(|error| error.to_string());
    match signed_in {
        Ok(auth) => {
            if let Err(error) = auth
                .login(LoginRequest { username, password }, crate::sign_in::now())
                .await
            {
                log::warn!("this device did not sign in again after the password change: {error}");
            }
        }
        Err(error) => log::warn!("this device did not sign in again: {error}"),
    }
    Ok(())
}
