//! Exact production audit cursors, fixed date filters and local empty-page continuation.

use super::{Result, is_problem};
use papo_sdk::{HistoryOrder, Session, Uuid, operations::audit::AuditRequest};

pub async fn prove(owner: &Session, member: &Session, address: &str) -> Result {
    reqwest::Client::new()
        .post(format!("{address}/__proof/operations-audit"))
        .send()
        .await?
        .error_for_status()?;
    let base = AuditRequest {
        action: Some("sdk.proof.delete".into()),
        actor: Some(owner.account().user()),
        entity_type: Some("sdk_proof".into()),
        since: Some("2026-01-01T00:00:00.123456Z".into()),
        until: Some("2026-01-01T00:00:00.123456Z".into()),
        ..Default::default()
    };
    assert!(is_problem(
        &member.operations().audit_logs(base.clone(), None).await,
        403,
        &papo_api::ProblemKind::Forbidden,
    ));
    for order in [HistoryOrder::Asc, HistoryOrder::Desc] {
        let request = AuditRequest {
            order: Some(order),
            ..base.clone()
        };
        let mut cursor = None;
        let mut records = Vec::new();
        loop {
            let page = owner
                .operations()
                .audit_logs(request.clone(), cursor)
                .await?
                .value;
            records.extend(page.entries);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(records.len(), 105);
        let ids = records
            .iter()
            .map(|entry| entry.id.as_deref().ok_or("audit identity"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert!(ids.windows(2).all(|pair| if order == HistoryOrder::Asc {
            pair[0] < pair[1]
        } else {
            pair[0] > pair[1]
        }));
    }
    let hidden = AuditRequest {
        channel: Some(Uuid::nil()),
        deletions: Some(true),
        ..base
    };
    let first = owner
        .operations()
        .audit_logs(hidden.clone(), None)
        .await?
        .value;
    assert!(first.entries.is_empty() && first.next.is_some());
    let second = owner
        .operations()
        .audit_logs(hidden, first.next)
        .await?
        .value;
    assert!(second.entries.is_empty() && second.next.is_none());
    Ok(())
}
