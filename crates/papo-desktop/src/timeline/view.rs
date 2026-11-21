//! Draws the shown timeline: one scroll viewport per (account, channel)
//! that follows its bottom, over Rune's index-built virtual rows keyed by
//! the SDK items' identities. Messages are `message_row`'s; this draws the
//! rest (date separators, gaps, loading rows) and wires the reports the
//! anchor, paging and gap fills read.
//!
//! Pure: rows read the host's state when Rune builds them, and their
//! handlers name their timeline and item by key, never by index.

use std::borrow::Cow;
use std::ops::Range;

use papo_sdk::Uuid;
use papo_sdk_ui::TimelineItem;
use rune::{El, GrowthSpec, ScrollSpec, Tx, col, el, memo, row, text};

use super::local_time::{LocalTime, day_label};
use super::message_row::message_row;
use super::{OpenTimeline, Phase, TimelineKey, row_key};
use crate::design::{
    DATE_RULE_GAP_PT, DATE_RULE_PT, FOCUS_RING_PT, Lead, MessageLayout, Palette, RADIUS_AVATAR_PT,
    RADIUS_CONTROL_PT, SKELETON_BAR_PT, SKELETON_WIDTHS, SPACE_SM_PT, TIMELINE_INSET_PT,
    TYPE_CAPTION, TYPE_SMALL, TYPE_SMALL_MEDIUM, Theme,
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
    /// The viewport's top is at `top_pt` in window points.
    fn viewport_moved(&mut self, key: TimelineKey, top_pt: f32, tx: &mut Tx);
    /// Message `id` is laid out from `edges_pt[0]` to `edges_pt[1]`,
    /// top to bottom, in window points.
    fn row_moved(&mut self, key: TimelineKey, id: Uuid, edges_pt: [f32; 2], tx: &mut Tx);
    /// Whether the viewport shows its bottom. Returns whether the view
    /// changed.
    fn bottom_shown(&mut self, key: TimelineKey, at_bottom: bool) -> bool;
    /// Retry on the failed gap with this cursor.
    fn retry_gap(&mut self, key: TimelineKey, cursor_id: &str);
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
    let rows = col()
        .key("rows")
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
    col()
        .key(viewport_key(key))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .label(strings::timeline_label())
        .on_at_bottom_if(move |host: &mut S, at_bottom| host.bottom_shown(key, at_bottom))
        .on_layout_tx(move |host: &mut S, rect, tx| {
            host.viewport_moved(key, rect[1], tx);
            tx.live_only();
        })
        .child(rows)
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
    let previous = index
        .checked_sub(1)
        .and_then(|before| open.items.get(before))
        .map(AsRef::as_ref);
    match item.as_ref() {
        TimelineItem::Message { id, message, .. } => {
            message_row::<S>(open, *id, message, previous, style)
        }
        TimelineItem::DateSeparator { day, before } => date_row(*day, *before, style),
        TimelineItem::Gap(gap) if open.failed_gaps.contains(&gap.cursor.id) => {
            gap_failed::<S>(open.key, &gap.cursor.id, style)
        }
        // A gap being filled shows its own skeleton already.
        TimelineItem::Loading(_) if open.has_fill() => el().w_full(),
        // A gap fills itself as it nears; its skeleton stands for it.
        TimelineItem::Gap(_) | TimelineItem::Loading(_) => skeleton(style),
        // The unread marker is D3c's; a local echo comes with the composer.
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
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
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
        .children(SKELETON_WIDTHS.iter().map(|&(name, body)| {
            row()
                .w_full()
                .pt(spacing.between_groups_pt)
                .gap(spacing.lead_gap_pt)
                .items_start()
                .child(match spacing.lead {
                    Lead::Avatar(size_pt) => el()
                        .size(size_pt, size_pt)
                        .radius(RADIUS_AVATAR_PT)
                        .fill(palette.surface_hover),
                    Lead::Gutter(width_pt) => el().w(width_pt),
                })
                .child(
                    col()
                        .grow(1.0)
                        .gap(bar_gap_pt)
                        .pt(bar_gap_pt)
                        .child(bar(name))
                        .child(bar(body)),
                )
        }))
}
