//! A message's reaction chips (`docs/design.md` §10, Reactions): one 24 px
//! chip per emoji under the message, on the content column, 4 px apart and
//! wrapping. Each shows the emoji and how many reacted; yours wears the
//! accent border on `accent.soft` with the count in the link colour. A
//! click toggles yours; a chip whose change is on its way waits. An add
//! chip (`smile-plus`) ends the row while the pointer or the keyboard is
//! on the message. The chips are not Tab stops: a focused message's keys
//! and its menu reach the same reactions.
//!
//! Custom emoji are left out until the SDK has an emoji catalog to name
//! them. Spoken: "🧀, 4, including you" (the emoji stands for its name;
//! the desktop has no CLDR name table).

use papo_sdk::Uuid;
use papo_sdk_ui::ReactionChip;
use papo_sdk_ui::operations::ReactionEmoji;
use rune::{El, icon, row, text};

use super::TimelineKey;
use super::view::TimelineHost;
use crate::design::{
    CHIP_ICON_PT, PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY,
    TYPE_SMALL_MEDIUM,
};
use crate::strings;

/// A chip's height, in points.
const CHIP_HEIGHT_PT: f32 = 24.0;
/// How strongly a chip waiting for its change is drawn.
const WAITING_OPACITY: f32 = 0.6;

/// A chip as its row draws it: the SDK's chip, and whether this account
/// may toggle it now.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) struct Chip {
    pub(super) chip: ReactionChip,
    pub(super) toggles: bool,
}

/// The chips message `id` shows (Unicode emoji only, in the SDK's order),
/// and the add chip when `add`. None when there is nothing to show.
pub(super) fn chips<S: TimelineHost>(
    key: TimelineKey,
    id: Uuid,
    chips: &[Chip],
    add: bool,
    palette: &Palette,
) -> Option<El> {
    if chips.is_empty() {
        return None;
    }
    let mut cells: Vec<El> = chips
        .iter()
        .filter_map(|chip| chip_cell::<S>(key, id, chip, palette))
        .collect();
    if cells.is_empty() {
        return None;
    }
    if add {
        cells.push(
            row()
                .key("chip-add")
                .h(CHIP_HEIGHT_PT)
                .px(SPACE_MD_PT)
                .center()
                .radius(RADIUS_CONTROL_PT)
                .fill(palette.surface_sidebar)
                .border(PILL_OUTLINE_PT, palette.line_subtle)
                .hover(palette.surface_hover)
                .role("button")
                .label(strings::action_add_reaction())
                .help(strings::action_add_reaction())
                .group_opacity(0.0, 1.0)
                .on_press_tx(move |host: &mut S, tx| {
                    host.bar_pressed(key, id, super::hover_bar::BarPress::AddReaction, tx);
                })
                .child(icon(&crate::icons::SMILE_PLUS, CHIP_ICON_PT).color(palette.text_secondary)),
        );
    }
    Some(
        row()
            .key("chips")
            .w_full()
            .wrap()
            .gap(SPACE_SM_PT)
            .role("group")
            .children(cells),
    )
}

fn chip_cell<S: TimelineHost>(
    key: TimelineKey,
    id: Uuid,
    chip: &Chip,
    palette: &Palette,
) -> Option<El> {
    let ReactionEmoji::Unicode(emoji) = &chip.chip.emoji else {
        return None;
    };
    let ReactionChip {
        count,
        mine,
        waiting,
        ..
    } = chip.chip;
    let label = if mine {
        strings::reaction_label_mine(emoji, count)
    } else {
        strings::reaction_label(emoji, count)
    };
    let (fill, line, ink) = if mine {
        (palette.accent_soft, palette.accent, palette.link)
    } else {
        (
            palette.surface_sidebar,
            palette.line_subtle,
            palette.text_secondary,
        )
    };
    let mut cell = row()
        .key(format!("chip-{emoji}"))
        .h(CHIP_HEIGHT_PT)
        .px(SPACE_MD_PT)
        .gap(SPACE_SM_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .fill(fill)
        .border(PILL_OUTLINE_PT, line)
        .role("button")
        .label(label)
        .child(text(emoji.clone()).font(TYPE_BODY))
        .child(text(count.to_string()).font(TYPE_SMALL_MEDIUM).color(ink));
    if waiting {
        cell = cell.opacity(WAITING_OPACITY).busy(true);
    } else if chip.toggles {
        let emoji = emoji.clone();
        cell = cell
            .hover(palette.surface_hover)
            .on_press_tx(move |host: &mut S, tx| {
                host.bar_pressed(
                    key,
                    id,
                    super::hover_bar::BarPress::React {
                        emoji: emoji.clone(),
                        add: !mine,
                    },
                    tx,
                );
            });
    }
    Some(cell)
}
