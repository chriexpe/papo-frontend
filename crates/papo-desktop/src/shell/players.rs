//! The shell's side of provider players (`docs/design.md` §15.3–§15.5, §15.7): the WebView
//! thread, started the first time a player poster is wanted (so a person who never sees one
//! pays nothing), its events, the players' surfaces, where each box is (for the out-of-view
//! rule), the ring's half-second, and one sound at a time with D7c's media. The rules are
//! `provider_players`'; presses, keys, menus and the viewer are `player_presses`'; the
//! setting's page is `settings_players`'.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use papo_sdk_ui::PlayerProvider;
use rune::{Cursor, NativeSurface};

use super::Shell;
use crate::embedded_players::EmbeddedPlayers;
use crate::link_cards::player::{PlayerShown, PlayersView};
use crate::message_media::received::MediaAt;
use crate::provider_players::{Command, PlayerKey, PlayerStore};
use crate::web_view::{Events, PageHost, PageId, PageSpec};

/// What starts the WebViews (keeping sites' data under the first argument, on the GPU at the
/// second): the WPE thread in the app, a recorder in tests.
pub(crate) type Starter = fn(std::path::PathBuf, Option<String>, Events) -> Box<dyn PageHost>;

/// The WebView thread, as far as the shell knows it.
pub(super) enum Web {
    /// Not needed yet.
    NotStarted,
    /// Opening WPE WebKit; pages asked for meanwhile wait on its thread.
    Starting(Box<dyn PageHost>),
    Ready(Box<dyn PageHost>),
    /// WPE WebKit is missing here: players stay off.
    Unavailable,
}

/// Provider players' state on the shell.
pub(crate) struct Players {
    pub(super) store: PlayerStore,
    /// Settings › General › Embedded players.
    pub(crate) settings: EmbeddedPlayers,
    pub(super) web: Web,
    /// Each live player's frames.
    pub(super) surfaces: HashMap<PlayerKey, NativeSurface>,
    /// Where each player's box was last drawn (window points).
    pub(super) rects: HashMap<PlayerKey, [f32; 4]>,
    /// The size and scale each live page was last given.
    pub(super) sent: HashMap<PlayerKey, ([f32; 2], f32)>,
    /// Loading players whose ring shows.
    pub(super) ringing: HashSet<PlayerKey>,
    /// The players the pointer is over, and the one with the keyboard.
    pub(super) hovered: HashSet<PlayerKey>,
    pub(super) focused: Option<PlayerKey>,
    pub(super) cursors: HashMap<PlayerKey, Cursor>,
    /// Where each player was last started from: its card's media place.
    pub(super) ats: HashMap<PlayerKey, MediaAt>,
    /// The sites that have stored data, as the WebView thread last said.
    pub(crate) sites_with_data: BTreeSet<PlayerProvider>,
    /// When the next player falls due for release, on the window's clock.
    pub(super) next_release_s: Option<f64>,
    waker: Option<rune::Waker>,
    starter: Starter,
}

impl Default for Players {
    /// The WPE thread in the app; in unit tests a recorder, so none opens WPE.
    fn default() -> Players {
        #[cfg(not(test))]
        let starter: Starter = |root, render_node, events| {
            Box::new(crate::web_view::WebViews::start(root, render_node, events))
        };
        #[cfg(test)]
        let starter: Starter = |_, _, _| Box::new(super::tests::players::Recorder);
        Players::with_starter(starter)
    }
}

impl Players {
    /// Players whose WebViews `starter` starts (the WPE thread in the app, a recorder in tests).
    pub(crate) fn with_starter(starter: Starter) -> Players {
        Players {
            store: PlayerStore::default(),
            settings: EmbeddedPlayers::default(),
            web: Web::NotStarted,
            surfaces: HashMap::new(),
            rects: HashMap::new(),
            sent: HashMap::new(),
            ringing: HashSet::new(),
            hovered: HashSet::new(),
            focused: None,
            cursors: HashMap::new(),
            ats: HashMap::new(),
            sites_with_data: BTreeSet::new(),
            next_release_s: None,
            waker: None,
            starter,
        }
    }

    /// The window starts: frames landing on a surface redraw it.
    pub(crate) fn set_waker(&mut self, waker: rune::Waker) {
        self.waker = Some(waker);
    }

    /// Whether a press on play can load a player: switched on, and WPE not known missing.
    pub(crate) fn loadable(&self) -> bool {
        self.settings.settings().on && !matches!(self.web, Web::Unavailable)
    }

    pub(super) fn views(&self) -> Option<&dyn PageHost> {
        match &self.web {
            Web::Starting(views) | Web::Ready(views) => Some(views.as_ref()),
            Web::NotStarted | Web::Unavailable => None,
        }
    }

    /// What views read.
    pub(crate) fn view(&self, viewer: Option<PlayerKey>) -> PlayersView {
        let shown = self
            .store
            .keys()
            .filter_map(|key| {
                let phase = self.store.phase(key)?;
                Some((
                    key,
                    PlayerShown {
                        phase,
                        ring: self.ringing.contains(&key),
                        surface: self.surfaces.get(&key).cloned(),
                        chrome: self.hovered.contains(&key) || self.focused == Some(key),
                        cursor: self.cursors.get(&key).copied().unwrap_or_default(),
                        in_viewer: viewer == Some(key),
                    },
                ))
            })
            .collect();
        PlayersView {
            loadable: self.loadable(),
            shown: Arc::new(shown),
        }
    }

    /// The cursor `key`'s page asked for.
    pub(crate) fn cursor(&self, key: PlayerKey) -> Cursor {
        self.cursors.get(&key).copied().unwrap_or_default()
    }

    /// The surface of `key`'s live player.
    pub(crate) fn surface(&self, key: PlayerKey) -> Option<NativeSurface> {
        self.surfaces.get(&key).cloned()
    }
}

impl Shell {
    /// A player poster is wanted (shown or near): the WebView thread starts, once, if players
    /// are on, so a press on play finds WPE already open or known missing.
    pub(super) fn want_players(&mut self) {
        if self.players.settings.settings().on {
            self.start_web_views();
        }
    }

    /// Starts the WebView thread, once: for players, or for the Embedded players page, which
    /// reads and clears sites' data even with players off.
    pub(super) fn start_web_views(&mut self) {
        if !matches!(self.players.web, Web::NotStarted) {
            return;
        }
        let Some(root) = crate::profile_dir::profile_dir().map(|profile| profile.join("web"))
        else {
            self.players.web = Web::Unavailable;
            return;
        };
        // Before the SDK opens (and in tests) nothing carries events to the window.
        let events: Events = match &self.sdk {
            Some(sdk) => {
                let updates = sdk.updates();
                Arc::new(move |event| {
                    updates.send_tx(move |shell: &mut Shell, tx| shell.web_event(event, tx));
                })
            }
            None => Arc::new(|_| {}),
        };
        // The GPU Rune draws on, so WPE renders where Rune imports (R-013).
        let render_node = render_node();
        self.players.web = Web::Starting((self.players.starter)(root, render_node, events));
    }

    /// Carries out what the store decided.
    pub(super) fn run_player_commands(&mut self, commands: Vec<Command>) {
        for command in commands {
            match command {
                Command::Open {
                    key,
                    page,
                    player,
                    muted,
                } => self.open_page(key, page, &player, muted),
                Command::Close(page) => self.close_page(page),
                Command::Mute(page, muted) => {
                    if let Some(views) = self.players.views() {
                        views.mute(page, muted);
                    }
                }
            }
        }
    }

    fn open_page(
        &mut self,
        key: PlayerKey,
        page: PageId,
        player: &papo_sdk_ui::ProviderPlayer,
        muted: bool,
    ) {
        let Some(views) = self.players.views() else {
            let commands = self.players.store.failed(page);
            self.run_player_commands(commands);
            return;
        };
        let surface = NativeSurface::new();
        if let Some(waker) = &self.players.waker {
            surface.set_waker(Arc::clone(waker));
        }
        let (size_pt, _) = self.player_box(key);
        let kept = player.clone();
        views.open(
            page,
            PageSpec {
                address: player.address(),
                keeps: Arc::new(move |address| kept.keeps(address)),
                site: player.provider.key().to_owned(),
                size_pt,
                scale: self.scale,
                surface: surface.clone(),
                muted,
            },
        );
        self.players.surfaces.insert(key, surface);
        self.players.sent.insert(key, (size_pt, self.scale));
        self.players.cursors.remove(&key);
    }

    fn close_page(&mut self, page: PageId) {
        if let Some(views) = self.players.views() {
            views.close(page);
        }
        let live: HashSet<PlayerKey> = self
            .players
            .store
            .live_pages()
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        self.players.surfaces.retain(|key, _| live.contains(key));
        self.players.ringing.retain(|key| live.contains(key));
        self.players.sent.retain(|key, _| live.contains(key));
        self.players.hovered.retain(|key| live.contains(key));
        if self.players.focused.is_some_and(|key| !live.contains(&key)) {
            self.players.focused = None;
        }
    }

    /// The box player `key` is drawn in now, in points, and whether it is the viewer's.
    pub(super) fn player_box(&self, key: PlayerKey) -> ([f32; 2], bool) {
        if self.players.store.viewer() == Some(key) {
            return (self.player_viewer_box(), true);
        }
        let size = self
            .players
            .rects
            .get(&key)
            .map_or([448.0, 252.0], |rect| [rect[2], rect[3]]);
        (size, false)
    }

    /// Player `key`'s box is at `rect` (window points): its page follows the box's size.
    pub(crate) fn player_moved(&mut self, key: PlayerKey, rect: [f32; 4]) {
        self.players.rects.insert(key, rect);
        self.size_page(key);
    }

    /// Sizes `key`'s live page to the box it is drawn in now (its row's or the viewer's) at the
    /// window's scale, when either changed since it was last told.
    pub(super) fn size_page(&mut self, key: PlayerKey) {
        let Some(page) = self.players.store.page(key) else {
            return;
        };
        let (size_pt, _) = self.player_box(key);
        let scale = self.scale;
        // Layout moves a box by fractions of a point; only a real change resizes the page.
        let changed = self
            .players
            .sent
            .get(&key)
            .is_none_or(|(sent, sent_scale)| {
                (sent[0] - size_pt[0]).abs() >= 0.5
                    || (sent[1] - size_pt[1]).abs() >= 0.5
                    || (sent_scale - scale).abs() >= 0.01
            });
        if changed && let Some(views) = self.players.views() {
            views.resize(page, size_pt, scale);
            self.players.sent.insert(key, (size_pt, scale));
        }
    }

    /// The boxes of the players `drawn` (the previews of the messages shown or near) are kept;
    /// any other's is forgotten, so a live player whose row is gone counts as out of view.
    pub(super) fn players_drawn(&mut self, drawn: &HashSet<PlayerKey>) {
        self.players.rects.retain(|key, _| drawn.contains(key));
    }

    /// Whose player `key` is.
    pub(super) fn player_provider(&self, key: PlayerKey) -> Option<PlayerProvider> {
        self.players.store.provider(key)
    }

    /// Other media started sounding (an audio, a video's sound, the viewer's video): every
    /// player goes quiet.
    pub(super) fn quiet_players(&mut self) {
        let commands = self.players.store.quiet_all();
        self.run_player_commands(commands);
    }

    /// A player started sounding: audio pauses and videos go quiet (§15.3).
    pub(super) fn quiet_other_media(&mut self) {
        self.videos.store.mute_all();
        self.reconcile_videos();
        self.pause_audio();
    }
}

/// The render node of the GPU Rune draws on, once a window has been made there.
fn render_node() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        rune::native_surface::linux::import_target().and_then(|target| target.render_node)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}
