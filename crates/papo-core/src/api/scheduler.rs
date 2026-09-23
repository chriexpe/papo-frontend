use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::net::RefreshTicket;

const FAIRNESS_BURST: usize = 4;
const RETRY_BACKOFF: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TaskOwner {
    pub generation: u64,
    pub session_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ReconcileKey {
    ChannelHistory(String),
    ServerMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReconcilePriority {
    Background,
    ActiveServer,
    UserRequested,
    Visible,
}

#[derive(Debug, Clone)]
pub(crate) enum ReconcileKind {
    ChannelHistory { ticket: RefreshTicket },
    ServerMetadata { user_id: Option<String> },
}

#[derive(Debug, Clone)]
pub(crate) struct ReconcileRequest {
    pub owner: TaskOwner,
    pub priority: ReconcilePriority,
    pub kind: ReconcileKind,
}

impl ReconcileRequest {
    pub fn key(&self) -> ReconcileKey {
        match &self.kind {
            ReconcileKind::ChannelHistory { ticket } => {
                ReconcileKey::ChannelHistory(ticket.channel_id.clone())
            }
            ReconcileKind::ServerMetadata { .. } => ReconcileKey::ServerMetadata,
        }
    }

    fn request_id(&self) -> Option<u64> {
        match &self.kind {
            ReconcileKind::ChannelHistory { ticket } => Some(ticket.request_id),
            ReconcileKind::ServerMetadata { .. } => None,
        }
    }
}

#[derive(Debug)]
struct Queued {
    request: ReconcileRequest,
    sequence: u64,
    not_before: Instant,
}

#[derive(Debug)]
struct Running {
    key: ReconcileKey,
    owner: TaskOwner,
    request_id: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct StartedReconcile {
    pub run_id: u64,
    pub request: ReconcileRequest,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SchedulerStats {
    pub queued: usize,
    pub in_flight: usize,
}

#[derive(Debug, Default)]
pub(crate) struct SubmitResult {
    pub accepted: bool,
    pub abort_run_ids: Vec<u64>,
}

pub(crate) struct ReconcileScheduler {
    queued: Vec<Queued>,
    running: HashMap<u64, Running>,
    active_by_key: HashMap<ReconcileKey, u64>,
    retry_until: HashMap<ReconcileKey, Instant>,
    max_in_flight: usize,
    next_run_id: u64,
    next_sequence: u64,
    priority_streak: usize,
}

impl ReconcileScheduler {
    pub fn new(max_in_flight: usize) -> Self {
        assert!(max_in_flight > 0);
        Self {
            queued: Vec::new(),
            running: HashMap::new(),
            active_by_key: HashMap::new(),
            retry_until: HashMap::new(),
            max_in_flight,
            next_run_id: 0,
            next_sequence: 0,
            priority_streak: 0,
        }
    }

    pub fn stats(&self) -> SchedulerStats {
        SchedulerStats {
            queued: self.queued.len(),
            in_flight: self.running.len(),
        }
    }

    pub fn submit(&mut self, mut request: ReconcileRequest, now: Instant) -> SubmitResult {
        let key = request.key();

        if let Some(index) = self
            .queued
            .iter()
            .position(|queued| queued.request.key() == key)
        {
            let queued = &mut self.queued[index];
            if Self::supersedes(&request, &queued.request) {
                request.priority = request.priority.max(queued.request.priority);
                let sequence = queued.sequence;
                let not_before = queued.not_before;
                *queued = Queued {
                    request,
                    sequence,
                    not_before,
                };
                log::debug!("reconcile scheduler: superseded queued {key:?}");
                return SubmitResult {
                    accepted: true,
                    abort_run_ids: Vec::new(),
                };
            }

            if Self::equivalent(&request, &queued.request) {
                queued.request.priority = queued.request.priority.max(request.priority);
                log::debug!("reconcile scheduler: coalesced queued {key:?}");
            }
            return SubmitResult::default();
        }

        if let Some(run_id) = self.active_by_key.get(&key).copied() {
            let Some(running) = self.running.get(&run_id) else {
                self.active_by_key.remove(&key);
                return self.submit(request, now);
            };

            if Self::supersedes_running(&request, running) {
                self.active_by_key.remove(&key);
                self.running.remove(&run_id);
                self.enqueue(request, now);
                log::debug!("reconcile scheduler: superseded running {key:?} run={run_id}");
                return SubmitResult {
                    accepted: true,
                    abort_run_ids: vec![run_id],
                };
            }

            if Self::equivalent_running(&request, running) {
                log::debug!("reconcile scheduler: coalesced running {key:?} run={run_id}");
            }
            return SubmitResult::default();
        }

        self.enqueue(request, now);
        SubmitResult {
            accepted: true,
            abort_run_ids: Vec::new(),
        }
    }

    fn enqueue(&mut self, request: ReconcileRequest, now: Instant) {
        let key = request.key();
        let not_before = self.retry_until.get(&key).copied().unwrap_or(now);
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.queued.push(Queued {
            request,
            sequence: self.next_sequence,
            not_before,
        });
        log::debug!("reconcile scheduler: queued {key:?}");
    }

    pub fn start_ready(&mut self, now: Instant) -> Vec<StartedReconcile> {
        let mut started = Vec::new();

        while self.running.len() < self.max_in_flight {
            let Some(index) = self.next_ready_index(now) else {
                break;
            };

            let queued = self.queued.remove(index);
            let key = queued.request.key();
            self.next_run_id = self.next_run_id.saturating_add(1);
            let run_id = self.next_run_id;
            self.active_by_key.insert(key.clone(), run_id);
            self.running.insert(
                run_id,
                Running {
                    key: key.clone(),
                    owner: queued.request.owner,
                    request_id: queued.request.request_id(),
                },
            );
            log::debug!("reconcile scheduler: started {key:?} run={run_id}");
            started.push(StartedReconcile {
                run_id,
                request: queued.request,
            });
        }

        started
    }

    fn next_ready_index(&mut self, now: Instant) -> Option<usize> {
        let ready: Vec<usize> = self
            .queued
            .iter()
            .enumerate()
            .filter_map(|(index, queued)| (queued.not_before <= now).then_some(index))
            .collect();
        if ready.is_empty() {
            return None;
        }

        let oldest = *ready
            .iter()
            .min_by_key(|index| self.queued[**index].sequence)
            .expect("ready is non-empty");

        let highest = *ready
            .iter()
            .max_by_key(|index| {
                let queued = &self.queued[**index];
                (queued.request.priority, std::cmp::Reverse(queued.sequence))
            })
            .expect("ready is non-empty");

        let index = if self.priority_streak >= FAIRNESS_BURST {
            self.priority_streak = 0;
            oldest
        } else {
            let selected_priority = self.queued[highest].request.priority;
            let has_lower = ready
                .iter()
                .any(|index| self.queued[*index].request.priority < selected_priority);
            if has_lower {
                self.priority_streak = self.priority_streak.saturating_add(1);
            } else {
                self.priority_streak = 0;
            }
            highest
        };

        Some(index)
    }

    pub fn next_ready_at(&self) -> Option<Instant> {
        if self.running.len() >= self.max_in_flight {
            return None;
        }
        self.queued.iter().map(|queued| queued.not_before).min()
    }

    pub fn complete(&mut self, run_id: u64, success: bool, now: Instant) -> bool {
        let Some(running) = self.running.remove(&run_id) else {
            return false;
        };
        let current = self.active_by_key.get(&running.key) == Some(&run_id);
        if current {
            self.active_by_key.remove(&running.key);
            if success {
                self.retry_until.remove(&running.key);
            } else {
                self.retry_until
                    .insert(running.key.clone(), now + RETRY_BACKOFF);
            }
        }
        log::debug!(
            "reconcile scheduler: completed {:?} run={} current={} success={}",
            running.key,
            run_id,
            current,
            success
        );
        current
    }

    pub fn invalidate_owner(&mut self, owner: TaskOwner) -> Vec<u64> {
        self.queued.retain(|queued| queued.request.owner == owner);
        self.retry_until.clear();

        let stale: Vec<u64> = self
            .running
            .iter()
            .filter_map(|(run_id, running)| (running.owner != owner).then_some(*run_id))
            .collect();

        for run_id in &stale {
            if let Some(running) = self.running.remove(run_id)
                && self.active_by_key.get(&running.key) == Some(run_id)
            {
                self.active_by_key.remove(&running.key);
            }
        }

        stale
    }

    pub fn cancel_all(&mut self) -> Vec<u64> {
        self.queued.clear();
        self.retry_until.clear();
        self.active_by_key.clear();
        let running = self.running.keys().copied().collect();
        self.running.clear();
        running
    }

    fn supersedes(incoming: &ReconcileRequest, existing: &ReconcileRequest) -> bool {
        if incoming.owner != existing.owner {
            return true;
        }
        match (&incoming.kind, &existing.kind) {
            (
                ReconcileKind::ChannelHistory { ticket: incoming },
                ReconcileKind::ChannelHistory { ticket: existing },
            ) => incoming.request_id > existing.request_id,
            (ReconcileKind::ServerMetadata { .. }, ReconcileKind::ServerMetadata { .. }) => false,
            _ => false,
        }
    }

    fn equivalent(incoming: &ReconcileRequest, existing: &ReconcileRequest) -> bool {
        incoming.owner == existing.owner
            && incoming.key() == existing.key()
            && incoming.request_id() == existing.request_id()
    }

    fn supersedes_running(incoming: &ReconcileRequest, existing: &Running) -> bool {
        if incoming.owner != existing.owner {
            return true;
        }
        match &incoming.kind {
            ReconcileKind::ChannelHistory { ticket } => existing
                .request_id
                .is_some_and(|request_id| ticket.request_id > request_id),
            ReconcileKind::ServerMetadata { .. } => false,
        }
    }

    fn equivalent_running(incoming: &ReconcileRequest, existing: &Running) -> bool {
        incoming.owner == existing.owner
            && incoming.key() == existing.key
            && incoming.request_id() == existing.request_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(generation: u64) -> TaskOwner {
        TaskOwner {
            generation,
            session_epoch: 1,
        }
    }

    fn channel(channel_id: &str, request_id: u64, priority: ReconcilePriority) -> ReconcileRequest {
        ReconcileRequest {
            owner: owner(3),
            priority,
            kind: ReconcileKind::ChannelHistory {
                ticket: RefreshTicket {
                    channel_id: channel_id.to_owned(),
                    generation: 3,
                    request_id,
                },
            },
        }
    }

    fn metadata(priority: ReconcilePriority) -> ReconcileRequest {
        ReconcileRequest {
            owner: owner(3),
            priority,
            kind: ReconcileKind::ServerMetadata {
                user_id: Some("eu".to_owned()),
            },
        }
    }

    #[test]
    fn single_flight_channel_reconciliation() {
        let now = Instant::now();
        let mut scheduler = ReconcileScheduler::new(2);
        assert!(scheduler.submit(channel("geral", 10, ReconcilePriority::Visible), now).accepted);
        assert!(!scheduler.submit(channel("geral", 10, ReconcilePriority::Visible), now).accepted);
        assert!(!scheduler.submit(channel("geral", 10, ReconcilePriority::Visible), now).accepted);
        assert_eq!(scheduler.stats().queued, 1);

        let started = scheduler.start_ready(now);
        assert_eq!(started.len(), 1);
        assert_eq!(scheduler.stats().in_flight, 1);
        assert!(!scheduler.submit(channel("geral", 10, ReconcilePriority::Visible), now).accepted);
    }

    #[test]
    fn newer_ticket_supersedes_older_ticket() {
        let now = Instant::now();
        let mut scheduler = ReconcileScheduler::new(2);
        scheduler.submit(channel("geral", 10, ReconcilePriority::Visible), now);
        let first = scheduler.start_ready(now).pop().expect("first run");

        let result = scheduler.submit(channel("geral", 11, ReconcilePriority::Visible), now);
        assert!(result.accepted);
        assert_eq!(result.abort_run_ids, vec![first.run_id]);
        assert_eq!(scheduler.stats().queued, 1);

        assert!(!scheduler.complete(first.run_id, true, now));
        let second = scheduler.start_ready(now);
        assert_eq!(second.len(), 1);
        match &second[0].request.kind {
            ReconcileKind::ChannelHistory { ticket } => assert_eq!(ticket.request_id, 11),
            _ => panic!("expected channel history"),
        }
    }

    #[test]
    fn priority_orders_visible_before_metadata() {
        let now = Instant::now();
        let mut scheduler = ReconcileScheduler::new(1);
        scheduler.submit(metadata(ReconcilePriority::ActiveServer), now);
        scheduler.submit(channel("geral", 1, ReconcilePriority::Visible), now);

        let started = scheduler.start_ready(now);
        assert!(matches!(
            started[0].request.kind,
            ReconcileKind::ChannelHistory { .. }
        ));
    }

    #[test]
    fn concurrency_is_bounded_and_completion_releases_capacity() {
        let now = Instant::now();
        let mut scheduler = ReconcileScheduler::new(2);
        scheduler.submit(channel("a", 1, ReconcilePriority::Visible), now);
        scheduler.submit(channel("b", 2, ReconcilePriority::Visible), now);
        scheduler.submit(channel("c", 3, ReconcilePriority::Visible), now);

        let first = scheduler.start_ready(now);
        assert_eq!(first.len(), 2);
        assert_eq!(scheduler.stats().in_flight, 2);
        assert_eq!(scheduler.start_ready(now).len(), 0);

        assert!(scheduler.complete(first[0].run_id, true, now));
        let next = scheduler.start_ready(now);
        assert_eq!(next.len(), 1);
        assert!(scheduler.stats().in_flight <= 2);
    }

    #[test]
    fn obsolete_generation_queued_work_is_dropped() {
        let now = Instant::now();
        let mut scheduler = ReconcileScheduler::new(1);
        scheduler.submit(channel("geral", 1, ReconcilePriority::Visible), now);
        let stale_runs = scheduler.invalidate_owner(TaskOwner {
            generation: 4,
            session_epoch: 1,
        });

        assert!(stale_runs.is_empty());
        assert_eq!(scheduler.stats().queued, 0);
        assert!(scheduler.start_ready(now).is_empty());
    }

    #[test]
    fn obsolete_in_flight_completion_is_not_current() {
        let now = Instant::now();
        let mut scheduler = ReconcileScheduler::new(1);
        scheduler.submit(channel("geral", 1, ReconcilePriority::Visible), now);
        let run = scheduler.start_ready(now).pop().expect("run");
        let stale_runs = scheduler.invalidate_owner(TaskOwner {
            generation: 4,
            session_epoch: 1,
        });
        assert_eq!(stale_runs, vec![run.run_id]);
        assert!(!scheduler.complete(run.run_id, true, now));
    }

    #[test]
    fn independent_servers_have_independent_budgets() {
        let now = Instant::now();
        let mut a = ReconcileScheduler::new(1);
        let mut b = ReconcileScheduler::new(1);
        a.submit(channel("a", 1, ReconcilePriority::Visible), now);
        a.submit(channel("a2", 2, ReconcilePriority::Visible), now);
        b.submit(channel("b", 1, ReconcilePriority::Visible), now);

        assert_eq!(a.start_ready(now).len(), 1);
        assert_eq!(a.stats().queued, 1);
        assert_eq!(b.start_ready(now).len(), 1);
    }

    #[test]
    fn metadata_is_single_flight() {
        let now = Instant::now();
        let mut scheduler = ReconcileScheduler::new(2);
        assert!(scheduler.submit(metadata(ReconcilePriority::ActiveServer), now).accepted);
        assert!(!scheduler.submit(metadata(ReconcilePriority::ActiveServer), now).accepted);
        let run = scheduler.start_ready(now).pop().expect("metadata");
        assert!(!scheduler.submit(metadata(ReconcilePriority::ActiveServer), now).accepted);
        assert!(scheduler.complete(run.run_id, true, now));
    }

    #[test]
    fn failed_refresh_is_retryable_without_immediate_spin() {
        let now = Instant::now();
        let mut scheduler = ReconcileScheduler::new(1);
        scheduler.submit(channel("geral", 1, ReconcilePriority::Visible), now);
        let run = scheduler.start_ready(now).pop().expect("run");
        assert!(scheduler.complete(run.run_id, false, now));

        assert!(scheduler.submit(channel("geral", 2, ReconcilePriority::Visible), now).accepted);
        assert!(scheduler.start_ready(now).is_empty());
        assert_eq!(scheduler.start_ready(now + RETRY_BACKOFF).len(), 1);
    }

    #[test]
    fn cancel_all_makes_running_work_logically_obsolete() {
        let now = Instant::now();
        let mut scheduler = ReconcileScheduler::new(1);
        scheduler.submit(channel("geral", 1, ReconcilePriority::Visible), now);
        let run = scheduler.start_ready(now).pop().expect("run");
        assert_eq!(scheduler.cancel_all(), vec![run.run_id]);
        assert!(!scheduler.complete(run.run_id, true, now));
        assert_eq!(scheduler.stats().queued, 0);
    }
}
