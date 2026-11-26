//! The shared content cache: account isolation, bounds, weak entries and emoji revisions.
use super::*;
use papo_sdk::ServerAddress;

fn none() -> CustomEmojis {
    CustomEmojis::default()
}

fn account(user_seed: u128, host: &str) -> AccountId {
    let server = ServerAddress::parse(host).expect("valid server URL").id();
    AccountId::new(server, Uuid::from_u128(user_seed))
}

fn message(id: u128, body: &str, edited_at: Option<&str>) -> Arc<Message> {
    Arc::new(Message {
        id: Some(Uuid::from_u128(id).to_string()),
        content: Some(body.into()),
        edited_at: edited_at.map(str::to_owned),
        ..Message::default()
    })
}

#[test]
fn reuses_across_metadata_diffs_and_drops_when_weak_holders_go_away() {
    let key = ContentCacheKey {
        account: account(0xA1, "https://cache-reuse.invalid"),
        channel: Uuid::from_u128(0xC1),
        message: Uuid::from_u128(0xD1),
    };
    let msg_v1 = message(0xD1, "**hello** world", None);
    let parsed_v1 = get_or_parse(Some(key), &msg_v1, &none());
    let again = get_or_parse(Some(key), &msg_v1, &none());
    assert!(Arc::ptr_eq(&parsed_v1, &again));

    let mut reaction_updated = (*msg_v1).clone();
    reaction_updated.reactions = Some(Vec::new());
    let msg_v1_react = Arc::new(reaction_updated);
    let after_react = get_or_parse(Some(key), &msg_v1_react, &none());
    assert!(Arc::ptr_eq(&parsed_v1, &after_react));

    let edited = message(0xD1, "**edited** world", Some("2026-01-01T00:01:00Z"));
    let parsed_v2 = get_or_parse(Some(key), &edited, &none());
    assert!(!Arc::ptr_eq(&parsed_v1, &parsed_v2));

    let weak = Arc::downgrade(&parsed_v2);
    drop(parsed_v2);
    assert!(
        weak.upgrade().is_none(),
        "cache must never hold strong references"
    );
}

#[test]
fn a_new_custom_emoji_revision_parses_again() {
    let key = ContentCacheKey {
        account: account(11, "https://emoji.invalid"),
        channel: Uuid::from_u128(0xC5),
        message: Uuid::from_u128(0xD5),
    };
    let msg = message(0xD5, ":gato:", None);
    let before = get_or_parse(Some(key), &msg, &none());
    let catalog = papo_sdk::EmojiCatalog {
        revision: 7,
        emojis: vec![papo_sdk::CustomEmoji {
            id: Uuid::from_u128(0xE1),
            name: "gato".into(),
            format: "png".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        }],
    };
    let after = get_or_parse(Some(key), &msg, &CustomEmojis::new(&catalog));
    assert!(!Arc::ptr_eq(&before, &after));
    assert!(matches!(
        after.blocks(),
        [crate::message_content::Block::Paragraph(inlines)]
            if matches!(inlines.as_slice(), [crate::message_content::Inline::CustomEmoji { .. }])
    ));
}

#[test]
fn isolates_different_accounts_even_with_identical_ids_and_content() {
    let channel = Uuid::from_u128(0xC2);
    let msg_id = Uuid::from_u128(0xD2);
    let msg = message(0xD2, "same text", None);

    let a = get_or_parse(
        Some(ContentCacheKey {
            account: account(1, "https://iso.invalid"),
            channel,
            message: msg_id,
        }),
        &msg,
        &none(),
    );
    let b = get_or_parse(
        Some(ContentCacheKey {
            account: account(2, "https://iso.invalid"),
            channel,
            message: msg_id,
        }),
        &msg,
        &none(),
    );
    assert!(!Arc::ptr_eq(&a, &b));
}

#[test]
fn bounds_live_entries_without_unbounded_growth() {
    let acct = account(9, "https://bound.invalid");
    let channel = Uuid::from_u128(0xC3);
    let mut cache = CacheState::default();
    let mut retained = Vec::new();
    for index in 0..=(MAX_CACHED_REVISIONS * 2 + 32) {
        let msg_id = Uuid::from_u128(index as u128 + 1);
        let msg = message(index as u128 + 1, "entry", None);
        let parsed = Arc::new(MessageContent::parse("entry"));
        cache.insert(
            ContentCacheKey {
                account: acct,
                channel,
                message: msg_id,
            },
            &msg,
            &parsed,
            0,
        );
        retained.push((msg, parsed));
    }
    assert!(cache.entries.len() <= MAX_CACHED_REVISIONS);
}

#[test]
fn reclaims_dropped_entries_during_eviction() {
    let acct = account(10, "https://reclaim.invalid");
    let channel = Uuid::from_u128(0xC4);
    let mut cache = CacheState::default();
    let first_id = Uuid::from_u128(1);
    let first_msg = message(1, "drop me", None);
    let first_parsed = Arc::new(MessageContent::parse("drop me"));
    cache.insert(
        ContentCacheKey {
            account: acct,
            channel,
            message: first_id,
        },
        &first_msg,
        &first_parsed,
        0,
    );
    let mut kept = Vec::new();
    for index in 2..=MAX_CACHED_REVISIONS {
        let msg_id = Uuid::from_u128(index as u128);
        let msg = message(index as u128, "keep", None);
        let parsed = Arc::new(MessageContent::parse("keep"));
        cache.insert(
            ContentCacheKey {
                account: acct,
                channel,
                message: msg_id,
            },
            &msg,
            &parsed,
            0,
        );
        kept.push((msg, parsed));
    }
    drop(first_msg);
    drop(first_parsed);

    let extra_id = Uuid::from_u128(99_999);
    let extra_msg = message(99_999, "extra", None);
    let extra_parsed = Arc::new(MessageContent::parse("extra"));
    cache.insert(
        ContentCacheKey {
            account: acct,
            channel,
            message: extra_id,
        },
        &extra_msg,
        &extra_parsed,
        0,
    );
    assert_eq!(cache.entries.len(), MAX_CACHED_REVISIONS);
    assert_eq!(cache.evictions_since_sweep, 0);
    assert!(cache.entries.contains_key(&ContentCacheKey {
        account: acct,
        channel,
        message: extra_id,
    }));
}
