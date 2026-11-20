//! Targeted post-ack metadata refresh; never refetches the member roster or overwrites
//! concurrent live state. Wire validation belongs to `resource_admission`.

use super::RefreshFault;
use crate::{
    CacheResource, ClientError, Session, Uuid,
    generation::Epoch,
    history::access::Refetch,
    reconcile::ResourceConnection,
    reconcile_fetch,
    resource_admission::Admission,
    resource_commit::{self, FetchResult, FetchStamp, Observation, Outcome},
    resources::{AdmittedMutation, ProfileBatch},
};
use papo_api::{
    RestClient,
    gateway::{Event, KnownEvent, MessageDeleteEvent},
};

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
            ClientError::NotFound => RefreshFault::Unavailable,
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
    let mut stamp = FetchStamp::capture(&session.inner, epoch.generation, Refetch::Partial).await?;
    let connection = connection(session, rest).await?;
    let fetched = fetch(
        session,
        epoch,
        &connection,
        &mut stamp.observing(&session.inner, None),
        refresh,
    )
    .await;
    let guard = resource_commit::reserve(&session.inner).await;
    let outcome =
        resource_commit::commit(&session.inner, &connection, guard, stamp, fetched).await?;
    Ok(match outcome {
        Outcome::Accepted => Ok(()),
        Outcome::ConcurrentChange => Err(RefreshFault::ConcurrentChange),
    })
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
    connection: &ResourceConnection,
    observation: &mut Observation<'_>,
    refresh: Refresh,
) -> FetchResult {
    let admission = Admission::new(&session.inner);
    let mut mutation = AdmittedMutation::default();
    match refresh {
        Refresh::None | Refresh::Pins(_) | Refresh::Policy | Refresh::DeletedMessage { .. } => {}
        Refresh::Server => reconcile_fetch::server(connection, admission, &mut mutation).await?,
        Refresh::Channels | Refresh::ChannelSettings(_) => {
            reconcile_fetch::channels(connection, admission, &mut mutation).await?;
            if let Refresh::ChannelSettings(required) = refresh
                && mutation
                    .channels
                    .as_ref()
                    .is_none_or(|channels| !channels.iter().any(|channel| channel.id == required))
            {
                observation
                    .before_rest(CacheResource::Channels, &mutation)
                    .await?;
                return Err(ClientError::NotFound.into());
            }
        }
        Refresh::Roles | Refresh::RolesMembers(_) => {
            reconcile_fetch::role_change(connection, admission, observation, &mut mutation).await?;
            if let Refresh::RolesMembers(users) = refresh {
                if mutation.channels.is_some() {
                    observation
                        .before_rest(CacheResource::Channels, &mutation)
                        .await?;
                }
                observation
                    .before_rest(CacheResource::Roles, &mutation)
                    .await?;
                for batch in users.chunks(50) {
                    observation
                        .before_rest(CacheResource::Members, &mutation)
                        .await?;
                    reconcile_fetch::summaries(connection, admission, batch, &mut mutation).await?;
                }
            }
        }
        Refresh::Social => {
            crate::scoped_request::refresh_policy(session, epoch, &connection.rest).await?;
            reconcile_fetch::social(connection, admission, observation, &mut mutation).await?;
        }
        Refresh::Member(user) => {
            reconcile_fetch::summaries(connection, admission, &[user], &mut mutation).await?;
        }
        Refresh::Profile(user) => {
            let wire = connection
                .client
                .inner
                .request(connection.rest.profile(user))
                .await?;
            mutation
                .profiles
                .push(ProfileBatch::single(admission.profile(user, wire)?));
            reconcile_fetch::summaries(connection, admission, &[user], &mut mutation).await?;
        }
    }
    Ok(mutation)
}
