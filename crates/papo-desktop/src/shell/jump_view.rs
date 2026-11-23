//! Draws Ctrl+K's field (`docs/design.md` §12): a card near the top of the
//! window over the dimmed app, its field first, then one list of places,
//! each with its kind (# for a channel, the person's avatar, the server's
//! tile), its name and @username, its server under it and its unread badge;
//! the highlighted row on `surface.selected`. Pure: reads the shell; what a
//! jump does is `jump`'s.

use papo_sdk_ui::{JumpKind, JumpTarget};
use rune::{El, KeyEvent, ScrollSpec, col, el, icon, row, text, text_input};

use super::Shell;
use super::jump::{JUMP_FIELD_KEY, target_key};
use crate::badges::Badge;
use crate::channel_pane::{avatar, row_badge};
use crate::design::{
    AVATAR_ROW, DM_ROW_HEIGHT_PT, Look, MENU_ROW_HEIGHT_PT, PILL_ICON_PT, PILL_OUTLINE_PT,
    POPUP_SHADOW_PT, RADIUS_CONTROL, RADIUS_POPUP, SPACE_MD, SPACE_SM, SPACE_XXL, TYPE_BODY,
    TYPE_CAPTION, TYPE_SMALL,
};
use crate::person_avatar::person_avatar;
use crate::strings;

/// The card's width, in points.
const JUMP_WIDTH_PT: f32 = 560.0;
/// The rows shown before the list scrolls.
const VISIBLE_ROWS: f32 = 9.0;

/// The open field over the dimmed app; none while it is closed.
pub(super) fn view(shell: &Shell, look: &Look) -> Option<El> {
    let jump = shell.jump.as_ref()?;
    let (drop, blur) = POPUP_SHADOW_PT;
    let mut field = text_input(&jump.edit, look.font(TYPE_BODY))
        .key(JUMP_FIELD_KEY)
        .label(strings::shortcuts_jump())
        .w_full()
        .color(look.palette.text_primary)
        .placeholder(strings::shortcuts_jump(), look.palette.text_muted)
        .caret(look.palette.accent, look.palette.text_selection)
        .on_edit(Shell::jump_field_mut)
        .on_input_change(Shell::jump_edited)
        .on_input_key(|shell: &mut Shell, key: &KeyEvent, tx| shell.jump_key(key, tx));
    if let Some(chosen) = jump.found.get(jump.highlight) {
        field = field.active_descendant(target_key(chosen)).expanded(true);
    }
    let rows: Vec<El> = jump
        .found
        .iter()
        .enumerate()
        .map(|(index, target)| target_row(shell, target, index == jump.highlight, look))
        .collect();
    let list = if rows.is_empty() {
        col().w_full().padding(look.space(SPACE_MD)).child(
            text(strings::jump_none())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .role("status"),
        )
    } else {
        col()
            .w_full()
            .max_h(DM_ROW_HEIGHT_PT * VISIBLE_ROWS)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .w_full()
                    .role("listbox")
                    .label(strings::jump_list())
                    .children(rows),
            )
    };
    let card = col()
        .w(JUMP_WIDTH_PT)
        .padding(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM))
        .radius(look.radius(RADIUS_POPUP))
        .fill(look.palette.surface_content)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .shadow(drop, blur, look.palette.popup_shadow)
        .role("dialog")
        .label(strings::shortcuts_jump())
        .modal(false, Shell::close_jump)
        .child(
            row()
                .w_full()
                .h(MENU_ROW_HEIGHT_PT)
                .px(look.space(SPACE_MD))
                .center_y()
                .radius(look.radius(RADIUS_CONTROL))
                .fill(look.palette.surface_hover)
                .child(field),
        )
        .child(list);
    // Over the whole window, the card centred near its top.
    Some(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .fill(look.palette.scrim)
            .child(
                col()
                    .w_full()
                    .h_full()
                    .items_center()
                    .pt(look.space(SPACE_XXL) * 2.0)
                    .child(card),
            ),
    )
}

/// One place: its kind, name and @username, its server, its badge.
fn target_row(shell: &Shell, target: &JumpTarget, chosen: bool, look: &Look) -> El {
    let server = shell.navigation().servers().get(target.account.server());
    let server_name = server.map(|server| server.name.clone()).unwrap_or_default();
    let badge = badge_of(shell, target);
    let surface = if chosen {
        look.palette.surface_selected
    } else {
        look.palette.surface_content
    };
    let lead = match target.kind {
        JumpKind::Channel { .. } => el()
            .size(AVATAR_ROW.side_pt, AVATAR_ROW.side_pt)
            .center()
            .child(icon(&crate::icons::HASH, PILL_ICON_PT).color(look.palette.text_secondary)),
        JumpKind::Direct { peer: user, .. } | JumpKind::Person { user } => {
            let presence = shell
                .people(target.account)
                .and_then(|people| people.presence_mark(user));
            person_avatar(&target.name, AVATAR_ROW, presence, surface, look)
        }
        JumpKind::Server => avatar(&target.name, None, look),
    };
    let handle = target
        .username
        .as_ref()
        .filter(|username| **username != target.name)
        .map(|username| format!("@{username}"));
    let sub = match target.kind {
        JumpKind::Server => strings::jump_server().to_owned(),
        _ => server_name,
    };
    let spoken = format!("{}, {sub}", target.name);
    let key = target_key(target);
    let picked = key.clone();
    let line = row()
        .key(key)
        .w_full()
        .h(DM_ROW_HEIGHT_PT)
        .px(look.space(SPACE_SM))
        .gap(look.space(SPACE_MD))
        .center_y()
        .radius(look.radius(RADIUS_CONTROL))
        .role("option")
        .label(badge.label(&spoken))
        .on_press_tx(move |shell: &mut Shell, tx| shell.jump_picked(&picked, tx))
        .child(lead)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .a11y_hidden()
                .child(
                    row()
                        .gap(look.space(SPACE_SM))
                        .child(
                            text(target.name.clone())
                                .font(look.font(TYPE_BODY))
                                .color(look.palette.text_primary)
                                .truncate(),
                        )
                        .children(handle.map(|handle| {
                            text(handle)
                                .font(look.font(TYPE_SMALL))
                                .color(look.palette.text_muted)
                                .grow(1.0)
                                .min_w(0.0)
                                .truncate()
                        })),
                )
                .child(
                    text(sub)
                        .font(look.font(TYPE_CAPTION))
                        .color(look.palette.text_muted)
                        .truncate(),
                ),
        )
        .children(row_badge(badge, look));
    if chosen {
        line.fill(look.palette.surface_selected)
    } else {
        line.hover(look.palette.surface_hover)
    }
}

/// A place's unread badge: a channel's or a conversation's, as the panes
/// show them; none for people and servers.
fn badge_of(shell: &Shell, target: &JumpTarget) -> Badge {
    let Some(server) = shell.navigation().servers().get(target.account.server()) else {
        return Badge::default();
    };
    match target.kind {
        JumpKind::Channel { channel } => server
            .text_channels
            .iter()
            .find(|entry| entry.id == channel.to_string())
            .map(|entry| entry.badge)
            .unwrap_or_default(),
        JumpKind::Direct { channel, .. } => server
            .direct_badges
            .get(&channel)
            .copied()
            .unwrap_or_default(),
        JumpKind::Person { .. } | JumpKind::Server => Badge::default(),
    }
}
