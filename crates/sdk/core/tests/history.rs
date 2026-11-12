//! Exercises public account-scoped history over loopback TCP; pure model/storage invariants remain beside implementation.

#![cfg(feature = "test-support")]

use papo_api::{HistoryOrder, HistoryQuery, Uuid};
use papo_sdk::{ConnectionState, HistoryHandle, HistorySource, Session, SessionEvents, UnixMicros};
use papo_test_server::{Config, HistorySeed};
use std::collections::BTreeSet;

#[path = "history_cases/access.rs"]
mod access;
mod fixture;
#[path = "history_cases/held_page.rs"]
mod held_page;
#[path = "history_cases/held_pins.rs"]
mod held_pins;
#[path = "history_cases/live.rs"]
mod live;
#[path = "history_cases/reconnect.rs"]
mod reconnect;
#[path = "history_cases/routing.rs"]
mod routing;
use fixture::{Fixture, Result, login, register, wait_for};

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const FIRST: &str = "70000000-0000-4000-8000-000000000001";
const AT: &str = "2026-01-01T01:00:00.123456+01:00";

fn recipe(count: usize) -> HistorySeed {
    HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: "20000000-0000-4000-8000-000000000001".into(),
        first_message_id: FIRST.into(),
        count,
        first_created_at: AT.into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 7,
    }
}

fn config(count: usize) -> Config {
    Config {
        history: vec![recipe(count)],
        ..Config::default()
    }
}
fn channel() -> Uuid {
    Uuid::parse_str(CHANNEL).unwrap()
}
fn query(order: HistoryOrder) -> HistoryQuery<'static> {
    HistoryQuery {
        order,
        limit: Some(100),
        ..HistoryQuery::default()
    }
}

async fn online(session: &Session) -> Result<papo_sdk::Generation> {
    let mut status = session.subscribe_connection();
    Ok(wait_for(&mut status, |status| {
        status.state == ConnectionState::Online && status.generation == session.generation()
    })
    .await?
    .generation)
}

async fn signed_in(fixture: &Fixture, username: &str) -> Result<Session> {
    let handle = fixture.client().handle();
    let server = handle
        .add_server(&fixture.server().base_url(), UnixMicros(0))
        .await?;
    let auth = handle.auth(server)?;
    register(&auth, username).await?;
    let session = login(&auth, username).await?;
    fixture::online(&session).await?;
    online(&session).await?;
    Ok(session)
}

async fn wait_event(events: &mut SessionEvents, kind: &str) -> Result {
    loop {
        if events.next().await?.value.kind() == kind {
            return Ok(());
        }
    }
}

async fn disconnect(fixture: &Fixture, session: &Session) -> Result {
    fixture.server().set_gateway_accepting(false).await;
    let mut status = session.subscribe_connection();
    fixture.server().disconnect_gateways();
    wait_for(&mut status, |status| {
        matches!(status.state, ConnectionState::Backoff { .. })
    })
    .await?;
    Ok(())
}

async fn traverse(
    history: &HistoryHandle,
    order: HistoryOrder,
    source: Option<HistorySource>,
) -> Result<Vec<String>> {
    let mut cursor = None;
    let mut observed = Vec::new();
    let mut seen = BTreeSet::new();
    loop {
        let page = history
            .page(HistoryQuery {
                cursor: cursor.as_ref(),
                order,
                limit: Some(37),
            })
            .await?;
        if let Some(source) = source {
            assert_eq!(page.source, source);
        }
        if let Some(gap) = &page.gap {
            assert!(page.has_more);
            assert_eq!(gap.order, order);
            assert!(page.next_cursor.as_ref() == Some(&gap.cursor));
            assert_ne!(source, Some(HistorySource::Cache));
        }
        let more = page.has_more;
        let next = page.next_cursor.clone();
        for message in page.messages {
            let id = message
                .id
                .clone()
                .ok_or("missing fixture message identity")?;
            assert!(seen.insert(id.clone()));
            observed.push(id);
        }
        if !more {
            return Ok(observed);
        }
        assert!(next.is_some() && next != cursor);
        cursor = next;
    }
}

#[test]
fn exact_microsecond_cursors_page_both_directions_then_cache_offline_without_network() -> Result {
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture, "history-pages").await?;
        // Isolate explicit paging from the automatic unread-channel reconnect fetch.
        fixture.server().seed_history(recipe(205)).await?;
        let history = session.history(channel()).await?;
        assert_eq!(history.account(), session.account());
        assert_eq!(history.channel(), channel());
        for order in [HistoryOrder::Desc, HistoryOrder::Asc] {
            let page = history
                .page(HistoryQuery {
                    order,
                    limit: Some(7),
                    ..HistoryQuery::default()
                })
                .await?;
            assert_eq!(page.source, HistorySource::Server);
            assert!(!page.stale);
            assert_eq!(page.generation, session.generation());
            if order == HistoryOrder::Asc {
                assert_eq!(page.messages[0].cursor().unwrap().created_at, AT);
            }
        }
        let descending = traverse(&history, HistoryOrder::Desc, None).await?;
        let ascending = traverse(&history, HistoryOrder::Asc, Some(HistorySource::Cache)).await?;
        let expected: Vec<_> = recipe(205)
            .messages()?
            .into_iter()
            .map(|message| message.id.unwrap())
            .collect();
        assert_eq!(ascending, expected);
        assert_eq!(descending, expected.into_iter().rev().collect::<Vec<_>>());
        let mut held = fixture
            .server()
            .hold_next_rest("GET", &format!("/channels/{CHANNEL}/messages"))
            .await?;
        assert_eq!(
            history.page(query(HistoryOrder::Desc)).await?.source,
            HistorySource::Cache
        );
        disconnect(&fixture, &session).await?;
        assert_eq!(
            traverse(&history, HistoryOrder::Desc, Some(HistorySource::Cache)).await?,
            descending
        );
        assert_eq!(
            traverse(&history, HistoryOrder::Asc, Some(HistorySource::Cache)).await?,
            ascending
        );
        // If any cache-first/offline call attempted HTTP, this hold would have started.
        let started = tokio::time::timeout(std::time::Duration::ZERO, held.wait_started()).await;
        assert!(started.is_err());
        held.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn chronological_cache_and_detached_pin_payloads_survive_clean_client_restart() -> Result {
    use papo_sdk::secret::SecretKey;
    let mut fixture = Fixture::new(config(45))?;
    let account = fixture.run(async {
        let session = signed_in(&fixture, "history-restore").await?;
        fixture.server().set_message_pin(FIRST, Some(AT)).await?;
        let history = session.history(channel()).await?;
        assert_eq!(
            traverse(&history, HistoryOrder::Desc, None).await?.len(),
            45
        );
        assert_eq!(history.pins().await?.value.len(), 1);
        let saved = fixture
            .client()
            .handle()
            .subscribe_registry()
            .borrow()
            .accounts
            .iter()
            .find(|account| account.id == session.account())
            .cloned()
            .ok_or("saved account missing")?;
        assert!(
            fixture
                .store
                .snapshot(SecretKey::new(saved.id, saved.credential_generation))
                .is_some()
        );
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    fixture.close_client()?;
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        online(&session).await?;
        let history = session.history(channel()).await?;
        assert_eq!(
            history
                .page(query(HistoryOrder::Desc))
                .await?
                .messages
                .len(),
            45
        );
        assert_eq!(history.pins().await?.value.len(), 1);
        disconnect(&fixture, &session).await?;
        assert_eq!(
            traverse(&history, HistoryOrder::Asc, Some(HistorySource::Cache))
                .await?
                .len(),
            45
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn two_accounts_keep_own_reactions_and_pins_independent_without_cross_account_cache_reads() -> Result
{
    use papo_api::wire::{MessageReactionSummary, MessageUserReaction};
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let first = signed_in(&fixture, "history-one").await?;
        let handle = fixture.client().handle();
        let auth = handle.auth(first.account().server())?;
        register(&auth, "history-two").await?;
        let second = login(&auth, "history-two").await?;
        online(&second).await?;
        // Both account caches must fetch after their account-local reaction fixtures are set.
        fixture.server().seed_history(recipe(12)).await?;
        let unicode = char::from_u32(0x1f44d)
            .ok_or("invalid fixture scalar")?
            .to_string();
        fixture
            .server()
            .set_message_reaction(
                FIRST,
                MessageReactionSummary {
                    unicode: Some(unicode.clone()),
                    count: Some(1),
                    ..MessageReactionSummary::default()
                },
            )
            .await?;
        fixture
            .server()
            .set_user_reactions(
                &first.account().user().to_string(),
                FIRST,
                vec![MessageUserReaction {
                    id: Some("80000000-0000-4000-8000-000000000001".into()),
                    unicode: Some(unicode),
                    ..MessageUserReaction::default()
                }],
            )
            .await?;
        fixture.server().set_message_pin(FIRST, Some(AT)).await?;
        let first_history = first.history(channel()).await?;
        let second_history = second.history(channel()).await?;
        first_history.page(query(HistoryOrder::Desc)).await?;
        second_history.page(query(HistoryOrder::Desc)).await?;
        let own = first_history.page(query(HistoryOrder::Asc)).await?;
        let other = second_history.page(query(HistoryOrder::Asc)).await?;
        assert_eq!(
            own.messages[0].user_reactions.as_ref().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            other.messages[0].user_reactions.as_ref().map(Vec::len),
            Some(0)
        );
        assert_eq!(
            first_history.pins().await?.value[0]
                .user_reactions
                .as_ref()
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            second_history.pins().await?.value[0]
                .user_reactions
                .as_ref()
                .map(Vec::len),
            Some(0)
        );
        first.auth().logout().await?;
        assert!(matches!(
            first_history.page(query(HistoryOrder::Desc)).await,
            Err(papo_sdk::HistoryError::Closed(
                papo_sdk::HistoryClosedReason::AccountClosed
            ))
        ));
        assert_eq!(
            second_history
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
