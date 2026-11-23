//! Draws Direct's New message button and its picker (`docs/design.md`
//! §10, Direct messages): the picker grows down from the pane pill, as wide
//! as it, with a "Find someone" field, then the people from every server
//! the rail shows, one row per server: avatar with presence, the name and
//! @username, the server under them, the highlighted row on
//! `surface.selected`. Only the rows on screen are built. Pure: reads the
//! shell; what the picker does is `shell::new_message`'s.

use std::borrow::Cow;

use papo_sdk_ui::DirectCandidate;
use rune::{El, GrowthSpec, KeyEvent, ScrollSpec, col, icon, row, text, text_input};

use crate::design::{
    AVATAR_ROW, DM_ROW_HEIGHT_PT, FOCUS_RING_PT, MENU_ROW_HEIGHT_PT, PILL_BUTTON_PT, PILL_ICON_PT,
    Palette, RADIUS_CONTROL_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_CAPTION, TYPE_SMALL,
};
use crate::icons::NEW_MESSAGE;
use crate::person_avatar::person_avatar;
use crate::shell::{FIND_FIELD_KEY, NEW_MESSAGE_BUTTON_KEY, Shell, candidate_key};
use crate::strings;

/// The rows shown before the list scrolls.
const VISIBLE_ROWS: f32 = 8.0;

/// New message, in the pane pill.
pub(super) fn new_message_button(open: bool, palette: &Palette) -> El {
    let tone = if open {
        palette.accent
    } else {
        palette.text_secondary
    };
    row()
        .key(NEW_MESSAGE_BUTTON_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(strings::direct_new())
        .help(strings::direct_new())
        .expanded(open)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .hover(palette.surface_hover)
        .on_press_tx(Shell::new_message_pressed)
        .child(icon(&NEW_MESSAGE, PILL_ICON_PT).color(tone))
}

/// The open picker's body, for the menu overlay to place; none while it
/// is closed.
pub(crate) fn body(shell: &Shell, palette: &Palette) -> Option<El> {
    let picker = shell.shown_dm_picker()?;
    let found = &picker.found;
    let mut field = text_input(&picker.edit, TYPE_BODY)
        .key(FIND_FIELD_KEY)
        .label(strings::direct_find())
        .w_full()
        .color(palette.text_primary)
        .placeholder(strings::direct_find(), palette.text_muted)
        .caret(palette.accent, palette.text_selection)
        .on_edit(Shell::find_field_mut)
        .on_input_change(Shell::find_edited)
        .on_input_key(|shell: &mut Shell, key: &KeyEvent, tx| shell.find_key(key, tx));
    if let Some(chosen) = found.get(picker.highlight) {
        field = field
            .active_descendant(candidate_key(chosen))
            .expanded(true);
    }
    let field = row()
        .w_full()
        .h(MENU_ROW_HEIGHT_PT)
        .px(SPACE_SM_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .fill(palette.surface_hover)
        .child(field);
    let list = if found.is_empty() {
        col().w_full().padding(SPACE_MD_PT).child(
            text(strings::direct_find_none())
                .font(TYPE_SMALL)
                .color(palette.text_muted)
                .role("status"),
        )
    } else {
        let palette_copy = *palette;
        col()
            .w_full()
            .max_h(DM_ROW_HEIGHT_PT * VISIBLE_ROWS)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .key("direct-find-rows")
                    .w_full()
                    .role("listbox")
                    .label(strings::direct_find_list())
                    .growth(GrowthSpec::default())
                    .virtual_rows(
                        found.len(),
                        DM_ROW_HEIGHT_PT,
                        |shell: &Shell, index| {
                            Cow::Owned(
                                shell
                                    .shown_dm_picker()
                                    .and_then(|picker| picker.found.get(index))
                                    .map_or_else(|| format!("find-gone-{index}"), candidate_key),
                            )
                        },
                        move |shell: &Shell, index| candidate_row(shell, index, &palette_copy),
                    ),
            )
    };
    Some(
        col()
            .w_full()
            .padding(SPACE_SM_PT)
            .gap(SPACE_SM_PT)
            .role("dialog")
            .label(strings::direct_new())
            .child(field)
            .child(list),
    )
}

/// Row `index`: the person, on their server.
fn candidate_row(shell: &Shell, index: usize, palette: &Palette) -> El {
    let Some(picker) = shell.shown_dm_picker() else {
        return col();
    };
    let Some(candidate) = picker.found.get(index) else {
        return col();
    };
    let chosen = index == picker.highlight;
    let presence = shell
        .people(candidate.account)
        .and_then(|people| people.presence_mark(candidate.user));
    let server = server_name(shell, candidate);
    let surface = if chosen {
        palette.surface_selected
    } else {
        palette.surface_pill
    };
    let handle = candidate
        .username
        .as_ref()
        .filter(|username| **username != candidate.name)
        .map(|username| format!("@{username}"));
    let label = match &handle {
        Some(handle) => format!("{} {handle}, {server}", candidate.name),
        None => format!("{}, {server}", candidate.name),
    };
    let line = row()
        .key(candidate_key(candidate))
        .w_full()
        .h(DM_ROW_HEIGHT_PT)
        .px(SPACE_SM_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .role("option")
        .label(label)
        .on_press_tx(move |shell: &mut Shell, tx| shell.person_picked(index, tx))
        .child(person_avatar(
            &candidate.name,
            AVATAR_ROW,
            presence,
            surface,
            palette,
        ))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .a11y_hidden()
                .child(
                    row()
                        .gap(SPACE_SM_PT)
                        .child(
                            text(candidate.name.clone())
                                .font(TYPE_BODY)
                                .color(palette.text_primary)
                                .truncate(),
                        )
                        .children(handle.map(|handle| {
                            text(handle)
                                .font(TYPE_SMALL)
                                .color(palette.text_muted)
                                .grow(1.0)
                                .min_w(0.0)
                                .truncate()
                        })),
                )
                .child(
                    text(server)
                        .font(TYPE_CAPTION)
                        .color(palette.text_muted)
                        .truncate(),
                ),
        );
    // `surface.hover` is too close to the pill's own surface to show.
    if chosen {
        line.fill(palette.surface_selected)
    } else {
        line
    }
}

/// The name of the server `candidate`'s account is on.
fn server_name(shell: &Shell, candidate: &DirectCandidate) -> String {
    shell
        .navigation()
        .servers()
        .servers
        .iter()
        .find(|server| {
            server
                .account
                .as_ref()
                .is_some_and(|account| account.id == candidate.account)
        })
        .map(|server| server.name.clone())
        .unwrap_or_default()
}
