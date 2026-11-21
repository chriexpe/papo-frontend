//! The root view: the app (the conversation surface with the composer pill),
//! and the server and account screens in the host that fits: the whole window
//! on first run or when no saved account can sign in, a popup over the app
//! otherwise.
//!
//! Owns presentation state (the composer's edit buffer and focus, which
//! screens are open, the "Connected to" pill) and the link to the SDK client.
//! Server, account and message state come from the SDK and are never copied
//! here: the account watch posts only what the shell decides with. Sending is
//! not wired: the field keeps what is typed. The shown channel's timeline is
//! drawn above the composer (`shell/timeline.rs`).

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{AccountId, AuthService, ServerId};
use rune::{Image, ImageCache, TextEdit};

use crate::account_watch::{self, Accounts};
use crate::design::MessageLayout;
use crate::navigation::Navigation;
use crate::reconnect_hints;
use crate::sdk_bridge::{BridgeHost, OpenFailure, RequestSlot, Sdk};
use crate::server_list;
use crate::sign_in::{self, Event, Fields, Host, SignIn};
use crate::tabs::Tabs;
use crate::timeline::local_time::LocalTime;

/// The element key of the composer's text field.
pub(crate) const COMPOSER_KEY: &str = "composer";

/// The root view's state.
pub(crate) struct Shell {
    composer: TextEdit,
    sdk: Option<Sdk<Shell>>,
    stage: Stage,
    /// What the server and account screens hold. Kept outside the flow so a
    /// field's editor always has its text; reset, wiping the passwords, each
    /// time the screens close.
    fields: Fields,
    /// The flow's address probe and account requests; only the newest counts.
    sign_in_requests: RequestSlot,
    /// The renewing server's name and icon.
    server_card_requests: RequestSlot,
    /// Decodes server icons off the UI thread; set when the window starts.
    images: Option<ImageCache>,
    /// Held by each change to the server list the flow makes, and by each
    /// probe, so a probe sees the removals sent before it.
    registry_writes: Arc<tokio::sync::Mutex<()>>,
    /// Which server and channel are selected, the rail's order, the menu.
    nav: Navigation,
    /// Each server's decoded icon, and the bytes it was decoded from.
    icons: HashMap<ServerId, Image>,
    icon_bytes: HashMap<ServerId, Arc<[u8]>>,
    /// The newest reorder sent; only its answer counts.
    reorder_requests: RequestSlot,
    reorder: Reorder,
    /// The open timelines and the places of closed ones.
    tabs: Tabs,
    /// How messages are laid out: Normal until the Appearance settings.
    message_layout: MessageLayout,
    /// The time zone and clock message times are read in.
    local_time: LocalTime,
    /// When the shown timeline's place is next saved, on the window's
    /// clock; set while it moves.
    save_due_s: Option<f64>,
}

/// The reorder on its way to the SDK, and the newest one waiting for it.
#[derive(Default)]
struct Reorder {
    in_flight: bool,
    waiting: Option<Vec<ServerId>>,
}

/// Where the app stands.
pub(crate) enum Stage {
    /// The SDK has not opened, or has not said which accounts are saved.
    Starting,
    /// The SDK could not open; the app cannot work.
    Unavailable(OpenFailure),
    Running {
        /// The accounts' summary, as the account watch last posted it.
        accounts: Accounts,
        /// The server and account screens, when open.
        flow: Option<Box<SignIn>>,
        /// The server just signed in to, for the "Connected to" pill.
        connected: Option<String>,
        /// Signed-out accounts whose sign-in screen was shown, so closing it
        /// does not bring it straight back.
        offered: Vec<AccountId>,
    },
}

impl Default for Shell {
    fn default() -> Self {
        Shell {
            composer: TextEdit::new(""),
            sdk: None,
            stage: Stage::Starting,
            fields: Fields::default(),
            sign_in_requests: RequestSlot::default(),
            server_card_requests: RequestSlot::default(),
            images: None,
            registry_writes: Arc::default(),
            nav: Navigation::default(),
            icons: HashMap::new(),
            icon_bytes: HashMap::new(),
            reorder_requests: RequestSlot::default(),
            reorder: Reorder::default(),
            tabs: Tabs::default(),
            message_layout: MessageLayout::default(),
            local_time: LocalTime::system(),
            save_due_s: None,
        }
    }
}

impl BridgeHost for Shell {
    fn sdk_opened(&mut self, sdk: Sdk<Self>) {
        sdk.watch(|client, updates| account_watch::run(client, updates, Shell::accounts_changed));
        sdk.watch(|client, updates| server_list::run(client, updates, Shell::servers_changed));
        sdk.run(reconnect_hints::run);
        self.sdk = Some(sdk);
    }

    fn sdk_failed(&mut self, failure: OpenFailure) {
        self.stage = Stage::Unavailable(failure);
    }
}

impl Shell {
    /// Gives the shell the window's image decoder (`AppSpec::setup`).
    pub(crate) fn set_image_cache(&mut self, images: ImageCache) {
        self.images = Some(images);
    }

    /// Takes the account watch's summary, and opens the screens it calls for.
    pub(crate) fn accounts_changed(&mut self, accounts: Accounts) -> bool {
        match &mut self.stage {
            Stage::Unavailable(_) => return false,
            Stage::Starting => {
                self.stage = Stage::Running {
                    accounts,
                    flow: None,
                    connected: None,
                    offered: Vec::new(),
                };
                self.composer.focused = true;
            }
            Stage::Running {
                accounts: current,
                offered,
                ..
            } => {
                // An account that works again is offered again if it lapses.
                offered.retain(|id| accounts.signed_out.iter().any(|out| out.id == *id));
                *current = accounts;
            }
        }
        let first_run = matches!(&self.stage, Stage::Running { accounts, flow: None, .. } if accounts.saved == 0);
        if first_run {
            let flow = SignIn::new(Host::Window, &mut self.fields);
            self.open_sign_in(flow, None);
        } else {
            self.offer_sign_in_again();
        }
        true
    }

    /// With no screens open, signing in again for the first signed-out
    /// account not offered yet. Never the first-run server step: right after
    /// a sign-in the summary may not count the new account yet.
    fn offer_sign_in_again(&mut self) {
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

    fn open_sign_in(&mut self, flow: SignIn, first: Option<sign_in::Effect>) {
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
        self.composer.focused = true;
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

    fn dismiss_connected(&mut self) {
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

mod accounts;
mod navigate;
#[cfg(test)]
mod tests;
mod timeline;
mod view;

pub(crate) use view::view;
