//! Draws the conversation's pins panel (`docs/design.md` §10; the header
//! it opens from is `header`'s): it takes the actions pill's place, its
//! header row "Pinned messages" and
//! the count with 📌 where it was, and the pins grow down, each a message
//! on its own divided by hairlines, Unpin on hover or focus. Shrunk
//! (`PanelPeek`), it keeps its header and one line for the opened pin.

use std::sync::Arc;

use rune::{Cx, El, KeyEvent, ScrollSpec, col, el, icon, memo, row, text};

use super::Shell;
use super::pins::{PINS_PANEL_KEY, PinsPanel, pin_row_key};
use crate::design::{
    FOCUS_RING_PT, HEADER_PILL_HEIGHT, Look, PILL_BUTTON_PT, PILL_ICON_PT, PILL_OUTLINE_PT,
    RADIUS_PILL, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XL, Surfaces, TYPE_BODY_STRONG, TYPE_SMALL,
    TYPE_TITLE,
};
use crate::strings;
use crate::timeline::pinned::Pin;
use crate::timeline::replies::{OriginalText, original_text};
use crate::timeline::standalone::standalone_message;
use crate::timeline::{OpenTimeline, Style};

/// The panel's widest (`size.panel`), in points.
const PANEL_MAX_PT: f32 = 440.0;
/// How much of the conversation's height the panel may take.
const PANEL_HEIGHT_SHARE: f32 = 0.7;
/// The empty panel's pin icon, in points.
const EMPTY_ICON_PT: f32 = 32.0;

/// The open panel over the conversation's top-right, when it is open on
/// the shown conversation.
pub(super) fn panel(shell: &Shell, style: &Style, cx: &Cx<Shell>) -> Option<El> {
    let state = shell.shown_pins_panel()?;
    let open = shell.tabs.shown()?;
    let look = style.look;
    let [width_pt, height_pt] = cx
        .layout_size("conversation")
        .unwrap_or([PANEL_MAX_PT, 600.0]);
    let width = PANEL_MAX_PT.min(width_pt - 2.0 * look.space(SPACE_LG));
    let count = open.pins.list().map(<[_]>::len);
    let head = row()
        .w_full()
        .h(look.space(HEADER_PILL_HEIGHT))
        .pl(look.space(SPACE_LG))
        .pr(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM))
        .center_y()
        .child(
            text(strings::pins_title())
                .font(look.font(TYPE_TITLE))
                .color(look.palette.text_primary)
                .role("heading"),
        )
        .children(count.map(|count| {
            text(count.to_string())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
        }))
        .child(el().grow(1.0))
        .child(super::header::pins_button(true, &look));
    let body = if state.peek {
        peek_line(open, &state, &look)
    } else {
        Some(list(
            shell,
            open,
            &state,
            style,
            height_pt * PANEL_HEIGHT_SHARE,
        ))
    };
    Some(
        col()
            .key(PINS_PANEL_KEY)
            .w(width)
            .radius(look.radius(RADIUS_PILL))
            .pill_surface(&look)
            .role("dialog")
            .label(strings::pins_title())
            .pin(
                None,
                Some(look.space(SPACE_LG)),
                Some(look.space(SPACE_LG)),
                None,
            )
            .on_key(|shell: &mut Shell, event: &KeyEvent, tx| {
                if event.key == rune::Key::Escape {
                    shell.close_pins(tx);
                    return true;
                }
                false
            })
            // Leaving the panel after opening a pin shrinks it: a press on a
            // message or a scroll of the conversation never reaches a handler
            // of the panel's, and the pointer always leaves it first.
            // Coming back onto the shrunk panel (or scrolling over it, which
            // comes this way) stretches it back.
            .on_hover_change(|shell: &mut Shell, hovered, tx| {
                if hovered {
                    shell.pins_hovered(tx);
                } else {
                    shell.pins_left();
                }
            })
            // A press on the shrunk panel stretches it back.
            .on_background_press_tx(|shell: &mut Shell, tx| {
                if shell.shown_pins_panel().is_some_and(|panel| panel.peek) {
                    shell.pins_pressed(false, tx);
                }
            })
            // After the background press, which leaves an element unfocusable:
            // the panel takes the keyboard Ctrl+P gives it.
            .focusable(true)
            .child(head)
            .children(body),
    )
}

/// The opened pin in one line, on `highlight.selected`.
fn peek_line(open: &OpenTimeline, state: &PinsPanel, look: &Look) -> Option<El> {
    let id = state.opened?;
    let pin = open.pins.get(id)?;
    let original = papo_sdk_ui::ReplyOriginal::Held {
        id,
        message: Arc::clone(&pin.message),
        content: Arc::clone(&pin.content),
    };
    peek_row(&original, open, look)
}

/// A shrunk panel's one line for the message opened from it (a pin, a
/// search result), on `highlight.selected`: the author, then the reply
/// line's snippet, cut with an ellipsis at the panel's edge.
pub(super) fn peek_row(
    original: &papo_sdk_ui::ReplyOriginal,
    open: &OpenTimeline,
    look: &Look,
) -> Option<El> {
    let OriginalText::Message { name, snippet, .. } = original_text(original, open) else {
        return None;
    };
    let line = row()
        .w_full()
        .px(look.space(SPACE_LG))
        .py(look.space(SPACE_MD))
        .gap(look.space(SPACE_SM))
        .center_y()
        // Concentric with the panel, whose edge is one `space.sm` out.
        .radius(Look::nested_radius(
            look.radius(RADIUS_PILL),
            look.space(SPACE_SM),
        ))
        .fill(look.on_pill(look.palette.surface_selected))
        .child(
            text(name)
                .font(look.font(TYPE_BODY_STRONG))
                .color(look.palette.text_primary)
                .max_lines(1),
        )
        .child(
            text(snippet)
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_secondary)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    // Inset by padding, so the highlight stays inside the panel's corners.
    Some(
        col()
            .w_full()
            .px(look.space(SPACE_SM))
            .pb(look.space(SPACE_SM))
            .child(line),
    )
}

/// The pins, newest first; the empty state; a failure; or a wait while
/// they are not known.
fn list(
    shell: &Shell,
    open: &OpenTimeline,
    state: &PinsPanel,
    style: &Style,
    max_height_pt: f32,
) -> El {
    let look = style.look;
    let Some(pins) = open.pins.list() else {
        if open.pins.failed {
            return el()
                .w_full()
                .px(look.space(SPACE_LG))
                .pb(look.space(SPACE_MD))
                .child(
                    text(strings::pins_failed())
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.danger)
                        .role("alert"),
                );
        }
        return el()
            .w_full()
            .h(look.space(HEADER_PILL_HEIGHT) * 2.0)
            .busy(true)
            .label(strings::pins_loading());
    };
    if pins.is_empty() {
        return col()
            .w_full()
            .py(look.space(SPACE_XL))
            .gap(look.space(SPACE_MD))
            .center()
            .child(icon(&crate::icons::PIN, EMPTY_ICON_PT).color(look.palette.text_muted))
            .child(
                text(strings::pins_empty())
                    .font(look.font(TYPE_SMALL))
                    .color(look.palette.text_secondary),
            );
    }
    let key = open.key;
    // Media take the panel's box: 0.55 × its height (§10).
    let mut style = style.clone();
    style.media.bounds = papo_sdk_ui::MediaBounds::new(
        shell.conversations.settings().image_size,
        f32::INFINITY,
        max_height_pt * papo_sdk_ui::HOST_HEIGHT_SHARE,
    );
    style.media.surface = crate::message_media::received::MediaSurface::Pins;
    let style = &style;
    let mut rows = Vec::new();
    for (index, pin) in pins.iter().enumerate() {
        if index > 0 {
            rows.push(
                el().w_full()
                    .h(PILL_OUTLINE_PT)
                    .fill(look.palette.line_subtle)
                    .a11y_hidden(),
            );
        }
        let unpin = shell.may_unpin(key, pin.id);
        let marked = state.opened == Some(pin.id);
        rows.push(pin_row(open, pin, unpin, marked, style));
    }
    col()
        .w_full()
        .max_h(max_height_pt)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        // Inset, so each pin's rounded highlight sits inside the panel's
        // corners, as a menu's rows do.
        .px(look.space(SPACE_SM))
        .pb(look.space(SPACE_SM))
        .role("list")
        .focus_group()
        .children(rows)
}

/// One pin: the message on its own, pressed (or Enter) to open it, with
/// Unpin on hover or focus.
fn pin_row(open: &OpenTimeline, pin: &Pin, unpin: bool, marked: bool, style: &Style) -> El {
    let key = open.key;
    let id = pin.id;
    let look = style.look;
    // Everything the drawn message reads: its payload and parse, the
    // names it shows, the day its time is read against, the density.
    let version = (
        style.look,
        style.layout,
        style.today,
        Arc::as_ptr(&pin.message) as usize,
        Arc::as_ptr(&pin.content) as usize,
        open.resources
            .as_ref()
            .map(|resources| Arc::as_ptr(resources) as usize),
        marked,
        unpin,
        open.revealed(id),
        // Its media as drawn: an audio read or playing, a picture loaded.
        crate::timeline::received_signature(
            crate::message_media::received::MediaAt {
                surface: crate::message_media::received::MediaSurface::Pins,
                account: key.account,
                channel: key.channel,
                message: id,
                index: 0,
            },
            pin.message.attachments.as_deref().unwrap_or_default(),
            &style.media,
        ),
    );
    // Concentric with the panel, whose edge is one `space.sm` out.
    let row_radius_pt = Look::nested_radius(look.radius(RADIUS_PILL), look.space(SPACE_SM));
    memo(format!("pin-{id}"), version, || {
        let shown = standalone_message::<Shell>(open, id, &pin.message, &pin.content, style);
        let mut line = col()
            .key(pin_row_key(id))
            .w_full()
            .padding(look.space(SPACE_LG))
            .radius(row_radius_pt)
            .role("listitem")
            .label(strings::pins_jump())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .hover_group()
            .on_press_tx(move |shell: &mut Shell, tx| shell.open_pin(key, id, false, tx))
            .on_key(move |shell: &mut Shell, event: &KeyEvent, tx| {
                if event.key != rune::Key::Enter {
                    return false;
                }
                shell.open_pin(key, id, true, tx);
                true
            })
            .child(shown);
        line = if marked {
            line.fill(look.on_pill(look.palette.surface_selected))
        } else {
            line.hover(look.on_pill(look.palette.surface_hover))
        };
        if unpin {
            line = line.child(
                row()
                    .key(format!("unpin-{id}"))
                    .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
                    .center()
                    // Concentric with its row, one `space.sm` in.
                    .radius(Look::nested_radius(row_radius_pt, look.space(SPACE_SM)))
                    .fill(look.palette.surface_pill)
                    .role("button")
                    .label(strings::pins_unpin())
                    .help(strings::pins_unpin())
                    .focusable(true)
                    .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                    .group_opacity(0.0, 1.0)
                    .pin(
                        None,
                        Some(look.space(SPACE_SM)),
                        Some(look.space(SPACE_SM)),
                        None,
                    )
                    .on_press(move |shell: &mut Shell| shell.unpin_from_panel(key, id))
                    .child(
                        icon(&crate::icons::PIN_OFF, PILL_ICON_PT)
                            .color(look.palette.text_secondary),
                    ),
            );
        }
        line
    })
}
