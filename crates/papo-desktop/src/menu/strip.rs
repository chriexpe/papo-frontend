//! The message menu's quick-reaction strip (`docs/design.md` §10): the
//! account's eight quick reactions in one row. The strip is one item of the
//! menu, so ↑ and ↓ pass it like any other; ← and → choose an emoji, and
//! Enter or Space reacts with it. A click reacts with the emoji clicked. An
//! emoji you already reacted with wears your chip's colours, and choosing
//! it takes your reaction back, as the chip does.

use crate::reaction_emoji::ShownReaction;
use papo_sdk::Uuid;
use rune::{El, Key, KeyEvent, Tx, el, row};

use super::MenuAction;
use crate::design::{
    FOCUS_RING_PT, Look, MENU_ROW_HEIGHT_PT, PILL_OUTLINE_PT, RADIUS_PILL, SPACE_SM, SPACE_XS,
};
use crate::emoji_images::EmojiLooks;
use crate::shell::Shell;
use crate::strings;
use crate::timeline::TimelineKey;

/// The strip's element key.
pub(crate) const STRIP_KEY: &str = "menu-strip";
/// An emoji cell's size, in points.
const CELL_PT: f32 = 28.0;

/// The strip of one message's menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Strip {
    pub(crate) key: TimelineKey,
    pub(crate) id: Uuid,
    /// The quick reactions, most used first, and whether each is yours.
    pub(crate) emoji: Vec<(ShownReaction, bool)>,
    /// The one ← and → are on.
    pub(crate) chosen: usize,
}

impl Strip {
    /// The action reacting with emoji `index` is: toggling yours.
    pub(crate) fn action(&self, index: usize) -> Option<MenuAction> {
        let (shown, _) = self.emoji.get(index)?;
        Some(MenuAction::React {
            key: self.key,
            id: self.id,
            emoji: shown.emoji.clone(),
        })
    }
}

/// Draws the strip, the menu's first item when `first`, its custom emoji as `images` hold them
/// now (an image may land while the menu is open).
pub(super) fn view(strip: &Strip, first: bool, images: &EmojiLooks, look: &Look) -> El {
    // Concentric with the menu, whose edge is one `space.sm` out.
    let strip_radius_pt = Look::nested_radius(look.radius(RADIUS_PILL), look.space(SPACE_SM));
    let cells = strip
        .emoji
        .iter()
        .enumerate()
        .map(|(index, (shown, mine))| {
            let action = strip.action(index);
            let mut cell = row()
                .key(format!("{STRIP_KEY}-{index}"))
                .size(CELL_PT, CELL_PT)
                .center()
                // Concentric with the strip; the corner's gap is the
                // smaller of its side padding and the space above the cell.
                .radius(Look::nested_radius(
                    strip_radius_pt,
                    look.space(SPACE_XS)
                        .min((MENU_ROW_HEIGHT_PT - CELL_PT) / 2.0),
                ))
                .a11y_hidden()
                .hover(look.on_pill(look.palette.surface_hover))
                .on_press_tx(move |shell: &mut Shell, tx| {
                    if let Some(action) = action.clone() {
                        shell.menu_picked(action, tx);
                    }
                })
                .child(
                    shown
                        .clone()
                        .drawn(strip.key.account, images)
                        .glyph(CELL_PT - 4.0, look),
                );
            if *mine {
                cell = cell
                    .fill(look.palette.accent_soft)
                    .border(PILL_OUTLINE_PT, look.palette.accent);
            }
            if index == strip.chosen {
                // Drawn while the strip has the keyboard: on the emoji ← and → are on.
                cell = cell.focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring);
            }
            cell
        });
    let chosen = strip.emoji.get(strip.chosen);
    let label = chosen.map_or_else(String::new, |(shown, mine)| {
        if *mine {
            strings::reaction_take_back(&shown.name)
        } else {
            strings::reaction_react(&shown.name)
        }
    });
    let action = strip.action(strip.chosen);
    el().key(STRIP_KEY)
        .w_full()
        .min_h(MENU_ROW_HEIGHT_PT)
        .radius(strip_radius_pt)
        .role("menuitem")
        .label(label)
        .focusable(true)
        .focus_selected(first)
        .on_press_tx(move |shell: &mut Shell, tx| {
            if let Some(action) = action.clone() {
                shell.menu_picked(action, tx);
            }
        })
        .on_key(|shell: &mut Shell, event: &KeyEvent, _tx: &mut Tx| shell.menu_strip_key(event))
        .child(
            row()
                .w_full()
                .h(MENU_ROW_HEIGHT_PT)
                .px(look.space(SPACE_XS))
                .justify_between()
                .center_y()
                .children(cells),
        )
}

impl Shell {
    /// ← or → on the menu's strip: the emoji beside the chosen one is
    /// chosen, wrapping at the ends. Returns whether the key was taken.
    fn menu_strip_key(&mut self, event: &KeyEvent) -> bool {
        let mods = event.mods;
        if mods.cmd || mods.ctrl || mods.alt || mods.logo || mods.shift {
            return false;
        }
        let Some(strip) = self.open_menu_mut().and_then(super::OpenMenu::strip_mut) else {
            return false;
        };
        let count = strip.emoji.len();
        if count == 0 {
            return false;
        }
        strip.chosen = match event.key {
            Key::Right => (strip.chosen + 1) % count,
            Key::Left => (strip.chosen + count - 1) % count,
            _ => return false,
        };
        true
    }
}
