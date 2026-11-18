//! Authoritative non-message REST replacement; journaling and publication belong to the
//! connection pump, and wire validation to `resource_admission`.

use crate::{
    ClientError, ResourceSnapshot,
    generation::Epoch,
    reconcile::ResourceConnection,
    resource_admission::{Admission, Roster, RosterPage},
    session::SessionInner,
};
use papo_api::{Cursor, HistoryOrder, wire::UserSummary};

/// Fetches every group and admits each one before any of them can replace the image.
/// # Errors
/// A rejected group fails the whole replacement with `ResourceRejected`; the caller keeps
/// the last published snapshot and cache untouched.
pub(crate) async fn fetch(
    connection: &ResourceConnection,
    session: &SessionInner,
    epoch: &Epoch,
) -> Result<ResourceSnapshot, ClientError> {
    let client = &connection.client.inner;
    let rest = &connection.rest;
    let admission = Admission::new(session);
    let mut snapshot = ResourceSnapshot::empty(epoch.generation);
    let mut server = admission.server(client.request(rest.server()).await?)?;
    server.icon_blob = None;
    snapshot.server = Some(server.into());
    snapshot.channels = admission
        .channels(client.request(rest.channels()).await?)?
        .into();
    snapshot.roles = admission.roles(client.request(rest.roles()).await?)?.into();
    snapshot.members = members(connection, admission).await?.into();
    snapshot.dms = admission
        .dms(client.request(rest.direct_conversations()).await?)?
        .into();
    snapshot.blocks = admission
        .blocks(client.request(rest.blocked_users()).await?)?
        .into();
    session.epochs.validate(epoch.generation)?;
    Ok(snapshot)
}

async fn members(
    connection: &ResourceConnection,
    admission: Admission<'_>,
) -> Result<Vec<UserSummary>, ClientError> {
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
            .await?;
        match roster.page(page)? {
            RosterPage::More(next) => boundary = Some(next),
            RosterPage::Complete(members) => return Ok(members),
        }
    }
}
