//! Exact raw audit pagination and explicitly local filters; audit authorization stays on the backend.

use super::{OperationError, Operations};
use crate::Generated;
use crate::{AccountId, Generation, HistoryOrder, Uuid};
pub use papo_api::wire::AuditLogEntry;
use papo_api::{Cursor, administration::AuditLogQuery, wire::AuditLogList};

/// Backend-supported filters plus clearly local channel/deletion selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditRequest {
    /// Exact backend action, for example `message.delete`.
    pub action: Option<String>,
    /// Actor identity used by the backend (not actor display name).
    pub actor: Option<Uuid>,
    /// Exact backend entity type.
    pub entity_type: Option<String>,
    /// Inclusive lower date bound, preserved on every page in either order.
    pub since: Option<String>,
    /// Inclusive upper date bound.
    pub until: Option<String>,
    /// Optional ordering; omission uses the backend's descending default.
    pub order: Option<HistoryOrder>,
    /// Local exact `metadata.channel_id` filter; absent metadata never matches.
    pub channel: Option<Uuid>,
    /// Local action suffix selection: true selects `.delete`, false excludes it.
    pub deletions: Option<bool>,
}

/// Opaque continuation of the same query/account/generation, including empty filtered pages.
#[derive(Clone)]
pub struct AuditCursor {
    account: AccountId,
    generation: Generation,
    request: AuditRequest,
    created_at: String,
    id: String,
}

/// Visible rows and continuation derived from the last raw backend row before local filters.
#[derive(Clone)]
pub struct AuditPage {
    /// Matching records in original backend order, never a fabricated sorted page.
    pub entries: Vec<AuditLogEntry>,
    /// Exact continuation, even when all raw rows were hidden by local filters.
    pub next: Option<AuditCursor>,
}

impl Operations {
    /// Reads one raw page (at most 100 rows), then applies local-only filters.
    /// `since` remains an inclusive lower date bound, not the last row's time:
    /// the backend resolves `last_id` to the exact cursor tuple in its ledger.
    /// # Errors
    /// Returns malformed filters/pages, stale cursors, backend denial or session errors.
    pub async fn audit_logs(
        &self,
        request: AuditRequest,
        cursor: Option<AuditCursor>,
    ) -> Result<Generated<AuditPage>, OperationError> {
        validate_request(&request)?;
        let account = self.session.account();
        let generation = self.session.generation();
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.account != account || cursor.generation != generation)
        {
            return Err(OperationError::InvalidInput);
        }
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.request != request)
        {
            return Err(OperationError::InvalidInput);
        }
        let query_request = request.clone();
        let query_cursor = cursor.clone();
        let result = self
            .read(move |rest| async move {
                let actor = query_request.actor.map(|id| id.to_string());
                let query = AuditLogQuery {
                    action: query_request.action.as_deref(),
                    actor_id: actor.as_deref(),
                    entity_type: query_request.entity_type.as_deref(),
                    until: query_request.until.as_deref(),
                };
                let wire_cursor = Cursor {
                    since: query_request.since.as_deref().unwrap_or(""),
                    last_id: query_cursor.as_ref().map(|cursor| cursor.id.as_str()),
                    order: query_request.order,
                };
                rest.audit_logs(&query, Some(&wire_cursor)).await
            })
            .await?;
        if cursor.as_ref().is_some_and(|cursor| {
            cursor.account != account || cursor.generation != result.generation
        }) {
            return Err(OperationError::InvalidInput);
        }
        let value = project(
            result.value,
            &request,
            cursor.as_ref(),
            account,
            result.generation,
        )?;
        self.session.inner.epochs.validate(result.generation)?;
        Ok(Generated {
            generation: result.generation,
            value,
        })
    }
}

fn validate_request(request: &AuditRequest) -> Result<(), OperationError> {
    let since = request.since.as_deref().map(timestamp).transpose()?;
    let until = request.until.as_deref().map(timestamp).transpose()?;
    if matches!((since, until), (Some(since), Some(until)) if since > until) {
        return Err(OperationError::InvalidInput);
    }
    Ok(())
}

fn timestamp(value: &str) -> Result<chrono::DateTime<chrono::FixedOffset>, OperationError> {
    chrono::DateTime::parse_from_rfc3339(value).map_err(|_| OperationError::InvalidData)
}

fn tuple(
    entry: &AuditLogEntry,
) -> Result<(chrono::DateTime<chrono::FixedOffset>, Uuid), OperationError> {
    let time = timestamp(
        entry
            .created_at
            .as_deref()
            .ok_or(OperationError::InvalidData)?,
    )?;
    let id = Uuid::parse_str(entry.id.as_deref().ok_or(OperationError::InvalidData)?)
        .map_err(|_| OperationError::InvalidData)?;
    Ok((time, id))
}

fn project(
    page: AuditLogList,
    request: &AuditRequest,
    cursor: Option<&AuditCursor>,
    account: AccountId,
    generation: Generation,
) -> Result<AuditPage, OperationError> {
    let entries = page.logs.ok_or(OperationError::InvalidData)?;
    let more = page.has_more.ok_or(OperationError::InvalidData)?;
    validate_page(&entries, more, request, cursor)?;
    let next = if more {
        let last = entries.last().ok_or(OperationError::InvalidData)?;
        Some(AuditCursor {
            account,
            generation,
            request: request.clone(),
            created_at: last.created_at.clone().ok_or(OperationError::InvalidData)?,
            id: last.id.clone().ok_or(OperationError::InvalidData)?,
        })
    } else {
        None
    };
    let entries = entries
        .into_iter()
        .filter(|entry| matches_local(entry, request))
        .collect();
    Ok(AuditPage { entries, next })
}

fn validate_page(
    entries: &[AuditLogEntry],
    more: bool,
    request: &AuditRequest,
    cursor: Option<&AuditCursor>,
) -> Result<(), OperationError> {
    if entries.len() > 100 || (more && entries.is_empty()) {
        return Err(OperationError::InvalidData);
    }
    let ascending = request.order == Some(HistoryOrder::Asc);
    let since = request.since.as_deref().map(timestamp).transpose()?;
    let until = request.until.as_deref().map(timestamp).transpose()?;
    let mut previous = cursor
        .map(|cursor| {
            let id = Uuid::parse_str(&cursor.id).map_err(|_| OperationError::InvalidData)?;
            Ok::<_, OperationError>((timestamp(&cursor.created_at)?, id))
        })
        .transpose()?;
    for entry in entries {
        let point = tuple(entry)?;
        if since.is_some_and(|since| point.0 < since) || until.is_some_and(|until| point.0 > until)
        {
            return Err(OperationError::InvalidData);
        }
        if previous.is_some_and(|previous| {
            if ascending {
                point <= previous
            } else {
                point >= previous
            }
        }) {
            return Err(OperationError::InvalidData);
        }
        previous = Some(point);
    }
    Ok(())
}

fn matches_local(entry: &AuditLogEntry, request: &AuditRequest) -> bool {
    if let Some(channel) = request.channel {
        let actual = entry
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok());
        if actual != Some(channel) {
            return false;
        }
    }
    if let Some(deletions) = request.deletions {
        let deletion = entry
            .action
            .as_deref()
            .is_some_and(|action| action.ends_with(".delete"));
        if deletion != deletions {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    //! Pure pinned-backend ordering/filter regressions; no sockets or clocks.
    use super::*;

    fn entry(id: u128, timestamp: &str, action: &str) -> AuditLogEntry {
        AuditLogEntry {
            id: Some(Uuid::from_u128(id).to_string()),
            created_at: Some(timestamp.into()),
            action: Some(action.into()),
            ..AuditLogEntry::default()
        }
    }

    #[test]
    fn date_lower_bound_does_not_move_with_descending_cursor() {
        let request = AuditRequest {
            since: Some("2030-01-01T00:00:00Z".into()),
            until: Some("2030-01-03T00:00:00Z".into()),
            ..AuditRequest::default()
        };
        let entries = vec![
            entry(3, "2030-01-02T00:00:00Z", "message.delete"),
            entry(2, "2030-01-01T00:00:00Z", "message.delete"),
        ];
        assert!(validate_page(&entries, true, &request, None).is_ok());
        assert!(
            validate_page(
                &entries,
                true,
                &AuditRequest {
                    order: Some(HistoryOrder::Asc),
                    ..request
                },
                None
            )
            .is_err()
        );
    }

    #[test]
    fn equal_times_use_exact_ids_and_duplicate_or_empty_more_is_rejected() {
        let entries = vec![
            entry(2, "2030-01-02T00:00:00Z", "emoji.delete"),
            entry(1, "2030-01-02T00:00:00Z", "message.delete"),
        ];
        assert!(validate_page(&entries, true, &AuditRequest::default(), None).is_ok());
        assert!(
            validate_page(
                &[entries[0].clone(), entries[0].clone()],
                false,
                &AuditRequest::default(),
                None
            )
            .is_err()
        );
        assert!(validate_page(&[], true, &AuditRequest::default(), None).is_err());
    }

    #[test]
    fn invisible_raw_page_preserves_its_exact_continuation() {
        let address = crate::ServerAddress::parse("https://audit.invalid").unwrap();
        let account = AccountId::new(address.id(), Uuid::from_u128(1));
        let request = AuditRequest {
            deletions: Some(true),
            ..AuditRequest::default()
        };
        let raw = vec![
            entry(2, "2030-01-02T00:00:00.123456Z", "emoji.create"),
            entry(1, "2030-01-01T00:00:00Z", "role.create"),
        ];
        let page = project(
            AuditLogList {
                logs: Some(raw),
                has_more: Some(true),
            },
            &request,
            None,
            account,
            Generation(1),
        )
        .unwrap();
        assert!(page.entries.is_empty());
        let cursor = page.next.unwrap();
        assert_eq!(cursor.id, Uuid::from_u128(1).to_string());
        assert_eq!(cursor.created_at, "2030-01-01T00:00:00Z");
        assert_eq!(cursor.request, request);
        assert!(
            validate_page(
                &[entry(3, "2030-01-03T00:00:00Z", "message.delete")],
                false,
                &request,
                Some(&cursor)
            )
            .is_err()
        );
    }

    #[test]
    fn channel_and_deletion_filters_do_not_accept_missing_metadata() {
        let entry = entry(2, "2030-01-02T00:00:00Z", "message.delete");
        assert!(matches_local(
            &entry,
            &AuditRequest {
                deletions: Some(true),
                ..AuditRequest::default()
            }
        ));
        assert!(!matches_local(
            &entry,
            &AuditRequest {
                deletions: Some(false),
                ..AuditRequest::default()
            }
        ));
        assert!(!matches_local(
            &entry,
            &AuditRequest {
                channel: Some(Uuid::nil()),
                ..AuditRequest::default()
            }
        ));
    }
}
