//! Draws the shown timeline: one scroll viewport per (account, channel)
//! that follows its bottom, over Rune's index-built virtual rows keyed by
//! the SDK items' identities, with the reading controls floating over it
//! (`controls`). Messages are `message_row`'s; this draws the rest (date
//! separators, gaps, loading rows) and wires the reports the anchor,
//! paging, gap fills and read reports read.
//!
//! Pure: rows read the host's state when Rune builds them, and their
//! handlers name their timeline and item by key, never by index.

use std::borrow::Cow;
use std::ops::Range;

use papo_sdk::Uuid;
use papo_sdk_ui::TimelineItem;
use rune::{El, GrowthSpec, KeyEvent, ScrollSpec, Tx, col, el, memo, row, text};

use super::content::Press;
use super::content::copy::{self, Source};
use super::controls::{jump_stack, seeking_pill};
use super::echoes::SendState;
use super::grouping::previous_message;
use super::hover_bar::BarPress;
use super::local_time::{LocalTime, day_label};
use super::message_row::message_row;
use super::row_parts::{Origin, RowMessage};
use super::{OpenTimeline, Phase, TimelineKey, row_key};
use crate::design::{
    DATE_RULE_GAP_PT, DATE_RULE_PT, Density, FOCUS_RING_PT, Lead, MESSAGE_ROW_INSET_PT,
    MessageLayout, Palette, RADIUS_AVATAR_PT, RADIUS_CONTROL_PT, SKELETON_BAR_PT, SKELETON_WIDTHS,
    SPACE_SM_PT, TIMELINE_INSET_PT, TYPE_CAPTION, TYPE_SMALL, TYPE_SMALL_MEDIUM, Theme,
};
use crate::strings;

/// A row's height before Rune measures it, in points: a one-line message
/// at Normal density.
const ESTIMATED_ROW_PT: f32 = 44.0;

/// The state the timeline is drawn from, and the reports its rows send.
/// Every report names the timeline it came from; one no longer shown is
/// ignored.
pub(crate) trait TimelineHost: Sized + 'static {
    /// The shown timeline.
    fn timeline(&self) -> Option<&OpenTimeline>;
    /// Rows `range` are visible. Returns whether the view changed.
    fn rows_visible(&mut self, key: TimelineKey, range: Range<usize>) -> bool;
    /// The viewport's top is at `top_pt` in window points, and it is
    /// `height_pt` tall.
    fn viewport_moved(&mut self, key: TimelineKey, top_pt: f32, height_pt: f32, tx: &mut Tx);
    /// Message `id` is laid out from `edges_pt[0]` to `edges_pt[1]`,
    /// top to bottom, in window points.
    fn row_moved(&mut self, key: TimelineKey, id: Uuid, edges_pt: [f32; 2], tx: &mut Tx);
    /// Whether the viewport shows its bottom. Returns whether the view
    /// changed.
    fn bottom_shown(&mut self, key: TimelineKey, at_bottom: bool) -> bool;
    /// Retry on the failed gap with this cursor.
    fn retry_gap(&mut self, key: TimelineKey, cursor_id: &str);
    /// A press inside message `message`'s content.
    fn content_pressed(&mut self, key: TimelineKey, message: Uuid, press: Press, tx: &mut Tx);
    /// Code block `index` (its text's index) of message `message` was
    /// copied.
    fn code_copied(&mut self, key: TimelineKey, message: Uuid, index: usize, tx: &mut Tx);
    /// A press on a message of timeline `key` that nothing in it took.
    fn message_pressed(&mut self, key: TimelineKey);
    /// The pointer entered or left message `message`. Returns whether the
    /// view changed.
    fn message_hovered(&mut self, key: TimelineKey, message: Uuid, hovered: bool) -> bool;
    /// ⌛ was pressed.
    fn hourglass_pressed(&mut self, key: TimelineKey);
    /// ↓ was pressed.
    fn down_pressed(&mut self, key: TimelineKey, tx: &mut Tx);
    /// Cancel on ⌛'s pill was pressed.
    fn seek_cancelled(&mut self, key: TimelineKey);
    /// The mark of your unsent message `local`, in state `state`, was
    /// pressed: its menu opens.
    fn send_state_pressed(&mut self, key: TimelineKey, local: Uuid, state: SendState);
    /// A key on the focused message `id`. Returns whether it was taken.
    fn message_key(&mut self, key: TimelineKey, id: Uuid, event: &KeyEvent, tx: &mut Tx) -> bool;
    /// A reply line was pressed: its original `original` is shown.
    fn reply_line_pressed(&mut self, key: TimelineKey, original: Uuid, tx: &mut Tx);
    /// The ⚠ of your failed edit `local` was pressed: its menu opens.
    fn change_mark_pressed(&mut self, key: TimelineKey, local: Uuid);
    /// Try again on your failed change `local`.
    fn change_retried(&mut self, key: TimelineKey, local: Uuid);
    /// Dismiss on your failed change `local`: it is dropped.
    fn change_discarded(&mut self, key: TimelineKey, local: Uuid);
    /// A press on message `id`'s hover bar.
    fn bar_pressed(&mut self, key: TimelineKey, id: Uuid, press: BarPress, tx: &mut Tx);
    /// The pointer came to rest on one of message `id`'s chips: who
    /// reacted is read, for its hint.
    fn chip_hovered(&mut self, key: TimelineKey, id: Uuid);
    /// The reactions list was asked for beside message `id`'s chip for
    /// `emoji`, by a secondary click on it.
    fn chip_menu_requested(&mut self, key: TimelineKey, id: Uuid, emoji: String, tx: &mut Tx);
    /// A link's menu was asked for, by a secondary click on `url` at `at`,
    /// in window points.
    fn link_menu_requested(&mut self, key: TimelineKey, url: String, at: [f32; 2], tx: &mut Tx);
    /// Message `id`'s menu was asked for: by a secondary click at `at`, in
    /// window points, else from its hover bar.
    fn message_menu_requested(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        at: Option<[f32; 2]>,
        tx: &mut Tx,
    );
}

/// How the rows look: the same for every row of one view.
#[derive(Clone)]
pub(crate) struct Style {
    pub(crate) layout: MessageLayout,
    pub(crate) theme: Theme,
    pub(crate) palette: Palette,
    pub(crate) time: LocalTime,
    /// The current civil day, read once per view: "Today" and "Yesterday".
    pub(crate) today: i64,
    /// The "Unread hourglass" setting: ⌛ and the skipped block's edges.
    pub(crate) hourglass: bool,
    /// The message the composer points at (a reply's original, or the
    /// message being edited): it wears `highlight.selected`.
    pub(crate) selected: Option<Uuid>,
    /// What reacting reads, the same for every row.
    pub(crate) reacting: super::reacting::Reacting,
}

/// The shown timeline: its rows, skeleton rows while it opens, and
/// nothing when none shows or it could not open.
pub(crate) fn view<S: TimelineHost>(host: &S, style: Style) -> El {
    let Some(open) = host.timeline() else {
        return el().w_full().grow(1.0);
    };
    let key = open.key;
    match open.phase {
        Phase::Open => {}
        Phase::Opening => {
            return col()
                .key(viewport_key(key))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .justify_end()
                .px(TIMELINE_INSET_PT)
                .busy(true)
                .label(strings::loading_messages())
                .child(skeleton(&style));
        }
        Phase::Failed | Phase::Closed => return el().w_full().grow(1.0),
    }
    let source = Source {
        items: open.items.clone(),
        resources: open.resources.clone(),
        revealed: open.revealed.clone(),
        jumbo: style.layout.density != Density::ExtraCompact,
        time: style.time.clone(),
        today: style.today,
    };
    let stack = jump_stack::<S>(open, &style);
    let seeking = open
        .seek
        .is_some()
        .then(|| seeking_pill::<S>(key, &style.palette));
    let rows = col()
        .key("rows")
        // Its messages are the list ↑ and ↓ move along (`docs/design.md`
        // §12); a focused one takes R, E and Delete.
        .role("list")
        .focus_group()
        // Every message's text is one selection, which copies as the
        // messages show (`content::copy`).
        .selection_group(true)
        .selection_copy(move |selection| copy::text_of(&source, selection))
        .w_full()
        .px(TIMELINE_INSET_PT)
        .pb(TIMELINE_INSET_PT)
        .growth(GrowthSpec::default())
        .virtual_rows(
            open.items.len(),
            ESTIMATED_ROW_PT,
            |host: &S, index| {
                host.timeline()
                    .and_then(|open| open.items.get(index))
                    .map_or(Cow::Borrowed("missing"), |item| Cow::Owned(row_key(item)))
            },
            move |host: &S, index| {
                host.timeline()
                    .map_or_else(el, |open| item_row::<S>(open, index, &style))
            },
        )
        .virtual_rows_version(open.keys_version)
        .on_visible_rows(move |host: &mut S, range| host.rows_visible(key, range));
    let viewport = col()
        .key(viewport_key(key))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: open.follows_newest(),
        })
        .label(strings::timeline_label())
        .on_at_bottom_if(move |host: &mut S, at_bottom| host.bottom_shown(key, at_bottom))
        .on_layout_tx(move |host: &mut S, rect, tx| {
            host.viewport_moved(key, rect[1], rect[3], tx);
            tx.live_only();
        })
        .child(rows);
    col()
        .key("timeline")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(viewport)
        .children(stack)
        .children(seeking)
}

/// The viewport's key: each timeline has its own, so showing another opens
/// a new viewport rather than scrolling this one. Channel IDs are UUIDs, so
/// the account's user and the channel tell timelines apart.
pub(crate) fn viewport_key(key: TimelineKey) -> String {
    format!("timeline-{}-{}", key.account.user(), key.channel)
}

/// The row for item `index`.
fn item_row<S: TimelineHost>(open: &OpenTimeline, index: usize, style: &Style) -> El {
    let Some(item) = open.items.get(index) else {
        return el();
    };
    // The SDK's unread marker moves as rows are read; drawn as nothing,
    // it groups nothing either, so reading moves no row. The divider held
    // for the visit splits its group instead (`reading`).
    let previous = previous_message(&open.items, &open.echoes, index);
    match item.as_ref() {
        TimelineItem::Message {
            id,
            message,
            content,
            reply,
            pending,
            ..
        } => {
            let row = RowMessage {
                id: *id,
                message,
                content,
                reply: reply.as_ref(),
                origin: Origin::History(pending.as_deref()),
            };
            message_row::<S>(open, &row, previous, style)
        }
        TimelineItem::LocalEcho { id, reply, .. } => match open.echoes.get(*id) {
            Some(echo) => {
                let row = RowMessage {
                    id: *id,
                    message: &echo.message,
                    content: &echo.content,
                    reply: reply.as_ref(),
                    origin: Origin::Echo(echo.state),
                };
                message_row::<S>(open, &row, previous, style)
            }
            // The SDK echoes only new messages; an echo it could not draw
            // as one shows nothing.
            None => el().w_full(),
        },
        TimelineItem::DateSeparator { day, before } => date_row(*day, *before, style),
        TimelineItem::Gap(gap) if open.failed_gaps.contains(&gap.cursor.id) => {
            gap_failed::<S>(open.key, &gap.cursor.id, style)
        }
        // A gap being filled shows its own skeleton already.
        TimelineItem::Loading(_) if open.fills_beside(index) => el().w_full(),
        // A gap fills itself as it nears; its skeleton stands for it.
        TimelineItem::Gap(_) | TimelineItem::Loading(_) => skeleton(style),
        // The unread marker shows as the divider held for the visit, over
        // the rows.
        _ => el().w_full(),
    }
}

/// "Today", centred between two hairlines.
fn date_row(day: i64, before: Uuid, style: &Style) -> El {
    let palette = style.palette;
    let label = day_label(day, style.today);
    let between_groups_pt = style.layout.density.spacing().between_groups_pt;
    memo(
        format!("d{before}"),
        (style.theme, style.layout, &label),
        || {
            let rule = || {
                el().grow(1.0)
                    .h(DATE_RULE_PT)
                    .fill(palette.line_subtle)
                    .a11y_hidden()
            };
            row()
                .w_full()
                .pt(between_groups_pt)
                .center_y()
                .gap(DATE_RULE_GAP_PT)
                .role("separator")
                .child(rule())
                .child(
                    text(label.clone())
                        .font(TYPE_CAPTION)
                        .color(palette.text_muted),
                )
                .child(rule())
        },
    )
}

/// "Couldn't load these messages", with Retry.
fn gap_failed<S: TimelineHost>(key: TimelineKey, cursor_id: &str, style: &Style) -> El {
    let palette = style.palette;
    let cursor = cursor_id.to_owned();
    let retry = text(strings::retry())
        .key("retry")
        .font(TYPE_SMALL_MEDIUM)
        .color(palette.link)
        .role("button")
        .focusable(true)
        .radius(RADIUS_CONTROL_PT)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .on_press(move |host: &mut S| host.retry_gap(key, &cursor));
    row()
        .w_full()
        .pt(style.layout.density.spacing().between_groups_pt)
        .justify_center()
        .center_y()
        .gap(SPACE_SM_PT * 2.0)
        .role("alert")
        .child(
            text(strings::gap_failed())
                .font(TYPE_SMALL)
                .color(palette.text_muted),
        )
        .child(retry)
}

/// Placeholder messages while a page loads: an avatar square and grey
/// bars, at the current density.
fn skeleton(style: &Style) -> El {
    let palette = style.palette;
    let spacing = style.layout.density.spacing();
    let (bar_pt, bar_gap_pt) = SKELETON_BAR_PT;
    let bar = |fraction: f32| {
        el().h(bar_pt)
            .w_frac(fraction)
            .radius(bar_pt / 2.0)
            .fill(palette.surface_hover)
    };
    col()
        .w_full()
        .a11y_hidden()
        .children(
            SKELETON_WIDTHS
                .iter()
                .enumerate()
                .map(|(place, &(name, body))| {
                    // A message row's own inset and padding, so the placeholders
                    // line up with the messages around them. They are a fixed set:
                    // each one's place is its identity.
                    col()
                        .key(format!("skeleton-{place}"))
                        .w_full()
                        .pt(spacing.between_groups_pt)
                        .child(
                            row()
                                .w_full()
                                .px(MESSAGE_ROW_INSET_PT)
                                .py(spacing.row_padding_pt)
                                .gap(spacing.lead_gap_pt)
                                .items_start()
                                .child(match spacing.lead {
                                    Lead::Avatar(size_pt) => el()
                                        .key("lead")
                                        .size(size_pt, size_pt)
                                        .radius(RADIUS_AVATAR_PT)
                                        .fill(palette.surface_hover),
                                    Lead::Gutter(width_pt) => el().key("lead").w(width_pt),
                                })
                                .child(
                                    col()
                                        .grow(1.0)
                                        .gap(bar_gap_pt)
                                        .pt(bar_gap_pt)
                                        .child(bar(name))
                                        .child(bar(body)),
                                ),
                        )
                }),
        )
}
