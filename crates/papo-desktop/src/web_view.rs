//! Provider pages in a WebView (`docs/design.md` §15.3–§15.5; `docs/papo-desktop.md` §5): on
//! Linux, WPE WebKit found when the app runs, rendering each page into dmabufs that Rune draws
//! in the page's box, so a player scrolls and clips with its row, under the glass pills, with
//! menus, dialogs and the viewer drawn above it. Everything WPE does runs on one thread of its
//! own with its own GLib main loop, so nothing here polls the window's frames.
//!
//! Owns the WebView side: opening, sizing, input, sound and closing pages, and each site's
//! stored data. Which players are live, consent, budgets and drawing are the player's
//! (`provider_players`); which address may load is the SDK's (`ProviderPlayer`).
//!
//! Without WPE WebKit (not installed, too old, or no GPU that imports dmabufs) the thread
//! reports [`WebEvent::Unavailable`] and the players stay off (§15.7); nothing else changes.
//! Other platforms have no WebView adapter yet, so they report the same.

use std::path::PathBuf;
use std::sync::Arc;

use rune::NativeSurface;

#[cfg(target_os = "linux")]
mod classes;
#[cfg(target_os = "linux")]
mod frames;
#[cfg(target_os = "linux")]
mod glib;
pub(crate) mod keys;
#[cfg(target_os = "linux")]
mod library;
#[cfg(target_os = "linux")]
mod page;
#[cfg(target_os = "linux")]
mod policy;
#[cfg(target_os = "linux")]
mod runtime;
#[cfg(target_os = "linux")]
mod sites;

use keys::KeyPress;

/// A page, numbered by its opener.
pub(crate) type PageId = u64;

/// Where the WebView thread sends what happens; it runs on that thread, so it only posts.
pub(crate) type Events = Arc<dyn Fn(WebEvent) + Send + Sync>;

/// What happens in the WebViews.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WebEvent {
    /// Pages can be opened.
    Ready,
    /// They can't, and why (for the log).
    Unavailable(String),
    /// Something happened in page `0`.
    Page(PageId, PageEvent),
    /// Whether `site` has cookies or other data stored.
    SiteData { site: String, has_data: bool },
}

/// What happens in a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PageEvent {
    /// Its first frame is drawn: the player is up.
    Painted,
    /// It couldn't load, or its process ended.
    Failed,
    /// It started or stopped making sound.
    Sound(bool),
    /// The provider asked for full screen: Papo's viewer takes the player.
    FullScreen,
    /// The person clicked something that leaves the player: open it in the browser.
    OpenInBrowser(String),
    /// The cursor the page wants over it, by CSS name.
    Cursor(String),
}

/// What a page opens with.
#[derive(Clone)]
pub(crate) struct PageSpec {
    /// The player's address (the SDK's `ProviderPlayer::address`).
    pub(crate) address: String,
    /// Whether an address is still the player; anything else goes to the browser.
    pub(crate) keeps: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    /// The site whose storage the page uses (`PlayerProvider::key`).
    pub(crate) site: String,
    /// The box, in points.
    pub(crate) size_pt: [f32; 2],
    /// Pixels a point.
    pub(crate) scale: f32,
    /// Where its frames go.
    pub(crate) surface: NativeSurface,
    /// Whether it starts without sound.
    pub(crate) muted: bool,
}

/// A pointer button the page hears. The secondary button opens Papo's menu instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerButton {
    Primary,
}

/// Input for a page, in points from its box's top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PageInput {
    Move {
        x: f32,
        y: f32,
    },
    Leave,
    Down {
        x: f32,
        y: f32,
        button: PointerButton,
    },
    Up {
        x: f32,
        y: f32,
        button: PointerButton,
    },
    /// A wheel or touchpad, in WPE's direction (positive scrolls down and right).
    Wheel {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        precise: bool,
    },
    Key(KeyPress),
}

/// What the players ask of the WebViews: [`WebViews`] in the app, a recorder in tests.
pub(crate) trait PageHost {
    /// Opens page `id` (an id still open is closed first).
    fn open(&self, id: PageId, spec: PageSpec);
    /// Page `id`'s box is now `size_pt` at `scale`.
    fn resize(&self, id: PageId, size_pt: [f32; 2], scale: f32);
    /// Forwards `input` to page `id`.
    fn input(&self, id: PageId, input: PageInput);
    /// Page `id` has the keyboard, or no longer has it.
    fn focus(&self, id: PageId, focused: bool);
    /// Mutes or unmutes page `id`.
    fn mute(&self, id: PageId, muted: bool);
    /// Closes page `id`.
    fn close(&self, id: PageId);
    /// Reports whether `site` has data stored ([`WebEvent::SiteData`]).
    fn report_site(&self, site: String);
    /// Removes everything `site` stored, then reports it.
    fn clear_site(&self, site: String);
}

/// The WebView thread and the pages on it.
pub(crate) struct WebViews {
    #[cfg(target_os = "linux")]
    thread: runtime::Thread,
}

impl WebViews {
    /// Starts the WebView thread: it opens WPE WebKit and its display on the GPU at
    /// `render_node` (the one Rune draws on, R-013), keeping sites' data under `root`, then
    /// reports [`WebEvent::Ready`] or [`WebEvent::Unavailable`] to `events`.
    pub(crate) fn start(root: PathBuf, render_node: Option<String>, events: Events) -> WebViews {
        #[cfg(target_os = "linux")]
        {
            WebViews {
                thread: runtime::Thread::start(root, render_node, events),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (root, render_node);
            events(WebEvent::Unavailable(
                "no WebView on this platform yet".into(),
            ));
            WebViews {}
        }
    }

    fn call(&self, id: PageId, call: PageCall) {
        #[cfg(target_os = "linux")]
        self.thread.run(move |runtime| runtime.call(id, call));
        #[cfg(not(target_os = "linux"))]
        let _ = (id, call);
    }
}

impl PageHost for WebViews {
    fn open(&self, id: PageId, spec: PageSpec) {
        #[cfg(target_os = "linux")]
        self.thread.run(move |runtime| runtime.open(id, spec));
        #[cfg(not(target_os = "linux"))]
        let _ = (id, spec);
    }

    fn resize(&self, id: PageId, size_pt: [f32; 2], scale: f32) {
        self.call(id, PageCall::Resize { size_pt, scale });
    }

    fn input(&self, id: PageId, input: PageInput) {
        self.call(id, PageCall::Input(input));
    }

    fn focus(&self, id: PageId, focused: bool) {
        self.call(id, PageCall::Focus(focused));
    }

    fn mute(&self, id: PageId, muted: bool) {
        self.call(id, PageCall::Mute(muted));
    }

    fn close(&self, id: PageId) {
        #[cfg(target_os = "linux")]
        self.thread.run(move |runtime| runtime.close(id));
        #[cfg(not(target_os = "linux"))]
        let _ = id;
    }

    fn report_site(&self, site: String) {
        #[cfg(target_os = "linux")]
        self.thread.run(move |runtime| runtime.report_site(&site));
        #[cfg(not(target_os = "linux"))]
        let _ = site;
    }

    fn clear_site(&self, site: String) {
        #[cfg(target_os = "linux")]
        self.thread.run(move |runtime| runtime.clear_site(&site));
        #[cfg(not(target_os = "linux"))]
        let _ = site;
    }
}

/// A change to an open page.
#[derive(Clone, Copy, Debug)]
enum PageCall {
    Resize { size_pt: [f32; 2], scale: f32 },
    Input(PageInput),
    Focus(bool),
    Mute(bool),
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests;
