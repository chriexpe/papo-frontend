//! Provider players from the keyboard and in Settings (`docs/design.md` §15.3, §15.6, §15.7),
//! headless and without WPE: the WebViews are a recorder. Space on a focused message asks for a
//! site not allowed and Enter plays once; Always allow skips the ask next time and Remove asks
//! again; the first Esc in a live page returns to the message and never reaches the page; O
//! opens the viewer; with players off or WPE missing the poster's button opens the page and
//! nothing loads; the Embedded players page lists allowed sites and sites with data.

use std::cell::RefCell;
use std::sync::Arc;

use papo_sdk::Message;
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{PlayerProvider, TimelineItem};
use rune::{Key, Runtime};
use serde_json::json;

use super::navigation::click;
use super::replying::{conversation, message_id};
use super::settings::open;
use super::{FRAME_S, Shell, press};
use crate::provider_players::{Phase, PlayerKey};
use crate::settings::{PageId, RowId};
use crate::timeline::rows::message_key;
use crate::web_view::{PageEvent, PageHost, PageId as Page, PageInput, PageSpec, WebEvent};

thread_local! {
    /// What the recorder was asked, on this test's thread.
    static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The WebViews in unit tests: each call is written down, nothing loads.
pub(crate) struct Recorder;

fn record(call: String) {
    CALLS.with_borrow_mut(|calls| calls.push(call));
}

fn calls() -> Vec<String> {
    CALLS.with_borrow(Clone::clone)
}

impl PageHost for Recorder {
    fn open(&self, id: Page, spec: PageSpec) {
        record(format!(
            "open {id} {} {} muted={}",
            spec.site, spec.address, spec.muted
        ));
    }
    fn resize(&self, id: Page, _size_pt: [f32; 2], _scale: f32) {
        record(format!("resize {id}"));
    }
    fn input(&self, id: Page, input: PageInput) {
        record(format!("input {id} {input:?}"));
    }
    fn focus(&self, id: Page, focused: bool) {
        record(format!("focus {id} {focused}"));
    }
    fn mute(&self, id: Page, muted: bool) {
        record(format!("mute {id} {muted}"));
    }
    fn close(&self, id: Page) {
        record(format!("close {id}"));
    }
    fn report_site(&self, site: String) {
        record(format!("report {site}"));
    }
    fn clear_site(&self, site: String) {
        record(format!("clear {site}"));
    }
}

fn embed_id() -> papo_sdk::Uuid {
    papo_sdk::Uuid::from_u128(0x9500)
}

/// The conversation with message 3 linking a YouTube video, as the server's oEmbed crawl
/// sends it.
fn with_youtube() -> Runtime<Shell> {
    CALLS.with_borrow_mut(Vec::clear);
    let mut rt = conversation("");
    rt.update(move |shell, _tx| {
        let id = message_id(3);
        let message: Message = serde_json::from_value(json!({
            "id": id.to_string(),
            "author_id": papo_sdk::Uuid::from_u128(0x4002).to_string(),
            "content": "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            "created_at": "2026-11-20T12:03:00Z",
            "embeds": [{
                "id": embed_id().to_string(),
                "fetch_method": "oembed",
                "provider": "YouTube",
                "url": "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
                "title": "Big Buck Bunny",
                "video": { "url": "https://www.youtube.com/embed/aqz-KE-bpKQ" },
            }],
        }))
        .unwrap();
        let open = shell.tabs.shown_mut().unwrap();
        open.items.set(
            2,
            Arc::new(TimelineItem::Message {
                id,
                message: Arc::new(message),
                content: Arc::new(MessageContent::parse(
                    "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
                )),
                reply: None,
                pending: None,
            }),
        );
    });
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<Shell>) {
    for _ in 0..4 {
        rt.frame(FRAME_S);
    }
}

fn player_key(rt: &Runtime<Shell>) -> PlayerKey {
    let shown = rt.state.tabs.shown().unwrap().key;
    PlayerKey {
        account: shown.account,
        channel: shown.channel,
        message: message_id(3),
        embed: embed_id(),
    }
}

fn focus_message(rt: &mut Runtime<Shell>) {
    rt.update(|_shell, tx| tx.focus(message_key(message_id(3)), true));
    settle(rt);
}

fn opened() -> Vec<String> {
    calls()
        .into_iter()
        .filter(|call| call.starts_with("open"))
        .collect()
}

/// Close player.
fn close(rt: &mut Runtime<Shell>, key: PlayerKey) {
    let at = crate::message_media::received::MediaAt {
        surface: crate::message_media::received::MediaSurface::Timeline,
        account: key.account,
        channel: key.channel,
        message: key.message,
        index: 0,
    };
    rt.update(move |shell, tx| {
        shell.player_event(at, key, crate::link_cards::player::PlayerEvent::Close, tx);
    });
    settle(rt);
}

/// Space, then Enter on the ask's focused Play.
fn play_once(rt: &mut Runtime<Shell>) {
    focus_message(rt);
    press(rt, Key::Text(" ".to_owned()));
    settle(rt);
    press(rt, Key::Enter);
    settle(rt);
}

#[test]
fn space_asks_for_a_site_not_allowed_and_enter_plays_once() {
    let mut rt = with_youtube();
    let key = player_key(&rt);
    focus_message(&mut rt);
    press(&mut rt, Key::Text(" ".to_owned()));
    settle(&mut rt);
    assert_eq!(rt.state.players.store.phase(key), Some(Phase::Asking));
    assert_eq!(
        opened(),
        Vec::<String>::new(),
        "nothing loads before the answer"
    );
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains("Load the YouTube player"), "{tree}");
    assert!(tree.contains("Playing loads YouTube's player."), "{tree}");
    press(&mut rt, Key::Enter);
    settle(&mut rt);
    assert_eq!(
        opened(),
        vec!["open 1 youtube https://www.youtube.com/embed/aqz-KE-bpKQ?autoplay=1 muted=false"]
    );
    assert!(matches!(
        rt.state.players.store.phase(key),
        Some(Phase::Loading { .. })
    ));
    assert!(
        !rt.state.players.settings.allows(PlayerProvider::YouTube),
        "once only"
    );
    rt.update(|shell, tx| {
        shell.web_event(WebEvent::Page(1, PageEvent::Painted), tx);
    });
    settle(&mut rt);
    assert_eq!(rt.state.players.store.phase(key), Some(Phase::Playing));
}

#[test]
fn always_allow_skips_the_ask_and_remove_asks_again() {
    let mut rt = with_youtube();
    let key = player_key(&rt);
    focus_message(&mut rt);
    press(&mut rt, Key::Text("k".to_owned()));
    settle(&mut rt);
    click(
        &mut rt,
        &format!("{}-always", crate::link_cards::player::page_key(key)),
    );
    settle(&mut rt);
    assert!(rt.state.players.settings.allows(PlayerProvider::YouTube));
    assert_eq!(opened().len(), 1);
    close(&mut rt, key);
    focus_message(&mut rt);
    press(&mut rt, Key::Text(" ".to_owned()));
    settle(&mut rt);
    assert_eq!(opened().len(), 2, "no ask the second time");
    rt.update(|shell, tx| shell.settings_players_remove(PlayerProvider::YouTube, tx));
    assert!(!rt.state.players.settings.allows(PlayerProvider::YouTube));
    close(&mut rt, key);
    focus_message(&mut rt);
    press(&mut rt, Key::Text(" ".to_owned()));
    settle(&mut rt);
    assert_eq!(rt.state.players.store.phase(key), Some(Phase::Asking));
}

#[test]
fn the_first_escape_in_a_live_page_returns_to_the_message_and_never_reaches_it() {
    let mut rt = with_youtube();
    let key = player_key(&rt);
    play_once(&mut rt);
    rt.update(|shell, tx| {
        shell.web_event(WebEvent::Page(1, PageEvent::Painted), tx);
    });
    settle(&mut rt);
    // Space on the message with a live player moves the keyboard into it.
    focus_message(&mut rt);
    press(&mut rt, Key::Text(" ".to_owned()));
    settle(&mut rt);
    assert_eq!(rt.state.players.focused, Some(key));
    press(&mut rt, Key::Text("k".to_owned()));
    assert!(
        calls().iter().any(|call| call.starts_with("input 1 Key")),
        "{:?}",
        calls()
    );
    let before = calls().len();
    press(&mut rt, Key::Escape);
    settle(&mut rt);
    assert!(
        !calls()[before..]
            .iter()
            .any(|call| call.starts_with("input")),
        "the page never hears the first Esc"
    );
    assert_ne!(rt.state.players.focused, Some(key));
    assert!(rt.state.players.store.page(key).is_some(), "still playing");
}

#[test]
fn o_opens_the_viewer_with_the_same_page_and_escape_gives_it_back() {
    let mut rt = with_youtube();
    let key = player_key(&rt);
    rt.update(|shell, _tx| {
        if let Some(settings) = shell.players.settings.allow(PlayerProvider::YouTube) {
            shell.players.settings.written(settings, true);
        }
    });
    focus_message(&mut rt);
    press(&mut rt, Key::Text("o".to_owned()));
    settle(&mut rt);
    assert_eq!(rt.state.players.store.viewer(), Some(key));
    assert!(rt.find("provider-viewer").is_some());
    let resized = calls()
        .iter()
        .filter(|call| call.starts_with("resize"))
        .count();
    press(&mut rt, Key::Escape);
    settle(&mut rt);
    assert_eq!(rt.state.players.store.viewer(), None);
    assert_eq!(opened().len(), 1, "the same page, never loaded again");
    assert!(
        calls()
            .iter()
            .filter(|call| call.starts_with("resize"))
            .count()
            > resized
    );
}

#[test]
fn with_players_off_or_wpe_missing_nothing_loads() {
    let mut rt = with_youtube();
    let key = player_key(&rt);
    rt.update(|shell, tx| {
        shell.web_event(WebEvent::Unavailable("no WPE here".into()), tx);
    });
    settle(&mut rt);
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains("Open in browser"), "{tree}");
    focus_message(&mut rt);
    press(&mut rt, Key::Text(" ".to_owned()));
    settle(&mut rt);
    assert_eq!(rt.state.players.store.phase(key), None);
    assert_eq!(opened(), Vec::<String>::new());
}

#[test]
fn switching_players_off_releases_them() {
    let mut rt = with_youtube();
    let key = player_key(&rt);
    play_once(&mut rt);
    assert!(rt.state.players.store.page(key).is_some());
    rt.update(|shell, tx| shell.settings_players_on(false, tx));
    settle(&mut rt);
    assert_eq!(rt.state.players.store.page(key), None);
    assert!(calls().contains(&"close 1".to_owned()));
}

#[test]
fn embedded_players_lists_allowed_sites_and_sites_with_data() {
    let mut rt = super::app();
    open(&mut rt);
    click(&mut rt, &format!("settings-nav-{:?}", PageId::General));
    assert!(rt.find(&RowId::Players.key()).is_some());
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains("0 sites allowed"), "{tree}");
    click(&mut rt, &RowId::Players.key());
    assert!(rt.find(&RowId::PlayersOn.key()).is_some());
    assert!(
        calls().contains(&"report youtube".to_owned()),
        "{:?}",
        calls()
    );
    assert!(
        rt.find(&RowId::PlayersSite(PlayerProvider::YouTube).key())
            .is_none()
    );
    rt.update(|shell, tx| {
        shell.web_event(
            WebEvent::SiteData {
                site: "youtube".into(),
                has_data: true,
            },
            tx,
        );
    });
    rt.frame(FRAME_S);
    let site = RowId::PlayersSite(PlayerProvider::YouTube).key();
    assert!(
        rt.find(&site).is_some(),
        "a site played once keeps its data"
    );
    assert!(rt.find(&format!("{site}-clear")).is_some());
    assert!(rt.find(&format!("{site}-remove")).is_none(), "not allowed");
    click(&mut rt, &format!("{site}-clear"));
    assert!(calls().contains(&"clear youtube".to_owned()));
}

#[test]
fn a_live_player_whose_row_is_no_longer_drawn_is_released_after_ten_seconds() {
    let mut rt = with_youtube();
    let key = player_key(&rt);
    play_once(&mut rt);
    assert!(rt.state.players.store.page(key).is_some());
    rt.update(|shell, tx| {
        // A jump elsewhere: its row is no longer laid out or wanted.
        shell.players_drawn(&std::collections::HashSet::new());
        shell.poll_players(100.0, tx);
        shell.poll_players(105.0, tx);
    });
    assert!(
        rt.state.players.store.page(key).is_some(),
        "a quick pass costs nothing"
    );
    rt.update(|shell, tx| {
        shell.poll_players(110.5, tx);
    });
    assert_eq!(rt.state.players.store.page(key), None);
    assert_eq!(rt.state.players.store.phase(key), Some(Phase::Stopped));
    assert!(calls().contains(&"close 1".to_owned()));
}

#[test]
fn a_press_anywhere_on_the_poster_plays() {
    let mut rt = with_youtube();
    let key = player_key(&rt);
    // A pointer press away from the disc: the poster's corner.
    let (x, y) = rt
        .find(&format!(
            "{}-poster",
            crate::link_cards::player::page_key(key)
        ))
        .expect("the poster's press layer");
    rt.pointer_move(x + 20.0, y + 20.0);
    rt.pointer_down(x + 20.0, y + 20.0);
    rt.pointer_up(x + 20.0, y + 20.0);
    settle(&mut rt);
    assert_eq!(rt.state.players.store.phase(key), Some(Phase::Asking));
    assert!(rt.state.viewer.is_none(), "not the picture viewer");
}
