//! The Emoji tab's list (`docs/design.md` §16.2): one virtual row per section, its heading
//! scrolling with it (Christian, 2026-11-26: no heading held at the top), and inside it a virtual
//! list of its rows of cells, so a server's thousands of emoji build (and fetch) only the rows on
//! screen.

use std::borrow::Cow;

use papo_sdk::AccountId;
use papo_sdk_ui::EmojiLanguage;
use rune::{El, GrowthSpec, ScrollSpec, TextStyle, col, el, memo, row, text};

use super::{EMOJI_ROWS_KEY, EMOJI_SCROLL_KEY, PickerLook, section_label, server_tile, sized};
use crate::design::{
    EMOJI_CELL_CUSTOM_PT, EMOJI_CELL_UNICODE_PT, EMOJI_HEADING_PT, EMOJI_SECTION_GAP_PT,
    FOCUS_RING_PT, Look, TYPE_BODY, TYPE_CAPTION, WEIGHT_SEMIBOLD,
};
use crate::emoji_images::{EmojiKey, EmojiLooks, emoji_square};
use crate::emoji_picker::{
    Block, CELL_GAP_PT, EmojiChoice, EmojiPicker, Row, Section, block_key, block_rows_key,
    cell_row_key,
};
use crate::shell::Shell;

/// The sections, one virtual row each (its heading and its rows of cells): only the sections on
/// screen are built.
pub(super) fn grid(picker: &EmojiPicker, given: &PickerLook<'_>, cell_pt: f32, padding: f32) -> El {
    let draw = CellLook {
        look: *given.look,
        emoji: given.emoji.clone(),
        account: given.account,
        server: given.server.to_owned(),
        language: given.language,
        side_pt: cell_pt,
    };
    // Most sections run to a few dozen rows: an estimate near theirs keeps the bar steady.
    let estimate_pt = EMOJI_HEADING_PT + 8.0 * (cell_pt + CELL_GAP_PT) + EMOJI_SECTION_GAP_PT;
    let list = col()
        .key(EMOJI_ROWS_KEY)
        .w_full()
        .role("listbox")
        .label(crate::strings::picker_emoji())
        .growth(GrowthSpec::default())
        .virtual_rows(
            picker.blocks().len(),
            estimate_pt,
            |shell: &Shell, index| {
                let key = shell
                    .emoji_picker()
                    .and_then(|picker| picker.blocks().get(index))
                    .map_or_else(String::new, |block| block_key(block.section));
                Cow::Owned(key)
            },
            move |shell: &Shell, index| {
                let block = shell
                    .emoji_picker()
                    .and_then(|picker| Some((picker, picker.blocks().get(index)?)));
                match block {
                    Some((picker, block)) => section_block(picker, index, block, &draw),
                    None => el(),
                }
            },
        )
        .virtual_rows_version(picker.version)
        .on_visible_rows(|shell: &mut Shell, range| shell.emoji_sections_shown(range));
    col()
        .key(EMOJI_SCROLL_KEY)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(padding)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(list)
}

/// One section: its heading, then its rows of cells, themselves virtual.
fn section_block(picker: &EmojiPicker, index: usize, block: &Block, draw: &CellLook) -> El {
    let heading = heading(block.section, block.count, &draw.server, &draw.look).w_full();
    let section = block.section;
    let row_draw = draw.clone();
    let rows = col()
        .key(block_rows_key(section))
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            block.cell_rows().len(),
            draw.side_pt + CELL_GAP_PT,
            move |_shell: &Shell, row| Cow::Owned(cell_row_key(section, row)),
            move |shell: &Shell, row| {
                let cells = shell.emoji_picker().and_then(|picker| {
                    let block = picker.blocks().get(index)?;
                    match picker.rows().get(block.cell_rows().start + row)? {
                        Row::Cells { cells, .. } => Some((picker, cells)),
                        Row::Heading { .. } => None,
                    }
                });
                match cells {
                    Some((picker, cells)) => cells_row(picker, section, row, cells, &row_draw),
                    None => el(),
                }
            },
        )
        .virtual_rows_version(picker.version)
        .on_visible_rows(move |shell: &mut Shell, range| shell.emoji_rows_shown(section, range));
    col()
        .w_full()
        .pb(EMOJI_SECTION_GAP_PT)
        .role("group")
        .label(section_label(section, &draw.server))
        .child(heading)
        .child(rows)
}

/// A section's heading (§16.2): caption-sized, 600, `text.muted`; this server's with its tile
/// and its count, a search's with its count.
fn heading(section: Section, count: usize, server: &str, look: &Look) -> El {
    let label = section_label(section, server);
    let counted = matches!(section, Section::Server | Section::Unicode);
    row()
        .w_full()
        .h(EMOJI_HEADING_PT)
        .px(4.0)
        .gap(6.0)
        .center_y()
        .role("heading")
        .children((section == Section::Server).then(|| server_tile(server, 14.0, look)))
        .child(
            text(label)
                .font(TextStyle {
                    weight: WEIGHT_SEMIBOLD,
                    ..look.font(TYPE_CAPTION)
                })
                .color(look.palette.text_muted),
        )
        .children(counted.then(|| {
            text(count.to_string())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted)
        }))
}

/// What a cell reads.
#[derive(Clone)]
struct CellLook {
    look: Look,
    emoji: EmojiLooks,
    account: AccountId,
    server: String,
    language: EmojiLanguage,
    side_pt: f32,
}

/// Row `row` of `section`'s cells. Memoized: the pointer crossing cells commits the view (the
/// preview bar follows it), and a row redraws only when what it draws changes: the list, the
/// keyboard's cell in it, or a custom emoji's image.
fn cells_row(
    picker: &EmojiPicker,
    section: Section,
    row_at: usize,
    cells: &[usize],
    draw: &CellLook,
) -> El {
    let choices: Vec<(usize, &EmojiChoice)> = cells
        .iter()
        .filter_map(|at| Some((*at, picker.cells().get(*at)?)))
        .collect();
    let keyboard = picker.cursor.filter(|at| cells.contains(at));
    let images: Vec<_> = choices
        .iter()
        .filter_map(|(_, choice)| match choice {
            EmojiChoice::Custom(emoji) => Some(draw.emoji.look(EmojiKey {
                account: draw.account,
                id: emoji.id,
            })),
            EmojiChoice::Unicode(_) => None,
        })
        .collect();
    let version = (
        picker.version,
        keyboard,
        images,
        draw.look,
        draw.side_pt.to_bits(),
        &draw.server,
        draw.language == EmojiLanguage::English,
    );
    memo(cell_row_key(section, row_at), version, || {
        row()
            .w_full()
            .h(draw.side_pt + CELL_GAP_PT)
            .gap(CELL_GAP_PT)
            .pb(CELL_GAP_PT)
            .children(
                choices
                    .iter()
                    .map(|(at, choice)| cell(*at, section, choice, keyboard == Some(*at), draw)),
            )
    })
}

/// One cell (§16.2): square, radius 6; a custom emoji at 30 pt, a Unicode one at 26 pt from the
/// colour emoji font; hover or the keyboard tints it.
fn cell(at: usize, section: Section, choice: &EmojiChoice, keyboard: bool, draw: &CellLook) -> El {
    let look = &draw.look;
    let glyph: El = match choice {
        EmojiChoice::Unicode(emoji) => text(emoji.glyph)
            .font(sized(look.font(TYPE_BODY), EMOJI_CELL_UNICODE_PT))
            .a11y_hidden(),
        EmojiChoice::Custom(emoji) => {
            let key = EmojiKey {
                account: draw.account,
                id: emoji.id,
            };
            let side_pt = EMOJI_CELL_CUSTOM_PT.min(draw.side_pt - 4.0);
            emoji_square(
                &draw.emoji.look(key),
                side_pt,
                &choice.code(),
                draw.side_pt - 4.0,
                look,
            )
        }
    };
    let label = super::spoken(choice, &draw.server, draw.language);
    let tint = look.on_pill(look.palette.surface_hover);
    let mut cell = row()
        .key(choice.key(section))
        .size(draw.side_pt, draw.side_pt)
        .center()
        .radius(6.0)
        .role("option")
        .label(label)
        .hover(tint)
        .on_hover_change(move |shell: &mut Shell, on, _tx| shell.emoji_hovered(at, on))
        .on_press_tx(move |shell: &mut Shell, tx| {
            let keep = tx.mods().shift;
            shell.emoji_cell_pressed(at, keep, tx);
        })
        .child(glyph);
    if keyboard {
        cell = cell.focus_selected(true).fill(tint).outline(
            FOCUS_RING_PT.0,
            FOCUS_RING_PT.1,
            look.palette.focus_ring,
        );
    }
    cell
}
