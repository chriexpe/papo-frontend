//! Owns public history revocation/eviction proofs; leaves backend permission evaluation and cache schema outside.

use super::{
    CHANNEL, Fixture, Result, channel, config, disconnect, login, online, query, recipe, signed_in,
    wait_event,
};
use papo_api::{HistoryOrder, HistoryQuery, Uuid};
use papo_sdk::{HistoryClosedReason, HistoryError};
use papo_test_server::{Failure, RestStep};

#[test]
fn forbidden_history_fetch_permanently_closes_warm_handle_and_pin_snapshot() -> Result {
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-denied").await?;
        fixture.server().seed_history(recipe(25)).await?;
        let history = session.history(channel()).await?;
        let page = history
            .page(HistoryQuery {
                limit: Some(5),
                ..HistoryQuery::default()
            })
            .await?;
        let cursor = page.messages.last().unwrap().cursor().unwrap();
        fixture
            .server()
            .set_channel_readable(&session.account().user().to_string(), CHANNEL, false)
            .await?;
        assert!(matches!(
            history
                .page(HistoryQuery {
                    cursor: Some(&cursor),
                    limit: Some(5),
                    ..HistoryQuery::default()
                })
                .await,
            Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
        ));
        assert!(matches!(
            history.page(query(HistoryOrder::Desc)).await,
            Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
        ));
        assert!(matches!(
            history.pins().await,
            Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
        ));
        fixture
            .server()
            .set_channel_readable(&session.account().user().to_string(), CHANNEL, true)
            .await?;
        session.reconnect()?;
        online(&session).await?;
        let fresh = session.history(channel()).await?;
        assert_eq!(
            fresh.page(query(HistoryOrder::Desc)).await?.messages.len(),
            25
        );
        assert!(matches!(
            history.page(query(HistoryOrder::Desc)).await,
            Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn missing_history_page_is_typed_not_found_and_does_not_serve_old_cached_messages() -> Result {
    let path = format!("/channels/{CHANNEL}/messages");
    let mut scenario = config(0);
    scenario.rest = vec![
        RestStep {
            method: "GET".into(),
            path: path.clone(),
            delay_ms: 0,
            failure: None,
        },
        RestStep {
            method: "GET".into(),
            path,
            delay_ms: 0,
            failure: Some(Failure {
                status: 404,
                slug: "not-found".into(),
            }),
        },
    ];
    let fixture = Fixture::new(scenario)?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-missing").await?;
        fixture.server().seed_history(recipe(25)).await?;
        let history = session.history(channel()).await?;
        let first = history
            .page(HistoryQuery {
                limit: Some(5),
                ..HistoryQuery::default()
            })
            .await?;
        let cursor = first.messages.last().unwrap().cursor().unwrap();
        assert!(matches!(
            history
                .page(HistoryQuery {
                    cursor: Some(&cursor),
                    ..query(HistoryOrder::Desc)
                })
                .await,
            Err(HistoryError::Closed(HistoryClosedReason::NotFound))
        ));
        assert!(matches!(
            history.page(query(HistoryOrder::Desc)).await,
            Err(HistoryError::Closed(HistoryClosedReason::NotFound))
        ));
        assert!(matches!(
            history.pins().await,
            Err(HistoryError::Closed(HistoryClosedReason::NotFound))
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn live_channel_deletion_closes_warm_history_with_sticky_first_observed_reason() -> Result {
    let fixture = Fixture::new(config(12))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-channel").await?;
        assert!(matches!(
            session.history(Uuid::from_u128(3)).await,
            Err(HistoryError::Unverified)
        ));
        let history = session.history(channel()).await?;
        history.page(query(HistoryOrder::Desc)).await?;
        let mut events = session.subscribe_events();
        assert!(fixture.server().delete_channel(CHANNEL).await);
        wait_event(&mut events, "channel_delete").await?;
        let closed = history
            .page(query(HistoryOrder::Desc))
            .await
            .err()
            .ok_or("deleted channel returned cached history")?;
        assert!(
            matches!(
                closed,
                HistoryError::Closed(
                    HistoryClosedReason::ChannelDeleted | HistoryClosedReason::NotFound
                )
            ),
            "{closed:?}"
        );
        let HistoryError::Closed(reason) = closed else {
            unreachable!()
        };
        assert!(
            matches!(history.pins().await, Err(HistoryError::Closed(current)) if current == reason)
        );
        assert!(matches!(
            session.history(channel()).await,
            Err(HistoryError::Unverified)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn disconnected_channel_disappearance_closes_handle_after_fresh_list_authority() -> Result {
    let fixture = Fixture::new(config(12))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-vanished").await?;
        let history = session.history(channel()).await?;
        history.page(query(HistoryOrder::Desc)).await?;
        disconnect(&fixture, &session).await?;
        assert!(fixture.server().delete_channel(CHANNEL).await);
        fixture.server().set_gateway_accepting(true).await;
        session.reconnect()?;
        online(&session).await?;
        assert!(matches!(
            history.page(query(HistoryOrder::Desc)).await,
            Err(HistoryError::Closed(HistoryClosedReason::ChannelRemoved))
        ));
        assert!(matches!(
            history.pins().await,
            Err(HistoryError::Closed(HistoryClosedReason::ChannelRemoved))
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn account_replacement_never_retargets_existing_history_handle() -> Result {
    let fixture = Fixture::new(config(12))?;
    fixture.run(async {
        let first = signed_in(&fixture, "history-replace").await?;
        let old = first.history(channel()).await?;
        old.page(query(HistoryOrder::Desc)).await?;
        let auth = fixture.client().handle().auth(first.account().server())?;
        let current = login(&auth, "history-replace").await?;
        online(&current).await?;
        assert!(matches!(
            old.page(query(HistoryOrder::Desc)).await,
            Err(HistoryError::Closed(HistoryClosedReason::AccountClosed))
        ));
        assert_eq!(
            current
                .history(channel())
                .await?
                .page(query(HistoryOrder::Desc))
                .await?
                .messages
                .len(),
            12
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn live_channel_deletion_without_inflight_rest_has_deterministic_channel_deleted_reason() -> Result
{
    let fixture = Fixture::new(config(12))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-event").await?;
        let history = session.history(channel()).await?;
        let mut events = session.subscribe_events();
        assert!(fixture.server().delete_channel(CHANNEL).await);
        wait_event(&mut events, "channel_delete").await?;
        assert!(matches!(
            history.page(query(HistoryOrder::Desc)).await,
            Err(HistoryError::Closed(HistoryClosedReason::ChannelDeleted))
        ));
        assert!(matches!(
            session.history(channel()).await,
            Err(HistoryError::Unverified)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
