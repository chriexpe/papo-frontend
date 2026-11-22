//! Mentions as pills, their wire form, the @ query and the people it finds.

use papo_sdk::{Generation, ResourceSnapshot, UserSummary, Uuid};
use papo_sdk_ui::message_content::decode_composer_mentions;
use rune::TextEdit;

use super::{Candidate, Query, Target, candidates, pick, query, restore, wire};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x1000 + n)
}

/// A roster of `names`, in order, the first one's own.
fn roster(names: &[&str]) -> ResourceSnapshot {
    let members = names
        .iter()
        .enumerate()
        .map(|(n, name)| UserSummary {
            id: Some(id(n as u128).to_string()),
            username: Some((*name).to_owned()),
            // The account's own row must say its roles.
            roles: Some(Vec::new()),
            ..UserSummary::default()
        })
        .collect();
    ResourceSnapshot::fixture(
        Generation::for_test(1),
        None,
        Vec::new(),
        Vec::new(),
        members,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        id(0),
    )
    .unwrap()
}

/// A field holding `text` with the caret at its end.
fn typed(text: &str) -> TextEdit {
    let mut edit = TextEdit::new(text);
    edit.set_caret(text.len());
    edit
}

fn names(found: &[Candidate]) -> Vec<&str> {
    found
        .iter()
        .map(|candidate| candidate.name.as_str())
        .collect()
}

#[test]
fn an_at_starting_a_word_opens_a_query() {
    assert_eq!(
        query(&typed("oi @ma")),
        Some(Query {
            range: 3..6,
            text: "ma".to_owned()
        })
    );
    assert_eq!(query(&typed("@")).map(|q| q.text), Some(String::new()));
}

#[test]
fn an_address_or_a_space_is_no_query() {
    assert_eq!(query(&typed("ana@example")), None);
    assert_eq!(query(&typed("@ma rina")), None);
}

#[test]
fn names_starting_with_the_query_come_first() {
    let snapshot = roster(&["ana", "mariana", "marina"]);
    let found = candidates(&snapshot, "MAR", false);
    assert_eq!(names(&found), ["mariana", "marina"]);
    let found = candidates(&snapshot, "ana", false);
    assert_eq!(
        names(&found),
        ["ana", "mariana"],
        "then names containing it"
    );
}

#[test]
fn everyone_leads_only_where_allowed() {
    let snapshot = roster(&["eva"]);
    let found = candidates(&snapshot, "e", true);
    assert_eq!(found[0].target, Target::Everyone);
    assert_eq!(names(&found), ["everyone", "eva"]);
    assert_eq!(names(&candidates(&snapshot, "e", false)), ["eva"]);
}

#[test]
fn a_picked_person_becomes_one_pill_and_a_space() {
    let snapshot = roster(&["ana", "marina"]);
    let mut edit = typed("oi @mar");
    let found = candidates(&snapshot, "mar", false);
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
    let snapshot = roster(&["ana", "marina"]);
    let mut edit = typed("@mar");
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &candidates(&snapshot, "mar", false)[0]);
    edit.insert("olá");
    assert_eq!(wire(&edit), format!("@mention(<@{}>) olá", id(1)));
}

#[test]
fn a_pill_moves_with_the_text_before_it() {
    let snapshot = roster(&["marina"]);
    let mut edit = typed("@m");
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &candidates(&snapshot, "m", false)[0]);
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
    let snapshot = roster(&["eva"]);
    let mut edit = typed("@ev");
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &candidates(&snapshot, "ev", true)[0]);
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
    let snapshot = roster(&["ana", "marina"]);
    let mut edit = typed("@an");
    let asked = query(&edit).unwrap();
    pick(&mut edit, &asked, &candidates(&snapshot, "an", false)[0]);
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

#[test]
fn an_empty_nickname_falls_back_to_the_username() {
    assert_eq!(super::display_name(Some(""), Some("ana")), Some("ana"));
    assert_eq!(super::display_name(Some("Ana"), Some("ana")), Some("Ana"));
    assert_eq!(super::display_name(Some(""), None), None);
}
