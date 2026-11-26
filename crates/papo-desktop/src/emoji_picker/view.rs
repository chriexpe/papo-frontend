//! The Emoji tab drawn (`docs/design.md` §16.1, §16.2): grown upward from the text pill (or the
//! hover bar, for a reaction), its right edge on the pill's, glass; the search well, the category
//! strip, the list (`grid`) and the preview bar. Pure: it reads the picker and hands presses to
//! the shell.

use papo_sdk::AccountId;
use papo_sdk_ui::{EmojiCategory, EmojiLanguage};
use rune::{Align, El, KeyEvent, Side, TextStyle, col, icon, row, text, text_input};

use super::{CELL_GAP_PT, COLUMNS, EmojiChoice, EmojiPicker, Section, block_key, section_key};
use crate::design::{
    EMOJI_PICKER_GAP_PT, EMOJI_PICKER_MAX_PT, EMOJI_PREVIEW_BAR_PT, EMOJI_PREVIEW_EMOJI_PT,
    EMOJI_SEARCH_PT, EMOJI_STRIP_PT, Look, PILL_ICON_PT, RADIUS_PILL, SPACE_MD, SPACE_SM, Surfaces,
    TYPE_BODY, TYPE_CAPTION, TYPE_SMALL, WEIGHT_SEMIBOLD,
};
use crate::emoji_images::{EmojiKey, EmojiLooks, emoji_square};
use crate::icons::{BALL, BULB, CAR, CLOCK, CUP, FLAG, HEART, LEAF, SEARCH, SMILE, X};
use crate::shell::Shell;
use crate::strings;

mod grid;

/// The element key of the open picker.
pub(crate) const EMOJI_PICKER_KEY: &str = "emoji-picker";
/// The element key of its search field.
pub(crate) const EMOJI_SEARCH_KEY: &str = "emoji-search";
/// The element key of its list of sections.
pub(crate) const EMOJI_ROWS_KEY: &str = "emoji-rows";
/// The element key of the list's scroll viewport.
pub(crate) const EMOJI_SCROLL_KEY: &str = "emoji-scroll";

/// What drawing the picker reads besides the picker.
pub(crate) struct PickerLook<'a> {
    pub(crate) look: &'a Look,
    /// The emoji images as they stand.
    pub(crate) emoji: EmojiLooks,
    /// The account the picker lists.
    pub(crate) account: AccountId,
    /// Its server's name: this server's section, the tile, "where it is from".
    pub(crate) server: &'a str,
    /// The element it grows from (the text pill, or the hover bar), and its width.
    pub(crate) anchor: &'a str,
    pub(crate) anchor_width_pt: Option<f32>,
    /// The conversation's height: the picker is at most 60 % of it.
    pub(crate) conversation_height_pt: Option<f32>,
    /// The language names are spoken in.
    pub(crate) language: EmojiLanguage,
}

/// The open Emoji tab.
pub(crate) fn view(picker: &EmojiPicker, given: &PickerLook<'_>) -> El {
    let look = given.look;
    let (max_width, max_height) = EMOJI_PICKER_MAX_PT;
    let width = given
        .anchor_width_pt
        .map_or(max_width, |anchor| anchor.min(max_width));
    let height = given
        .conversation_height_pt
        .map_or(max_height, |room| (room * 0.6).min(max_height));
    let padding = look.space(SPACE_SM);
    let cell_pt = cell_side(width, padding);
    let mut panel = col()
        .key(EMOJI_PICKER_KEY)
        .w(width)
        .h(height)
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .clip_rounded(look.radius(RADIUS_PILL))
        .role("dialog")
        .label(strings::picker_emoji())
        // A press outside closes it (§16.1); Escape is the search's first (it clears first).
        .modal_tx(true, |shell: &mut Shell, tx| shell.close_emoji_picker(tx))
        .anchored_fit(given.anchor, Side::Top, Align::End, EMOJI_PICKER_GAP_PT)
        .child(search(picker, look, padding));
    if picker.query().is_empty() {
        panel = panel.child(strip(picker, given, padding));
    }
    let body = if picker.rows().is_empty() {
        no_match(picker.query(), look)
    } else {
        grid::grid(picker, given, cell_pt, padding)
    };
    panel.child(body).child(preview_bar(picker, given))
}

/// A cell's side: nine to the panel's inner width, [`CELL_GAP_PT`] between.
fn cell_side(width: f32, padding: f32) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "nine columns: exact in f32")]
    let columns = COLUMNS as f32;
    ((width - 2.0 * padding - (columns - 1.0) * CELL_GAP_PT) / columns).max(1.0)
}

/// The search well on top (§16.1): 36 pt, `surface.hover`, its corners concentric with the
/// panel's; ✕ clears it.
fn search(picker: &EmojiPicker, look: &Look, padding: f32) -> El {
    let radius = Look::nested_radius(look.radius(RADIUS_PILL), padding);
    let mut field = text_input(&picker.search, look.font(TYPE_BODY))
        .key(EMOJI_SEARCH_KEY)
        .label(strings::picker_search_emoji())
        .grow(1.0)
        .color(look.palette.text_primary)
        .placeholder(strings::picker_search_emoji(), look.palette.text_muted)
        .caret(look.palette.accent, look.palette.text_selection)
        .on_edit(Shell::emoji_search_mut)
        .on_input_change(Shell::emoji_searched)
        .on_input_key(|shell: &mut Shell, key: &KeyEvent, tx| shell.emoji_picker_key(key, tx));
    if let Some(chosen) = picker.chosen_key() {
        field = field.active_descendant(chosen).expanded(true);
    }
    let clear = (!picker.search.text.is_empty()).then(|| {
        row()
            .key("emoji-search-clear")
            .size(22.0, 22.0)
            .center()
            .radius(11.0)
            .role("button")
            .label(strings::picker_clear())
            .on_press_tx(|shell: &mut Shell, tx| shell.emoji_search_cleared(tx))
            .hover(look.on_pill(look.palette.surface_hover))
            .child(icon(&X, 14.0).color(look.palette.text_secondary))
    });
    row().w_full().padding(padding).pb(padding / 2.0).child(
        row()
            .w_full()
            .h(EMOJI_SEARCH_PT)
            .px(look.space(SPACE_MD))
            .gap(look.space(SPACE_MD))
            .center_y()
            .radius(radius)
            .fill(look.palette.surface_hover)
            .child(icon(&SEARCH, 16.0).color(look.palette.text_muted))
            .child(field)
            .children(clear),
    )
}

/// The category strip (§16.2): one icon per section, each a jump to it.
fn strip(picker: &EmojiPicker, given: &PickerLook<'_>, padding: f32) -> El {
    let look = given.look;
    let buttons = picker.sections().into_iter().map(|section| {
        let label = section_label(section, given.server);
        let glyph: El = match section {
            Section::Server => server_tile(given.server, 20.0, look),
            other => icon(section_icon(other), PILL_ICON_PT)
                .color(look.palette.text_secondary)
                .group_color(look.palette.text_primary),
        };
        row()
            .key(format!("emoji-jump-{}", section_key(section)))
            .size(30.0, 28.0)
            .center()
            .radius(7.0)
            .role("button")
            .label(label.clone())
            .help(label)
            .hover_group()
            .hover(look.on_pill(look.palette.surface_hover))
            // Its heading lands on the list's top edge.
            .on_press_tx(move |_: &mut Shell, tx| tx.scroll_key_to(block_key(section), 0.0))
            .child(glyph)
    });
    row()
        .w_full()
        .h(EMOJI_STRIP_PT)
        .px(padding)
        .gap(2.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], look.palette.line_subtle)
        .children(buttons)
}

/// How a cell is spoken (§16.5): a custom emoji's code and source (":gato:, Lumen"), a Unicode
/// one's name ("face with tears of joy").
pub(super) fn spoken(choice: &EmojiChoice, server: &str, language: EmojiLanguage) -> String {
    match choice {
        EmojiChoice::Custom(_) => format!("{}, {server}", choice.code()),
        EmojiChoice::Unicode(emoji) => emoji.name(language).to_owned(),
    }
}

/// The preview bar (§16.2): the emoji under the pointer or the keyboard at 28 pt, its code at
/// 600 and where it is from; with none, the Shift hint.
fn preview_bar(picker: &EmojiPicker, given: &PickerLook<'_>) -> El {
    let look = given.look;
    let content: Vec<El> = match picker.previewed() {
        None => vec![
            text(strings::picker_shift_hint())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        ],
        Some(choice) => {
            let (glyph, source) = match choice {
                EmojiChoice::Unicode(emoji) => (
                    text(emoji.glyph).font(sized(look.font(TYPE_BODY), EMOJI_PREVIEW_EMOJI_PT)),
                    strings::picker_unicode().to_owned(),
                ),
                EmojiChoice::Custom(emoji) => {
                    let key = EmojiKey {
                        account: given.account,
                        id: emoji.id,
                    };
                    (
                        emoji_square(
                            &given.emoji.look(key),
                            EMOJI_PREVIEW_EMOJI_PT,
                            &choice.code(),
                            EMOJI_PREVIEW_EMOJI_PT * 3.0,
                            look,
                        ),
                        given.server.to_owned(),
                    )
                }
            };
            vec![
                glyph.a11y_hidden(),
                col()
                    .min_w(0.0)
                    .child(
                        text(choice.code())
                            .font(TextStyle {
                                weight: WEIGHT_SEMIBOLD,
                                ..look.font(TYPE_SMALL)
                            })
                            .color(look.palette.text_primary)
                            .truncate(),
                    )
                    .child(
                        text(source)
                            .font(look.font(TYPE_CAPTION))
                            .color(look.palette.text_muted)
                            .truncate(),
                    ),
            ]
        }
    };
    row()
        .key("emoji-preview")
        .w_full()
        .h(EMOJI_PREVIEW_BAR_PT)
        .px(look.space(SPACE_MD) + 4.0)
        .gap(look.space(SPACE_MD))
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], look.palette.line_subtle)
        .a11y_hidden()
        .children(content)
}

/// "No emoji match “{q}”", with the search icon at 32 pt (§13).
fn no_match(query: &str, look: &Look) -> El {
    col()
        .w_full()
        .grow(1.0)
        .center()
        .gap(look.space(SPACE_MD))
        .child(icon(&SEARCH, 32.0).color(look.palette.text_muted))
        .child(
            text(strings::picker_no_emoji(query))
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .role("status"),
        )
}

/// The server's tile: its initial on the accent, as the canvas draws it.
pub(super) fn server_tile(server: &str, side_pt: f32, look: &Look) -> El {
    let initial: String = server
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect())
        .unwrap_or_default();
    row()
        .size(side_pt, side_pt)
        .center()
        .radius(side_pt * 0.3)
        .fill(look.palette.accent)
        .a11y_hidden()
        .child(
            text(initial)
                .font(TextStyle {
                    weight: WEIGHT_SEMIBOLD,
                    ..sized(look.font(TYPE_CAPTION), side_pt * 0.55)
                })
                .color(look.palette.accent_on),
        )
}

pub(super) fn section_label(section: Section, server: &str) -> String {
    match section {
        Section::Frequent => strings::picker_frequent().to_owned(),
        Section::Server => server.to_owned(),
        Section::Unicode => strings::picker_unicode().to_owned(),
        Section::Category(category) => strings::emoji_category(category).to_owned(),
    }
}

fn section_icon(section: Section) -> &'static rune::Icon {
    match section {
        Section::Frequent => &CLOCK,
        Section::Server | Section::Unicode | Section::Category(EmojiCategory::SmileysAndPeople) => {
            &SMILE
        }
        Section::Category(EmojiCategory::AnimalsAndNature) => &LEAF,
        Section::Category(EmojiCategory::FoodAndDrink) => &CUP,
        Section::Category(EmojiCategory::Activities) => &BALL,
        Section::Category(EmojiCategory::TravelAndPlaces) => &CAR,
        Section::Category(EmojiCategory::Objects) => &BULB,
        Section::Category(EmojiCategory::Symbols) => &HEART,
        Section::Category(EmojiCategory::Flags) => &FLAG,
    }
}

/// `font` at `size_pt`, its line as tall as it.
pub(super) fn sized(font: TextStyle, size_pt: f32) -> TextStyle {
    TextStyle {
        size: size_pt,
        line_height: size_pt * 1.2,
        ..font
    }
}
