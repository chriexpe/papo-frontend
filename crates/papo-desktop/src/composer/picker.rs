//! The field's picker (`docs/design.md` §3, "Composer surfaces"; §16.2): typing `@` suggests
//! people, typing `:` suggests emoji. It grows upward out of the text pill, as wide as it, up to
//! six rows and then it scrolls. A person's row: avatar, name, @username, @everyone first where
//! allowed. An emoji's row: the emoji, its code and where it is from, as the emoji picker's
//! preview bar says them. ↑ and ↓ move, Enter or Tab picks, Escape closes; the keyboard stays in
//! the field throughout.
//!
//! Its state is the query being typed and what it found; the shell finds people in the
//! account's roster and emoji with the SDK's search. The view is pure.

use std::ops::Range;

use papo_sdk::{AccountId, MentionTarget};
use papo_sdk_ui::{EmojiLanguage, EmojiQuery, EmojiSuggestion, MentionQuery, MentionSuggestion};
use rune::{Align, Cx, El, ScrollSpec, Side, TextStyle, col, row, text};

use crate::channel_pane::avatar;
use crate::design::{
    Look, MENU_ROW_HEIGHT_PT, RADIUS_PILL, SPACE_LG, SPACE_SM, Surfaces, TYPE_BODY, TYPE_SMALL,
    WEIGHT_SEMIBOLD, emoji_box,
};
use crate::emoji_images::{EmojiKey, EmojiLooks, emoji_square};
use crate::shell::Shell;
use crate::strings;
use crate::timeline::TimelineKey;

/// The element key of the open picker.
pub(crate) const PICKER_KEY: &str = "field-picker";

/// The rows shown before the list scrolls.
const VISIBLE_ROWS: f32 = 6.0;

/// What is being typed at the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FieldQuery {
    /// An `@` and a name: people.
    Mention(MentionQuery),
    /// A `:` and a name: emoji.
    Emoji(EmojiQuery),
}

impl FieldQuery {
    /// Where it starts and which kind it is: what Escape keeps closed.
    pub(crate) fn place(&self) -> (usize, bool) {
        (
            self.byte_range().start,
            matches!(self, FieldQuery::Emoji(_)),
        )
    }

    /// The byte range from the `@` or `:` to the caret: what a pick replaces.
    pub(crate) fn byte_range(&self) -> Range<usize> {
        match self {
            FieldQuery::Mention(query) => query.byte_range.clone(),
            FieldQuery::Emoji(query) => query.byte_range.clone(),
        }
    }
}

/// What the picker's rows offer.
#[derive(Debug, PartialEq)]
pub(crate) enum Rows {
    People(Vec<MentionSuggestion>),
    Emoji(Vec<EmojiSuggestion>),
}

impl Rows {
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn len(&self) -> usize {
        match self {
            Rows::People(people) => people.len(),
            Rows::Emoji(emoji) => emoji.len(),
        }
    }

    /// Row `at`'s choice.
    fn get(&self, at: usize) -> Option<Choice> {
        match self {
            Rows::People(people) => people.get(at).map(|person| Choice::Person(person.target)),
            Rows::Emoji(emoji) => emoji.get(at).cloned().map(Choice::Emoji),
        }
    }
}

/// What a row picks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    Person(MentionTarget),
    Emoji(EmojiSuggestion),
}

/// The picker over one field's query.
#[derive(Debug, PartialEq)]
pub(crate) struct Picker {
    pub(super) key: TimelineKey,
    pub(super) query: FieldQuery,
    pub(super) rows: Rows,
    /// The highlighted row: what Enter or Tab picks.
    pub(super) selected: usize,
}

impl Picker {
    /// Moves the highlight `by` rows, wrapping round the ends.
    pub(super) fn step(&mut self, by: isize) {
        self.selected = wrapped_step(self.selected, self.rows.len(), by);
    }

    /// The highlighted row's key.
    pub(crate) fn chosen_key(&self) -> Option<String> {
        self.chosen().map(|choice| row_key(&choice))
    }

    /// What the highlighted row picks.
    pub(crate) fn chosen(&self) -> Option<Choice> {
        self.rows.get(self.selected)
    }

    /// The row that picks `choice`, if any.
    pub(super) fn position(&self, choice: &Choice) -> Option<usize> {
        match (&self.rows, choice) {
            (Rows::People(people), Choice::Person(target)) => {
                people.iter().position(|person| person.target == *target)
            }
            (Rows::Emoji(found), Choice::Emoji(emoji)) => {
                found.iter().position(|suggestion| suggestion == emoji)
            }
            _ => None,
        }
    }

    /// The custom emoji its rows draw: what is fetched for it.
    pub(crate) fn custom_emoji(&self) -> Vec<papo_sdk::Uuid> {
        match &self.rows {
            Rows::People(_) => Vec::new(),
            Rows::Emoji(emoji) => emoji
                .iter()
                .filter_map(|suggestion| match suggestion {
                    EmojiSuggestion::Custom(emoji) => Some(emoji.id),
                    EmojiSuggestion::Unicode(_) => None,
                })
                .collect(),
        }
    }
}

/// Row `at` of a list of `count` rows moved `by` rows, wrapping round the
/// ends: the field picker's and the search field's ↑ and ↓.
pub(crate) fn wrapped_step(at: usize, count: usize, by: isize) -> usize {
    if count == 0 {
        return 0;
    }
    let count = isize::try_from(count).unwrap_or(isize::MAX);
    let at = isize::try_from(at).unwrap_or(0);
    usize::try_from((at + by).rem_euclid(count)).unwrap_or(0)
}

/// The element key of the row of `choice`, by what it picks, so a row keeps its identity as the
/// query narrows. The field names the highlighted one as its active descendant, since the
/// keyboard stays in the field.
pub(crate) fn row_key(choice: &Choice) -> String {
    match choice {
        Choice::Person(MentionTarget::Everyone) => "mention-everyone".to_owned(),
        Choice::Person(MentionTarget::User(id)) => format!("mention-{id}"),
        Choice::Emoji(EmojiSuggestion::Custom(emoji)) => format!("suggest-emoji-{}", emoji.id),
        Choice::Emoji(EmojiSuggestion::Unicode(emoji)) => format!("suggest-emoji-{}", emoji.code),
    }
}

/// What an emoji row reads besides the picker.
pub(crate) struct EmojiRows<'a> {
    /// What custom emoji draw as, for the account the field writes as.
    pub(crate) images: EmojiLooks,
    pub(crate) account: AccountId,
    /// The server's name: where its custom emoji are from.
    pub(crate) server: &'a str,
    /// The language a Unicode emoji's name is spoken in.
    pub(crate) language: EmojiLanguage,
}

/// Draws `picker` above the pill keyed `pill_key`, as wide as it.
pub(crate) fn view(
    picker: &Picker,
    pill_key: &str,
    emoji: &EmojiRows<'_>,
    look: &Look,
    cx: &Cx<Shell>,
) -> El {
    let width = cx.layout_size(pill_key).map(|[width, _]| width);
    let rows: Vec<El> = match &picker.rows {
        Rows::People(people) => people
            .iter()
            .enumerate()
            .map(|(index, person)| person_row(person, index == picker.selected, look))
            .collect(),
        Rows::Emoji(found) => found
            .iter()
            .enumerate()
            .map(|(index, suggestion)| emoji_row(suggestion, index == picker.selected, emoji, look))
            .collect(),
    };
    let label = match picker.rows {
        Rows::People(_) => strings::mention_picker(),
        Rows::Emoji(_) => strings::emoji_suggestions(),
    };
    let list = col()
        .key("field-picker-rows")
        .w_full()
        .max_h(MENU_ROW_HEIGHT_PT * VISIBLE_ROWS)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .children(rows);
    let mut panel = col()
        .key(PICKER_KEY)
        .padding(look.space(SPACE_SM))
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .role("listbox")
        .label(label)
        .anchored(pill_key, Side::Top, Align::Start, look.space(SPACE_SM))
        .child(list);
    if let Some(width) = width {
        panel = panel.w(width);
    }
    panel
}

/// A row's frame: its key and label, its press, and the highlight.
fn row_frame(choice: Choice, label: String, selected: bool, look: &Look) -> El {
    let line = row()
        .key(row_key(&choice))
        .w_full()
        .h(MENU_ROW_HEIGHT_PT)
        .px(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM))
        .center_y()
        // Concentric with the picker, whose edge is one `space.sm` out.
        .radius(Look::nested_radius(
            look.radius(RADIUS_PILL),
            look.space(SPACE_SM),
        ))
        .role("option")
        .label(label)
        .on_press_tx(move |shell: &mut Shell, tx| shell.suggestion_picked(&choice, tx));
    // `surface.hover` is too close to the pill's own surface to show.
    if selected {
        line.fill(look.on_pill(look.palette.surface_selected))
    } else {
        line
    }
}

fn person_row(candidate: &MentionSuggestion, selected: bool, look: &Look) -> El {
    let label = match &candidate.username {
        Some(username) if *username != candidate.name => {
            format!("{} @{username}", candidate.name)
        }
        _ => candidate.name.clone(),
    };
    let handle = match (&candidate.target, &candidate.username) {
        // A person with no nickname shows their username once, as the name.
        (MentionTarget::User(_), Some(username)) if *username == candidate.name => None,
        (MentionTarget::User(_), Some(username)) => Some(format!("@{username}")),
        (MentionTarget::Everyone, _) => Some(strings::mention_everyone_hint().to_owned()),
        (MentionTarget::User(_), None) => None,
    };
    row_frame(Choice::Person(candidate.target), label, selected, look)
        .child(avatar(&candidate.name, None, look))
        .child(
            text(candidate.name.clone())
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_primary)
                .truncate(),
        )
        .children(handle.map(|handle| {
            text(handle)
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .truncate()
        }))
        .child(row().grow(1.0))
        .pr(look.space(SPACE_LG))
}

/// An emoji's row (§16.2): the emoji (a custom one at line height, a Unicode one from the colour
/// font), its code at 600 and where it is from, spoken as the emoji picker speaks a cell.
fn emoji_row(
    suggestion: &EmojiSuggestion,
    selected: bool,
    rows: &EmojiRows<'_>,
    look: &Look,
) -> El {
    let font = look.font(TYPE_BODY);
    let (glyph, code, source, spoken) = match suggestion {
        EmojiSuggestion::Custom(emoji) => {
            let key = EmojiKey {
                account: rows.account,
                id: emoji.id,
            };
            let code = format!(":{}:", emoji.name);
            let (side_pt, _) = emoji_box(font, false);
            let glyph = emoji_square(&rows.images.look(key), side_pt, &code, side_pt, look);
            // ":gato:, Lumen", as the emoji picker speaks a cell.
            let spoken = format!("{code}, {}", rows.server);
            (glyph, code, rows.server.to_owned(), spoken)
        }
        EmojiSuggestion::Unicode(emoji) => (
            text(emoji.glyph).font(font).a11y_hidden(),
            format!(":{}:", emoji.code),
            strings::picker_unicode().to_owned(),
            // Its name, in the app's language: "face with tears of joy".
            emoji.name(rows.language).to_owned(),
        ),
    };
    row_frame(Choice::Emoji(suggestion.clone()), spoken, selected, look)
        .child(
            row()
                .size(font.line_height, font.line_height)
                .center()
                .child(glyph),
        )
        .child(
            text(code)
                .font(TextStyle {
                    weight: WEIGHT_SEMIBOLD,
                    ..font
                })
                .color(look.palette.text_primary)
                .truncate(),
        )
        .child(
            text(source)
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .truncate(),
        )
        .child(row().grow(1.0))
        .pr(look.space(SPACE_LG))
}

#[cfg(test)]
mod tests {
    use papo_sdk::Uuid;

    use super::{FieldQuery, Picker, Rows};
    use crate::timeline::TimelineKey;
    use papo_sdk::MentionTarget;
    use papo_sdk_ui::{MentionQuery, MentionSuggestion};

    fn picker(rows: usize) -> Picker {
        Picker {
            key: TimelineKey {
                account: papo_sdk::AccountId::new(
                    papo_sdk::ServerAddress::parse("https://papo.example.invalid")
                        .unwrap()
                        .id(),
                    Uuid::nil(),
                ),
                channel: Uuid::from_u128(1),
            },
            query: FieldQuery::Mention(MentionQuery {
                byte_range: 0..1,
                text: String::new(),
            }),
            rows: Rows::People(
                (0..rows)
                    .map(|n| MentionSuggestion {
                        target: MentionTarget::User(Uuid::from_u128(n as u128)),
                        name: format!("p{n}"),
                        username: None,
                    })
                    .collect(),
            ),
            selected: 0,
        }
    }

    #[test]
    fn the_arrows_wrap_round_the_ends() {
        let mut picker = picker(3);
        picker.step(-1);
        assert_eq!(picker.selected, 2);
        picker.step(1);
        assert_eq!(picker.selected, 0);
    }
}
