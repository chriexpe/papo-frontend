//! Resource-specific live event merges; connection journaling and generation checks live in
//! the pump, and wire validation in `resource_admission`.

use crate::{
    CacheResource, ClientError, ResourceSnapshot, Uuid, reconcile::ResourceConnection,
    resource_admission::Admission,
};
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

/// Applies one relevant event to `snapshot` by refetching the groups it names.
/// # Errors
/// A rejected group returns `ResourceRejected` before `snapshot` is published; the pump
/// then ends this generation so the next reconnect resynchronizes every group.
pub(crate) async fn apply(
    admission: Admission<'_>,
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
            let channels = connection
                .client
                .inner
                .request(connection.rest.channels())
                .await?;
            snapshot.channels = admission.channels(channels)?.into();
            Ok(Some(vec![CacheResource::Channels]))
        }
        KnownEvent::UserJoin(event) => {
            let user = admission.event_user(&event.user_id)?;
            refresh_member(admission, connection, snapshot, user).await
        }
        KnownEvent::RoleAdd(event) => {
            let user = admission.event_user(&event.user_id)?;
            refresh_roles(admission, connection, snapshot, user).await
        }
        KnownEvent::RoleRemove(event) => {
            let user = admission.event_user(&event.user_id)?;
            refresh_roles(admission, connection, snapshot, user).await
        }
        KnownEvent::AvatarUpdate(event) => {
            let user = admission.event_user(&event.user_id)?;
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
            refresh_member(admission, connection, snapshot, user).await
        }
        KnownEvent::DMUpdate(_) => {
            // A journaled payload can predate a reverse block or hide. Only the current list
            // proves both membership/access and visibility; own blocks alone are insufficient.
            let dms = connection
                .client
                .inner
                .request(connection.rest.direct_conversations())
                .await?;
            let blocks = connection
                .client
                .inner
                .request(connection.rest.blocked_users())
                .await?;
            // Both lists are admitted before either replaces the image.
            let dms = admission.dms(dms)?;
            snapshot.blocks = admission.blocks(blocks)?.into();
            snapshot.dms = dms.into();
            Ok(Some(vec![CacheResource::Dms, CacheResource::Blocks]))
        }
        _ => Ok(None),
    }
}

async fn refresh_member(
    admission: Admission<'_>,
    connection: &ResourceConnection,
    snapshot: &mut ResourceSnapshot,
    user: Uuid,
) -> Result<Option<Vec<CacheResource>>, ClientError> {
    let request = UserSummaryBatchRequest {
        ids: vec![user.to_string()],
    };
    let summaries = connection
        .client
        .inner
        .request(connection.rest.user_summaries(&request))
        .await?;
    let summaries = admission.summaries(&[user], summaries)?;
    let user = user.to_string();
    let user = user.as_str();
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
    admission: Admission<'_>,
    connection: &ResourceConnection,
    snapshot: &mut ResourceSnapshot,
    user: Uuid,
) -> Result<Option<Vec<CacheResource>>, ClientError> {
    refresh_member(admission, connection, snapshot, user).await?;
    // A newly created role can arrive with its first assignment; only this small definition list is refreshed.
    let roles = connection
        .client
        .inner
        .request(connection.rest.roles())
        .await?;
    snapshot.roles = admission.roles(roles)?.into();
    Ok(Some(vec![
        CacheResource::Members,
        CacheResource::Profiles,
        CacheResource::Roles,
    ]))
}
