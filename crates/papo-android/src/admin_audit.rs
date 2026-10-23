//! Both frontends use core's audit filtering and sentence construction.
use crate::bindings::{Backend, CoreError, NativeClient};
use papo_core::{
    api::models,
    audit::{self, AuditLanguage, Filter, Kind, Period, Piece},
    state::Store,
};

#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct AuditFilter {
    pub person: String,
    pub kind: String,
    pub channel_id: String,
    pub period: String,
    pub cursor: String,
    pub portuguese: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AuditPart {
    pub kind: String,
    pub text: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AuditEntryView {
    pub id: String,
    pub actor_id: String,
    pub target_id: String,
    pub channel_id: String,
    pub action: String,
    pub parts: Vec<AuditPart>,
    pub deleted_text: String,
    pub at: i64,
    pub day: String,
    pub time: String,
    pub danger: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AuditPage {
    pub entries: Vec<AuditEntryView>,
    pub cursor: String,
    pub has_more: bool,
}

impl AuditFilter {
    fn core(&self) -> Result<Filter, String> {
        Ok(Filter {
            person: self.person.clone(),
            channel: (!self.channel_id.is_empty()).then(|| self.channel_id.clone()),
            kind: match self.kind.as_str() {
                "" | "all" => Kind::All,
                "deleted" => Kind::Deleted,
                "messages" => Kind::Messages,
                "channels" => Kind::Channels,
                "roles" => Kind::Roles,
                "members" => Kind::Members,
                "server" => Kind::Server,
                _ => return Err("Choose a valid audit filter".into()),
            },
            period: match self.period.as_str() {
                "" | "all" => Period::All,
                "today" => Period::Today,
                "week" => Period::Week,
                "month" => Period::Month,
                _ => return Err("Choose a valid date filter".into()),
            },
        })
    }
}
fn page(
    s: &Store,
    filter: &Filter,
    logs: &[models::AuditLogEntry],
    has_more: bool,
    language: AuditLanguage,
) -> AuditPage {
    let now = chrono::Local::now();
    let person = |id: &str| s.member(id).map(|p| p.name.clone());
    let channel = |id: &str| s.channel(id).map(|c| c.name.clone());
    let names = audit::Names {
        person: &person,
        channel: &channel,
    };
    let entries = logs
        .iter()
        .filter(|entry| {
            filter.keeps(
                entry,
                &|id| s.member(id).map(|p| (p.name.clone(), p.username.clone())),
                now,
            )
        })
        .map(|entry| AuditEntryView {
            id: entry.id.clone(),
            actor_id: entry.actor_id.clone().unwrap_or_default(),
            target_id: entry.target().unwrap_or_default().into(),
            channel_id: entry.channel().unwrap_or_default().into(),
            action: entry.action.clone(),
            parts: audit::sentence(entry, language, &names)
                .into_iter()
                .map(|piece| match piece {
                    Piece::Person(text) => AuditPart {
                        kind: "person".into(),
                        text,
                    },
                    Piece::Channel(text) => AuditPart {
                        kind: "channel".into(),
                        text,
                    },
                    Piece::Text(text) => AuditPart {
                        kind: "text".into(),
                        text,
                    },
                })
                .collect(),
            deleted_text: audit::deleted_text(entry).unwrap_or_default().into(),
            at: entry
                .created_at
                .map(|at| at.timestamp_millis())
                .unwrap_or_default(),
            day: entry
                .created_at
                .map(|at| audit::day_label(at, now, language))
                .unwrap_or_else(|| "Unknown date".into()),
            time: entry
                .created_at
                .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
                .unwrap_or_default(),
            danger: audit::is_danger(&entry.action),
        })
        .collect();
    AuditPage {
        entries,
        cursor: logs.last().map(|e| e.id.clone()).unwrap_or_default(),
        has_more: has_more && !logs.is_empty(),
    }
}
#[uniffi::export]
impl NativeClient {
    pub fn administration_audit(
        &self,
        owner: String,
        filter: AuditFilter,
    ) -> Result<AuditPage, CoreError> {
        let core = filter.core()?;
        let language = if filter.portuguese {
            AuditLanguage::Portuguese
        } else {
            AuditLanguage::English
        };
        self.inspect_admin(&owner, |s| {
            if s.can_manage_server() {
                Ok(())
            } else {
                Err("You don’t have permission to view the audit log".into())
            }
        })?;
        {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            if let Some(Backend::Demo(c)) = backend.as_ref() {
                let start = if filter.cursor.is_empty() {
                    0
                } else {
                    c.store
                        .audit_logs
                        .iter()
                        .position(|e| e.id == filter.cursor)
                        .map_or(c.store.audit_logs.len(), |i| i + 1)
                };
                let end = (start + 25).min(c.store.audit_logs.len());
                return Ok(page(
                    &c.store,
                    &core,
                    &c.store.audit_logs[start..end],
                    end < c.store.audit_logs.len(),
                    language,
                ));
            }
        }
        let request = self.scoped_api(&owner)?;
        let mut query = core.query(chrono::Local::now());
        query.last_id = (!filter.cursor.is_empty()).then_some(filter.cursor);
        let logs = crate::account::run(request.api.audit_logs(&query))?;
        self.finish_request(&request, vec![])?;
        self.inspect_admin(&owner, |s| {
            Ok(page(s, &core, &logs.logs, logs.has_more, language))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filtered_empty_pages_keep_the_raw_cursor_to_reach_older_matches() {
        let client = NativeClient::demo();
        for index in 0..26 {
            client
                .administer(
                    "me".into(),
                    crate::administration::AdministrationChange::CreateRole {
                        name: format!("Role {index}"),
                        color: String::new(),
                        permissions: crate::discovery::Capabilities::default(),
                    },
                )
                .unwrap();
        }
        let first = client
            .administration_audit(
                "me".into(),
                AuditFilter {
                    kind: "deleted".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(first.entries.is_empty());
        assert!(first.has_more);
        assert!(!first.cursor.is_empty());
        let next = client
            .administration_audit(
                "me".into(),
                AuditFilter {
                    kind: "deleted".into(),
                    cursor: first.cursor.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(next.entries.is_empty());
        assert!(!next.has_more);
        assert_ne!(next.cursor, first.cursor);
    }
}
