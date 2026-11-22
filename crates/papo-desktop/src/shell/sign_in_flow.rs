//! The shell's side of the server and account screens: which screen is
//! open and in which host, offering a lapsed account's sign-in once, closing
//! and finishing the flow, and the handles and request slots the flow's SDK
//! calls use. The screens themselves are `sign_in`'s.

use std::sync::Arc;

use papo_sdk::{AccountId, AuthService, ServerId};

use super::{Shell, Stage};
use crate::sdk_bridge::{RequestSlot, Sdk};
use crate::sign_in::{self, Event, Fields, Host, SignIn};

impl Shell {
    /// With no screens open, signing in again for the first signed-out
    /// account not offered yet. Never the first-run server step: right after
    /// a sign-in the summary may not count the new account yet.
    pub(super) fn offer_sign_in_again(&mut self) {
        let Stage::Running {
            accounts,
            flow: None,
            offered,
            ..
        } = &mut self.stage
        else {
            return;
        };
        let host = if accounts.none_usable() {
            Host::Window
        } else {
            Host::Popup
        };
        let pending = accounts
            .signed_out
            .iter()
            .find(|account| !account.signing_in && !offered.contains(&account.id))
            .cloned();
        if let Some(account) = pending {
            offered.push(account.id);
            let (flow, discover) = SignIn::renewing(host, &account, &mut self.fields);
            self.open_sign_in(flow, Some(discover));
        }
    }

    /// Signs `account` in again, in the popup, when it is signed out and no
    /// screen is open: picked in the server pill's accounts. Returns whether
    /// it was signed out.
    pub(crate) fn sign_in_again(&mut self, account: AccountId) -> bool {
        let Stage::Running {
            accounts,
            flow: None,
            offered,
            ..
        } = &mut self.stage
        else {
            return false;
        };
        let Some(lapsed) = accounts
            .signed_out
            .iter()
            .find(|lapsed| lapsed.id == account)
            .cloned()
        else {
            return false;
        };
        // Closing this screen must not bring it straight back.
        if !offered.contains(&account) {
            offered.push(account);
        }
        let (flow, discover) = SignIn::renewing(Host::Popup, &lapsed, &mut self.fields);
        self.open_sign_in(flow, Some(discover));
        true
    }

    /// Opens the add-server screen in the popup: the rail's +.
    pub(crate) fn add_server(&mut self) {
        if self.sign_in().is_none() && matches!(self.stage, Stage::Running { .. }) {
            let flow = SignIn::new(Host::Popup, &mut self.fields);
            self.open_sign_in(flow, None);
        }
    }

    pub(super) fn open_sign_in(&mut self, flow: SignIn, first: Option<sign_in::Effect>) {
        let Stage::Running { flow: open, .. } = &mut self.stage else {
            return;
        };
        *open = Some(Box::new(flow));
        self.nav.dismiss_overlays();
        if let Some(effect) = first {
            let mut effects = rune::machine::Effects::none();
            effects.push(effect);
            sign_in::run(self, effects);
        }
    }

    /// The person closes the screens (✕, Escape). A server the flow leaves
    /// without an account is removed.
    pub(crate) fn close_sign_in(&mut self) {
        let left = self.sign_in().and_then(SignIn::abandon);
        if let Some(server) = left {
            sign_in::remove_left_server(self, server);
        }
        self.end_sign_in();
    }

    /// Ends the flow: answers still on their way are dropped and the typed
    /// passwords wiped. Then the next account waiting to sign in again, if
    /// any, gets its screen.
    pub(crate) fn end_sign_in(&mut self) {
        let Stage::Running { flow, .. } = &mut self.stage else {
            return;
        };
        if flow.take().is_none() {
            return;
        }
        self.fields = Fields::default();
        self.sign_in_requests.issue();
        self.server_card_requests.issue();
        self.composer.focus();
        self.offer_sign_in_again();
    }

    /// Signed in to `server`: the screens close and the pill names it. A
    /// server just added is selected; signing in again keeps the selection.
    pub(crate) fn finish_sign_in(&mut self, server: ServerId, name: String) {
        let added = self.sign_in().is_some_and(|flow| !flow.is_renewing());
        self.end_sign_in();
        if added {
            self.nav.select_server(server);
            self.sync_timeline(None);
        }
        if let Stage::Running { connected, .. } = &mut self.stage {
            *connected = Some(name);
        }
    }

    pub(super) fn dismiss_connected(&mut self) {
        if let Stage::Running { connected, .. } = &mut self.stage {
            *connected = None;
        }
    }

    /// Steps the open flow with `event` and carries out what it asks.
    /// Returns whether the view changed: false with no flow open.
    pub(crate) fn sign_in_event(&mut self, event: Event) -> bool {
        let Stage::Running {
            flow: Some(flow), ..
        } = &mut self.stage
        else {
            return false;
        };
        let effects = flow.step(&mut self.fields, event);
        sign_in::run(self, effects);
        true
    }

    pub(crate) fn sign_in(&self) -> Option<&SignIn> {
        match &self.stage {
            Stage::Running { flow, .. } => flow.as_deref(),
            Stage::Starting | Stage::Unavailable(_) => None,
        }
    }

    pub(crate) fn sign_in_fields_mut(&mut self) -> &mut Fields {
        &mut self.fields
    }

    /// Keeps the sign-in handle a request fetched, for the steps after it.
    pub(crate) fn keep_sign_in_auth(&mut self, auth: Option<AuthService>) {
        if let (
            Stage::Running {
                flow: Some(flow), ..
            },
            Some(auth),
        ) = (&mut self.stage, auth)
        {
            flow.auth = Some(auth);
        }
    }

    /// Starts decoding the server's icon; it shows once decoded.
    pub(crate) fn load_server_icon(&mut self, bytes: Vec<u8>) {
        let Some(images) = &self.images else {
            return;
        };
        let image = images.load(bytes);
        if let Stage::Running {
            flow: Some(flow), ..
        } = &mut self.stage
        {
            flow.set_icon(image);
        }
    }

    pub(crate) fn registry_writes(&self) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(&self.registry_writes)
    }

    pub(crate) fn sdk(&self) -> Option<&Sdk<Shell>> {
        self.sdk.as_ref()
    }

    pub(crate) fn sign_in_requests(&self) -> &RequestSlot {
        &self.sign_in_requests
    }

    pub(crate) fn sign_in_requests_mut(&mut self) -> &mut RequestSlot {
        &mut self.sign_in_requests
    }

    pub(crate) fn server_card_requests(&self) -> &RequestSlot {
        &self.server_card_requests
    }

    pub(crate) fn server_card_requests_mut(&mut self) -> &mut RequestSlot {
        &mut self.server_card_requests
    }
}
