//! Change password, from the shell's side (`docs/design.md` §8, Privacy
//! and security): the form's fields, Show and "Stay signed in on this
//! device"; Save checks §7's rules and the repeat first, then the account
//! prepares its own reset and sets the new password. The backend signs
//! every session of the account out, this one too; with the box ticked this
//! device signs in again with the new password.
//!
//! Owns these transitions; the form's look is `settings::password_form`'s,
//! the calls the SDK's.

use papo_sdk::{AccountId, ChangePasswordRequest, LoginRequest};
use rune::{TextEdit, Tx};

use super::Shell;
use super::settings_account::AccountCallError;
use crate::settings::password_form::NEW_PASSWORD_KEY;
use crate::settings::{PageId, PasswordProblem, RowId, Target};
use crate::sign_in::password_rules_met;

impl Shell {
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
) -> Result<(), AccountCallError> {
    let session = client.session(account)?;
    session
        .operations()
        .prepare_password_reset(account.user())
        .await?;
    session
        .auth()
        .change_password(ChangePasswordRequest {
            password: password.clone(),
        })
        .await?;
    let Some(username) = stay else {
        return Ok(());
    };
    // The password changed whatever happens next: a sign-in that fails
    // leaves this device to ask for it, as an ended session does.
    match client.auth(account.server()) {
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
