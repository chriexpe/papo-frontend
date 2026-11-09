//! Resource-specific live event merges; connection journaling and generation checks live in the pump.

use crate::{CacheResource, ClientError, ResourceSnapshot, reconcile::ResourceConnection};
use papo_api::gateway::{Event, KnownEvent, PresenceMember};
use std::sync::Arc;

pub(crate) fn relevant(event: &Event) -> bool {
    matches!(
        event,
        Event::Known(
            KnownEvent::ChannelCreate(_)
                | KnownEvent::ChannelUpdate(_)
                | KnownEvent::ChannelDelete(_)
                | KnownEvent::UserJoin(_)
                | KnownEvent::RoleAdd(_)
                | KnownEvent::RoleRemove(_)
                | KnownEvent::AvatarUpdate(_)
                | KnownEvent::DMUpdate(_)
                | KnownEvent::PresenceSync(_)
                | KnownEvent::PresenceUpdate(_)
        )
    )
}

pub(crate) async fn apply(
    connection: &ResourceConnection,
    snapshot: &mut ResourceSnapshot,
    event: &Event,
) -> Result<Option<Vec<CacheResource>>, ClientError> {
    let Event::Known(event) = event else {
        return Ok(None);
    };
    match event {
        KnownEvent::ChannelCreate(_)
        | KnownEvent::ChannelUpdate(_)
        | KnownEvent::ChannelDelete(_) => {
            // Events omit overwrites and some channel fields; only the backend's list is authoritative.
            snapshot.channels = connection
                .client
                .inner
                .request(connection.rest.channels())
                .await?
                .channels
                .unwrap_or_default()
                .into();
            Ok(Some(vec![CacheResource::Channels]))
        }
        KnownEvent::UserJoin(event) => refresh_member(connection, snapshot, &event.user_id).await,
        KnownEvent::RoleAdd(event) => refresh_member(connection, snapshot, &event.user_id).await,
        KnownEvent::RoleRemove(event) => refresh_member(connection, snapshot, &event.user_id).await,
        KnownEvent::AvatarUpdate(event) => {
            refresh_member(connection, snapshot, &event.user_id).await
        }
        KnownEvent::DMUpdate(event) => {
            let dms = Arc::make_mut(&mut snapshot.dms);
            if let Some(dm) = dms.iter_mut().find(|dm| dm.id == event.dm.id) {
                *dm = event.dm.clone();
            } else {
                dms.push(event.dm.clone());
            }
            snapshot.blocks = connection
                .client
                .inner
                .request(connection.rest.blocked_users())
                .await?
                .users
                .into();
            Ok(Some(vec![CacheResource::Dms, CacheResource::Blocks]))
        }
        KnownEvent::PresenceSync(event) => {
            snapshot.presence = Arc::new(event.members.clone());
            Ok(Some(Vec::new()))
        }
        KnownEvent::PresenceUpdate(event) => {
            let presence = Arc::make_mut(&mut snapshot.presence);
            if let Some(member) = presence
                .iter_mut()
                .find(|member| member.user_id == event.user_id)
            {
                member.status.clone_from(&event.status);
                member.status_message.clone_from(&event.status_message);
            } else {
                presence.push(PresenceMember {
                    user_id: event.user_id.clone(),
                    status: event.status.clone(),
                    status_message: event.status_message.clone(),
                    user_voice: Vec::new(),
                });
            }
            Ok(Some(Vec::new()))
        }
        _ => Ok(None),
    }
}

async fn refresh_member(
    connection: &ResourceConnection,
    snapshot: &mut ResourceSnapshot,
    user: &str,
) -> Result<Option<Vec<CacheResource>>, ClientError> {
    snapshot.members = crate::reconcile_fetch::members(connection).await?.into();
    // Profile changes invalidate only requested profiles. A new member does not fetch an avatar blob.
    Arc::make_mut(&mut snapshot.profiles).retain(|profile| profile.id.as_deref() != Some(user));
    Ok(Some(vec![CacheResource::Members, CacheResource::Profiles]))
}
