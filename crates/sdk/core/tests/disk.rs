//! Physical-budget convergence, canonical gap publication and exact pagination after bounded eviction.

#![cfg(feature = "test-support")]
mod fixture;
use fixture::{Fixture, Result, login, register};
use papo_sdk::{HistoryError, HistoryOrder, HistoryQuery, ReadingError, UnixMicros, Uuid};
use papo_test_server::{Config, HistorySeed};

fn seed() -> HistorySeed {
    HistorySeed {
        channel_id: Uuid::from_u128(10_000).to_string(),
        author_id: Uuid::from_u128(999).to_string(),
        first_message_id: Uuid::from_u128(1_000_000).to_string(),
        count: 2000,
        first_created_at: "2026-01-01T00:00:00.123456Z".into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 7,
    }
}

#[test]
fn physical_budget_evicts_oldest_prefixes_publishes_gaps_and_keeps_exact_refetch_boundaries()
-> Result {
    let mut f = Fixture::new(Config {
        history: vec![seed()],
        ..Config::default()
    })?;
    f.config.disk.bytes = 1024 * 1024;
    f.config.disk.maintenance_interval = std::time::Duration::from_hours(1);
    f.reopen()?;
    f.run(async {
        let client = f.client().handle();
        let server = client
            .add_server(&f.server().base_url(), UnixMicros(0))
            .await?;
        let auth = client.auth(server)?;
        register(&auth, "physical-budget").await?;
        let session = login(&auth, "physical-budget").await?;
        fixture::online(&session).await?;
        let channel = seed().channel_id.parse()?;
        let history = session.history(channel).await?;
        let mut cursor = None;
        loop {
            let page = history
                .page(HistoryQuery {
                    cursor: cursor.as_ref(),
                    limit: Some(100),
                    ..HistoryQuery::default()
                })
                .await?;
            cursor = page.next_cursor;
            if !page.has_more {
                break;
            }
        }
        let mut watch = history.subscribe().await?;
        let before = watch.snapshot.clone();
        assert_eq!(before.messages.len(), 2000);
        assert!(client.disk_usage().await?.pressure);
        let report = client.trim_disk().await?;
        assert!(report.history_turns > 0);
        assert!(report.max_history_rows_per_turn <= 80);
        assert!(report.reclaimed_bytes > 0);
        assert!(!report.pressure);
        let after = history.subscribe().await?.snapshot;
        assert!(after.messages.len() < before.messages.len());
        assert!(!after.messages.is_empty(), "{report:?}");
        assert!(after.version > before.version);
        assert!(!after.chunks[0].oldest_complete);
        assert!(!after.gaps.is_empty());
        assert_eq!(after.tombstones.len(), 0);
        let first = after.messages.front().unwrap();
        let cursor = first.cursor().unwrap();
        let page = history
            .page(HistoryQuery {
                cursor: Some(&cursor),
                order: HistoryOrder::Desc,
                limit: Some(100),
            })
            .await?;
        assert_eq!(page.messages.len(), 100);
        assert!(page.messages.iter().all(|message| message.id != first.id));
        assert!(matches!(
            session
                .report_visible(
                    channel,
                    before.generation,
                    before.version,
                    &[before.messages[0].id.as_deref().unwrap().parse()?]
                )
                .await,
            Err(ReadingError::History(HistoryError::StaleSnapshot))
        ));
        watch.updates.changed().await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
