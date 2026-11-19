//! Badge publications follow account-local admitted access, quarantined uncertainty and fresh regrant incarnations.

use super::*;
use papo_api::wire::{
    Channel, ChannelPermissionEntry, ChannelPermissions, DirectConversation, Role, RolePermissions,
    UserSummary,
};
use papo_sdk::{HistoryError, RoleSummary, UserProfile, operations::MetadataScope};

fn restricted(name: &str, allowed: bool) -> Channel {
    Channel {
        id: Some(CHANNEL.into()),
        name: Some(name.into()),
        r#type: Some("text".into()),
        position: Some(0),
        permissions: Some(vec![ChannelPermissionEntry {
            role_id: Some(ROLE.into()),
            permissions: Some(ChannelPermissions {
                read_channel: Some(allowed),
                ..ChannelPermissions::default()
            }),
            ..ChannelPermissionEntry::default()
        }]),
        ..Channel::default()
    }
}

async fn absent(changes: &mut watch::Receiver<Arc<BadgesSnapshot>>, channel: Uuid) -> Result {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if !changes.borrow_and_update().channels.contains_key(&channel) {
                return Ok::<_, Box<dyn std::error::Error>>(());
            }
            changes.changed().await?;
        }
    })
    .await??;
    Ok(())
}

#[test]
fn account_settings_and_read_points_do_not_cross_accounts_on_the_same_server() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let first = account(&f, "badge-first").await?;
        let second = account(&f, "badge-second").await?;
        let mut first_changes = first.subscribe_badges();
        let mut second_changes = second.subscribe_badges();
        let mention = format!("@mention(<@{}>)", first.account().user());
        load_and_anchor(
            &f,
            &first,
            channel(),
            vec![
                message(1, channel(), AUTHOR.parse()?, "anchor"),
                message(2, channel(), AUTHOR.parse()?, &mention),
            ],
        )
        .await?;
        second
            .history(channel())
            .await?
            .page(HistoryQuery::default())
            .await?;
        read_through(&second, channel(), message_id(1)).await?;
        count(&mut first_changes, channel(), UnreadCount::Exact(1)).await?;
        let second_badge = badge(&mut second_changes, channel(), |value| {
            value.mention_count == UnreadCount::Exact(0) && value.has_unread == Some(true)
        })
        .await?;
        let mut settings = first.notifications().settings().await?;
        settings.muted.insert(channel());
        first.notifications().set_settings(settings).await?;
        count(&mut first_changes, channel(), UnreadCount::Exact(0)).await?;
        assert!(Arc::ptr_eq(
            &second_badge,
            &second_changes.borrow().channels[&channel()]
        ));
        read_through(&first, channel(), message_id(2)).await?;
        assert_eq!(
            second
                .reading_state(channel())
                .unwrap()
                .local_point
                .unwrap()
                .message_id,
            message_id(1)
        );
        assert_eq!(
            second_changes.borrow().channels[&channel()].has_unread,
            Some(true)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn conflicting_resource_quarantine_preserves_alternative_trigger_eligibility() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-quarantine").await?;
        let peer = account(&f, "badge-peer").await?;
        let mut changes = session.subscribe_badges();
        load_and_anchor(
            &f,
            &session,
            channel(),
            vec![
                message(1, channel(), AUTHOR.parse()?, "anchor"),
                message(
                    2,
                    channel(),
                    AUTHOR.parse()?,
                    &format!("@mention(<@{}>)", session.account().user()),
                ),
            ],
        )
        .await?;
        count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        let mut settings = session.notifications().settings().await?;
        settings.direct_messages = false;
        settings.mentions = true;
        session.notifications().set_settings(settings).await?;
        f.server()
            .set_direct_conversations(
                &session.account().user().to_string(),
                vec![DirectConversation {
                    id: CHANNEL.into(),
                    user: UserSummary {
                        id: Some(peer.account().user().to_string()),
                        username: Some("peer".into()),
                        ..UserSummary::default()
                    },
                    ..DirectConversation::default()
                }],
            )
            .await?;
        session
            .operations()
            .refresh_metadata(MetadataScope::Social)
            .await?;
        assert!(matches!(
            session.history(channel()).await,
            Err(HistoryError::AccessUnknown)
        ));
        let unknown = badge(&mut changes, channel(), |value| value.has_unread.is_none()).await?;
        assert!(matches!(unknown.mention_count, UnreadCount::AtLeast(_)));
        f.server()
            .set_direct_conversations(&session.account().user().to_string(), Vec::new())
            .await?;
        session
            .operations()
            .refresh_metadata(MetadataScope::Social)
            .await?;
        count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn accepted_denial_removes_badges_and_regrant_never_reuses_old_exact_count() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-denial").await?;
        f.server()
            .set_roles(vec![Role {
                id: Some(ROLE.into()),
                permissions: Some(RolePermissions::default()),
                ..Role::default()
            }])
            .await;
        f.server()
            .set_profile(UserProfile {
                id: Some(session.account().user().to_string()),
                username: Some("badge-denial".into()),
                roles: Some(vec![RoleSummary {
                    id: Some(ROLE.into()),
                    ..RoleSummary::default()
                }]),
                ..UserProfile::default()
            })
            .await?;
        f.server().set_channel(restricted("allowed", true)).await?;
        session
            .operations()
            .refresh_metadata(MetadataScope::Roles)
            .await?;
        session
            .operations()
            .refresh_metadata(MetadataScope::Profile(session.account().user()))
            .await?;
        session
            .operations()
            .refresh_metadata(MetadataScope::Channels)
            .await?;
        let mut changes = session.subscribe_badges();
        load_and_anchor(
            &f,
            &session,
            channel(),
            vec![
                message(1, channel(), AUTHOR.parse()?, "anchor"),
                message(
                    2,
                    channel(),
                    AUTHOR.parse()?,
                    &format!("@mention(<@{}>)", session.account().user()),
                ),
            ],
        )
        .await?;
        count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        event(&session, "channel_update", async {
            f.server().set_channel(restricted("denied", false)).await?;
            Ok(())
        })
        .await?;
        absent(&mut changes, channel()).await?;
        assert!(session.history(channel()).await.is_err());
        event(&session, "channel_update", async {
            f.server()
                .set_channel(restricted("regranted", true))
                .await?;
            Ok(())
        })
        .await?;
        let fresh = session
            .history(channel())
            .await?
            .subscribe()
            .await?
            .snapshot;
        assert!(fresh.messages.is_empty());
        count(&mut changes, channel(), UnreadCount::AtLeast(0)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn held_cold_bootstrap_does_not_turn_empty_cached_census_into_complete_absence() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let client = f.client().handle();
        let server = client
            .add_server(&f.server().base_url(), UnixMicros(0))
            .await?;
        let auth = client.auth(server)?;
        register(&auth, "badge-bootstrap").await?;
        let mut held = f.server().hold_next_rest("GET", "/channels").await?;
        let session = login(&auth, "badge-bootstrap").await?;
        held.wait_started().await?;
        let resources = session.subscribe_resources();
        assert!(resources.borrow().version > 0);
        assert!(!resources.borrow().channel_census_complete);
        let mut changes = session.subscribe_badges();
        loop {
            if changes.borrow_and_update().version > 0 {
                break;
            }
            changes.changed().await?;
        }
        let incomplete = changes.borrow().clone();
        assert!(!incomplete.channels_complete);
        assert!(incomplete.channels.is_empty());
        held.release();
        fixture::online(&session).await?;
        loop {
            if changes.borrow_and_update().channels_complete {
                break;
            }
            changes.changed().await?;
        }
        let complete = changes.borrow();
        assert!(complete.version > incomplete.version);
        assert!(complete.channels.is_empty());
        assert!(resources.borrow().channel_census_complete);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
