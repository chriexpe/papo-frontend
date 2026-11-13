//! Bounded generation-scoped inbox evidence reuse for live triggers; policy and delivery stay in live.

use super::{NotificationError, inbox, live::record_candidate, policy::Candidate};
use crate::{Generation, Session, Uuid};
use std::collections::{BTreeMap, VecDeque};

#[derive(Default)]
pub(super) struct TriggerEvidence {
    scope: Option<(Generation, u64)>,
    records: BTreeMap<Uuid, Candidate>,
    order: VecDeque<Uuid>,
}
impl TriggerEvidence {
    pub fn clear(&mut self) {
        self.records.clear();
        self.order.clear();
    }

    fn select_scope(&mut self, generation: Generation, revision: u64) {
        if self.scope != Some((generation, revision)) {
            self.scope = Some((generation, revision));
            self.clear();
        }
    }

    fn remember(&mut self, mut candidate: Candidate) {
        candidate.trigger_revision = self.scope.map(|(_, revision)| revision);
        let id = candidate.message;
        if self.records.insert(id, candidate).is_some() {
            return;
        }
        self.order.push_back(id);
        if self.order.len() > 1000
            && let Some(oldest) = self.order.pop_front()
        {
            self.records.remove(&oldest);
        }
    }

    pub async fn resolve(
        &mut self,
        session: &Session,
        generation: Generation,
        message: Uuid,
    ) -> Result<Option<Candidate>, NotificationError> {
        session.inner.epochs.validate(generation)?;
        let revision = session
            .inner
            .notifications
            .inbox_revision
            .load(std::sync::atomic::Ordering::Acquire);
        self.select_scope(generation, revision);
        if let Some(candidate) = self.records.get(&message) {
            return Ok(Some(candidate.clone()));
        }
        // An ephemeral 'all' event is not proof of a reply. Only positive inbox evidence is reused.
        let epoch = session.inner.epochs.current();
        let rest = crate::scoped_request::authenticated(session, &epoch).await?;
        let mut cursor = None;
        for _ in 0..10 {
            let page = inbox::page(
                session,
                &rest,
                generation,
                cursor.as_ref(),
                crate::HistoryOrder::Desc,
            )
            .await?;
            let current = session
                .inner
                .notifications
                .inbox_revision
                .load(std::sync::atomic::Ordering::Acquire);
            if current != self.scope.map_or(0, |(_, revision)| revision) {
                // A completed local read invalidates even a response already waiting in flight.
                self.select_scope(generation, current);
                cursor = None;
                continue;
            }
            for row in page.records {
                let candidate = record_candidate(&row.record)?;
                self.remember(candidate);
            }
            if let Some(candidate) = self.records.get(&message) {
                return Ok(Some(candidate.clone()));
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_is_bounded_and_invalidated_by_idle_generation_or_local_reads() {
        let mut evidence = TriggerEvidence::default();
        evidence.select_scope(Generation(1), 0);
        for index in 0..1001 {
            let mut candidate = crate::notifications::tests::candidate();
            candidate.message = Uuid::from_u128(index);
            evidence.remember(candidate);
        }
        assert_eq!(evidence.records.len(), 1000);
        assert_eq!(evidence.order.len(), 1000);
        assert!(!evidence.records.contains_key(&Uuid::from_u128(0)));
        evidence.select_scope(Generation(1), 0);
        assert_eq!(evidence.records.len(), 1000);
        evidence.select_scope(Generation(1), 1);
        assert!(evidence.records.is_empty());
        evidence.remember(crate::notifications::tests::candidate());
        evidence.select_scope(Generation(2), 1);
        assert!(evidence.records.is_empty());
        evidence.remember(crate::notifications::tests::candidate());
        evidence.clear();
        assert!(evidence.records.is_empty());
        assert!(evidence.order.is_empty());
    }

    #[test]
    fn completed_read_between_lookup_and_admission_does_not_claim_stale_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let (mut client, session) = crate::notifications::tests::fixture(directory.path());
        client
            .block_on(async {
                let mut candidate = crate::notifications::tests::candidate();
                candidate.trigger_revision = Some(0);
                session
                    .inner
                    .notifications
                    .inbox_revision
                    .store(1, std::sync::atomic::Ordering::Release);
                let resources = std::sync::Arc::clone(&session.inner.resources.borrow());
                assert!(
                    super::super::live::deliver_at(
                        &session,
                        session.generation(),
                        candidate,
                        &resources,
                        0
                    )
                    .await
                    .unwrap()
                    .is_none()
                );
                let connection =
                    rusqlite::Connection::open(directory.path().join("state.sqlite")).unwrap();
                assert_eq!(
                    connection
                        .query_row("SELECT count(*) FROM notification_ledger", [], |row| row
                            .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
            })
            .unwrap();
        client.shutdown().unwrap();
    }
}
