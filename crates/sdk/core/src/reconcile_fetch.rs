//! Fetches scoped resource facts. Every endpoint attributes malformed responses to its
//! requested group before the shared commit boundary decides whether to quarantine.

use crate::{
    CacheResource, Uuid,
    reconcile::ResourceConnection,
    resource_admission::{Admission, Roster, RosterPage},
    resource_commit::{FetchResult, Observation, ScopedFailure, unresolved_overwrites},
    resources::{AdmittedMutation, UserObservation},
};
use papo_api::{Cursor, HistoryOrder, wire::UserSummaryBatchRequest};

pub(crate) async fn fetch(
    observation: &mut Observation<'_>,
    connection: &ResourceConnection,
    admission: Admission<'_>,
) -> FetchResult {
    let mut mutation = AdmittedMutation {
        complete: true,
        ..Default::default()
    };
    server(connection, admission, &mut mutation).await?;
    observation
        .before_rest(CacheResource::Server, &mutation)
        .await?;
    channels(connection, admission, &mut mutation).await?;
    observation
        .before_rest(CacheResource::Channels, &mutation)
        .await?;
    roles(connection, admission, &mut mutation).await?;
    observation
        .before_rest(CacheResource::Roles, &mutation)
        .await?;
    mutation.roster = Some(members(connection, admission).await?);
    observation
        .before_rest(CacheResource::Members, &mutation)
        .await?;
    social(connection, admission, observation, &mut mutation).await?;
    Ok(mutation)
}

pub(crate) async fn server(
    connection: &ResourceConnection,
    admission: Admission<'_>,
    mutation: &mut AdmittedMutation,
) -> Result<(), ScopedFailure> {
    let wire = connection
        .client
        .inner
        .request(connection.rest.server())
        .await
        .map_err(|error| ScopedFailure::for_group(CacheResource::Server, error))?;
    mutation.server = Some(admission.server(wire)?);
    Ok(())
}

pub(crate) async fn channels(
    connection: &ResourceConnection,
    admission: Admission<'_>,
    mutation: &mut AdmittedMutation,
) -> Result<(), ScopedFailure> {
    let wire = connection
        .client
        .inner
        .request(connection.rest.channels())
        .await
        .map_err(|error| ScopedFailure::for_group(CacheResource::Channels, error))?;
    mutation.channels = Some(admission.channels(wire)?);
    Ok(())
}

async fn roles(
    connection: &ResourceConnection,
    admission: Admission<'_>,
    mutation: &mut AdmittedMutation,
) -> Result<(), ScopedFailure> {
    let wire = connection
        .client
        .inner
        .request(connection.rest.roles())
        .await
        .map_err(|error| ScopedFailure::for_group(CacheResource::Roles, error))?;
    mutation.roles = Some(admission.roles(wire)?);
    Ok(())
}

/// A role-only readback cannot tell whether a deleted role's overwrite survived the
/// backend's textual key deletion. Resolve that ambiguity in the same logical update.
pub(crate) async fn role_change(
    connection: &ResourceConnection,
    admission: Admission<'_>,
    observation: &mut Observation<'_>,
    mutation: &mut AdmittedMutation,
) -> Result<(), ScopedFailure> {
    roles(connection, admission, mutation).await?;
    let dependent = unresolved_overwrites(&observation.stamp.base, mutation);
    if !dependent.is_empty() {
        observation
            .before_rest(CacheResource::Roles, mutation)
            .await?;
        channels(connection, admission, mutation).await?;
    }
    Ok(())
}

pub(crate) async fn social(
    connection: &ResourceConnection,
    admission: Admission<'_>,
    observation: &mut Observation<'_>,
    mutation: &mut AdmittedMutation,
) -> Result<(), ScopedFailure> {
    let client = &connection.client.inner;
    let dms = client
        .request(connection.rest.direct_conversations())
        .await
        .map_err(|error| ScopedFailure::for_group(CacheResource::Dms, error))?;
    mutation.dms = Some(admission.dms(dms)?);
    observation
        .before_rest(CacheResource::Dms, mutation)
        .await?;
    let blocks = client
        .request(connection.rest.blocked_users())
        .await
        .map_err(|error| ScopedFailure::for_group(CacheResource::Blocks, error))?;
    mutation.blocks = Some(admission.blocks(blocks)?);
    Ok(())
}

pub(crate) async fn summaries(
    connection: &ResourceConnection,
    admission: Admission<'_>,
    ids: &[Uuid],
    mutation: &mut AdmittedMutation,
) -> Result<(), ScopedFailure> {
    let request = UserSummaryBatchRequest {
        ids: ids.iter().map(ToString::to_string).collect(),
    };
    let observed = async {
        let wire = connection
            .client
            .inner
            .request(connection.rest.user_summaries(&request))
            .await
            .map_err(|error| ScopedFailure::for_group(CacheResource::Members, error))?;
        Ok::<_, ScopedFailure>(admission.summaries(ids, wire)?)
    }
    .await;
    let batch = observed.map_err(|error| {
        if ids.contains(&connection.cache.account_id().user()) {
            error
        } else {
            error.display()
        }
    })?;
    mutation.summaries.push(batch);
    Ok(())
}

async fn members(
    connection: &ResourceConnection,
    admission: Admission<'_>,
) -> Result<Vec<UserObservation>, ScopedFailure> {
    let mut roster = Roster::new(admission);
    let mut boundary = None;
    loop {
        let cursor = boundary
            .as_ref()
            .map(|(since, id): &(String, String)| Cursor {
                since,
                last_id: Some(id),
                order: Some(HistoryOrder::Asc),
            });
        let page = connection
            .client
            .inner
            .request(connection.rest.members(cursor.as_ref()))
            .await
            .map_err(|error| ScopedFailure::for_group(CacheResource::Members, error))?;
        match roster.page(page)? {
            RosterPage::More(next) => boundary = Some(next),
            RosterPage::Complete(members) => return Ok(members),
        }
    }
}
