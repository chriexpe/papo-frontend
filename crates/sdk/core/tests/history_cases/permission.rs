//! Cached history follows accepted read permission, not channel-list presence: role and overwrite
//! denial, unknown evidence, warm offline and every resource producer.
//! Fixture REST denial (`set_channel_readable`) is scripted separately; it is not the Go engine.

use super::{CHANNEL, Fixture, Result, channel, disconnect, login, online, register, wait_event};
use papo_api::{
    HistoryQuery, Uuid,
    wire::{ChannelPermissionEntry, ChannelPermissions, RolePermissions},
};
use papo_sdk::{
    Channel, HistoryClosedReason, HistoryError, HistoryHandle, HistorySource, HistoryUpdate,
    ReadingError, ReadingPosition, Role, RoleSummary, Session, UnixMicros, UserProfile,
    VisibilityContinuity, operations::MetadataScope,
};
use papo_test_server::{Config, HistorySeed};

pub(super) const ROLE: &str = "30000000-0000-4000-8000-000000000001";
pub(super) const DENIED: HistoryError = HistoryError::Closed(HistoryClosedReason::PermissionDenied);

pub(super) async fn account(fixture: &Fixture, username: &str) -> Result<Session> {
    let handle = fixture.client().handle();
    let address = fixture.server().base_url();
    let server = match handle.add_server(&address, UnixMicros(0)).await {
        Ok(server) => server,
        // A second account on the same server reuses its saved entry.
        Err(_) => handle
            .subscribe_registry()
            .borrow()
            .servers
            .first()
            .map(|server| server.address.id())
            .ok_or("saved server missing")?,
    };
    let auth = handle.auth(server)?;
    register(&auth, username).await?;
    let session = login(&auth, username).await?;
    online(&session).await?;
    Ok(session)
}

pub(super) fn profile(session: &Session, username: &str, assigned: bool) -> UserProfile {
    UserProfile {
        id: Some(session.account().user().to_string()),
        username: Some(username.into()),
        roles: Some(if assigned {
            vec![RoleSummary {
                id: Some(ROLE.into()),
                ..RoleSummary::default()
            }]
        } else {
            Vec::new()
        }),
        ..UserProfile::default()
    }
}

pub(super) fn restricted(name: &str, read: bool) -> Channel {
    Channel {
        id: Some(CHANNEL.into()),
        name: Some(name.into()),
        r#type: Some("text".into()),
        position: Some(0),
        permissions: Some(vec![ChannelPermissionEntry {
            role_id: Some(ROLE.into()),
            permissions: Some(ChannelPermissions {
                read_channel: Some(read),
                ..ChannelPermissions::default()
            }),
            ..ChannelPermissionEntry::default()
        }]),
        ..Channel::default()
    }
}

/// A role-restricted channel the given accounts can read, with 12 cached messages.
pub(super) async fn prepare(fixture: &Fixture, sessions: &[(&Session, &str)]) -> Result {
    fixture
        .server()
        .set_roles(vec![Role {
            id: Some(ROLE.into()),
            permissions: Some(RolePermissions::default()),
            ..Role::default()
        }])
        .await;
    for (session, username) in sessions {
        fixture
            .server()
            .set_profile(profile(session, username, true))
            .await?;
    }
    fixture
        .server()
        .set_channel(restricted("restricted", true))
        .await?;
    for (session, _) in sessions {
        session.reconnect()?;
        online(session).await?;
    }
    fixture
        .server()
        .seed_history(HistorySeed {
            channel_id: CHANNEL.into(),
            author_id: Uuid::nil().to_string(),
            first_message_id: "70000000-0000-4000-8000-000000000001".into(),
            count: 12,
            first_created_at: "2026-01-01T00:00:00Z".into(),
            timestamp_step_us: 1,
            equal_timestamp_count: 1,
        })
        .await?;
    Ok(())
}

pub(super) async fn warm(session: &Session) -> Result<HistoryHandle> {
    let history = session.history(channel()).await?;
    assert_eq!(
        history.page(HistoryQuery::default()).await?.messages.len(),
        12
    );
    Ok(history)
}

pub(super) fn same(left: &HistoryError, right: &HistoryError) -> bool {
    match (left, right) {
        (HistoryError::Closed(left), HistoryError::Closed(right)) => left == right,
        (HistoryError::AccessUnknown, HistoryError::AccessUnknown) => true,
        _ => false,
    }
}

async fn assert_handles_refused(
    session: &Session,
    history: &HistoryHandle,
    expected: &HistoryError,
) {
    let page = history.page(HistoryQuery::default()).await;
    assert!(matches!(&page, Err(error) if same(error, expected)));
    let opened = session.history(channel()).await;
    assert!(matches!(&opened, Err(error) if same(error, expected)));
}

/// The old handle, a new open, a pre-existing observer and a saved position are all refused.
async fn assert_refused(
    session: &Session,
    history: &HistoryHandle,
    visible: (papo_sdk::Generation, u64, Uuid),
    expected: &HistoryError,
) -> Result {
    assert_handles_refused(session, history, expected).await;
    let report = session
        .reading_observer()
        .report_visible(
            channel(),
            visible.0,
            visible.1,
            &[visible.2],
            VisibilityContinuity::Discontinuous,
        )
        .await;
    assert!(
        matches!(&report, Err(ReadingError::History(error)) if same(error, expected)),
        "{report:?}"
    );
    let position = session
        .save_reading_position(
            channel(),
            Some(ReadingPosition {
                message_id: visible.2,
                offset_units: 0,
            }),
        )
        .await;
    assert!(matches!(&position, Err(ReadingError::History(error)) if same(error, expected)));
    Ok(())
}

#[derive(Clone, Copy)]
enum Loss {
    Role,
    Overwrite,
}

fn accepted_denial(loss: Loss) -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let session = account(&fixture, "access-denied").await?;
        prepare(&fixture, &[(&session, "access-denied")]).await?;
        let history = warm(&session).await?;
        let subscription = history.subscribe().await?;
        let snapshot = subscription.snapshot.clone();
        let visible = (
            snapshot.generation,
            snapshot.version,
            snapshot.messages[0].id.as_deref().unwrap().parse()?,
        );
        let mut events = session.subscribe_events();
        match loss {
            Loss::Role => {
                fixture
                    .server()
                    .set_profile(profile(&session, "access-denied", false))
                    .await?;
                wait_event(&mut events, "role_remove").await?;
            }
            Loss::Overwrite => {
                // Metadata changes too, so the pinned backend emits `channel_update`.
                fixture
                    .server()
                    .set_channel(restricted("changed", false))
                    .await?;
                wait_event(&mut events, "channel_update").await?;
            }
        }
        assert!(matches!(
            *subscription.updates.borrow(),
            HistoryUpdate::Closed(HistoryClosedReason::PermissionDenied)
        ));
        assert_refused(&session, &history, visible, &DENIED).await?;
        assert!(
            session
                .reading_state(channel())
                .and_then(|state| state.local_point)
                .is_none()
        );
        // Losing the gateway does not reopen anything sealed by accepted evidence.
        disconnect(&fixture, &session).await?;
        assert_handles_refused(&session, &history, &DENIED).await;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn role_removal_closes_old_new_offline_subscribed_and_reading_access() -> Result {
    accepted_denial(Loss::Role)
}

#[test]
fn overwrite_denial_closes_old_new_offline_subscribed_and_reading_access() -> Result {
    accepted_denial(Loss::Overwrite)
}

#[test]
fn unknown_overwrite_evidence_quarantines_and_resumes_the_same_cached_incarnation() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let session = account(&fixture, "access-unknown").await?;
        prepare(&fixture, &[(&session, "access-unknown")]).await?;
        let history = warm(&session).await?;
        let subscription = history.subscribe().await?;
        let snapshot = subscription.snapshot.clone();
        let visible = (
            snapshot.generation,
            snapshot.version,
            snapshot.messages[0].id.as_deref().unwrap().parse()?,
        );
        let mut events = session.subscribe_events();
        let mut incomplete = restricted("incomplete", true);
        incomplete.permissions = None;
        fixture.server().set_channel(incomplete).await?;
        wait_event(&mut events, "channel_update").await?;

        // Unknown is not a grant and not a removal: nothing new is delivered or admitted.
        assert!(!subscription.updates.has_changed()?);
        assert_refused(&session, &history, visible, &HistoryError::AccessUnknown).await?;
        // A live message during quarantine still reaches the model and cache, undelivered.
        fixture
            .server()
            .publish_message(papo_api::wire::Message {
                id: Some("70000000-0000-4000-8000-000000000100".into()),
                channel_id: Some(CHANNEL.into()),
                author_id: Some(Uuid::nil().to_string()),
                content: Some(String::new()),
                created_at: Some("2026-01-01T00:00:01Z".into()),
                ..papo_api::wire::Message::default()
            })
            .await?;
        wait_event(&mut events, "message").await?;
        assert!(!subscription.updates.has_changed()?);

        // Any server fetch would now be refused, so a successful page proves the
        // last-good cache was kept rather than refetched.
        fixture
            .server()
            .set_channel_readable(&session.account().user().to_string(), CHANNEL, false)
            .await?;
        fixture
            .server()
            .set_channel(restricted("restored", true))
            .await?;
        wait_event(&mut events, "channel_update").await?;
        let page = history.page(HistoryQuery::default()).await?;
        assert_eq!(page.source, HistorySource::Cache);
        assert_eq!(page.messages.len(), 13);
        // The same subscription resumes with a full replacement that includes the message.
        assert!(matches!(
            &*subscription.updates.borrow(),
            HistoryUpdate::Reset(snapshot) if snapshot.messages.len() == 13
        ));
        assert_eq!(history.subscribe().await?.snapshot.messages.len(), 13);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn transport_loss_without_invalidation_keeps_warm_offline_access() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let session = account(&fixture, "access-offline").await?;
        prepare(&fixture, &[(&session, "access-offline")]).await?;
        let history = warm(&session).await?;
        disconnect(&fixture, &session).await?;
        let page = history.page(HistoryQuery::default()).await?;
        assert_eq!(page.source, HistorySource::Cache);
        assert_eq!(page.messages.len(), 12);
        let opened = session.history(channel()).await?;
        assert_eq!(opened.subscribe().await?.snapshot.messages.len(), 12);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[derive(Clone, Copy)]
enum Producer {
    Gateway,
    Operation,
    Reconnect,
}

/// The same accepted overwrite denial reaches the same outcome through every producer.
fn produced_by(producer: Producer) -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let session = account(&fixture, "access-producer").await?;
        prepare(&fixture, &[(&session, "access-producer")]).await?;
        let history = warm(&session).await?;
        let subscription = history.subscribe().await?;
        let mut events = session.subscribe_events();
        match producer {
            Producer::Gateway => {
                fixture
                    .server()
                    .set_channel(restricted("changed", false))
                    .await?;
                wait_event(&mut events, "channel_update").await?;
            }
            // Overwrite-only changes have no pinned backend event.
            Producer::Operation => {
                fixture
                    .server()
                    .set_channel(restricted("restricted", false))
                    .await?;
                let refreshed = session
                    .operations()
                    .refresh_metadata(MetadataScope::Channels)
                    .await?;
                assert_eq!(refreshed.value, papo_sdk::RefreshStatus::Updated);
            }
            Producer::Reconnect => {
                fixture
                    .server()
                    .set_channel(restricted("restricted", false))
                    .await?;
                session.reconnect()?;
                online(&session).await?;
            }
        }
        assert!(matches!(
            *subscription.updates.borrow(),
            HistoryUpdate::Closed(HistoryClosedReason::PermissionDenied)
        ));
        let opened = session.history(channel()).await;
        assert!(matches!(&opened, Err(error) if same(error, &DENIED)));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn gateway_overwrite_denial_closes_cached_access() -> Result {
    produced_by(Producer::Gateway)
}

#[test]
fn operation_refresh_overwrite_denial_closes_cached_access() -> Result {
    produced_by(Producer::Operation)
}

#[test]
fn reconnect_overwrite_denial_closes_cached_access() -> Result {
    produced_by(Producer::Reconnect)
}
