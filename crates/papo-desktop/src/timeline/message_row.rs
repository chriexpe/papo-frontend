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
use papo_sdk_ui::message_content::MessageContent;
use rune::{El, col, el, memo, row, text};

use super::boundary::boundary as boundary_line;
use super::change_marks::{change_mark, delete_failed_line};
use super::changes::ChangeState;
use super::content::{self, Context, Doc, Draw, Edited};
use super::echoes::SendState;
use super::hover_bar;
use super::local_time::LocalTime;
use super::no_avatar::without_avatars;
use super::reactions;
use super::reply_line::reply_line;
use super::row_media::{
    Body, ReceivedSignature, glides, media_of, previews_grow, previews_of, received_of, signature,
    unsent_of,
};
use super::row_parts::{Highlight, Origin, Parts, RowMessage, parts};
use super::rows::{author_key, avatar_key, echo_key, message_key, message_part_key};
use super::send_mark::send_mark;
use super::view::{Style, TimelineHost};
use super::{OpenTimeline, TimelineKey};
use crate::design::{
    AVATAR_TOP, DELETING_OPACITY, Density, FOCUS_RING_PT, HIGHLIGHT_ROW_RADIUS, Lead, Look,
    MESSAGE_BLOCK_GAP, MESSAGE_HEADER_GAP, MESSAGE_ROW_INSET, RADIUS_AVATAR, TYPE_BODY_STRONG,
    TYPE_CAPTION, TYPE_CAPTION_MEDIUM,
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
    // A GIF-only message shows only its GIF (§16, "A GIF-only message hides its link"); Copy
    // text still copies the link, read from the message.
    let text_hidden = papo_sdk_ui::hides_text(row_message.message);
    let hidden = MessageContent::default();
    let content = if text_hidden { &hidden } else { content };
    let key = open.key;
    let unsent = unsent_of(open, row_message, style);
    let unsent_version = unsent.as_ref().map(signature);
    let received = received_of(open, row_message, history, style);
    let revealed = open.revealed(id);
    let copied = open.copied(id);
    // The key too: two accounts on one server can show the same message,
    // and the row's handlers name their timeline.
    // The look too: a row is drawn at every Appearance setting.
    let version = (
        key,
        style.look,
        layout,
        &parts,
        revealed,
        copied,
        unsent_version,
        received.as_ref().map(|(_, signature)| signature.clone()),
        text_hidden,
        starred(row_message, style),
    );
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
        let look = row_look(&parts, style);
        let draw = Draw {
            look: &look,
            key,
            message: id,
            copied,
            edited: parts.edited_at.map(|at| Edited {
                hint: edited_hint(at, &style.time),
            }),
            standalone: false,
            emoji: &style.emoji,
        };
        let body = body_of::<S>(
            &doc,
            &draw,
            &parts,
            unsent.as_ref(),
            received.as_ref(),
            row_message,
            style,
            &look,
        );
        let mark = mark::<S>(key, id, &parts, &style.look);
        let line = parts.reply.as_ref().map(|line| {
            let spacing = look.row_spacing(layout);
            reply_line::<S>(key, line, spacing.lead, spacing.lead_gap_pt, &style.look)
        });
        let bar = parts
            .bar
            .as_ref()
            .map(|bar| hover_bar::bar::<S>(key, id, bar, &style.look));
        let (enter, exit) = hover_bar::row_motion();
        let name_press = author_press::<S>(key, row_message.message, history);
        let grows = previews_grow(received.as_ref(), style);
        let built = build(id, &parts, line, body, mark, style, name_press, |message| {
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
                    if !host.message_hovered(key, id, hovered) {
                        tx.live_only();
                    }
                })
                // A press anywhere on the message that nothing in it took
                // (text, a link, the bar): the conversation was pressed.
                // The row stays a list item, and still takes the keyboard.
                .on_background_press_tx(move |host: &mut S, _tx| host.message_pressed(key))
                .focusable(true)
                // Only the keyboard focuses a message (Christian,
                // 2026-11-22): a click leaves the keyboard where it was.
                .preserve_focus_on_press(true)
                .on_key(move |host: &mut S, event, tx| host.message_key(key, id, event, tx))
        });
        glides(built, grows)
    })
}

/// A message's body: its text, its blocks (link previews, then attachments) and what follows.
#[expect(
    clippy::too_many_arguments,
    reason = "the row's parts, each drawn by its own module"
)]
fn body_of<S: TimelineHost>(
    doc: &Doc,
    draw: &Draw<'_>,
    parts: &Parts<'_>,
    unsent: Option<&super::unsent_attachments::Unsent<'_>>,
    received: Option<&(crate::message_media::received::MediaAt, ReceivedSignature)>,
    row_message: &RowMessage<'_>,
    style: &Style,
    look: &Look,
) -> Body {
    Body {
        text: content::body::<S>(doc, draw),
        blocks: [
            previews_of::<S>(received, row_message, style, look),
            media_of::<S>(unsent, received, row_message, style, look),
        ]
        .into_iter()
        .flatten()
        .collect(),
        after: after::<S>(draw.key, draw.message, parts, look, &style.look),
    }
}

/// What follows a message's blocks: its reaction chips, and a failed deletion's line (drawn in
/// `full`, the row's look before it read quieter).
fn after<S: TimelineHost>(
    key: TimelineKey,
    id: Uuid,
    parts: &Parts<'_>,
    look: &Look,
    full: &Look,
) -> Vec<El> {
    let mut after = Vec::new();
    if let Some(chips) = reactions::chips::<S>(key, id, &parts.chips, parts.add_chip, look) {
        after.push(chips);
    }
    if let Some((local, ChangeState::DeleteFailed)) = parts.change {
        after.push(delete_failed_line::<S>(key, local, full));
    }
    after
}

/// The row's look: a message or edit still on its way reads quieter.
fn row_look(parts: &Parts<'_>, style: &Style) -> Look {
    let mut look = style.look;
    let quiet = matches!(
        parts.change,
        Some((_, ChangeState::Saving | ChangeState::EditFailed))
    );
    if parts.sending == Some(SendState::Pending) || quiet {
        look.palette.text_primary = look.palette.text_secondary;
    }
    look
}

/// What stands where the time would: your unsent message's send mark, or
/// your change in flight's mark.
fn mark<S: TimelineHost>(key: TimelineKey, id: Uuid, parts: &Parts<'_>, look: &Look) -> Option<El> {
    match (parts.sending, parts.change) {
        (Some(state), _) => Some(send_mark::<S>(key, id, state, look)),
        (None, Some((local, state))) => change_mark::<S>(key, local, state, look),
        (None, None) => None,
    }
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
#[expect(
    clippy::too_many_arguments,
    reason = "the row's parts, each built by its own owner"
)]
fn build(
    id: Uuid,
    parts: &Parts<'_>,
    line: Option<El>,
    body: Body,
    mark: Option<El>,
    style: &Style,
    name_press: impl Fn(El, String) -> El,
    report: impl FnOnce(El) -> El,
) -> El {
    let look = style.look;
    let spacing = look.row_spacing(style.layout);
    let above_pt = if parts.continued {
        spacing.within_group_pt
    } else {
        spacing.between_groups_pt
    };
    let name = || name_press(name_text(&parts.name, &look), author_key(id));
    // The mark, when there is one, stands where the time would; with Show
    // times off there is only the mark.
    let time = |mark: Option<El>| {
        mark.or_else(|| {
            style.layout.times.then(|| {
                text(parts.time.clone())
                    .key("time")
                    .font(look.font(TYPE_CAPTION))
                    .color(look.palette.text_muted)
            })
        })
    };
    let (lead, content) = match spacing.lead {
        Lead::Avatar(size_pt) => {
            let mut content = col().gap(look.space(MESSAGE_BLOCK_GAP));
            let lead = if parts.continued {
                // No header to hold the mark: the lead column does.
                row().key("lead").w(size_pt).justify_center().children(mark)
            } else {
                content = content.child(
                    row()
                        .key("header")
                        .gap(look.space(MESSAGE_HEADER_GAP))
                        .items_end()
                        .child(name())
                        .children(time(mark)),
                );
                avatar(&parts.name, size_pt, &look, |tile| {
                    name_press(tile, avatar_key(id))
                })
            };
            (Some(lead), content.child(body.joined(&look)))
        }
        Lead::Gutter(width_pt) => {
            let mut lead = el().key("lead").w(width_pt);
            if parts.show_time || mark.is_some() {
                lead = lead.children(time(mark));
            }
            (Some(lead), without_avatars(parts, body, None, style, name))
        }
        // No time column (Show times off without avatars): the mark goes
        // after the name, or on a continued row at the right end of its
        // first line, where it moves no text (§8).
        Lead::None => (None, without_avatars(parts, body, mark, style, name)),
    };
    let row_part = row()
        .w_full()
        .gap(spacing.lead_gap_pt)
        .items_start()
        .children(lead)
        .child(content.key("content").grow(1.0).min_w(0.0));
    let mut message = col()
        .key(message_part_key(id))
        .w_full()
        .px(look.space(MESSAGE_ROW_INSET))
        .py(spacing.row_padding_pt)
        .radius(look.radius(HIGHLIGHT_ROW_RADIUS))
        .role("listitem")
        // Keyboard focus: the ring on the outer edge, and the hover tone on
        // a row without a highlight of its own (Christian, 2026-11-22: a
        // highlight stays), neither of which moves anything.
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .children(line)
        .child(row_part);
    message = match parts.highlight {
        Highlight::Selected => message.fill(look.palette.surface_selected),
        Highlight::Mention => message.fill(look.palette.highlight_mention),
        Highlight::None => message.focus_fill(look.palette.surface_hover),
    };
    if matches!(parts.change, Some((_, ChangeState::Deleting))) {
        message = message
            .opacity(DELETING_OPACITY)
            .busy(true)
            .label(strings::delete_deleting());
    }
    // The boundary goes first: out of flow over the gap above, it is drawn under the message,
    // so the hover bar, which hangs into that gap, stays on top of it.
    col()
        .w_full()
        .pt(above_pt)
        .children(
            parts
                .boundary
                .map(|boundary| boundary_line(boundary, above_pt, &look)),
        )
        .child(report(message))
}

/// What makes a person's name or avatar open their card beside it, keyed
/// by the anchor it is given. A bridged author has no person behind them,
/// and an echo not yet sent no row to open from.
fn author_press<S: TimelineHost>(
    key: TimelineKey,
    message: &Message,
    history: bool,
) -> impl Fn(El, String) -> El {
    let author = papo_sdk_ui::AuthorIdentity::for_message(message);
    move |part: El, anchor: String| match (&author, history) {
        (papo_sdk_ui::AuthorIdentity::Person(user), true) => {
            let user = *user;
            part.key(anchor.clone())
                .on_press_tx(move |host: &mut S, tx| {
                    host.author_pressed(key, user, anchor.clone(), tx);
                })
        }
        _ => part,
    }
}

/// The author's name at the head of a group.
pub(super) fn name_text(name: &str, look: &Look) -> El {
    text(name.to_owned())
        .key("name")
        .font(look.font(TYPE_BODY_STRONG))
        .color(look.palette.text_primary)
}

/// The first letter of `name` on a rounded square, [`AVATAR_TOP`] below the
/// top of its lead box (`docs/design.md` §5, Distances are measured to what
/// is seen). The box holds the offset and the avatar, so a row too short
/// for them grows rather than let the avatar reach past its background
/// (Christian, 2026-11-23: at 12 pt Comfortable it reached 4.6 pt past).
pub(super) fn avatar(name: &str, size_pt: f32, look: &Look, press: impl FnOnce(El) -> El) -> El {
    el().key("lead")
        .size(size_pt, size_pt + look.text_multiple(AVATAR_TOP))
        .a11y_hidden()
        .child(press(
            el().key("avatar")
                .absolute(0.0, look.text_multiple(AVATAR_TOP))
                .size(size_pt, size_pt)
                .radius(look.radius(RADIUS_AVATAR))
                .center()
                .fill(look.palette.surface_hover)
                .child(
                    text(initial(name))
                        .font(look.font(TYPE_CAPTION_MEDIUM))
                        .color(look.palette.text_secondary),
                ),
        ))
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

/// Whether the row's GIF is a favorite: its star's fill, so adding or removing one redraws the
/// row.
fn starred(row_message: &RowMessage<'_>, style: &Style) -> bool {
    papo_sdk_ui::favorite_gif(row_message.message)
        .is_some_and(|gif| style.media.favorite_gifs.contains(&gif.id))
}
