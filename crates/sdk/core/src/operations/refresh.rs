//! Targeted post-ack metadata refresh; never refetches the member roster or overwrites
//! concurrent live state. Wire validation belongs to `resource_admission`.

use super::RefreshFault;
use crate::{
    CacheResource, ClientError, Session, Uuid,
    generation::Epoch,
    history::access::{Transition, accepted, affects_read_access},
    resource_admission::Admission,
};
use papo_api::{
    RestClient,
    gateway::{Event, KnownEvent, MessageDeleteEvent},
    wire::UserSummaryBatchRequest,
};
use std::sync::Arc;

pub(super) enum Refresh {
    None,
    Pins(Uuid),
    Server,
    Channels,
    ChannelSettings(Uuid),
    Roles,
    RolesMembers(Vec<Uuid>),
    Social,
    Policy,
    Profile(Uuid),
    Member(Uuid),
    DeletedMessage { channel: Uuid, message: Uuid },
}

pub(super) async fn apply(
    session: &Session,
    epoch: &Epoch,
    rest: &RestClient,
    refresh: Refresh,
) -> Result<(), RefreshFault> {
    refresh_current(session, epoch, rest, refresh)
        .await
        .map_err(|error| match error {
            ClientError::Closed => RefreshFault::Closed,
            ClientError::Superseded => RefreshFault::Superseded,
            ClientError::Cache(_) | ClientError::Storage(_) => RefreshFault::Cache,
            _ => RefreshFault::Request,
        })?
}

async fn refresh_current(
    session: &Session,
    epoch: &Epoch,
    rest: &RestClient,
    refresh: Refresh,
) -> Result<Result<(), RefreshFault>, ClientError> {
    session.inner.epochs.validate(epoch.generation)?;
    if let Refresh::DeletedMessage { channel, message } = refresh {
        let connection = connection(session, rest).await?;
        let event = Event::Known(KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
            id: message.to_string(),
            channel_id: channel.to_string(),
        })));
        crate::history::live::apply(&session.inner, &connection, epoch.generation, &event).await?;
        return Ok(Ok(()));
    }
    if let Refresh::Pins(channel) = refresh {
        crate::history::operation_receipts::refresh_pins(session, epoch.generation, channel)
            .await
            .map_err(|_| ClientError::ResourceData)?;
        session.inner.epochs.validate(epoch.generation)?;
        return Ok(Ok(()));
    }
    if matches!(refresh, Refresh::Policy) {
        crate::scoped_request::refresh_policy(session, epoch, rest).await?;
        return Ok(Ok(()));
    }
    let mut snapshot = (**session.inner.resources.borrow()).clone();
    if snapshot.generation != epoch.generation {
        return Err(ClientError::Superseded);
    }
    let version = snapshot.version;
    let required_channel = if let Refresh::ChannelSettings(channel) = refresh {
        Some(channel)
    } else {
        None
    };
    let changed = fetch(session, epoch, rest, &mut snapshot, refresh).await?;
    if required_channel.is_some_and(|id| {
        !snapshot
            .channels
            .iter()
            .any(|channel| channel.id.as_deref().and_then(|value| value.parse().ok()) == Some(id))
    }) {
        return Ok(Err(RefreshFault::Unavailable));
    }
    let _serial = session.inner.resource_serial.lock().await;
    session.inner.epochs.validate(epoch.generation)?;
    if session.inner.resources.borrow().version != version {
        return Ok(Err(RefreshFault::ConcurrentChange));
    }
    let connection = connection(session, rest).await?;
    let access = if affects_read_access(&changed) {
        // Before this generation's fetch is admitted, the base is the cache-loaded image and
        // admission is not yet decided; the pending fetch owns this change.
        if !accepted(&session.inner, epoch.generation).await {
            return Ok(Err(RefreshFault::ConcurrentChange));
        }
        Some(Transition::begin(&session.inner, &connection, &snapshot).await?)
    } else {
        None
    };
    connection.cache.update(&snapshot, &changed).await?;
    let admission_changed = match access {
        Some(access) => access.finish(&session.inner, &connection).await?,
        None => false,
    };
    if admission_changed
        || changed
            .iter()
            .any(|resource| matches!(resource, CacheResource::Channels | CacheResource::Dms))
    {
        crate::reading::reconcile::resources(&session.inner, &connection, &snapshot).await?;
    }
    session.inner.publish_resources(snapshot)?;
    Ok(Ok(()))
}

async fn connection(
    session: &Session,
    rest: &RestClient,
) -> Result<crate::reconcile::ResourceConnection, ClientError> {
    Ok(crate::reconcile::ResourceConnection {
        client: session.client.clone(),
        rest: rest.clone(),
        cache: session.client.inner.caches.open(session.account()).await?,
    })
}

async fn fetch(
    session: &Session,
    epoch: &Epoch,
    rest: &RestClient,
    snapshot: &mut crate::ResourceSnapshot,
    refresh: Refresh,
) -> Result<Vec<CacheResource>, ClientError> {
    let client = &session.client.inner;
    let admission = Admission::new(&session.inner);
    match refresh {
        Refresh::None | Refresh::Pins(_) | Refresh::Policy | Refresh::DeletedMessage { .. } => {
            Ok(Vec::new())
        }
        Refresh::Server => {
            let mut server = admission.server(client.request(rest.server()).await?)?;
            server.icon_blob = None;
            snapshot.server = Some(Arc::new(server));
            Ok(vec![CacheResource::Server])
        }
        Refresh::Channels | Refresh::ChannelSettings(_) => {
            snapshot.channels = admission
                .channels(client.request(rest.channels()).await?)?
                .into();
            Ok(vec![CacheResource::Channels])
        }
        Refresh::Roles | Refresh::RolesMembers(_) => {
            snapshot.roles = admission.roles(client.request(rest.roles()).await?)?.into();
            super::role_projection::apply(snapshot);
            if let Refresh::RolesMembers(users) = refresh {
                for batch in users.chunks(50) {
                    members(session, admission, rest, snapshot, batch).await?;
                }
            }
            Ok(vec![
                CacheResource::Roles,
                CacheResource::Members,
                CacheResource::Profiles,
                CacheResource::Channels,
                CacheResource::Dms,
                CacheResource::Blocks,
            ])
        }
        Refresh::Social => {
            crate::scoped_request::refresh_policy(session, epoch, rest).await?;
            let dms = client.request(rest.direct_conversations()).await?;
            let blocks = client.request(rest.blocked_users()).await?;
            // Both lists are admitted before either replaces the image.
            let dms = admission.dms(dms)?;
            snapshot.blocks = admission.blocks(blocks)?.into();
            snapshot.dms = dms.into();
            Ok(vec![CacheResource::Dms, CacheResource::Blocks])
        }
        Refresh::Member(user) => {
            members(session, admission, rest, snapshot, &[user]).await?;
            Ok(vec![
                CacheResource::Members,
                CacheResource::Profiles,
                CacheResource::Dms,
                CacheResource::Blocks,
            ])
        }
        Refresh::Profile(user) => {
            let mut profile = admission.profile(user, client.request(rest.profile(user)).await?)?;
            profile.avatar_blob = None;
            let id = user.to_string();
            let profiles = Arc::make_mut(&mut snapshot.profiles);
            profiles.retain(|value| value.id.as_deref() != Some(&id));
            profiles.push(profile);
            members(session, admission, rest, snapshot, &[user]).await?;
            Ok(vec![
                CacheResource::Profiles,
                CacheResource::Members,
                CacheResource::Dms,
                CacheResource::Blocks,
            ])
        }
    }
}

async fn members(
    session: &Session,
    admission: Admission<'_>,
    rest: &RestClient,
    snapshot: &mut crate::ResourceSnapshot,
    users: &[Uuid],
) -> Result<(), ClientError> {
    let ids: Vec<_> = users.iter().map(ToString::to_string).collect();
    let request = UserSummaryBatchRequest { ids: ids.clone() };
    let response = session
        .client
        .inner
        .request(rest.user_summaries(&request))
        .await?;
    let response = admission.summaries(users, response)?;
    for profile in Arc::make_mut(&mut snapshot.profiles) {
        if let Some(summary) = response.iter().find(|member| member.id == profile.id) {
            profile.roles.clone_from(&summary.roles);
        }
    }
    for dm in Arc::make_mut(&mut snapshot.dms) {
        if let Some(summary) = response.iter().find(|member| member.id == dm.user.id) {
            dm.user = summary.clone();
        }
    }
    for blocked in Arc::make_mut(&mut snapshot.blocks) {
        if let Some(summary) = response.iter().find(|member| member.id == blocked.id) {
            *blocked = summary.clone();
        }
    }
    let members = Arc::make_mut(&mut snapshot.members);
    members.retain(|member| !member.id.as_ref().is_some_and(|id| ids.contains(id)));
    members.extend(response);
    Ok(())
}
