//! Resource-specific live event merges; connection journaling and generation checks live in the pump.

use crate::{CacheResource, ClientError, ResourceSnapshot, reconcile::ResourceConnection};
use papo_api::{
    gateway::{Event, KnownEvent},
    wire::UserSummaryBatchRequest,
};
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
        KnownEvent::RoleAdd(event) => refresh_roles(connection, snapshot, &event.user_id).await,
        KnownEvent::RoleRemove(event) => refresh_roles(connection, snapshot, &event.user_id).await,
        KnownEvent::AvatarUpdate(event) => {
            let user = event
                .user_id
                .parse()
                .map_err(|_| ClientError::ResourceData)?;
            if connection
                .client
                .inner
                .media
                .invalidate_avatar(connection.cache.account_id(), user)
                .await
                .is_err()
            {
                connection
                    .client
                    .inner
                    .disk_status
                    .send_replace(crate::DiskStatus::Fault(crate::DiskFault::Media));
            }
            refresh_member(connection, snapshot, &event.user_id).await
        }
        KnownEvent::DMUpdate(_) => {
            // A journaled payload can predate a reverse block or hide. Only the current list
            // proves both membership/access and visibility; own blocks alone are insufficient.
            snapshot.dms = connection
                .client
                .inner
                .request(connection.rest.direct_conversations())
                .await?
                .dms
                .into();
            snapshot.blocks = connection
                .client
                .inner
                .request(connection.rest.blocked_users())
                .await?
                .users
                .into();
            Ok(Some(vec![CacheResource::Dms, CacheResource::Blocks]))
        }
        _ => Ok(None),
    }
}

async fn refresh_member(
    connection: &ResourceConnection,
    snapshot: &mut ResourceSnapshot,
    user: &str,
) -> Result<Option<Vec<CacheResource>>, ClientError> {
    let request = UserSummaryBatchRequest {
        ids: vec![user.to_owned()],
    };
    let summaries = connection
        .client
        .inner
        .request(connection.rest.user_summaries(&request))
        .await?;
    if summaries.len() > 1
        || summaries
            .iter()
            .any(|summary| summary.id.as_deref() != Some(user))
    {
        return Err(ClientError::ResourceData);
    }
    let members = Arc::make_mut(&mut snapshot.members);
    if let Some(summary) = summaries.into_iter().next() {
        if let Some(member) = members
            .iter_mut()
            .find(|member| member.id.as_deref() == Some(user))
        {
            *member = summary;
        } else {
            members.push(summary);
        }
    } else {
        members.retain(|member| member.id.as_deref() != Some(user));
    }
    // Profile changes invalidate only requested profiles. A new member does not fetch an avatar blob.
    Arc::make_mut(&mut snapshot.profiles).retain(|profile| profile.id.as_deref() != Some(user));
    Ok(Some(vec![CacheResource::Members, CacheResource::Profiles]))
}

async fn refresh_roles(
    connection: &ResourceConnection,
    snapshot: &mut ResourceSnapshot,
    user: &str,
) -> Result<Option<Vec<CacheResource>>, ClientError> {
    refresh_member(connection, snapshot, user).await?;
    // A newly created role can arrive with its first assignment; only this small definition list is refreshed.
    snapshot.roles = connection
        .client
        .inner
        .request(connection.rest.roles())
        .await?
        .roles
        .unwrap_or_default()
        .into();
    Ok(Some(vec![
        CacheResource::Members,
        CacheResource::Profiles,
        CacheResource::Roles,
    ]))
}
