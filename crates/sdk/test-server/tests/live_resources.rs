//! Owns pinned member/profile/DM event and recipient-isolation proofs over real TCP; leaves reconcile merging outside.

use papo_api::{
    gateway::{Event, HeartbeatAckEvent, KnownEvent},
    wire::{DirectConversation, DirectConversationList, RoleSummary, UserProfile, UserSummary},
};
use papo_test_server::{Config, Error, TestServer};

mod test_client;
use test_client::{Account, next, resource_event};

#[tokio::test]
async fn member_profile_changes_emit_pinned_events_and_dm_updates_stay_private() -> Result<(), Error>
{
    let server = TestServer::start(Config::default()).await?;
    let first = Account::register(&server, "first").await?;
    let second = Account::register(&server, "second").await?;
    let mut first_socket = first.socket(&server).await?;
    let mut second_socket = second.socket(&server).await?;
    assert_eq!(next(&mut first_socket).await?.kind(), "presence_sync");
    assert_eq!(next(&mut second_socket).await?.kind(), "presence_sync");

    let mut profile = UserProfile {
        id: Some("50000000-0000-4000-8000-000000000001".into()),
        username: Some("joined".into()),
        ..UserProfile::default()
    };
    server.set_profile(profile.clone()).await?;
    for socket in [&mut first_socket, &mut second_socket] {
        assert_eq!(resource_event(socket).await?.kind(), "user_join");
    }
    profile.roles = Some(vec![RoleSummary {
        id: Some("60000000-0000-4000-8000-000000000001".into()),
        ..RoleSummary::default()
    }]);
    server.set_profile(profile.clone()).await?;
    for socket in [&mut first_socket, &mut second_socket] {
        assert_eq!(resource_event(socket).await?.kind(), "role_add");
    }
    profile.roles = Some(Vec::new());
    profile.avatar_format = Some("png".into());
    server.set_profile(profile).await?;
    for socket in [&mut first_socket, &mut second_socket] {
        assert_eq!(resource_event(socket).await?.kind(), "role_remove");
        assert_eq!(resource_event(socket).await?.kind(), "avatar_update");
    }

    let dm = DirectConversation {
        id: "70000000-0000-4000-8000-000000000001".into(),
        user: UserSummary {
            id: Some(second.id.clone()),
            ..UserSummary::default()
        },
        created_at: "2026-01-01T00:00:00Z".into(),
        ..DirectConversation::default()
    };
    server.set_direct_conversations(&first.id, vec![dm]).await?;
    server
        .emit_event(KnownEvent::HeartbeatAck(Box::new(HeartbeatAckEvent {})))
        .await?;
    let Event::Known(KnownEvent::DMUpdate(update)) = resource_event(&mut first_socket).await?
    else {
        return Err(Error::Connection);
    };
    assert_eq!(update.dm.user.id.as_deref(), Some(second.id.as_str()));
    assert_eq!(
        resource_event(&mut first_socket).await?.kind(),
        "heartbeat_ack"
    );
    assert_eq!(
        resource_event(&mut second_socket).await?.kind(),
        "heartbeat_ack"
    );
    let first_dms: DirectConversationList = first.get("/dms").await?;
    let second_dms: DirectConversationList = second.get("/dms").await?;
    assert_eq!((first_dms.dms.len(), second_dms.dms.len()), (1, 0));
    first_socket
        .close(None)
        .await
        .map_err(|_| Error::Connection)?;
    second_socket
        .close(None)
        .await
        .map_err(|_| Error::Connection)?;
    server.shutdown().await
}
