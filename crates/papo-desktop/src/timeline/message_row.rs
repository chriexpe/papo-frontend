//! A message's row at each density (`docs/design.md` §5 and §6): a lead
//! column (the avatar, or Extra compact's time gutter) and one content
//! column holding the header and the text. The distance above the row is
//! the between-group or within-group spacing; the row's own padding is the
//! same either way, so every message is its own row with its own identity.
//!
//! The body is `content`'s. A reply has its reply line over it
//! (`reply_line`). The row's background is §4's: `highlight.selected` for
//! the message the composer points at or a jump just landed on, else
//! `highlight.mention` for one that mentions you or everyone, or replies to
//! you; no bar beside it. The author's name comes from the account's
//! members, a bridged author's from the message. A reading boundary
//! (`boundary`) is drawn over the distance above the row. A local echo of
//! your own message draws the same way, its send's mark (`send_mark`) in
//! the time's place and, while it waits, its text in the secondary colour;
//! your edits and deletions in flight draw by `change_marks`.
//!
//! A message from history is a focusable row: keys on it reach the host
//! (R reply, E edit, Delete, Escape), and it shows focus by `surface.hover`
//! and the focus ring on its outer edge, which takes no layout. What the
//! row shows is gathered by `row_parts`.

use papo_sdk::{Message, MessageKey, ResourceSnapshot, Uuid, parse_message_timestamp_us};
use rune::{El, col, el, memo, row, text};

use super::OpenTimeline;
use super::boundary::boundary as boundary_line;
use super::change_marks::{change_mark, delete_failed_line};
use super::changes::ChangeState;
use super::content::{self, Context, Doc, Draw, Edited};
use super::echoes::SendState;
use super::hover_bar;
use super::local_time::LocalTime;
use super::reactions;
use super::reply_line::reply_line;
use super::row_parts::{Highlight, Origin, Parts, RowMessage, parts};
use super::rows::{MESSAGE_PART, echo_key, message_key};
use super::send_mark::send_mark;
use super::view::{Style, TimelineHost};
use crate::design::{
    AVATAR_TOP_PT, DELETING_OPACITY, Density, FOCUS_RING_PT, HIGHLIGHT_ROW_RADIUS_PT, Lead,
    MESSAGE_BLOCK_GAP_PT, MESSAGE_HEADER_GAP_PT, MESSAGE_NAME_GAP_PT, MESSAGE_ROW_INSET_PT,
    Palette, RADIUS_AVATAR_PT, TYPE_BODY_STRONG, TYPE_CAPTION, TYPE_CAPTION_MEDIUM,
};
use crate::rail::initial;
use crate::strings;

/// A message's row. A message from history reports where its top is, for
/// the anchor. `previous` is the message drawn above it, if any.
pub(super) fn message_row<S: TimelineHost>(
    open: &OpenTimeline,
    row_message: &RowMessage<'_>,
    previous: Option<&Message>,
    style: &Style,
) -> El {
    let id = row_message.id;
    let layout = style.layout;
    let history = matches!(row_message.origin, Origin::History(_));
    let (parts, content) = parts(open, row_message, previous, style);
    let key = open.key;
    let revealed = open.revealed(id);
    let copied = open.copied(id);
    // The key too: two accounts on one server can show the same message,
    // and the row's handlers name their timeline.
    let version = (key, style.theme, layout, &parts, revealed, copied);
    let row_key = if history {
        message_key(id)
    } else {
        echo_key(id)
    };
    memo(row_key, version, || {
        let doc = Doc::new(
            content,
            &Context {
                resources: open.resources.as_deref(),
                revealed,
                jumbo: layout.density != Density::ExtraCompact,
            },
        );
        let mut palette = style.palette;
        let quiet = matches!(
            parts.change,
            Some((_, ChangeState::Saving | ChangeState::EditFailed))
        );
        if parts.sending == Some(SendState::Pending) || quiet {
            // A message or edit still on its way reads quieter.
            palette.text_primary = palette.text_secondary;
        }
        let draw = Draw {
            palette: &palette,
            key,
            message: id,
            copied,
            edited: parts.edited_at.map(|at| Edited {
                hint: edited_hint(at, &style.time),
            }),
        };
        let mut body = content::body::<S>(&doc, &draw);
        if let Some(chips) = reactions::chips::<S>(key, id, &parts.chips, parts.add_chip, &palette)
        {
            body = col().gap(MESSAGE_BLOCK_GAP_PT).child(body).child(chips);
        }
        if let Some((local, ChangeState::DeleteFailed)) = parts.change {
            body = col()
                .gap(MESSAGE_BLOCK_GAP_PT)
                .child(body)
                .child(delete_failed_line::<S>(key, local, &style.palette));
        }
        let mark = match (parts.sending, parts.change) {
            (Some(state), _) => Some(send_mark::<S>(key, id, state, &style.palette)),
            (None, Some((local, state))) => change_mark::<S>(key, local, state, &style.palette),
            (None, None) => None,
        };
        let line = parts.reply.as_ref().map(|line| {
            let spacing = layout.density.spacing();
            reply_line::<S>(key, line, spacing.lead, spacing.lead_gap_pt, &style.palette)
        });
        let bar = parts
            .bar
            .as_ref()
            .map(|bar| hover_bar::bar::<S>(key, id, bar, &style.palette));
        let (enter, exit) = hover_bar::row_motion();
        build(&parts, line, body, mark, style, |message| {
            if !history {
                return message;
            }
            message
                // The pointer on the row, or the keyboard on it, shows its bar.
                .hover_group()
                .hover_motion(enter, exit)
                .children(bar)
                .on_secondary_press(move |host: &mut S, press, tx| {
                    host.message_menu_requested(key, id, Some([press.x, press.y]), tx);
                })
                .on_layout_tx(move |host: &mut S, rect, tx| {
                    host.row_moved(key, id, [rect[1], rect[1] + rect[3]], tx);
                    tx.live_only();
                })
                // Ctrl+A selects the message under the pointer.
                .on_hover_change(move |host: &mut S, hovered, tx| {
                    host.message_hovered(key, id, hovered);
                    tx.live_only();
                })
                .focusable(true)
                // Only the keyboard focuses a message (Christian,
                // 2026-11-22): a click leaves the keyboard where it was.
                .preserve_focus_on_press(true)
                .on_key(move |host: &mut S, event, tx| host.message_key(key, id, event, tx))
        })
    })
}

/// The "(edited)" marker's hint: when, or nothing to add when the time
/// does not read.
fn edited_hint(edited_at: &str, time: &LocalTime) -> Option<String> {
    parse_message_timestamp_us(edited_at)
        .ok()
        .map(|at_us| time.edited_label(at_us))
}

/// The row: the distance above it, then the message itself (its reply line
/// and its row), whose top is the anchor, so a change of the distance above
/// leaves the text in place. `mark` takes the time's place: in the header,
/// else in the lead column.
fn build(
    parts: &Parts<'_>,
    line: Option<El>,
    body: El,
    mark: Option<El>,
    style: &Style,
    report: impl FnOnce(El) -> El,
) -> El {
    let palette = style.palette;
    let spacing = style.layout.density.spacing();
    let above_pt = if parts.continued {
        spacing.within_group_pt
    } else {
        spacing.between_groups_pt
    };
    let name = || {
        text(parts.name.clone())
            .key("name")
            .font(TYPE_BODY_STRONG)
            .color(palette.text_primary)
    };
    // The mark, when there is one, stands where the time would.
    let time = |mark: Option<El>| {
        mark.unwrap_or_else(|| {
            text(parts.time.clone())
                .key("time")
                .font(TYPE_CAPTION)
                .color(palette.text_muted)
        })
    };
    let (lead, content) = match spacing.lead {
        Lead::Avatar(size_pt) => {
            let mut content = col().gap(MESSAGE_BLOCK_GAP_PT);
            let lead = if parts.continued {
                // No header to hold the mark: the lead column does.
                row().key("lead").w(size_pt).justify_center().children(mark)
            } else {
                content = content.child(
                    row()
                        .key("header")
                        .gap(MESSAGE_HEADER_GAP_PT)
                        .items_end()
                        .child(name())
                        .child(time(mark)),
                );
                avatar(&parts.name, size_pt, &palette)
            };
            (lead, content.child(body))
        }
        Lead::Gutter(width_pt) => {
            let mut lead = el().key("lead").w(width_pt);
            if parts.show_time || mark.is_some() {
                lead = lead.child(time(mark));
            }
            let content = if style.layout.stacked {
                let mut stack = col().gap(MESSAGE_BLOCK_GAP_PT);
                if !parts.continued {
                    stack = stack.child(name());
                }
                stack.child(body)
            } else {
                // A continued row keeps the hidden name's width, so its
                // text stays on the column.
                let name = if parts.continued {
                    name().opacity(0.0).a11y_hidden()
                } else {
                    name()
                };
                row()
                    .gap(MESSAGE_NAME_GAP_PT)
                    .items_start()
                    .child(name)
                    .child(col().grow(1.0).min_w(0.0).child(body))
            };
            (lead, content)
        }
    };
    let row_part = row()
        .w_full()
        .gap(spacing.lead_gap_pt)
        .items_start()
        .child(lead)
        .child(content.key("content").grow(1.0).min_w(0.0));
    let mut message = col()
        .key(MESSAGE_PART)
        .w_full()
        .px(MESSAGE_ROW_INSET_PT)
        .py(spacing.row_padding_pt)
        .radius(HIGHLIGHT_ROW_RADIUS_PT)
        .role("listitem")
        // Keyboard focus: the ring on the outer edge, and the hover tone on
        // a row without a highlight of its own (Christian, 2026-11-22: a
        // highlight stays), neither of which moves anything.
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .children(line)
        .child(row_part);
    message = match parts.highlight {
        Highlight::Selected => message.fill(palette.surface_selected),
        Highlight::Mention => message.fill(palette.highlight_mention),
        Highlight::None => message.focus_fill(palette.surface_hover),
    };
    if matches!(parts.change, Some((_, ChangeState::Deleting))) {
        message = message
            .opacity(DELETING_OPACITY)
            .busy(true)
            .label(strings::delete_deleting());
    }
    let row = col().w_full().pt(above_pt).child(report(message));
    match parts.boundary {
        Some(boundary) => row.child(boundary_line(boundary, above_pt, &palette)),
        None => row,
    }
}

/// The first letter of `name` on a rounded square, `AVATAR_TOP_PT` below
/// the top of its lead box (`docs/design.md` §5, Distances are measured to
/// what is seen). The box is as tall as the avatar alone, so the offset
/// never makes a row taller than it was.
pub(crate) fn avatar(name: &str, size_pt: f32, palette: &Palette) -> El {
    el().key("lead").size(size_pt, size_pt).a11y_hidden().child(
        el().key("avatar")
            .absolute(0.0, AVATAR_TOP_PT)
            .size(size_pt, size_pt)
            .radius(RADIUS_AVATAR_PT)
            .center()
            .fill(palette.surface_hover)
            .child(
                text(initial(name))
                    .font(TYPE_CAPTION_MEDIUM)
                    .color(palette.text_secondary),
            ),
    )
}

/// The author's name: a member's nickname or username, a bridged author's
/// name, else "Someone".
pub(crate) fn author_name(message: &Message, resources: Option<&ResourceSnapshot>) -> String {
    if let Some(external) = &message.external_author {
        return external
            .name
            .clone()
            .unwrap_or_else(|| strings::someone().to_owned());
    }
    let member = message
        .author_id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
        .zip(resources)
        .and_then(|(id, resources)| resources.users.get(id));
    member
        .and_then(|user| user.nickname.clone().or_else(|| user.username.clone()))
        .unwrap_or_else(|| strings::someone().to_owned())
}

/// When the message was created, in Unix microseconds.
pub(crate) fn created_us(message: &Message) -> Option<i64> {
    MessageKey::from_message(message)
        .ok()
        .map(|key| key.created_at_us)
}
