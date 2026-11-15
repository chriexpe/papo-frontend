//! Canonical mention token syntax, context-free scan and edit round-trip unit tests.

use super::*;

#[test]
fn canonical_token_round_trips_and_handles_case() {
    let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let encoded = encode_user_mention(id);
    assert_eq!(encoded, "@mention(<@550e8400-e29b-41d4-a716-446655440000>)");
    assert_eq!(parse_user_mention(&encoded), Some(id));
    assert_eq!(
        parse_user_mention("@MENTION(<@550E8400-E29B-41D4-A716-446655440000>)"),
        Some(id)
    );
    assert_eq!(parse_user_mention("@mention(<@not-a-uuid>)"), None);
    assert_eq!(
        parse_user_mention(&format!("{encoded} trailing")),
        None,
        "exact token parser must reject trailing characters"
    );
}

#[test]
fn context_free_classification_matches_inside_code_and_punctuation() {
    let target = Uuid::parse_str("018f4b2c-1234-7890-abcd-000000000001").unwrap();
    let other = Uuid::parse_str("018f4b2c-1234-7890-abcd-000000000002").unwrap();
    let token = encode_user_mention(target);

    let inside_inline_code = format!("run `{}` now", token.to_ascii_uppercase());
    assert!(contains_user_mention(&inside_inline_code, target));
    assert!(!contains_user_mention(&inside_inline_code, other));

    let inside_fenced_code = format!("```\n{token}\n```");
    assert!(contains_user_mention(&inside_fenced_code, target));
    assert_eq!(extract_user_mentions(&inside_fenced_code), vec![target]);

    assert!(!contains_user_mention("plain @mention(<@invalid>)", target));
    assert!(contains_everyone_mention("hello `@EVERYONE`!"));
    assert!(!contains_everyone_mention("@everyones"));
}

#[test]
fn decode_and_encode_preserve_unknown_ids_and_adjacent_bindings() {
    let alice = Uuid::from_u128(10);
    let bob = Uuid::from_u128(20);
    let departed = Uuid::from_u128(30);
    let wire = format!(
        "oi {} e {} e {}",
        encode_user_mention(alice),
        encode_user_mention(departed),
        encode_user_mention(bob),
    );
    let decoded = decode_mentions_for_editing(&wire, |id| {
        if id == alice {
            Some("Alice *A*")
        } else if id == bob {
            Some("Bob")
        } else {
            None
        }
    });
    assert_eq!(
        decoded.text,
        format!("oi @Alice *A* e {} e @Bob", encode_user_mention(departed))
    );
    assert_eq!(decoded.bindings.len(), 2);

    let re_encoded = encode_mentions_with_bindings(&decoded.text, &decoded.bindings);
    assert_eq!(re_encoded, wire);
}

#[test]
fn stale_binding_does_not_corrupt_neighbor() {
    let a = Uuid::from_u128(1);
    let b = Uuid::from_u128(2);
    let display = "@Ada and @Bia";
    let bindings = vec![
        MentionBinding {
            id: a,
            start: 0,
            end: 4,
            text: "@Alice".into(),
        },
        MentionBinding {
            id: b,
            start: 9,
            end: 13,
            text: "@Bia".into(),
        },
    ];
    assert_eq!(
        encode_mentions_with_bindings(display, &bindings),
        format!("@Ada and {}", encode_user_mention(b))
    );
}

#[test]
fn non_hyphenated_hex_is_rejected_matching_backend_uuid_pattern() {
    let raw = "@mention(<@550e8400e29b41d4a716446655440000>)";
    assert_eq!(parse_user_mention(raw), None);
    assert!(!contains_user_mention(raw, Uuid::nil()));
}
