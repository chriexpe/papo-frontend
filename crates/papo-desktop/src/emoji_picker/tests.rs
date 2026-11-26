//! The Emoji tab's list and keys: its sections in order (§16.2: Frequently used, this server's,
//! the Unicode categories), a search's server matches first, the cursor's moves, and which
//! custom emoji are fetched.

use std::sync::Arc;

use papo_sdk::{AccountId, CustomEmoji, EmojiCatalog, ServerAddress, Uuid};
use papo_sdk_ui::EmojiCategory;

use super::{COLUMNS, EmojiChoice, EmojiPicker, PickerFor, Section, Step};
use crate::timeline::TimelineKey;

fn key() -> TimelineKey {
    TimelineKey {
        account: AccountId::new(
            ServerAddress::parse("https://emoji.invalid").unwrap().id(),
            Uuid::nil(),
        ),
        channel: Uuid::from_u128(1),
    }
}

fn custom(id: u128, name: &str) -> CustomEmoji {
    CustomEmoji {
        id: Uuid::from_u128(id),
        name: name.to_owned(),
        format: "png".to_owned(),
        created_at: String::new(),
    }
}

fn catalog(count: u128) -> EmojiCatalog {
    EmojiCatalog {
        revision: 1,
        emojis: (1..=count)
            .map(|n| custom(n, &format!("rocket{n}")))
            .collect(),
    }
}

fn picker(catalog: &EmojiCatalog, usage: &[(&str, u64)]) -> EmojiPicker {
    let usage: Arc<[(String, u64)]> = usage
        .iter()
        .map(|(emoji, count)| ((*emoji).to_owned(), *count))
        .collect();
    EmojiPicker::new(PickerFor::Field(key()), Some(catalog), usage)
}

#[test]
fn sections_come_frequent_then_the_server_then_every_category() {
    let picker = picker(&catalog(2), &[("🧀", 5)]);
    let mut expected = vec![Section::Frequent, Section::Server];
    expected.extend(EmojiCategory::ALL.map(Section::Category));
    assert_eq!(picker.sections(), expected);
    assert_eq!(
        picker.cells().first().map(EmojiChoice::code).as_deref(),
        Some(":cheese_wedge:")
    );
}

#[test]
fn a_search_lists_the_servers_matches_before_unicode_and_takes_no_frequent() {
    let mut picker = picker(&catalog(2), &[("🧀", 5)]);
    picker.search.replace_text("rocket");
    picker.rebuild(Some(&catalog(2)));
    assert_eq!(picker.sections(), [Section::Server, Section::Unicode]);
    assert_eq!(picker.cells()[0].code(), ":rocket1:");
    assert!(
        picker.cells().iter().any(|cell| cell.code() == ":rocket:"),
        "🚀 among the Unicode matches"
    );
}

#[test]
fn the_keyboard_enters_from_the_search_moves_by_cell_and_row_and_leaves_upward() {
    let mut picker = picker(&catalog(12), &[]);
    // No Frequently used: the server's twelve lead (a row of nine, then three).
    assert_eq!(picker.step(Step::Down), Some(0));
    assert_eq!(picker.step(Step::Right), Some(1));
    assert_eq!(
        picker.step(Step::Down),
        Some(10),
        "the next row, same column"
    );
    assert_eq!(
        picker.step(Step::Down),
        Some(12 + 1),
        "into the next section's first row"
    );
    assert_eq!(picker.step(Step::Up), Some(10));
    assert_eq!(picker.step(Step::End), Some(11), "the row's last cell");
    assert_eq!(picker.step(Step::Home), Some(9));
    assert_eq!(picker.step(Step::Up), Some(0));
    assert_eq!(picker.step(Step::Up), None, "back to the search");
}

#[test]
fn only_the_custom_emoji_of_rows_on_screen_are_fetched() {
    let mut picker = picker(&catalog(200), &[]);
    assert!(picker.shown_custom().is_empty(), "nothing on screen yet");
    // The server's section is the first; its rows 2 and 3 show.
    assert!(picker.sections_shown(0..2));
    assert!(picker.rows_shown(Section::Server, 2..4));
    let first = COLUMNS * 2;
    let expected: Vec<Uuid> = (first..first + 2 * COLUMNS)
        .map(|at| Uuid::from_u128(at as u128 + 1))
        .collect();
    assert_eq!(picker.shown_custom(), expected);
    assert!(
        !picker.rows_shown(Section::Server, 2..4),
        "the same rows: no change"
    );
    assert!(picker.sections_shown(1..3));
    assert!(
        picker.shown_custom().is_empty(),
        "the section scrolled away"
    );
    assert!(picker.sections_shown(0..2));
    assert_eq!(
        picker.shown_custom(),
        expected,
        "back in view: its rows as its list last said"
    );
}

#[test]
fn a_refined_search_keeps_what_the_lists_last_said_is_on_screen() {
    let mut picker = picker(&catalog(30), &[]);
    picker.search.replace_text("r");
    picker.rebuild(Some(&catalog(30)));
    assert!(picker.sections_shown(0..2));
    assert!(picker.rows_shown(Section::Server, 0..2));
    picker.search.replace_text("ro");
    picker.rebuild(Some(&catalog(30)));
    // The same sections and keys: Rune says nothing again, and the new matches are still asked
    // for.
    assert_eq!(picker.shown_custom().len(), 18);
}

#[test]
fn a_cell_in_frequently_used_and_its_own_section_has_a_key_for_each() {
    let mut picker = picker(&catalog(1), &[("00000000-0000-0000-0000-000000000001", 4)]);
    assert_eq!(picker.step(Step::Down), Some(0));
    assert_eq!(
        picker.chosen_key().as_deref(),
        Some("emoji-cell-frequent-00000000-0000-0000-0000-000000000001")
    );
    picker.step(Step::Down);
    assert_eq!(
        picker.chosen_key().as_deref(),
        Some("emoji-cell-server-00000000-0000-0000-0000-000000000001")
    );
    assert_eq!(
        picker.chosen_row_path().as_deref(),
        Some("emoji-section-server/emoji-section-server-rows/emoji-row-server-0")
    );
}

#[test]
fn frequently_used_holds_the_count_it_opened_with() {
    let mut picker = picker(&catalog(1), &[("👍", 2)]);
    let before = picker.cells()[0].clone();
    picker.rebuild(Some(&catalog(1)));
    assert_eq!(picker.cells()[0], before);
}
