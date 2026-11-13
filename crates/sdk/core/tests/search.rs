//! Detached search identity, exact pagination and cancellation over loopback; fixture scripts authorized results.
#![cfg(feature = "test-support")]
mod fixture;
use fixture::{Fixture, Result, login, register};
use papo_api::wire::{Message, SearchResult};
use papo_sdk::{DetachedMessageUpdate, SearchError, SearchQuery, Session, UnixMicros, Uuid};

async fn setup(f: &Fixture) -> Result<(Session, Uuid)> {
    let client = f.client().handle();
    let server = client
        .add_server(&f.server().base_url(), UnixMicros(0))
        .await?;
    let auth = client.auth(server)?;
    register(&auth, "search-first").await?;
    let session = login(&auth, "search-first").await?;
    fixture::online(&session).await?;
    let channel: Uuid = f
        .server()
        .create_channel("fixture")
        .await?
        .id
        .ok_or("channel")?
        .parse()?;
    let mut resources = session.subscribe_resources();
    loop {
        if resources
            .borrow()
            .channels
            .iter()
            .any(|c| c.id.as_deref().and_then(|s| s.parse().ok()) == Some(channel))
        {
            break;
        }
        resources.changed().await?;
    }
    Ok((session, channel))
}
fn row(index: u128, channel: Uuid) -> SearchResult {
    SearchResult {
        r#type: Some("message".into()),
        id: Some(Uuid::from_u128(index + 10000).to_string()),
        channel_id: Some(channel.to_string()),
        created_at: Some("2026-01-01T01:00:00.123456+01:00".into()),
        ..Default::default()
    }
}
#[test]
fn exact_search_pages_stay_detached_and_navigation_keeps_original_identity() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, channel) = setup(&f).await?;
        let rows = (1..=201).map(|i| row(i, channel)).collect();
        f.server()
            .set_search_results(session.account().user(), rows)
            .await?;
        let before = session.subscribe_reading().borrow().clone();
        let request = session.search(SearchQuery {
            contains_attachment: Some(false),
            ..Default::default()
        })?;
        let first = request.page(None).await?;
        assert_eq!(first.hits.len(), 100);
        let cursor = first.next.clone().ok_or("cursor")?;
        assert_eq!(cursor.created_at, "2026-01-01T01:00:00.123456+01:00");
        let second = request.page(Some(cursor)).await?;
        let third = request.page(second.next.clone()).await?;
        assert_eq!(third.hits.len(), 1);
        assert!(third.next.is_none());
        assert_ne!(first.hits[99].result.id, second.hits[0].result.id);
        let target = session.search_target(&first.hits[0])?;
        assert_eq!(target.account, session.account());
        assert_eq!(target.channel, channel);
        assert_eq!(session.subscribe_reading().borrow().version, before.version);
        let observations = f.server().take_search_requests().await;
        assert_eq!(observations.len(), 3);
        assert!(
            observations
                .iter()
                .all(|(_, request, _)| request.channel_id.is_none()
                    && request.contains_attachment == Some(false))
        );
        assert!(observations[1].2.contains("last_id="));
        assert!(observations[1].2.contains("%2B01%3A00"));
        session.reconnect()?;
        assert!(session.search_target(&first.hits[0]).is_err());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
#[test]
fn canceled_query_and_replaced_account_cannot_publish_held_search_results() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, channel) = setup(&f).await?;
        f.server()
            .set_search_results(session.account().user(), vec![row(1, channel)])
            .await?;
        let request = session.search(SearchQuery {
            text: Some(String::new()),
            channel: Some(channel),
            ..Default::default()
        })?;
        let mut hold = f.server().hold_next_rest("POST", "/search").await?;
        let copy = request.clone();
        let pending = tokio::spawn(async move { copy.page(None).await });
        hold.wait_started().await?;
        request.cancel();
        assert!(matches!(pending.await?, Err(SearchError::Canceled)));
        hold.release();
        let request = session.search(SearchQuery {
            channel: Some(channel),
            ..Default::default()
        })?;
        let mut hold = f.server().hold_next_rest("POST", "/search").await?;
        let pending = tokio::spawn(async move { request.page(None).await });
        hold.wait_started().await?;
        session.auth().logout().await?;
        hold.release();
        assert!(pending.await?.is_err());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
#[test]
fn full_detached_hydration_does_not_grant_coverage_or_advance_reads() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, channel) = setup(&f).await?;
        let result = row(1, channel);
        f.server()
            .set_search_results(session.account().user(), vec![result.clone()])
            .await?;
        f.server()
            .seed_messages(
                &channel.to_string(),
                vec![Message {
                    id: result.id.clone(),
                    channel_id: Some(channel.to_string()),
                    author_id: Some(session.account().user().to_string()),
                    created_at: result.created_at.clone(),
                    content: Some(String::new()),
                    ..Default::default()
                }],
            )
            .await?;
        let before = session.subscribe_reading().borrow().version;
        let page = session
            .search(SearchQuery {
                channel: Some(channel),
                ..Default::default()
            })?
            .page(None)
            .await?;
        let hydration = session.hydrate_search_hit(&page.hits[0])?;
        let DetachedMessageUpdate::Ready(message) = hydration.next_batch().await? else {
            return Err("missing hydrated message".into());
        };
        assert_eq!(message.id, result.id);
        assert_eq!(session.subscribe_reading().borrow().version, before);
        let counts = f.server().read_request_counts().await;
        assert_eq!(counts.uploads, 0);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn search_results_are_not_persisted_and_old_handles_fail_after_restart() -> Result {
    let mut f = Fixture::new(papo_test_server::Config::default())?;
    let account = f.run(async {
        let (session, channel) = setup(&f).await?;
        f.server()
            .set_search_results(session.account().user(), vec![row(1, channel)])
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    f.reopen()?;
    f.run(async {
        let session = f.client().handle().session(account)?;
        fixture::online(&session).await?;
        let query = session.search(SearchQuery {
            contains_attachment: Some(false),
            ..Default::default()
        })?;
        assert_eq!(query.page(None).await?.hits.len(), 1);
        assert!(f.store.calls(papo_sdk::secret::test_store::Operation::Read) > 0);
        let connection = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
        assert_eq!(
            connection.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name LIKE '%search%'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            0
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn detached_hydration_resumes_after_four_pages_without_mutating_canonical_snapshot() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, channel) = setup(&f).await?;
        let messages = (1..=501)
            .map(|index| {
                let record = row(index, channel);
                Message {
                    id: record.id,
                    channel_id: record.channel_id,
                    created_at: record.created_at,
                    ..Message::default()
                }
            })
            .collect();
        f.server()
            .seed_messages(&channel.to_string(), messages)
            .await?;
        f.server()
            .set_search_results(session.account().user(), vec![row(50, channel)])
            .await?;
        let history = session.history(channel).await?;
        history
            .page(papo_sdk::HistoryQuery {
                limit: Some(10),
                order: papo_sdk::HistoryOrder::Asc,
                ..papo_sdk::HistoryQuery::default()
            })
            .await?;
        let before = history.subscribe().await?.snapshot;
        let hit = session
            .search(SearchQuery {
                channel: Some(channel),
                ..SearchQuery::default()
            })?
            .page(None)
            .await?
            .hits
            .remove(0);
        let hydration = session.hydrate_search_hit(&hit)?;
        assert!(matches!(
            hydration.next_batch().await?,
            DetachedMessageUpdate::Pending
        ));
        assert!(matches!(
            hydration.next_batch().await?,
            DetachedMessageUpdate::Ready(_)
        ));
        let after = history.subscribe().await?.snapshot;
        assert_eq!(before.version, after.version);
        assert_eq!(before.messages.len(), after.messages.len());
        assert_eq!(before.chunks.len(), after.chunks.len());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
