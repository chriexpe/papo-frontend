//! The timeline's view, headless: the production view in Rune's `Runtime`
//! over an open timeline whose items are posted as the SDK would post them,
//! with no SDK window behind it. It proves the spacing model of
//! `docs/design.md` §5 at every density, that the reader's row keeps its
//! place through density changes, prepends, edits and reflow over 10,000
//! messages, and that an idle timeline draws nothing.

use std::ops::Range;
use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{AccountId, Message, ServerAddress, Uuid};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{TimelineItem, UtcOffset};
use rune::{Cx, El, ElementInfo, Env, Runtime, Tx, col};

use super::content::Press;
use super::echoes::SendState;
use super::local_time::LocalTime;
use super::rows::message_key;
use super::{OpenTimeline, Reacting, Request, Style, TimelineHost, TimelineKey, view};
use crate::design::{Density, MessageLayout, Palette, Theme};

const SIZE_PT: (f32, f32) = (800.0, 600.0);
const FRAME_S: f64 = 1.0 / 60.0;
/// Points within which two layout edges count as the same.
const EDGE_PT: f32 = 0.01;
/// How far the reader's row may move through a change, in points.
const HOLD_PT: f32 = 0.5;
const DENSITIES: [Density; 4] = [
    Density::Compact,
    Density::Normal,
    Density::Comfortable,
    Density::ExtraCompact,
];

/// 2026-11-20 12:00 UTC.
fn noon_us() -> i64 {
    1_795_176_000_000_000
}

/// The view's state: one open timeline and how it is laid out.
struct Host {
    open: OpenTimeline,
    layout: MessageLayout,
    /// History requests the visible rows asked for.
    requests: Vec<Request>,
    /// Presses inside message content, in order.
    presses: Vec<(Uuid, Press)>,
    /// The "Unread hourglass" setting.
    hourglass: bool,
    /// ⌛, ↓ and Cancel presses, in order.
    controls: Vec<&'static str>,
}

impl TimelineHost for Host {
    fn timeline(&self) -> Option<&OpenTimeline> {
        Some(&self.open)
    }

    fn rows_visible(&mut self, _key: TimelineKey, range: Range<usize>) -> bool {
        let requests = self.open.rows_visible(range);
        self.requests.extend(requests);
        false
    }

    fn viewport_moved(&mut self, _key: TimelineKey, top_pt: f32, height_pt: f32, _tx: &mut Tx) {
        self.open.tracker.viewport_moved(top_pt, height_pt);
    }

    fn row_moved(&mut self, _key: TimelineKey, id: Uuid, [top, bottom]: [f32; 2], _tx: &mut Tx) {
        self.open.tracker.row_moved(id, top, bottom);
    }

    fn bottom_shown(&mut self, _key: TimelineKey, at_bottom: bool) -> bool {
        self.open.tracker.bottom_shown(at_bottom)
    }

    fn retry_gap(&mut self, _key: TimelineKey, _cursor_id: &str) {}

    fn content_pressed(&mut self, _key: TimelineKey, message: Uuid, press: Press, _tx: &mut Tx) {
        if let Press::Spoiler(index) = press {
            self.open.reveal(message, index);
        }
        self.presses.push((message, press));
    }

    fn code_copied(&mut self, _key: TimelineKey, message: Uuid, index: usize, tx: &mut Tx) {
        self.open.code_copied(message, index, tx.clock() + 1.5);
    }

    fn message_hovered(&mut self, _key: TimelineKey, _message: Uuid, _hovered: bool) {}

    fn hourglass_pressed(&mut self, _key: TimelineKey) {
        self.controls.push("hourglass");
    }

    fn down_pressed(&mut self, _key: TimelineKey, _tx: &mut Tx) {
        self.controls.push("down");
    }

    fn seek_cancelled(&mut self, _key: TimelineKey) {
        self.controls.push("cancel");
    }

    fn send_state_pressed(&mut self, _key: TimelineKey, _local: Uuid, _state: SendState) {
        self.controls.push("send-state");
    }

    fn message_key(
        &mut self,
        _key: TimelineKey,
        _id: Uuid,
        _event: &rune::KeyEvent,
        _tx: &mut Tx,
    ) -> bool {
        false
    }

    fn reply_line_pressed(&mut self, _key: TimelineKey, _original: Uuid, _tx: &mut Tx) {
        self.controls.push("reply-line");
    }

    fn change_mark_pressed(&mut self, _key: TimelineKey, _local: Uuid) {
        self.controls.push("change-mark");
    }

    fn change_retried(&mut self, _key: TimelineKey, _local: Uuid) {
        self.controls.push("change-retried");
    }

    fn change_discarded(&mut self, _key: TimelineKey, _local: Uuid) {
        self.controls.push("change-discarded");
    }

    fn bar_pressed(
        &mut self,
        _key: TimelineKey,
        _id: Uuid,
        _press: super::hover_bar::BarPress,
        _tx: &mut Tx,
    ) {
        self.controls.push("bar");
    }

    fn link_menu_requested(
        &mut self,
        _key: TimelineKey,
        _url: String,
        _at: [f32; 2],
        _tx: &mut Tx,
    ) {
        self.controls.push("link-menu");
    }

    fn message_menu_requested(
        &mut self,
        _key: TimelineKey,
        _id: Uuid,
        _at: Option<[f32; 2]>,
        _tx: &mut Tx,
    ) {
        self.controls.push("menu");
    }
}

fn host_view(host: &Host, _cx: &mut Cx<Host>) -> El {
    let time = LocalTime::fixed(UtcOffset::from_seconds_east(0).unwrap(), noon_us);
    let style = Style {
        layout: host.layout,
        theme: Theme::Graphite,
        palette: Palette::new(Theme::Graphite),
        today: time.today(),
        time,
        hourglass: host.hourglass,
        selected: None,
        reacting: Reacting::unknown(),
    };
    col().w_full().h_full().child(view(host, style))
}

fn key() -> TimelineKey {
    let server = ServerAddress::parse("https://papo.example.invalid")
        .unwrap()
        .id();
    TimelineKey {
        account: AccountId::new(server, Uuid::nil()),
        channel: Uuid::from_u128(7),
    }
}

/// Message `n` by `author`, `seconds` after 2026-11-20 14:00 UTC.
fn message(n: u128, author: &str, seconds: u32, body: &str) -> Arc<TimelineItem> {
    let at = format!(
        "2026-11-20T{:02}:{:02}:{:02}Z",
        14 + seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    );
    let id = Uuid::from_u128(n);
    Arc::new(TimelineItem::Message {
        reply: None,
        pending: None,
        id,
        message: Arc::new(Message {
            id: Some(id.to_string()),
            author_id: Some(author.to_owned()),
            content: Some(body.to_owned()),
            created_at: Some(at),
            ..Message::default()
        }),
        content: Arc::new(MessageContent::parse(body)),
    })
}

fn runtime(items: Vector<Arc<TimelineItem>>, layout: MessageLayout) -> Runtime<Host> {
    let host = Host {
        open: OpenTimeline::loaded(key(), items),
        layout,
        requests: Vec::new(),
        presses: Vec::new(),
        hourglass: true,
        controls: Vec::new(),
    };
    let env = Env {
        system_dark: true,
        scale: 1.0,
    };
    let mut rt = Runtime::with_text(host, host_view, SIZE_PT, env, crate::fonts::text_system());
    rt.frame(0.0);
    until_idle(&mut rt);
    rt
}

fn until_idle(rt: &mut Runtime<Host>) {
    for _ in 0..240 {
        if !rt.frame(FRAME_S) {
            return;
        }
    }
    panic!("the timeline never rests");
}

/// The element whose keyed path ends with `suffix`.
fn element(rt: &Runtime<Host>, suffix: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|element| element.path.ends_with(suffix) && !element.exiting)
}

fn rect(rt: &Runtime<Host>, suffix: &str) -> [f32; 4] {
    element(rt, suffix)
        .unwrap_or_else(|| panic!("no element {suffix}"))
        .rect
}

/// Message `n`'s element keyed `part` ("message", "content", "body"…),
/// at any depth inside its row.
fn find_part(rt: &Runtime<Host>, n: u128, part: &str) -> Option<ElementInfo> {
    let row = format!("/{}/", message_key(Uuid::from_u128(n)));
    let part = format!("/{part}");
    rt.elements().into_iter().find(|element| {
        element.path.contains(&row) && element.path.ends_with(&part) && !element.exiting
    })
}

fn part(rt: &Runtime<Host>, n: u128, name: &str) -> [f32; 4] {
    find_part(rt, n, name)
        .unwrap_or_else(|| panic!("no {name} in message {n}"))
        .rect
}

fn has_part(rt: &Runtime<Host>, n: u128, part: &str) -> bool {
    find_part(rt, n, part).is_some()
}

fn bottom(rect: [f32; 4]) -> f32 {
    rect[1] + rect[3]
}

fn assert_near(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() < EDGE_PT,
        "{what}: {actual} instead of {expected}"
    );
}

mod changes;
mod content;
mod controls;
mod echoes;
mod hover_bar;
mod reading;
mod spacing;
