//! Authoritative non-message REST replacement; journaling and publication belong to the connection pump.

use crate::{
    ClientError, ResourceSnapshot, generation::Epoch, reconcile::ResourceConnection,
    session::SessionInner,
};
use papo_api::{Cursor, HistoryOrder, wire::UserSummary};
use std::collections::HashSet;

pub(crate) async fn fetch(
    connection: &ResourceConnection,
    session: &SessionInner,
    epoch: &Epoch,
) -> Result<ResourceSnapshot, ClientError> {
    let client = &connection.client.inner;
    let rest = &connection.rest;
    let mut snapshot = ResourceSnapshot::empty(epoch.generation);
    snapshot.server = Some(client.request(rest.server()).await?.into());
    snapshot.channels = client
        .request(rest.channels())
        .await?
        .channels
        .unwrap_or_default()
        .into();
    snapshot.roles = client
        .request(rest.roles())
        .await?
        .roles
        .unwrap_or_default()
        .into();
    snapshot.members = members(connection).await?.into();
    snapshot.dms = client
        .request(rest.direct_conversations())
        .await?
        .dms
        .into();
    snapshot.blocks = client.request(rest.blocked_users()).await?.users.into();
    session.epochs.validate(epoch.generation)?;
    Ok(snapshot)
}

pub(crate) async fn members(
    connection: &ResourceConnection,
) -> Result<Vec<UserSummary>, ClientError> {
    let mut members = Vec::new();
    let mut boundary: Option<(String, String)> = None;
    let mut cursors = HashSet::new();
    loop {
        let cursor = boundary.as_ref().map(|(since, id)| Cursor {
            since,
            last_id: Some(id),
            order: Some(HistoryOrder::Asc),
        });
        let page = connection
            .client
            .inner
            .request(connection.rest.members(cursor.as_ref()))
            .await?;
        let users = page.users.unwrap_or_default();
        if page.has_more != Some(true) {
            members.extend(users);
            return Ok(members);
        }
        let last = users.last().ok_or(ClientError::ResourceData)?;
        let next = (
            last.created_at.clone().ok_or(ClientError::ResourceData)?,
            last.id.clone().ok_or(ClientError::ResourceData)?,
        );
        if !cursors.insert(next.clone()) {
            return Err(ClientError::ResourceData);
        }
        boundary = Some(next);
        members.extend(users);
    }
}
