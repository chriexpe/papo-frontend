//! Audit log pagination, cursor scoping, and local metadata filter bindings across FFI.
//! Authorization and raw ledger cursor advancement remain owned by the SDK and backend.

use std::sync::Arc;

use papo_sdk::operations::audit::{AuditCursor, AuditLogEntry, AuditRequest};

use crate::{
    client::FfiClient,
    ids::{FfiAccountId, FfiHistoryOrder, parse_uuid},
    operation_errors::FfiOperationError,
    operations_records::{closed_operation_error, operation_session},
};

/// Opaque audit pagination cursor scoped to one `(account, generation, query)`.
#[derive(Clone, uniffi::Object)]
pub struct FfiAuditCursor {
    /// Underlying SDK audit cursor.
    pub(crate) inner: AuditCursor,
}

/// Audit log filter request combining server-side filters with local channel/deletion selection.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiAuditRequest {
    /// Exact backend action filter (e.g., `"message.delete"`).
    pub action: Option<String>,
    /// Actor user UUID string filter.
    pub actor_user_id: Option<String>,
    /// Exact entity type string filter.
    pub entity_type: Option<String>,
    /// Inclusive lower RFC3339 date bound.
    pub since: Option<String>,
    /// Inclusive upper RFC3339 date bound.
    pub until: Option<String>,
    /// Optional ordering direction (server defaults to descending).
    pub order: Option<FfiHistoryOrder>,
    /// Local filter matching exact `metadata.channel_id`.
    pub channel_id: Option<String>,
    /// Local filter matching `.delete` suffix when `true` or excluding when `false`.
    pub deletions: Option<bool>,
}

/// Typed scalar or canonical string value inside audit entry metadata.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum FfiAuditMetadataValue {
    /// Null metadata field.
    Null,
    /// Boolean flag.
    Bool {
        /// Boolean payload.
        value: bool,
    },
    /// Signed 64-bit integer.
    Integer {
        /// Integer payload.
        value: i64,
    },
    /// Floating-point number.
    Float {
        /// Float payload.
        value: f64,
    },
    /// Plain string value.
    Text {
        /// String payload.
        value: String,
    },
    /// Nested array/object canonical string.
    Structured {
        /// Compact canonical JSON string of nested object/array value.
        value: String,
    },
}

/// Key/value metadata entry attached to an audit log record.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiAuditMetadataField {
    /// Metadata key name.
    pub key: String,
    /// Typed metadata field value.
    pub value: FfiAuditMetadataValue,
}

/// Single authoritative backend audit log record.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FfiAuditLogEntry {
    /// Entry UUID string.
    pub id: Option<String>,
    /// Acting user username.
    pub actor_username: Option<String>,
    /// Recorded action token.
    pub action: Option<String>,
    /// Recorded entity type token.
    pub entity_type: Option<String>,
    /// Target user UUID string if applicable.
    pub target_user_id: Option<String>,
    /// Metadata fields sorted by key.
    pub metadata: Vec<FfiAuditMetadataField>,
    /// Exact backend RFC3339 timestamp string.
    pub created_at: Option<String>,
}

impl From<&AuditLogEntry> for FfiAuditLogEntry {
    fn from(e: &AuditLogEntry) -> Self {
        let metadata = e
            .metadata
            .as_ref()
            .map(|map| {
                map.iter()
                    .map(|(k, v)| FfiAuditMetadataField {
                        key: k.clone(),
                        value: match v {
                            serde_json::Value::Null => FfiAuditMetadataValue::Null,
                            serde_json::Value::Bool(b) => FfiAuditMetadataValue::Bool { value: *b },
                            serde_json::Value::Number(n) => {
                                if let Some(i) = n.as_i64() {
                                    FfiAuditMetadataValue::Integer { value: i }
                                } else {
                                    FfiAuditMetadataValue::Float {
                                        value: n.as_f64().unwrap_or(0.0),
                                    }
                                }
                            }
                            serde_json::Value::String(s) => {
                                FfiAuditMetadataValue::Text { value: s.clone() }
                            }
                            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                                FfiAuditMetadataValue::Structured {
                                    value: v.to_string(),
                                }
                            }
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            id: e.id.clone(),
            actor_username: e.actor_username.clone(),
            action: e.action.clone(),
            entity_type: e.entity_type.clone(),
            target_user_id: e.target_user_id.clone(),
            metadata,
            created_at: e.created_at.clone(),
        }
    }
}

/// Paginated audit log result carrying opaque continuation cursor even when local filter hid all rows.
#[derive(Clone, uniffi::Record)]
pub struct FfiAuditPage {
    /// Connection epoch authorizing this page.
    pub generation: u64,
    /// Filtered entries in backend order.
    pub entries: Vec<FfiAuditLogEntry>,
    /// Opaque continuation cursor when more raw backend rows remain.
    pub next_cursor: Option<Arc<FfiAuditCursor>>,
}

#[uniffi::export]
impl FfiClient {
    /// Fetches one bounded raw page of audit log entries and applies requested local filters.
    ///
    /// # Errors
    /// Returns malformed UUIDs, mismatched/stale cursor, backend denial, or session errors.
    pub async fn audit_logs(
        &self,
        account: FfiAccountId,
        request: FfiAuditRequest,
        cursor: Option<Arc<FfiAuditCursor>>,
    ) -> Result<FfiAuditPage, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let actor = request
            .actor_user_id
            .as_deref()
            .map(parse_uuid)
            .transpose()
            .map_err(|_| FfiOperationError::InvalidInput)?;
        let channel = request
            .channel_id
            .as_deref()
            .map(parse_uuid)
            .transpose()
            .map_err(|_| FfiOperationError::InvalidInput)?;
        let sdk_req = AuditRequest {
            action: request.action,
            actor,
            entity_type: request.entity_type,
            since: request.since,
            until: request.until,
            order: request.order.map(Into::into),
            channel,
            deletions: request.deletions,
        };
        let sdk_cursor = cursor.map(|c| c.inner.clone());
        self.run_scoped(closed_operation_error, async move {
            let g = session.operations().audit_logs(sdk_req, sdk_cursor).await?;
            Ok(FfiAuditPage {
                generation: g.generation.get(),
                entries: g.value.entries.iter().map(Into::into).collect(),
                next_cursor: g.value.next.map(|c| Arc::new(FfiAuditCursor { inner: c })),
            })
        })
        .await
    }
}
