//! Authoritative paginated inbox and explicit read acknowledgments; hydration never generates platform alerts.

use super::{NotificationError, NotificationService, id, policy};
use crate::{Session, Uuid};
use papo_api::{
    Cursor, HistoryOrder,
    wire::{MarkNotificationsReadRequest, Notification},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Exact backend notification tuple. This ID is never the delivery ledger key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxCursor {
    /// Exact backend notification creation timestamp.
    pub created_at: String,
    /// Notification row UUID.
    pub id: String,
}
impl InboxCursor {
    pub(crate) fn from_record(row: &Notification) -> Result<Self, NotificationError> {
        let cursor = Self {
            created_at: row
                .created_at
                .clone()
                .ok_or(NotificationError::InvalidData)?,
            id: row.id.clone().ok_or(NotificationError::InvalidData)?,
        };
        cursor.key()?;
        Ok(cursor)
    }
    pub(crate) fn key(&self) -> Result<(i64, Uuid), NotificationError> {
        let date = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map_err(|_| NotificationError::InvalidData)?;
        Ok((date.timestamp_micros(), id(Some(&self.id))?))
    }
}
/// Detached account-owned inbox row, without timeline coverage or implicit read acknowledgment.
#[derive(Clone)]
pub struct NotificationRecord {
    /// Source account.
    pub account: crate::AccountId,
    /// Fetch generation.
    pub generation: crate::Generation,
    /// Authoritative backend fields.
    pub record: Arc<Notification>,
    /// Direct @mention of this account, excluding ordinary DM/reply-only rows.
    pub actual_mention: bool,
}
/// One backend ordered inbox page; failures are per-account service results.
#[derive(Clone)]
pub struct InboxPage {
    /// Newest-first rows unless requested otherwise internally.
    pub records: Vec<NotificationRecord>,
    /// Exact next tuple only when backend `has_more`.
    pub next: Option<InboxCursor>,
}
impl NotificationService {
    /// Fetches inbox metadata without claims, banners, read changes or canonical coverage.
    /// # Errors
    /// Returns malformed page, account revocation, authentication or transport failure.
    pub async fn inbox(&self, cursor: Option<InboxCursor>) -> Result<InboxPage, NotificationError> {
        let session = self.session.clone();
        self.session.client.dispatch(async move {
            let epoch = session.inner.epochs.current();
            let work = async {
                let rest = crate::scoped_request::authenticated(&session, &epoch).await?;
                page(&session, &rest, epoch.generation, cursor.as_ref(), HistoryOrder::Desc).await
            };
            Ok(tokio::select! { biased; () = epoch.cancel.cancelled() => Err(crate::ClientError::Superseded.into()), result = work => result })
        }).await?
    }
    /// Marks only explicitly supplied backend row IDs read. A 404 is an accepted stale row.
    /// # Errors
    /// Returns foreign/stale records, malformed IDs or backend failure. Never retries HTTP sends.
    pub async fn mark_read(&self, records: &[NotificationRecord]) -> Result<(), NotificationError> {
        if records.len() > 1000 {
            return Err(NotificationError::InvalidData);
        }
        let generation = self.session.generation();
        let mut ids = Vec::with_capacity(records.len());
        for record in records {
            if record.account != self.session.account() || record.generation != generation {
                return Err(NotificationError::InvalidData);
            }
            ids.push(id(record.record.id.as_deref())?.to_string());
        }
        if ids.is_empty() {
            return Ok(());
        }
        let session = self.session.clone();
        self.session.client.dispatch(async move {
            let epoch = session.inner.epochs.current();
            session.inner.epochs.validate(generation)?;
            let work = async {
                let rest = crate::scoped_request::authenticated(&session, &epoch).await?;
                match session.client.inner.request(rest.mark_notifications_read(session.account().user(), &MarkNotificationsReadRequest { notification_ids: ids })).await {
                    Ok(_) | Err(crate::ClientError::Api(papo_api::Error::Http {status:404} | papo_api::Error::Problem {status:404,..})) => {},
                    Err(error) => return Err(error),
                }
                session.inner.epochs.validate(generation)
            };
            tokio::select! { biased; ()=epoch.cancel.cancelled()=>Err(crate::ClientError::Superseded), result=work=>result }
        }).await.map_err(Into::into)
    }
    /// Resolves a backend-confirmed inbox row to its original scoped target, without marking read.
    /// # Errors
    /// Rejects foreign/stale identity or missing channel.
    pub fn target(
        &self,
        record: &NotificationRecord,
    ) -> Result<super::NotificationTarget, NotificationError> {
        let target = super::NotificationTarget {
            account: record.account,
            generation: record.generation,
            channel: id(record.record.channel_id.as_deref())?,
            message: id(record.record.message_id.as_deref())?,
        };
        self.validate_target(&target)?;
        Ok(target)
    }
}
pub(crate) async fn page(
    session: &Session,
    rest: &papo_api::RestClient,
    generation: crate::Generation,
    cursor: Option<&InboxCursor>,
    order: HistoryOrder,
) -> Result<InboxPage, NotificationError> {
    let boundary = cursor.map(|c| Cursor {
        since: &c.created_at,
        last_id: Some(&c.id),
        order: Some(order),
    });
    let response = session
        .client
        .inner
        .request(rest.user_notifications_ordered(
            session.account().user(),
            boundary.as_ref(),
            order,
        ))
        .await?;
    session.inner.epochs.validate(generation)?;
    let rows = response.notifications.unwrap_or_default();
    if rows.len() > 100 {
        return Err(NotificationError::InvalidData);
    }
    let mut previous = cursor.map(InboxCursor::key).transpose()?;
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let key = InboxCursor::from_record(&row)?.key()?;
        if previous.is_some_and(|prior| {
            if order == HistoryOrder::Asc {
                key <= prior
            } else {
                key >= prior
            }
        }) {
            return Err(NotificationError::InvalidData);
        }
        previous = Some(key);
        id(row.channel_id.as_deref())?;
        id(row.message_id.as_deref())?;
        records.push(NotificationRecord {
            account: session.account(),
            generation,
            actual_mention: policy::mentions(
                row.message_content.as_deref().unwrap_or_default(),
                session.account().user(),
            ),
            record: Arc::new(row),
        });
    }
    let next = if response.has_more == Some(true) {
        Some(InboxCursor::from_record(
            &records.last().ok_or(NotificationError::InvalidData)?.record,
        )?)
    } else {
        None
    };
    Ok(InboxPage { records, next })
}
