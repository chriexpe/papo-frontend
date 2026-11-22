//! Mentions as pills, their wire form, and the @ query at the field's
//! caret. The ranking itself is the SDK's (`mention_suggestions`).

use papo_sdk::{MentionTarget, Uuid};
use papo_sdk_ui::message_content::decode_composer_mentions;
use papo_sdk_ui::{MentionQuery, MentionSuggestion};
use rune::TextEdit;

use super::{pick, query, restore, wire};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x1000 + n)
}

/// The suggestion of person `n`, called `name`.
fn person(n: u128, name: &str) -> MentionSuggestion {
    MentionSuggestion {
        target: MentionTarget::User(id(n)),
        name: name.to_owned(),
        username: Some(name.to_owned()),
    }
}

fn everyone() -> MentionSuggestion {
    MentionSuggestion {
        target: MentionTarget::Everyone,
        name: "everyone".to_owned(),
        username: None,
    }
}

/// A field holding `text` with the caret at its end.
fn typed(text: &str) -> TextEdit {
    let mut edit = TextEdit::new(text);
    edit.set_caret(text.len());
    edit
}

#[test]
fn an_at_starting_a_word_opens_a_query() {
    assert_eq!(
        query(&typed("oi @ma")),
        Some(MentionQuery {
            byte_range: 3..6,
            text: "ma".to_owned()
        })
    );
    assert_eq!(query(&typed("@")).map(|q| q.text), Some(String::new()));
}

#[test]
fn a_selection_or_an_at_inside_a_mention_is_no_query() {
    let mut selected = typed("oi @ma");
    selected.set_selection(3, 6);
    assert_eq!(query(&selected), None);
    let mut edit = typed("@mar");
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &person(1, "marina"));
    edit.set_caret(3);
    assert_eq!(query(&edit), None, "the caret sits inside the pill");
}

#[test]
fn a_picked_person_becomes_one_pill_and_a_space() {
    let mut edit = typed("oi @mar");
    let found = [person(1, "marina")];
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &found[0]);
    assert_eq!(edit.text, "oi @marina ");
    let pills: Vec<_> = edit
        .pills()
        .map(|(range, id)| (range, id.to_owned()))
        .collect();
    assert_eq!(pills, [(3..10, id(1).to_string())]);
    assert_eq!(query(&edit), None, "the picker closes");
}

#[test]
fn the_wire_form_carries_the_person_s_id() {
    let mut edit = typed("@mar");
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &person(1, "marina"));
    edit.insert("olá");
    assert_eq!(wire(&edit), format!("@mention(<@{}>) olá", id(1)));
}

#[test]
fn a_pill_moves_with_the_text_before_it() {
    let mut edit = typed("@m");
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &person(0, "marina"));
    edit.set_caret(0);
    edit.insert("ei ");
    assert_eq!(wire(&edit), format!("ei @mention(<@{}>) ", id(0)));
}

#[test]
fn a_restored_draft_gets_its_pills_back_and_no_history() {
    let wire_text = format!("oi @mention(<@{}>)!", id(1));
    let decoded = decode_composer_mentions(&wire_text, |_| Some("marina"));
    let mut edit = TextEdit::new("");
    restore(&mut edit, decoded);
    assert_eq!(edit.text, "oi @marina!");
    assert_eq!(wire(&edit), wire_text);
    assert!(!edit.has_history());
}

#[test]
fn everyone_is_plain_text() {
    let mut edit = typed("@ev");
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &everyone());
    assert_eq!(edit.text, "@everyone ");
    assert_eq!(edit.pills().count(), 0);
}

#[test]
fn a_person_the_roster_lacks_keeps_their_token() {
    let wire_text = format!("oi @mention(<@{}>)", id(7));
    let mut edit = TextEdit::new("");
    restore(&mut edit, decode_composer_mentions(&wire_text, |_| None));
    assert_eq!(edit.text, wire_text);
    assert_eq!(edit.pills().count(), 0);
}

#[test]
fn a_name_rune_cleans_never_moves_the_mentions_after_it() {
    // U+2028 is three bytes; Rune's pill label turns it into one space.
    let wire_text = format!("@mention(<@{}>) e @mention(<@{}>)", id(1), id(2));
    let decoded = decode_composer_mentions(&wire_text, |user| {
        Some(if user == id(1) {
            "a\u{2028}b"
        } else {
            "marina"
        })
    });
    let mut edit = TextEdit::new("");
    restore(&mut edit, decoded);
    let labels: Vec<&str> = edit.pills().map(|(range, _)| &edit.text[range]).collect();
    assert_eq!(labels, ["@a b", "@marina"]);
    assert_eq!(edit.text, "@a b e @marina", "no byte of the text was lost");
}

#[test]
fn a_failed_message_goes_first_and_both_keep_their_mentions() {
    let mut edit = typed("@an");
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &person(0, "ana"));
    let failed = format!("oi @mention(<@{}>)", id(1));
    super::prepend(
        &mut edit,
        decode_composer_mentions(&failed, |_| Some("marina")),
    );
    assert_eq!(edit.text, "oi @marina\n@ana ");
    assert_eq!(
        wire(&edit),
        format!("oi @mention(<@{}>)\n@mention(<@{}>) ", id(1), id(0))
    );
}
