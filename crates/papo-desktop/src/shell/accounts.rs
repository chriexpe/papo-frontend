//! The shell's side of the account switch: opening and closing the server
//! pill's accounts, making one the server's active account, and Add account.
//!
//! What the switch shows is `account_switch`'s; which account is active is
//! the SDK registry's. Switching asks the SDK and changes nothing here: the
//! registry's next snapshot moves the pill and the channel list, and each
//! account keeps its own session and cache.

use papo_sdk::AccountId;
use rune::Tx;

use super::Shell;
use crate::account_switch::account_key;
use crate::channel_pane::SWITCH_ACCOUNT_KEY;
use crate::sign_in::{Host, SignIn};

impl Shell {
    /// ⇄: opens the selected server's accounts, with the keyboard on the
    /// shown one, or closes them.
    pub(crate) fn toggle_accounts(&mut self, tx: &mut Tx) {
        if self.nav.accounts_open() {
            self.close_accounts();
            return;
        }
        let Some(server) = self.nav.selected() else {
            return;
        };
        let shown = server.account.as_ref().map(|account| account.id);
        self.nav.open_accounts();
        if let Some(shown) = shown {
            tx.focus(account_key(shown), false);
        }
    }

    /// Escape or a click outside.
    pub(crate) fn close_accounts(&mut self) {
        self.nav.close_accounts();
    }

    /// A row: `account` becomes the selected server's active account, and
    /// one that is signed out gets its sign-in screen. The keyboard goes
    /// back to ⇄.
    pub(crate) fn pick_account(&mut self, account: AccountId, tx: &mut Tx) {
        self.nav.close_accounts();
        tx.focus(SWITCH_ACCOUNT_KEY, false);
        let Some(server) = self.nav.selected() else {
            return;
        };
        if account.server() != server.id {
            return;
        }
        let already = server.account.as_ref().map(|shown| shown.id) == Some(account);
        // Its channels need a working session; without the screen the pane
        // would just go empty.
        self.sign_in_again(account);
        if already {
            return;
        }
        let Some(sdk) = &self.sdk else {
            return;
        };
        // The pill moves when the registry publishes the change; a failure
        // leaves it on the account it showed, which is still the active one.
        sdk.run(move |client| async move {
            if let Err(error) = client
                .set_active_account(account.server(), Some(account))
                .await
            {
                log::warn!("the account was not switched: {error}");
            }
        });
    }

    /// Add account: the account screen for the selected server, in the
    /// popup. The account that signs in there becomes the active one.
    pub(crate) fn add_account(&mut self) {
        self.nav.close_accounts();
        let Some(server) = self.nav.selected().cloned() else {
            return;
        };
        if self.sign_in().is_some() {
            return;
        }
        let icon = self.icons.get(&server.id).cloned();
        let flow = SignIn::adding_account(Host::Popup, &server, icon, &mut self.fields);
        self.open_sign_in(flow, None);
    }
}
