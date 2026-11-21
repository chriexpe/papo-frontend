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

use super::anchor::Anchor;
use super::local_time::LocalTime;
use super::rows::{anchor_key, message_key};
use super::{OpenTimeline, Request, Style, TimelineHost, TimelineKey, scroll_to, view};
use crate::design::{Density, MESSAGE_BLOCK_GAP_PT, MessageLayout, Palette, Theme};

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

    fn viewport_moved(&mut self, _key: TimelineKey, top_pt: f32, _tx: &mut Tx) {
        self.open.tracker.viewport_moved(top_pt);
    }

    fn row_moved(&mut self, _key: TimelineKey, id: Uuid, [top, bottom]: [f32; 2], _tx: &mut Tx) {
        self.open.tracker.row_moved(id, top, bottom);
    }

    fn bottom_shown(&mut self, _key: TimelineKey, at_bottom: bool) -> bool {
        self.open.tracker.bottom_shown(at_bottom);
        false
    }

    fn retry_gap(&mut self, _key: TimelineKey, _cursor_id: &str) {}
}

fn host_view(host: &Host, _cx: &mut Cx<Host>) -> El {
    let time = LocalTime::fixed(UtcOffset::from_seconds_east(0).unwrap(), noon_us);
    let style = Style {
        layout: host.layout,
        theme: Theme::Graphite,
        palette: Palette::new(Theme::Graphite),
        today: time.today(),
        time,
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
    };
    let env = Env {
        system_dark: true,
        scale: 1.0,
    };
    let mut rt = Runtime::new(host, host_view, SIZE_PT, env);
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

/// Ana twice in a minute, Bia, then Bia again half an hour later.
fn conversation() -> Vector<Arc<TimelineItem>> {
    Vector::from(vec![
        message(1, "ana", 0, "A little more like you."),
        message(2, "ana", 60, "Messages stay together."),
        message(3, "bia", 120, "oi"),
        message(4, "bia", 1920, "still here"),
    ])
}

fn layouts() -> Vec<MessageLayout> {
    let mut layouts = Vec::new();
    for density in DENSITIES {
        for grouped in [true, false] {
            for stacked in [true, false] {
                if density != Density::ExtraCompact && !stacked {
                    continue;
                }
                layouts.push(MessageLayout {
                    density,
                    grouped,
                    stacked,
                });
            }
        }
    }
    layouts
}

#[test]
fn rows_sit_at_the_spacing_of_their_density() {
    for layout in layouts() {
        let rt = runtime(conversation(), layout);
        let spacing = layout.density.spacing();
        let continued = if layout.grouped {
            spacing.within_group_pt
        } else {
            spacing.between_groups_pt
        };
        let rows: Vec<_> = (1..=4).map(|n| part(&rt, n, "message")).collect();
        let what = format!("{layout:?}");
        assert_near(rows[1][1] - bottom(rows[0]), continued, &what);
        assert_near(
            rows[2][1] - bottom(rows[1]),
            spacing.between_groups_pt,
            &what,
        );
        assert_near(
            rows[3][1] - bottom(rows[2]),
            spacing.between_groups_pt,
            &what,
        );
    }
}

#[test]
fn every_block_starts_on_its_messages_content_column() {
    for layout in layouts() {
        let rt = runtime(conversation(), layout);
        let what = format!("{layout:?}");
        let column = part(&rt, 1, "content")[0];
        for n in 1..=4 {
            let content = part(&rt, n, "content");
            assert_near(content[0], column, &what);
            let inline = layout.density == Density::ExtraCompact && !layout.stacked;
            if inline {
                // The name leads the line; the text keeps one column, a
                // hidden name's width reserved.
                assert_near(part(&rt, n, "body")[0], part(&rt, 1, "body")[0], &what);
                continue;
            }
            assert_near(part(&rt, n, "body")[0], column, &what);
            for header in ["header", "name"] {
                if has_part(&rt, n, header) {
                    let header = part(&rt, n, header);
                    assert_near(header[0], column, &what);
                    let gap = part(&rt, n, "body")[1] - bottom(header);
                    assert_near(gap, MESSAGE_BLOCK_GAP_PT, &what);
                }
            }
        }
    }
}

#[test]
fn a_continued_message_hides_its_header_only_when_grouped() {
    let grouped = runtime(conversation(), MessageLayout::default());
    assert!(has_part(&grouped, 1, "header"));
    assert!(!has_part(&grouped, 2, "header"));
    assert!(has_part(&grouped, 3, "header"));
    assert!(has_part(&grouped, 4, "header"), "half an hour later");
    let separate = runtime(
        conversation(),
        MessageLayout {
            grouped: false,
            ..MessageLayout::default()
        },
    );
    assert!(has_part(&separate, 2, "header"));
}

/// `count` messages from two authors taking turns, a minute apart.
fn long_history(count: u128) -> Vector<Arc<TimelineItem>> {
    (0..count)
        .map(|n| {
            let author = if n % 3 == 0 { "bia" } else { "ana" };
            let body = "words that wrap ".repeat(usize::try_from(n % 9).unwrap() + 1);
            message(n + 1_000, author, u32::try_from(n * 60).unwrap(), &body)
        })
        .collect()
}

/// Scrolls up from the bottom, and returns the first message in view (the
/// one the anchor names) with its top.
fn read_up(rt: &mut Runtime<Host>, wheel_pt: f32) -> (String, f32) {
    for _ in 0..4 {
        rt.wheel(400.0, 300.0, wheel_pt / 4.0);
        rt.frame(FRAME_S);
    }
    until_idle(rt);
    reader_row(rt)
}

fn reader_row(rt: &Runtime<Host>) -> (String, f32) {
    rt.elements()
        .into_iter()
        .filter(|element| element.path.ends_with("/message") && bottom(element.rect) > 0.0)
        .min_by(|a, b| a.rect[1].total_cmp(&b.rect[1]))
        .map(|element| (element.path, element.rect[1]))
        .expect("a message row in view")
}

fn top_of(rt: &Runtime<Host>, path: &str) -> f32 {
    rt.elements()
        .into_iter()
        .find(|element| element.path == path)
        .unwrap_or_else(|| panic!("{path} left the window"))
        .rect[1]
}

#[test]
fn the_message_being_read_keeps_its_place_when_the_density_changes() {
    let mut rt = runtime(long_history(300), MessageLayout::default());
    read_up(&mut rt, 1_500.0);
    let viewport = format!("timeline-{}-{}", Uuid::nil(), Uuid::from_u128(7));
    for density in [
        Density::Compact,
        Density::Comfortable,
        Density::ExtraCompact,
        Density::Normal,
    ] {
        let Anchor::Message { id, offset_pt } = rt.state.open.current_anchor() else {
            panic!("reading, not following the newest");
        };
        // What the Appearance settings will do: change it, keep the place.
        rt.update(|host, tx| {
            let anchor = host.open.current_anchor();
            host.layout.density = density;
            scroll_to(anchor, tx);
        });
        until_idle(&mut rt);
        let now_pt = rect(&rt, &anchor_key(id))[1] - rect(&rt, &viewport)[1];
        let moved = now_pt - offset_pt;
        assert!(moved.abs() < HOLD_PT, "{density:?} moved it {moved}");
    }
}

#[test]
fn ten_thousand_messages_page_edit_and_reflow_without_moving_the_reader() {
    let mut rt = runtime(long_history(10_000), MessageLayout::default());
    assert!(
        rt.elements().len() < 2_000,
        "only the window is built: {}",
        rt.elements().len()
    );
    let (path, top) = read_up(&mut rt, 2_000.0);
    let hold = |rt: &Runtime<Host>, step: &str| {
        let moved = top_of(rt, &path) - top;
        assert!(moved.abs() < HOLD_PT, "{step} moved it {moved}");
    };

    // An older page arrives above.
    rt.update(|host, _tx| {
        for n in 0..50 {
            host.open
                .items
                .push_front(message(n + 1, "ana", u32::try_from(n).unwrap(), "older"));
        }
        host.open.keys_version += 1;
    });
    until_idle(&mut rt);
    hold(&rt, "a prepended page");

    // A message above the view is edited and grows.
    let reader = rt
        .state
        .open
        .items
        .iter()
        .position(|item| path.contains(&super::row_key(item)))
        .expect("the reader's item");
    rt.update(|host, _tx| {
        let above = reader - 4;
        let TimelineItem::Message {
            id,
            message,
            content,
        } = host.open.items[above].as_ref()
        else {
            panic!("a message above");
        };
        let mut edited = Message::clone(message);
        edited.content = Some("edited and much longer ".repeat(12));
        edited.edited_at = Some("2026-11-21T09:00:00Z".to_owned());
        let edited = Arc::new(TimelineItem::Message {
            id: *id,
            message: Arc::new(edited),
            content: Arc::clone(content),
        });
        host.open.items.set(above, edited);
    });
    until_idle(&mut rt);
    hold(&rt, "an edit above");

    // The window narrows: every row reflows.
    rt.set_viewport((560.0, SIZE_PT.1), 1.0);
    until_idle(&mut rt);
    hold(&rt, "a reflow");
    assert!(rt.elements().len() < 2_000, "still only the window");
}

#[test]
fn the_reader_is_read_back_as_a_message_and_its_offset() {
    let mut rt = runtime(long_history(300), MessageLayout::default());
    assert_eq!(rt.state.open.current_anchor(), Anchor::Newest);
    read_up(&mut rt, 1_500.0);
    let Anchor::Message { id, offset_pt } = rt.state.open.current_anchor() else {
        panic!("not following the newest any more");
    };
    let viewport_top = rect(
        &rt,
        &format!("timeline-{}-{}", Uuid::nil(), Uuid::from_u128(7)),
    )[1];
    let row_top = rect(&rt, &anchor_key(id))[1];
    assert_near(offset_pt, row_top - viewport_top, "the anchor's offset");
}

#[test]
fn nearing_the_top_asks_for_the_older_page_once() {
    let mut rt = runtime(long_history(60), MessageLayout::default());
    read_up(&mut rt, 100_000.0);
    let older = |rt: &Runtime<Host>| {
        rt.state
            .requests
            .iter()
            .filter(|request| matches!(request, Request::Older))
            .count()
    };
    assert_eq!(older(&rt), 1);
    read_up(&mut rt, -200.0);
    read_up(&mut rt, 200.0);
    assert_eq!(older(&rt), 1, "one page at a time");
}

#[test]
fn an_idle_timeline_draws_nothing() {
    let mut rt = runtime(long_history(500), MessageLayout::default());
    for _ in 0..120 {
        rt.frame(FRAME_S);
    }
    assert!(!rt.frame(FRAME_S));
    assert!(rt.wake_deadline().is_none());
}
