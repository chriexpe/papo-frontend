//! Interleavings at the real producer boundary, including malformed evidence that fails in HTTP decoding.

use super::{Account, CHANNEL, Result, SEEDED_MESSAGES};
use papo_api::{
    Uuid,
    gateway::{ChannelUpdateEvent, Event, KnownEvent},
};
use papo_sdk::{
    HistoryClosedReason, HistoryError, RefreshFault, RefreshStatus, operations::MetadataScope,
};
use serde_json::json;
use std::sync::Arc;

async fn refresh(account: &Account<'_>, scope: MetadataScope) -> Result<RefreshStatus> {
    Ok(account
        .session
        .operations()
        .refresh_metadata(scope)
        .await?
        .value)
}

async fn set_id(account: &Account<'_>) -> Result<i64> {
    let path = account.cache.clone();
    let channel = Uuid::parse_str(CHANNEL)?;
    Ok(tokio::task::spawn_blocking(move || {
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?
            .query_row(
                "SELECT current_set FROM history_channels WHERE channel_id=?1",
                [channel.as_bytes()],
                |row| row.get(0),
            )
    })
    .await??)
}

#[test]
fn equal_gateway_observation_fences_both_held_success_and_held_decode_rejection() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "admission-fence").await?;
        let original = account.snapshot();
        let channel = Uuid::parse_str(CHANNEL)?;
        for older in [json!({"channels":[]}), json!({"channels":[false]})] {
            account.script("GET", "/channels", older).await?;
            let mut hold = account.server().hold_next_rest("GET", "/channels").await?;
            let ops = account.session.operations();
            let previous =
                tokio::spawn(async move { ops.refresh_metadata(MetadataScope::Channels).await });
            hold.wait_started().await?;
            let mut events = account.session.subscribe_events();
            account
                .server()
                .emit_event(KnownEvent::ChannelUpdate(Box::new(ChannelUpdateEvent {
                    channel_id: CHANNEL.into(),
                    ..Default::default()
                })))
                .await?;
            while !matches!(
                events.next().await?.value,
                Event::Known(KnownEvent::ChannelUpdate(_))
            ) {}
            assert!(
                Arc::ptr_eq(&original, &account.snapshot()),
                "equal observations must not publish"
            );
            hold.release();
            assert_eq!(
                previous.await??.value,
                RefreshStatus::Warning(RefreshFault::ConcurrentChange)
            );
            assert!(Arc::ptr_eq(&original, &account.snapshot()));
            account.session.history(channel).await?;
        }
        assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn channel_decode_failure_quarantines_until_equal_readback_resumes_the_same_history() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-quarantine").await?;
        let original = account.snapshot();
        let channel = Uuid::parse_str(CHANNEL)?;
        let set = set_id(&account).await?;
        let row = original.channels.get_arc(channel).ok_or("channel")?;
        account
            .script("GET", "/channels", json!({"channels":[false]}))
            .await?;
        assert_eq!(
            refresh(&account, MetadataScope::Channels).await?,
            RefreshStatus::Warning(RefreshFault::Request)
        );
        assert!(matches!(
            account.session.history(channel).await,
            Err(HistoryError::AccessUnknown)
        ));
        assert!(!account.snapshot().channel_census_complete);
        assert_eq!(set_id(&account).await?, set);
        assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        assert_eq!(
            refresh(&account, MetadataScope::Roles).await?,
            RefreshStatus::Updated
        );
        assert!(matches!(
            account.session.history(channel).await,
            Err(HistoryError::AccessUnknown)
        ));
        assert_eq!(
            refresh(&account, MetadataScope::Channels).await?,
            RefreshStatus::Updated
        );
        account.session.history(channel).await?;
        let restored = account.snapshot();
        assert!(restored.channel_census_complete);
        assert!(Arc::ptr_eq(
            &row,
            &restored.channels.get_arc(channel).ok_or("channel")?
        ));
        assert_eq!(set_id(&account).await?, set);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

async fn prepare_assigned_nonowner(account: &Account<'_>, role: Uuid) -> Result {
    let own = account.own();
    account
        .script(
            "GET",
            "/server",
            json!({"id":Uuid::from_u128(1),"owner_id":null}),
        )
        .await?;
    assert_eq!(
        refresh(account, MetadataScope::Server).await?,
        RefreshStatus::Updated
    );
    account
        .script(
            "GET",
            "/roles",
            json!({"roles":[{"id":role,"permissions":{}}]}),
        )
        .await?;
    assert_eq!(
        refresh(account, MetadataScope::Roles).await?,
        RefreshStatus::Updated
    );
    account
        .script(
            "POST",
            "/users/user_summary_batch",
            json!([{"id":own,"roles":[{"id":role}]}]),
        )
        .await?;
    assert_eq!(
        refresh(account, MetadataScope::Member(own)).await?,
        RefreshStatus::Updated
    );
    Ok(())
}

#[test]
fn noncanonical_role_delete_keeps_surviving_canonical_overwrite_and_requires_channel_readback_for_denial()
-> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account = Account::start(&fixture, "adm-delete").await?;
        let channel = Uuid::parse_str(CHANNEL)?;
        let role = Uuid::from_u128(0xabcd);
        let noncanonical = role.simple().to_string().to_uppercase();
        assert_eq!(noncanonical.parse::<Uuid>()?, role);
        assert_ne!(noncanonical, role.to_string());
        // Model the backend's UUID-equality role DELETE and its exact-text JSONB key DELETE.
        // The supplied path spelling misses this canonical key.
        let mut overwrites =
            std::collections::BTreeMap::from([(role.to_string(), json!({"read_channel":true}))]);
        overwrites.remove(&noncanonical);
        let surviving: Vec<_> = overwrites
            .into_iter()
            .map(|(role_id, permissions)| json!({"role_id":role_id,"permissions":permissions}))
            .collect();
        let restricted = json!({"channels":[{"id":CHANNEL,"type":"text","permissions":surviving}]});
        prepare_assigned_nonowner(&account, role).await?;
        account
            .script("GET", "/channels", restricted.clone())
            .await?;
        assert_eq!(
            refresh(&account, MetadataScope::Channels).await?,
            RefreshStatus::Updated
        );
        account.session.history(channel).await?;
        let set = set_id(&account).await?;
        account.script("GET", "/roles", json!({"roles":[]})).await?;
        account
            .server()
            .queue_operation_response("GET", "/channels", 503, json!({}))
            .await?;
        let mut held = account.server().hold_next_rest("GET", "/channels").await?;
        let ops = account.session.operations();
        let update = tokio::spawn(async move { ops.refresh_metadata(MetadataScope::Roles).await });
        held.wait_started().await?;
        assert!(matches!(
            account.session.history(channel).await,
            Err(HistoryError::AccessUnknown)
        ));
        assert_eq!(set_id(&account).await?, set);
        held.release();
        assert_eq!(
            update.await??.value,
            RefreshStatus::Warning(RefreshFault::Request)
        );
        assert!(matches!(
            account.session.history(channel).await,
            Err(HistoryError::AccessUnknown)
        ));
        assert_eq!(set_id(&account).await?, set);
        assert_eq!(account.cached_messages().await?, SEEDED_MESSAGES);
        // Channel-only evidence cannot revive the assignment from a rejected multi-group read.
        account
            .script("GET", "/channels", restricted.clone())
            .await?;
        assert_eq!(
            refresh(&account, MetadataScope::Channels).await?,
            RefreshStatus::Updated
        );
        assert!(matches!(
            account.session.history(channel).await,
            Err(HistoryError::AccessUnknown)
        ));
        account.script("GET", "/roles", json!({"roles":[]})).await?;
        account.script("GET", "/channels", restricted).await?;
        assert_eq!(
            refresh(&account, MetadataScope::Roles).await?,
            RefreshStatus::Updated
        );
        let result = account.snapshot();
        assert!(result.roles.get(role).is_none());
        let retained = &result.channels.get(channel).ok_or("channel")?.permissions;
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].key.text(), role.to_string());
        assert!(matches!(
            account.session.history(channel).await,
            Err(HistoryError::Closed(HistoryClosedReason::PermissionDenied))
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn observed_dm_omission_seals_before_block_read_and_profile_data_cannot_restore_access() -> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account=Account::start(&fixture,"adm-social").await?;
        let dm=Uuid::from_u128(104);
        let peer=Uuid::from_u128(105);
        let listed=json!({"dms":[{"id":dm,"user":{"id":peer,"username":"peer"},"created_at":"2026-01-01T00:00:00Z","unread_count":0}]});
        account.script("GET","/dms",listed.clone()).await?;
        assert_eq!(refresh(&account,MetadataScope::Social).await?,RefreshStatus::Updated);
        account.session.history(dm).await?;
        account.script("GET","/dms",json!({"dms":[]})).await?;
        account.server().queue_operation_response("GET","/users/blocks",503,json!({})).await?;
        let mut hold=account.server().hold_next_rest("GET","/users/blocks").await?;
        let ops=account.session.operations();
        let update=tokio::spawn(async move {ops.refresh_metadata(MetadataScope::Social).await});
        hold.wait_started().await?;
        assert!(matches!(account.session.history(dm).await,Err(HistoryError::AccessUnknown)));
        assert!(account.snapshot().dms.get(dm).is_some());
        assert!(!account.snapshot().channel_census_complete);
        account.session.history(Uuid::parse_str(CHANNEL)?).await?;
        hold.release();
        assert_eq!(update.await??.value,RefreshStatus::Warning(RefreshFault::Request));
        assert!(matches!(account.session.history(dm).await,Err(HistoryError::AccessUnknown)));
        account.script("GET","/dms",listed.clone()).await?;
        assert_eq!(refresh(&account,MetadataScope::Social).await?,RefreshStatus::Updated);
        account.session.history(dm).await?;
        account.script("POST","/users/user_summary_batch",json!([])).await?;
        assert_eq!(refresh(&account,MetadataScope::Member(peer)).await?,RefreshStatus::Updated);
        assert!(matches!(account.session.history(dm).await,Err(HistoryError::AccessUnknown)));
        account.script("POST","/users/profile_batch",json!({"profiles":[{"id":peer,"username":"peer"}]})).await?;
        account.session.profiles(&[peer]).await?;
        assert!(account.snapshot().users.get(peer).is_some());
        assert!(!account.snapshot().members.contains(peer));
        assert!(matches!(account.session.history(dm).await,Err(HistoryError::AccessUnknown)));
        account.script("GET","/dms",listed).await?;
        assert_eq!(refresh(&account,MetadataScope::Social).await?,RefreshStatus::Updated);
        account.session.history(dm).await?;
        Ok::<_,Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn reconnect_quarantines_a_loss_that_needs_both_the_new_overwrite_and_the_new_own_assignment()
-> Result {
    let fixture = super::fixture()?;
    fixture.run(async {
        let account=Account::start(&fixture,"adm-compound").await?;
        let a=Uuid::from_u128(110);
        let b=Uuid::from_u128(111);
        let channel=Uuid::parse_str(CHANNEL)?;
        prepare_assigned_nonowner(&account,a).await?;
        let definitions=json!({"roles":[{"id":a,"permissions":{}},{"id":b,"permissions":{}}]});
        account.script("GET","/roles", definitions.clone()).await?;
        assert_eq!(refresh(&account,MetadataScope::Roles).await?,RefreshStatus::Updated);
        let granted=|id:Uuid|json!({"role_id":id,"permissions":{"read_channel":true}});
        account.script("GET","/channels",json!({"channels":[{"id":channel, "type":"text","permissions":[granted(a),granted(b)]}]})).await?;
        assert_eq!(refresh(&account,MetadataScope::Channels).await?,RefreshStatus::Updated);
        account.session.history(channel).await?;
        let set=set_id(&account).await?;
        account.script("GET","/server",json!({"id":Uuid::from_u128(1),"owner_id":null})).await?;
        account.script("GET","/channels",json!({"channels":[{"id":channel,"type":"text","permissions":[granted(a)]}]})).await?;
        account.script("GET","/roles",definitions).await?;
        account.script("GET","/users",json!({"has_more":false,"users":[{"id":account.own(),"roles":[{"id":b}]}]})).await?;
        let previous=account.status().generation;
        let mut dms=account.server().hold_next_rest("GET","/dms").await?;
        account.session.reconnect()?;
        dms.wait_started().await?;
        assert!(matches!(account.session.history(channel).await,Err(HistoryError::AccessUnknown)));
        assert_eq!(set_id(&account).await?,set);
        assert_eq!(account.cached_messages().await?,SEEDED_MESSAGES);
        dms.release();
        super::online_at(&account.session,|s|s.generation>previous).await?;
        assert!(matches!(account.session.history(channel).await,Err(HistoryError::Closed(HistoryClosedReason::PermissionDenied))));
        Ok::<_,Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
