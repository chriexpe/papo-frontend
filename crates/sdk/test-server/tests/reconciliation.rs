//! Owns real-loopback reconciliation snapshots and channel mutation proofs; leaves core merge policy outside.

use std::time::Duration;

use futures_util::StreamExt;
use papo_api::{
    gateway::{Event, HeartbeatAckEvent, KnownEvent},
    wire::{
        BlockListResponse, ChannelList, ChannelPermissionEntry, ChannelPermissionsResponse,
        DirectConversation, DirectConversationList, MemberListResponse, ProfileBatchResponse, Role,
        RoleListResponse, Server, UserProfile, UserSummary,
    },
};
use papo_test_server::{Config, Error, TestServer};
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    client_async,
    tungstenite::{Error as SocketError, client::IntoClientRequest},
};

mod test_client;
use test_client::{Account, next, resource_event};

#[tokio::test]
async fn public_snapshots_match_pinned_types_and_profiles_are_available_on_demand()
-> Result<(), Error> {
    let server = TestServer::start(Config::default()).await?;
    let first = Account::register(&server, "first").await?;
    let second = Account::register(&server, "second").await?;
    let mut channel = server.create_channel("initial").await?;
    let channel_id = channel.id.clone().ok_or(Error::Configuration)?;
    let role_id = "20000000-0000-4000-8000-000000000001";
    channel.permissions = Some(vec![ChannelPermissionEntry {
        role_id: Some(role_id.into()),
        ..ChannelPermissionEntry::default()
    }]);
    server.set_channel(channel).await?;
    server
        .set_roles(vec![Role {
            id: Some(role_id.into()),
            name: Some("member".into()),
            ..Role::default()
        }])
        .await;
    let profile = UserProfile {
        id: Some(second.id.clone()),
        username: Some("second".into()),
        description: Some("profile".into()),
        ..UserProfile::default()
    };
    server.set_profile(profile).await?;
    let info: Server = first.get("/server").await?;
    assert_eq!(
        (info.channel_count, info.role_count, info.member_count),
        (Some(1), Some(1), Some(2))
    );
    let channels: ChannelList = first.get("/channels").await?;
    let channels = channels.channels.ok_or(Error::Connection)?;
    assert_eq!(channels[0].permissions.as_ref().map(Vec::len), Some(1));
    let permissions: ChannelPermissionsResponse = first
        .get(&format!("/channels/{channel_id}/permissions"))
        .await?;
    assert_eq!(
        permissions.permissions.ok_or(Error::Connection)?[0]
            .role_id
            .as_deref(),
        Some(role_id)
    );
    let roles: RoleListResponse = first.get("/roles").await?;
    assert_eq!(
        roles.roles.ok_or(Error::Connection)?[0].id.as_deref(),
        Some(role_id)
    );
    let members: MemberListResponse = first.get("/users").await?;
    assert_eq!(members.users.ok_or(Error::Connection)?.len(), 2);
    let profile: UserProfile = first.get(&format!("/users/{}/profile", second.id)).await?;
    assert_eq!(profile.description.as_deref(), Some("profile"));
    let response = first
        .http
        .post(format!("{}/users/profile_batch", first.base))
        .header("Cookie", &first.cookie)
        .json(&json!({"ids": [second.id, first.id, second.id]}))
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    let profiles: ProfileBatchResponse = response.json().await.map_err(|_| Error::Connection)?;
    assert_eq!(profiles.profiles.ok_or(Error::Connection)?.len(), 2);
    let unauthorized = first
        .http
        .get(format!("{}/channels", first.base))
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(unauthorized.status().as_u16(), 401);
    server.shutdown().await
}

#[tokio::test]
async fn private_snapshots_keep_dms_blocks_and_read_marks_account_scoped() -> Result<(), Error> {
    let server = TestServer::start(Config::default()).await?;
    let first = Account::register(&server, "first").await?;
    let second = Account::register(&server, "second").await?;
    let mut channel = server.create_channel("initial").await?;
    let channel_id = channel.id.clone().ok_or(Error::Configuration)?;
    channel.last_read_at = Some("shared-seed-must-not-leak".into());
    server.set_channel(channel).await?;
    let peer = UserSummary {
        id: Some(second.id.clone()),
        username: Some("second".into()),
        ..UserSummary::default()
    };
    server.set_blocks(&first.id, vec![peer.clone()]).await;
    let dm = DirectConversation {
        id: "30000000-0000-4000-8000-000000000001".into(),
        user: peer,
        created_at: "2026-01-01T00:00:00Z".into(),
        ..DirectConversation::default()
    };
    server.set_direct_conversations(&first.id, vec![dm]).await?;
    server
        .set_read_mark(
            &first.id,
            &channel_id,
            "40000000-0000-4000-8000-000000000001",
            "2026-01-02T00:00:00Z",
        )
        .await?;

    let channels: ChannelList = first.get("/channels").await?;
    let channels = channels.channels.ok_or(Error::Connection)?;
    assert_eq!(
        channels[0].last_read_at.as_deref(),
        Some("2026-01-02T00:00:00Z")
    );
    let other_channels: ChannelList = second.get("/channels").await?;
    assert!(
        other_channels.channels.ok_or(Error::Connection)?[0]
            .last_read_at
            .is_none()
    );
    let blocks: BlockListResponse = first.get("/users/blocks").await?;
    let other_blocks: BlockListResponse = second.get("/users/blocks").await?;
    let dms: DirectConversationList = first.get("/dms").await?;
    let other_dms: DirectConversationList = second.get("/dms").await?;
    assert_eq!(
        (
            blocks.users.len(),
            other_blocks.users.len(),
            dms.dms.len(),
            other_dms.dms.len()
        ),
        (1, 0, 1, 0)
    );
    server.shutdown().await
}

#[tokio::test]
async fn disconnected_channel_mutations_are_authoritative_and_live_changes_emit_typed_events()
-> Result<(), Error> {
    let server = TestServer::start(Config::default()).await?;
    let account = Account::register(&server, "first").await?;
    let retained = server.create_channel("retained").await?;
    let deleted = server.create_channel("deleted").await?;
    let retained_id = retained.id.ok_or(Error::Connection)?;
    let deleted_id = deleted.id.ok_or(Error::Connection)?;
    let mut old_socket = account.socket(&server).await?;
    assert_eq!(next(&mut old_socket).await?.kind(), "presence_sync");
    server.disconnect_gateways();
    let closed = tokio::time::timeout(Duration::from_secs(5), old_socket.next())
        .await
        .map_err(|_| Error::Connection)?;
    assert!(
        closed.is_none_or(
            |message| message.is_err() || message.is_ok_and(|message| message.is_close())
        )
    );
    assert!(server.rename_channel(&retained_id, "renamed").await);
    assert!(server.delete_channel(&deleted_id).await);
    let created = server.create_channel("offline-created").await?;

    let mut socket = account.socket(&server).await?;
    assert_eq!(next(&mut socket).await?.kind(), "presence_sync");
    let channels: ChannelList = account.get("/channels").await?;
    let channels = channels.channels.ok_or(Error::Connection)?;
    assert_eq!(channels.len(), 2);
    assert!(
        channels
            .iter()
            .any(|channel| channel.id.as_deref() == Some(&retained_id)
                && channel.name.as_deref() == Some("renamed"))
    );
    assert!(channels.iter().any(|channel| channel.id == created.id));
    assert!(
        !channels
            .iter()
            .any(|channel| channel.id.as_deref() == Some(&deleted_id))
    );
    server
        .emit_event(KnownEvent::HeartbeatAck(Box::new(HeartbeatAckEvent {})))
        .await?;
    assert_eq!(resource_event(&mut socket).await?.kind(), "heartbeat_ack");

    let live = server.create_channel("live").await?;
    let id = live.id.ok_or(Error::Connection)?;
    let Event::Known(KnownEvent::ChannelCreate(event)) = resource_event(&mut socket).await? else {
        return Err(Error::Connection);
    };
    assert_eq!(event.channel_id, id);
    assert!(server.rename_channel(&id, "live-renamed").await);
    let Event::Known(KnownEvent::ChannelUpdate(event)) = resource_event(&mut socket).await? else {
        return Err(Error::Connection);
    };
    assert_eq!(
        (event.channel_id.as_str(), event.name.as_str()),
        (id.as_str(), "live-renamed")
    );
    assert!(server.delete_channel(&id).await);
    let Event::Known(KnownEvent::ChannelDelete(event)) = resource_event(&mut socket).await? else {
        return Err(Error::Connection);
    };
    assert_eq!(event.channel_id, id);
    socket.close(None).await.map_err(|_| Error::Connection)?;
    server.shutdown().await
}

#[tokio::test]
async fn held_snapshot_precedes_live_event_and_release_does_not_block_mutation() -> Result<(), Error>
{
    let server = TestServer::start(Config::default()).await?;
    let account = Account::register(&server, "first").await?;
    let mut socket = account.socket(&server).await?;
    assert_eq!(next(&mut socket).await?.kind(), "presence_sync");
    let mut hold = server.hold_next_rest("GET", "/channels").await?;
    let http = account.http.clone();
    let url = format!("{}/channels", account.base);
    let cookie = account.cookie.clone();
    let request = tokio::spawn(async move { http.get(url).header("Cookie", cookie).send().await });
    tokio::time::timeout(Duration::from_secs(5), hold.wait_started())
        .await
        .map_err(|_| Error::Connection)??;
    assert!(matches!(
        hold.wait_started().await,
        Err(Error::Configuration)
    ));
    server.create_channel("after-capture").await?;
    assert_eq!(resource_event(&mut socket).await?.kind(), "channel_create");
    hold.release();
    let captured: ChannelList = request
        .await
        .map_err(|_| Error::Connection)?
        .map_err(|_| Error::Connection)?
        .json()
        .await
        .map_err(|_| Error::Connection)?;
    assert!(captured.channels.ok_or(Error::Connection)?.is_empty());
    let current: ChannelList = account.get("/channels").await?;
    assert_eq!(current.channels.ok_or(Error::Connection)?.len(), 1);
    socket.close(None).await.map_err(|_| Error::Connection)?;
    server.shutdown().await
}

#[tokio::test]
async fn gateway_admission_can_reject_and_resume_without_disconnecting_existing_sockets()
-> Result<(), Error> {
    let server = TestServer::start(Config::default()).await?;
    let account = Account::register(&server, "first").await?;
    let mut existing = account.socket(&server).await?;
    assert_eq!(next(&mut existing).await?.kind(), "presence_sync");
    server.set_gateway_accepting(false).await;
    let mut request = server
        .gateway_url()
        .into_client_request()
        .map_err(|_| Error::Connection)?;
    request.headers_mut().insert(
        "Cookie",
        account.cookie.parse().map_err(|_| Error::Connection)?,
    );
    let stream = TcpStream::connect(server.address())
        .await
        .map_err(|_| Error::Connection)?;
    let rejected = tokio::time::timeout(Duration::from_secs(5), client_async(request, stream))
        .await
        .map_err(|_| Error::Connection)?;
    assert!(
        matches!(rejected, Err(SocketError::Http(response)) if response.status().as_u16() == 503)
    );
    let _: Server = account.get("/server").await?;
    server
        .emit_event(KnownEvent::HeartbeatAck(Box::new(HeartbeatAckEvent {})))
        .await?;
    assert_eq!(resource_event(&mut existing).await?.kind(), "heartbeat_ack");
    server.disconnect_gateways();
    let closed = tokio::time::timeout(Duration::from_secs(5), existing.next())
        .await
        .map_err(|_| Error::Connection)?;
    assert!(
        closed.is_none_or(
            |message| message.is_err() || message.is_ok_and(|message| message.is_close())
        )
    );
    server.create_channel("disconnected").await?;
    server.set_gateway_accepting(true).await;
    let mut resumed = account.socket(&server).await?;
    assert_eq!(next(&mut resumed).await?.kind(), "presence_sync");
    let channels: ChannelList = account.get("/channels").await?;
    assert_eq!(channels.channels.ok_or(Error::Connection)?.len(), 1);
    resumed.close(None).await.map_err(|_| Error::Connection)?;
    server.shutdown().await
}

#[tokio::test]
async fn generated_channel_ids_do_not_replace_explicitly_seeded_channels() -> Result<(), Error> {
    let server = TestServer::start(Config::default()).await?;
    let account = Account::register(&server, "first").await?;
    server
        .set_channel(papo_api::wire::Channel {
            id: Some("10000000-0000-4000-8000-000000000001".into()),
            name: Some("seeded".into()),
            r#type: Some("text".into()),
            position: Some(1),
            ..papo_api::wire::Channel::default()
        })
        .await?;
    let created = server.create_channel("generated").await?;
    assert_eq!(
        created.id.as_deref(),
        Some("10000000-0000-4000-8000-000000000002")
    );
    let channels: ChannelList = account.get("/channels").await?;
    assert_eq!(channels.channels.ok_or(Error::Connection)?.len(), 2);
    server.shutdown().await
}
