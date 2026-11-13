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
                .add_account(id, None, None, false, UnixMicros(0))
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
