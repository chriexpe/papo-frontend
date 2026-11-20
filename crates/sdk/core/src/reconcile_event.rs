//! Fetches the admitted facts implied by a journaled event; the pump owns cancellation and ordering.

use crate::{
    CacheResource,
    reconcile::ResourceConnection,
    reconcile_fetch,
    resource_admission::Admission,
    resource_commit::{FetchResult, Observation},
    resources::AdmittedMutation,
};
use papo_api::gateway::{Event, KnownEvent};

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

pub(crate) async fn fetch(
    admission: Admission<'_>,
    connection: &ResourceConnection,
    observation: &mut Observation<'_>,
    event: &Event,
) -> FetchResult {
    let mut mutation = AdmittedMutation::default();
    let Event::Known(event) = event else {
        return Ok(mutation);
    };
    match event {
        KnownEvent::ChannelCreate(_)
        | KnownEvent::ChannelUpdate(_)
        | KnownEvent::ChannelDelete(_) => {
            reconcile_fetch::channels(connection, admission, &mut mutation).await?;
        }
        KnownEvent::UserJoin(event) => {
            let user = admission.event_user(&event.user_id)?;
            reconcile_fetch::summaries(connection, admission, &[user], &mut mutation).await?;
        }
        KnownEvent::RoleAdd(event) => {
            let user = admission.event_user(&event.user_id)?;
            reconcile_fetch::summaries(connection, admission, &[user], &mut mutation).await?;
            observation
                .before_rest(CacheResource::Members, &mutation)
                .await?;
            reconcile_fetch::role_change(connection, admission, observation, &mut mutation).await?;
        }
        KnownEvent::RoleRemove(event) => {
            let user = admission.event_user(&event.user_id)?;
            reconcile_fetch::summaries(connection, admission, &[user], &mut mutation).await?;
            observation
                .before_rest(CacheResource::Members, &mutation)
                .await?;
            reconcile_fetch::role_change(connection, admission, observation, &mut mutation).await?;
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
            reconcile_fetch::summaries(connection, admission, &[user], &mut mutation).await?;
            mutation.invalidate_profiles.push(user);
        }
        KnownEvent::DMUpdate(_) => {
            reconcile_fetch::social(connection, admission, observation, &mut mutation).await?;
        }
        _ => {}
    }
    Ok(mutation)
}
