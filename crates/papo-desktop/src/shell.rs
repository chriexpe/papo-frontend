//! The root view: the app (the conversation surface with the composer pill),
//! and the server and account screens in the host that fits: the whole window
//! on first run or when no saved account can sign in, a popup over the app
//! otherwise.
//!
//! Owns presentation state (the composer's edit buffer and focus, which
//! screens are open, the "Connected to" pill) and the link to the SDK client.
//! Server, account and message state come from the SDK and are never copied
//! here: the account watch posts only what the shell decides with. Writing is
//! the composer's (`composer`, `shell/composer.rs`). The shown channel's
//! timeline is drawn above the composer (`shell/timeline.rs`).

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{AccountId, ServerId};
use rune::{Image, ImageCache};

use crate::account_watch::{self, Accounts};
use crate::composer::Composer;
use crate::composer::prefs::WritingPrefs;
use crate::design::MessageLayout;
use crate::navigation::Navigation;
use crate::reading_prefs::ReadingPrefs;
use crate::reconnect_hints;
use crate::sdk_bridge::{BridgeHost, OpenFailure, RequestSlot, Sdk};
use crate::server_list;
use crate::sign_in::{Fields, Host, SignIn};
use crate::tabs::Tabs;
use crate::timeline::local_time::LocalTime;

/// The root view's state.
pub(crate) struct Shell {
    /// Each channel's edit buffer and where its draft stands.
    pub(crate) composer: Composer,
    /// Each channel's lane of draft writes, shared with the bridge's jobs.
    draft_writes: composer::DraftWrites,
    /// A new message's local identity: random in the app, fixed in tests.
    local_ids: fn() -> papo_sdk::Uuid,
    /// "Enter to send".
    writing: WritingPrefs,
    /// The unsent message whose "Send again?" is open.
    resend: Option<outgoing::Resend>,
    /// The deletion waiting for its confirmation.
    deleting: Option<deleting::DeleteAsk>,
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
    /// When the wake asked for that save arrives; one at a time.
    save_wake_s: Option<f64>,
    /// The current local day, as days since 1970-01-01: date separators
    /// name it "Today".
    today: i64,
    /// When, on the window's clock, the day is checked again.
    day_ends_s: Option<f64>,
    /// Links waiting to open, trusted domains, the message under the pointer.
    content: content::Content,
    /// The device's reading settings: open at the newest message, the
    /// hourglass.
    prefs: ReadingPrefs,
    /// Whether the window is on screen: read reports wait while it is not.
    window_shown: bool,
    /// The last ⌛ press's number; each takes the next.
    seeks: u64,
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
            composer: Composer::default(),
            draft_writes: composer::DraftWrites::default(),
            local_ids: papo_sdk::Uuid::new_v4,
            writing: WritingPrefs::default(),
            resend: None,
            deleting: None,
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
            save_wake_s: None,
            today: LocalTime::system().today(),
            day_ends_s: None,
            content: content::Content::default(),
            prefs: ReadingPrefs::default(),
            window_shown: true,
            seeks: 0,
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
    /// The window starts (`AppSpec::setup`): the shell takes its image
    /// decoder and reads this device's trusted domains and reading
    /// settings.
    pub(crate) fn setup(&mut self, waker: rune::Waker) {
        self.images = Some(ImageCache::new(waker));
        self.content.load();
        self.prefs = ReadingPrefs::load();
        self.writing = WritingPrefs::load();
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
                self.composer.focus();
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
}

mod accounts;
mod composer;
mod composer_input;
mod content;
pub(crate) mod deleting;
mod message_keys;
mod navigate;
mod outgoing;
mod reading;
mod replies;
mod sign_in_flow;
#[cfg(test)]
mod tests;
mod timeline;
mod timeline_host;
mod view;

pub(crate) use view::view;
