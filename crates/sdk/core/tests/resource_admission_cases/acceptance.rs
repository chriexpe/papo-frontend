//! Valid input keeps working under admission: an explicit `[]` is a real removal on every
//! producer, a backend-reachable non-canonical overwrite is kept verbatim, and a malformed
//! display-only row or label is dropped without rejecting its group.

use super::{Account, CHANNEL, Result, SEEDED_MESSAGES, online_at};
use papo_api::{
    Uuid,
    gateway::{ChannelUpdateEvent, KnownEvent, UserJoinEvent},
    wire::{DirectConversation, UserSummary},
};
use papo_sdk::{HistoryError, RefreshStatus, ResourceSnapshot, operations::MetadataScope};
use serde_json::json;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
enum Producer {
    Reconnect,
    Gateway,
    Operation,
}

async fn published(
    account: &Account<'_>,
    predicate: impl Fn(&ResourceSnapshot) -> bool,
) -> Result<Arc<ResourceSnapshot>> {
    let mut resources = account.session.subscribe_resources();
    loop {
        let snapshot = resources.borrow_and_update().clone();
        if predicate(&snapshot) {
            return Ok(snapshot);
        }
        resources.changed().await?;
    }
}

/// Reconnects against the real fixture state and waits for its full image.
async fn restore(account: &Account<'_>) -> Result {
    let before = account.status().generation;
    account.session.reconnect()?;
    online_at(&account.session, |status| {
        status.generation.get() > before.get()
    })
    .await?;
    published(account, |snapshot| {
        snapshot
            .channels
            .iter()
            .any(|channel| channel.id.as_deref() == Some(CHANNEL))
    })
    .await?;
    Ok(())
}

/// Makes `producer` refetch the channel list, consuming one scripted reply.
async fn trigger(account: &Account<'_>, producer: Producer) -> Result {
    match producer {
        Producer::Reconnect => account.session.reconnect()?,
        Producer::Gateway => {
            account
                .server()
                .emit_event(KnownEvent::ChannelUpdate(Box::new(ChannelUpdateEvent {
                    channel_id: CHANNEL.into(),
                    name: "history".into(),
                    ..ChannelUpdateEvent::default()
                })))
                .await?;
        }
        Producer::Operation => {
            let refreshed = account
                .session
                .operations()
                .refresh_metadata(MetadataScope::Channels)
                .await?;
            assert_eq!(refreshed.value, RefreshStatus::Updated, "{producer:?}");
        }
    }
    Ok(())
}

#[test]
fn an_explicit_empty_channel_list_removes_the_channel_on_every_producer() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-empty").await?;
        let channel = Uuid::parse_str(CHANNEL)?;
        for producer in [Producer::Reconnect, Producer::Gateway, Producer::Operation] {
            let status = account.status();
            account
                .script("GET", "/channels", json!({"channels": []}))
                .await?;
            trigger(&account, producer).await?;
            published(&account, |snapshot| snapshot.channels.is_empty()).await?;
            assert!(
                matches!(
                    account.session.history(channel).await,
                    Err(HistoryError::Unverified)
                ),
                "{producer:?}: removed channel still readable"
            );
            let online = online_at(&account.session, |_| true).await?;
            match producer {
                Producer::Reconnect => assert!(online.generation.get() > status.generation.get()),
                Producer::Gateway | Producer::Operation => assert_eq!(online, status),
            }
            assert_eq!(
                account.session.resource_admission().rejected_replacements,
                0
            );
            restore(&account).await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn an_explicit_empty_dm_list_removes_a_listed_conversation() -> Result {
    let fixture = super::Fixture::new(super::config()?)?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-dm").await?;
        let dm = DirectConversation {
            id: Uuid::from_u128(41).to_string(),
            user: UserSummary {
                id: Some(Uuid::from_u128(42).to_string()),
                ..UserSummary::default()
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            ..DirectConversation::default()
        };
        account
            .server()
            .set_direct_conversations(&account.own().to_string(), vec![dm])
            .await?;
        restore(&account).await?;
        published(&account, |snapshot| snapshot.dms.len() == 1).await?;
        account.script("GET", "/dms", json!({"dms": []})).await?;
        let refreshed = account
            .session
            .operations()
            .refresh_metadata(MetadataScope::Social)
            .await?;
        assert_eq!(refreshed.value, RefreshStatus::Updated);
        assert!(account.snapshot().dms.is_empty());
        assert_eq!(
            account.session.resource_admission().rejected_replacements,
            0
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_malformed_display_only_member_row_is_dropped_and_the_roster_admitted() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-display").await?;
        let own = account.own().to_string();
        let own_row = account
            .snapshot()
            .members
            .iter()
            .find(|member| member.id.as_deref() == Some(own.as_str()))
            .map(serde_json::to_value)
            .ok_or("own row")??;
        let other = Uuid::from_u128(77).to_string();
        let labelled = Uuid::from_u128(78).to_string();
        let role = Uuid::from_u128(79).to_string();
        let page = json!({"has_more": false, "users": [
            own_row,
            {"id": "", "username": "unattributable", "roles": []},
            {"id": other, "username": "other", "roles": []},
            {"id": labelled, "username": "labelled", "roles": [{"id": "label"}, {"id": role}]},
        ]});
        account.script("GET", "/users", page).await?;
        let before = account.status().generation;
        let dropped = account.session.resource_admission().dropped_rows;
        account.session.reconnect()?;
        let online = online_at(&account.session, |status| {
            status.generation.get() > before.get()
        })
        .await?;
        let snapshot = published(&account, |snapshot| {
            snapshot.generation == online.generation
        })
        .await?;
        let ids: Vec<_> = snapshot
            .members
            .iter()
            .map(|member| member.id.clone())
            .collect();
        assert_eq!(ids, vec![Some(own), Some(other), Some(labelled)]);
        let labels: Vec<_> = snapshot.members[2]
            .roles
            .iter()
            .flatten()
            .map(|label| label.id.clone())
            .collect();
        assert_eq!(labels, vec![Some(role)]);
        let diagnostics = account.session.resource_admission();
        assert_eq!(diagnostics.dropped_rows, dropped + 2);
        assert_eq!(diagnostics.rejected_replacements, 0);
        assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        // One reconnect, straight to Online: no rejection and no backoff in between.
        assert_eq!(online.generation.get(), before.get() + 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_non_canonical_overwrite_role_id_the_backend_can_store_is_admitted_verbatim() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-verbatim").await?;
        // The backend keys the overwrite by the route text, which Postgres accepted.
        let role = Uuid::from_u128(0xab).to_string().to_uppercase();
        let list = json!({"channels": [{"id": CHANNEL, "name": "history", "type": "text",
            "permissions": [{"role_id": role, "permissions": {"read_channel": true}}]}]});
        for producer in [Producer::Reconnect, Producer::Gateway, Producer::Operation] {
            let status = account.status();
            account.script("GET", "/channels", list.clone()).await?;
            trigger(&account, producer).await?;
            let snapshot = published(&account, |snapshot| {
                snapshot.channels.iter().any(|channel| {
                    channel.permissions.as_ref().is_some_and(|entries| {
                        entries
                            .iter()
                            .any(|entry| entry.role_id.as_deref() == Some(role.as_str()))
                    })
                })
            })
            .await?;
            assert_eq!(snapshot.channels.len(), 1, "{producer:?}");
            let online = online_at(&account.session, |_| true).await?;
            if !matches!(producer, Producer::Reconnect) {
                assert_eq!(online, status, "{producer:?}: reconnected");
            }
            assert_eq!(
                account.session.resource_admission().rejected_replacements,
                0
            );
            restore(&account).await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_malformed_label_in_a_summary_batch_is_dropped_on_gateway_and_operation() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-labels").await?;
        let status = account.status();
        let role = Uuid::from_u128(79).to_string();
        for (index, producer) in [Producer::Gateway, Producer::Operation]
            .into_iter()
            .enumerate()
        {
            let user = Uuid::from_u128(80 + u128::try_from(index)?);
            let id = user.to_string();
            let batch = json!([{"id": id, "username": "labelled",
                "roles": [{"id": "label"}, {"id": role}]}]);
            let dropped = account.session.resource_admission().dropped_rows;
            account
                .script("POST", "/users/user_summary_batch", batch)
                .await?;
            match producer {
                Producer::Gateway => {
                    account
                        .server()
                        .emit_event(KnownEvent::UserJoin(Box::new(UserJoinEvent {
                            user_id: id.clone(),
                        })))
                        .await?;
                }
                Producer::Operation => {
                    let refreshed = account
                        .session
                        .operations()
                        .refresh_metadata(MetadataScope::Member(user))
                        .await?;
                    assert_eq!(refreshed.value, RefreshStatus::Updated);
                }
                Producer::Reconnect => unreachable!("the roster path has its own test"),
            }
            let snapshot = published(&account, |snapshot| {
                snapshot
                    .members
                    .iter()
                    .any(|member| member.id.as_deref() == Some(id.as_str()))
            })
            .await?;
            let member = snapshot
                .members
                .iter()
                .find(|member| member.id.as_deref() == Some(id.as_str()))
                .ok_or("member")?;
            let labels: Vec<_> = member
                .roles
                .iter()
                .flatten()
                .map(|label| label.id.clone())
                .collect();
            assert_eq!(labels, vec![Some(role.clone())], "{producer:?}");
            let diagnostics = account.session.resource_admission();
            assert_eq!(diagnostics.dropped_rows, dropped + 1, "{producer:?}");
            assert_eq!(diagnostics.rejected_replacements, 0);
            assert_eq!(account.status(), status, "{producer:?}: reconnected");
            assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_full_server_of_500_channels_reconnects_and_refreshes_on_every_producer() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        // The seeded history channel plus 499 more reach the backend's maximum; no list
        // is scripted, so every producer reads the fixture's real 500-row list.
        for index in 0..499 {
            fixture
                .server()
                .create_channel(&format!("full-{index}"))
                .await?;
        }
        let account = Account::start(&fixture, "adm-full").await?;
        assert_eq!(account.snapshot().channels.len(), 500);
        for producer in [Producer::Reconnect, Producer::Gateway, Producer::Operation] {
            let before = account.status().generation;
            let version = account.snapshot().version;
            trigger(&account, producer).await?;
            let online = online_at(&account.session, |status| match producer {
                Producer::Reconnect => status.generation.get() > before.get(),
                Producer::Gateway | Producer::Operation => status.generation == before,
            })
            .await?;
            let snapshot = published(&account, |snapshot| {
                // The producer's own publication, so the next one cannot overlap it.
                snapshot.version > version && snapshot.generation == online.generation
            })
            .await?;
            assert_eq!(snapshot.channels.len(), 500, "{producer:?}");
            assert_eq!(
                account.session.resource_admission().rejected_replacements,
                0
            );
        }
        assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
