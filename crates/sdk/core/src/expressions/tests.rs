//! Atomic confirmed recents, account isolation and backward-compatible outgoing serialization.

use super::*;
use crate::{
    OperationKind, UnixMicros,
    storage_tests::{Fixture, account, address},
};

fn gif() -> Gif {
    Gif {
        id: "fixture".into(),
        url: "https://example.invalid/encoded.gif".into(),
        page_url: "https://example.invalid/item".into(),
        title: String::new(),
        ..Default::default()
    }
}

#[test]
fn selection_only_stages_an_ordinary_message_and_old_payloads_still_decode() {
    let message = gif().message().unwrap();
    assert_eq!(message.content, gif().url);
    assert!(message.gif.is_some());
    let old: crate::OutgoingMessage =
        serde_json::from_str(r#"{"content":"","reply_to":null,"embeds":[],"attachments":[]}"#)
            .unwrap();
    assert!(old.gif.is_none());
}

#[test]
fn recents_are_atomic_with_confirmation_and_isolated_by_account() {
    let fixture = Fixture::new();
    let server = address(1);
    let first = account(&server, 1);
    let other = account(&server, 2);
    fixture.runtime().block_on(async {
        let registry = fixture.storage().registry();
        registry.add_server(server, UnixMicros(0)).await.unwrap();
        for id in [first, other] {
            registry
                .add_account(id, "fixture".into(), None, None, false, UnixMicros(0))
                .await
                .unwrap();
        }
        let kind = OperationKind::Message(gif().message().unwrap());
        registry
            .writer
            .write(move |tx| -> Result<(), crate::OutgoingError> {
                confirmed(tx, first, &kind)?;
                Err(crate::OutgoingError::InvalidState)
            })
            .await
            .unwrap_err();
        let registry2 = registry.clone();
        let count = registry
            .readers
            .query(registry.path.clone(), |tx| {
                tx.query_row("SELECT count(*) FROM expression_recent", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .await
            .unwrap();
        assert_eq!(count, 0);
        registry2
            .writer
            .write(move |tx| -> Result<(), crate::OutgoingError> {
                confirmed(tx, first, &OperationKind::Message(gif().message().unwrap()))
            })
            .await
            .unwrap();
        let counts = registry
            .readers
            .query(registry.path.clone(), move |tx| {
                Ok((
                    tx.query_row(
                        "SELECT count(*) FROM expression_recent WHERE user_id=?1",
                        [first.user().as_bytes()],
                        |row| row.get::<_, i64>(0),
                    )?,
                    tx.query_row(
                        "SELECT count(*) FROM expression_recent WHERE user_id=?1",
                        [other.user().as_bytes()],
                        |row| row.get::<_, i64>(0),
                    )?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(counts, (1, 0));
    });
}

#[test]
fn allocating_expression_order_uses_an_index_instead_of_scanning_other_accounts() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let registry = fixture.storage().registry();
        registry
            .readers
            .query(registry.path.clone(), |tx| {
                for table in [
                    "expression_favorites",
                    "expression_recent",
                    "expression_usage",
                ] {
                    let plan: String = tx.query_row(
                        &format!("EXPLAIN QUERY PLAN SELECT max(used_order) FROM {table}"),
                        [],
                        |row| row.get(3),
                    )?;
                    assert!(plan.contains("USING COVERING INDEX"), "{plan}");
                }
                Ok(())
            })
            .await
            .unwrap();
    });
}

fn item(n: usize) -> serde_json::Value {
    serde_json::json!({
        "id": format!("gif{n:04}"), "url": format!("https://giphy.com/gifs/x-gif{n:04}"),
        "title": "A GIF",
        "images": {
            "original": {"url": "https://media.giphy.com/media/a/giphy.gif", "width": "480", "height": "270"},
            "fixed_width": {"url": "https://media.giphy.com/media/a/200w.gif", "width": "200", "height": "113"}
        }
    })
}

fn page(count: usize, offset: u32, total: u32) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "data": (0..count).map(item).collect::<Vec<_>>(),
        "pagination": {"offset": offset, "count": count, "total_count": total},
        "meta": {"status": 200}
    }))
    .unwrap()
}

#[test]
fn a_page_holds_fifty_gifs_with_their_sizes_and_preview() {
    let parsed = catalog::parse_page_for_test(&page(50, 0, 1000), 0, false).unwrap();
    assert_eq!(parsed.gifs.len(), 50);
    assert_eq!(parsed.next_offset, Some(50));
    let first = &parsed.gifs[0];
    assert_eq!((first.width, first.height), (480, 270));
    assert_eq!(
        first.preview_url,
        "https://media.giphy.com/media/a/200w.gif"
    );
    assert_eq!((first.preview_width, first.preview_height), (200, 113));
    assert!(catalog::parse_page_for_test(&page(51, 0, 1000), 0, false).is_err());
}

#[test]
fn trending_stops_at_its_last_offset_and_a_search_goes_further() {
    let trending = catalog::parse_page_for_test(&page(50, 450, 10_000), 450, false).unwrap();
    assert_eq!(
        trending.next_offset, None,
        "500 is past trending's last offset"
    );
    let search = catalog::parse_page_for_test(&page(50, 450, 10_000), 450, true).unwrap();
    assert_eq!(search.next_offset, Some(500));
}

#[test]
fn trending_searches_keep_clean_terms_in_order() {
    let body = serde_json::to_vec(&serde_json::json!({
        "data": ["bom dia", "  sexta  ", "", "x".repeat(51), "kkkk"],
        "meta": {"status": 200}
    }))
    .unwrap();
    assert_eq!(
        catalog::parse_searches_for_test(&body).unwrap(),
        ["bom dia", "sexta", "kkkk"]
    );
}

#[test]
fn a_gif_saved_before_sizes_still_decodes() {
    let old: Gif = serde_json::from_str(
        r#"{"id":"a","url":"https://x.invalid/a.gif","page_url":"https://x.invalid/a","title":""}"#,
    )
    .unwrap();
    assert_eq!((old.width, old.preview_url.as_str()), (0, ""));
}
