//! Draws the conversation's header and its pins panel (`docs/design.md`
//! §3 and §10): the title pill on the left, the actions pill on the right
//! (📌 alone in D5a: search and members come with their slices); the panel
//! takes the actions pill's place, its header row "Pinned messages" and
//! the count with 📌 where it was, and the pins grow down, each a message
//! on its own divided by hairlines, Unpin on hover or focus. Shrunk
//! (`PanelPeek`), it keeps its header and one line for the opened pin.

use std::sync::Arc;

use rune::{Cx, El, KeyEvent, ScrollSpec, col, el, icon, memo, row, text};

use super::Shell;
use super::pins::{PINS_BUTTON_KEY, PINS_PANEL_KEY, PinsPanel, pin_row_key};
use super::title::Title;
use crate::design::{
    FOCUS_RING_PT, HEADER_PILL_HEIGHT_PT, PILL_BUTTON_PT, PILL_ICON_PT, PILL_OUTLINE_PT, Palette,
    RADIUS_CONTROL_PT, RADIUS_PILL_PT, SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT, SPACE_XL_PT,
    TYPE_BODY_STRONG, TYPE_SMALL, TYPE_TITLE,
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

/// The header row over the conversation: the title pill and the actions
/// pill. None while no conversation shows.
pub(super) fn header(shell: &Shell, title: &Title<'_>, palette: &Palette) -> Option<El> {
    shell.tabs.shown()?;
    let title_pill = super::title::pill(title, palette);
    // The open panel takes the actions pill's place.
    let actions = shell
        .shown_pins_panel()
        .is_none()
        .then(|| actions_pill(false, dm_profile(shell, title), palette));
    Some(
        row()
            .key("conversation-header")
            .w_full()
            .px(SPACE_LG_PT)
            .pt(SPACE_LG_PT)
            .center_y()
            .justify_between()
            .child(title_pill)
            .children(actions),
    )
}

/// The actions pill: 📌, in the accent while the panel is open.
/// In a DM, whether its profile pane shows: the person button is there.
fn dm_profile(shell: &Shell, title: &Title<'_>) -> Option<bool> {
    matches!(title, Title::Direct { .. }).then_some(shell.dm_profile)
}

/// The actions pill: 📌, in the accent while the panel is open, and in a
/// DM the person button.
fn actions_pill(open: bool, profile: Option<bool>, palette: &Palette) -> El {
    row()
        .key("actions-pill")
        .h(HEADER_PILL_HEIGHT_PT)
        .px(SPACE_SM_PT)
        .center_y()
        .radius(RADIUS_PILL_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .role("toolbar")
        .label(strings::header_actions())
        .child(pins_button(open, palette))
        .children(profile.map(|shown| super::profile_pane::button(shown, palette)))
}

fn pins_button(open: bool, palette: &Palette) -> El {
    let tone = if open {
        palette.accent
    } else {
        palette.text_secondary
    };
    row()
        .key(PINS_BUTTON_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(strings::pins_title())
        .help(strings::pins_title())
        .help_keys([strings::shortcut_pins()])
        .expanded(open)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .hover(palette.surface_hover)
        .on_press_tx(|shell: &mut Shell, tx| shell.pins_pressed(false, tx))
        .child(icon(&crate::icons::PIN, PILL_ICON_PT).color(tone))
}

/// The open panel over the conversation's top-right, when it is open on
/// the shown conversation.
pub(super) fn panel(shell: &Shell, style: &Style, cx: &Cx<Shell>) -> Option<El> {
    let state = shell.shown_pins_panel()?;
    let open = shell.tabs.shown()?;
    let palette = style.palette;
    let [width_pt, height_pt] = cx
        .layout_size("conversation")
        .unwrap_or([PANEL_MAX_PT, 600.0]);
    let width = PANEL_MAX_PT.min(width_pt - 2.0 * SPACE_LG_PT);
    let count = open.pins.list().map(<[_]>::len);
    let head = row()
        .w_full()
        .h(HEADER_PILL_HEIGHT_PT)
        .pl(SPACE_LG_PT)
        .pr(SPACE_SM_PT)
        .gap(SPACE_SM_PT)
        .center_y()
        .child(
            text(strings::pins_title())
                .font(TYPE_TITLE)
                .color(palette.text_primary)
                .role("heading"),
        )
        .children(count.map(|count| {
            text(count.to_string())
                .font(TYPE_SMALL)
                .color(palette.text_muted)
        }))
        .child(el().grow(1.0))
        .child(pins_button(true, &palette));
    let body = if state.peek {
        peek_line(open, &state, &palette)
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
            .radius(RADIUS_PILL_PT)
            .fill(palette.surface_pill)
            .border(PILL_OUTLINE_PT, palette.line_pill)
            .role("dialog")
            .label(strings::pins_title())
            .pin(None, Some(SPACE_LG_PT), Some(SPACE_LG_PT), None)
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
fn peek_line(open: &OpenTimeline, state: &PinsPanel, palette: &Palette) -> Option<El> {
    let id = state.opened?;
    let pin = open.pins.get(id)?;
    let original = papo_sdk_ui::ReplyOriginal::Held {
        id,
        message: Arc::clone(&pin.message),
        content: Arc::clone(&pin.content),
    };
    let OriginalText::Message { name, snippet, .. } = original_text(&original, open) else {
        return None;
    };
    let line = row()
        .w_full()
        .px(SPACE_LG_PT)
        .py(SPACE_MD_PT)
        .gap(SPACE_SM_PT)
        .radius(RADIUS_CONTROL_PT)
        .fill(palette.surface_selected)
        .child(
            text(name)
                .font(TYPE_BODY_STRONG)
                .color(palette.text_primary),
        )
        .child(
            text(snippet)
                .font(TYPE_SMALL)
                .color(palette.text_secondary)
                .max_lines(1),
        );
    // Inset by padding, so the highlight stays inside the panel's corners.
    Some(col().w_full().px(SPACE_SM_PT).pb(SPACE_SM_PT).child(line))
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
    let palette = style.palette;
    let Some(pins) = open.pins.list() else {
        if open.pins.failed {
            return el().w_full().px(SPACE_LG_PT).pb(SPACE_MD_PT).child(
                text(strings::pins_failed())
                    .font(TYPE_SMALL)
                    .color(palette.danger)
                    .role("alert"),
            );
        }
        return el()
            .w_full()
            .h(HEADER_PILL_HEIGHT_PT * 2.0)
            .busy(true)
            .label(strings::pins_loading());
    };
    if pins.is_empty() {
        return col()
            .w_full()
            .py(SPACE_XL_PT)
            .gap(SPACE_MD_PT)
            .center()
            .child(icon(&crate::icons::PIN, EMPTY_ICON_PT).color(palette.text_muted))
            .child(
                text(strings::pins_empty())
                    .font(TYPE_SMALL)
                    .color(palette.text_secondary),
            );
    }
    let key = open.key;
    let mut rows = Vec::new();
    for (index, pin) in pins.iter().enumerate() {
        if index > 0 {
            rows.push(
                el().w_full()
                    .h(PILL_OUTLINE_PT)
                    .fill(palette.line_subtle)
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
        .px(SPACE_SM_PT)
        .pb(SPACE_SM_PT)
        .role("list")
        .focus_group()
        .children(rows)
}

/// One pin: the message on its own, pressed (or Enter) to open it, with
/// Unpin on hover or focus.
fn pin_row(open: &OpenTimeline, pin: &Pin, unpin: bool, marked: bool, style: &Style) -> El {
    let key = open.key;
    let id = pin.id;
    let palette = style.palette;
    // Everything the drawn message reads: its payload and parse, the
    // names it shows, the day its time is read against, the density.
    let version = (
        style.theme,
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
    );
    memo(format!("pin-{id}"), version, || {
        let shown = standalone_message::<Shell>(open, id, &pin.message, &pin.content, style);
        let mut line = col()
            .key(pin_row_key(id))
            .w_full()
            .padding(SPACE_LG_PT)
            .radius(RADIUS_CONTROL_PT)
            .role("listitem")
            .label(strings::pins_jump())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
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
            line.fill(palette.surface_selected)
        } else {
            line.hover(palette.surface_hover)
        };
        if unpin {
            line = line.child(
                row()
                    .key(format!("unpin-{id}"))
                    .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
                    .center()
                    .radius(RADIUS_CONTROL_PT)
                    .fill(palette.surface_pill)
                    .role("button")
                    .label(strings::pins_unpin())
                    .help(strings::pins_unpin())
                    .focusable(true)
                    .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
                    .group_opacity(0.0, 1.0)
                    .pin(None, Some(SPACE_SM_PT), Some(SPACE_SM_PT), None)
                    .on_press(move |shell: &mut Shell| shell.unpin_from_panel(key, id))
                    .child(
                        icon(&crate::icons::PIN_OFF, PILL_ICON_PT).color(palette.text_secondary),
                    ),
            );
        }
        line
    })
}
