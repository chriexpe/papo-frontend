//! A held history request can deliver an authoritative refusal after its handle enters quarantine.

use super::{
    Account, CHANNEL, Result,
    commit::{refresh, set_id},
    fixture, online_at,
};
use papo_api::{HistoryCursor, HistoryQuery, Uuid};
use papo_sdk::{
    HistoryClosedReason, HistoryError, RefreshFault, RefreshStatus, operations::MetadataScope,
};
use serde_json::json;

#[test]
fn held_history_forbidden_during_quarantine_requires_complete_regrant() -> Result {
    let fixture = fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "held-forbidden").await?;
        let generation = account.status().generation;
        let channel = Uuid::parse_str(CHANNEL)?;
        let old_set = set_id(&account).await?;
        let history = account.session.history(channel).await?;
        let path = format!("/channels/{CHANNEL}/messages");
        account
            .server()
            .queue_operation_response("GET", &path, 403, json!({}))
            .await?;
        let mut held = account.server().hold_next_rest("GET", &path).await?;
        let mut pending = {
            let history = history.clone();
            tokio::spawn(async move {
                // A cursor outside the warmed chunk forces a server fetch.
                let cursor = HistoryCursor {
                    created_at: "2025-01-01T00:00:00Z".into(),
                    id: Uuid::nil().to_string(),
                };
                history
                    .page(HistoryQuery {
                        cursor: Some(&cursor),
                        ..HistoryQuery::default()
                    })
                    .await
            })
        };
        tokio::select! {
            result = held.wait_started() => result?,
            _ = &mut pending => panic!("history completed before its response was held"),
        }

        account
            .script("GET", "/channels", json!({"channels":[false]}))
            .await?;
        assert_eq!(
            refresh(&account, MetadataScope::Channels).await?,
            RefreshStatus::Warning(RefreshFault::Request)
        );
        assert!(matches!(
            history.page(HistoryQuery::default()).await,
            Err(HistoryError::AccessUnknown)
        ));
        assert_eq!(account.status().generation, generation);
        assert_eq!(set_id(&account).await?, old_set);

        held.release();
        assert!(matches!(
            pending.await?,
            Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
        ));
        let cache_path = account.cache.clone();
        let revoked: bool = tokio::task::spawn_blocking(move || {
            rusqlite::Connection::open_with_flags(
                cache_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?
            .query_row(
                "SELECT revoked FROM history_sets WHERE id=?1",
                [old_set],
                |row| row.get(0),
            )
        })
        .await??;
        assert!(revoked);

        account
            .script(
                "GET",
                "/channels",
                json!({"channels":[{"id":CHANNEL,"type":"text","permissions":[]}]}),
            )
            .await?;
        assert_eq!(
            refresh(&account, MetadataScope::Channels).await?,
            RefreshStatus::Updated
        );
        assert!(account.snapshot().channel_census_complete);
        assert!(matches!(
            account.session.history(channel).await,
            Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
        ));
        assert_eq!(account.status().generation, generation);

        account.session.reconnect()?;
        online_at(&account.session, |status| status.generation > generation).await?;
        account.session.history(channel).await?.subscribe().await?;
        assert_ne!(set_id(&account).await?, old_set);
        assert!(matches!(
            history.page(HistoryQuery::default()).await,
            Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
