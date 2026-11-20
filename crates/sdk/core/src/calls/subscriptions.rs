//! Demand-driven video subscription planner with bounded retry backoff under backend rate limits.
//! Never predicts SFU slot indices; `voice_video_routes` owns slot attribution.

use crate::Uuid;
use papo_api::gateway::Command;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::time::Instant;

use super::{model::VideoTarget, reducer::VoiceReducer};

/// Minimum interval between consecutive `track_subscribe` commands (5/s backend limit → 200 ms).
const SUBSCRIBE_MIN_INTERVAL: Duration = Duration::from_millis(200);
/// Base backoff after a non-fatal subscription rejection (`voice-not-found`, `voice-rate-limited`, `voice-room-full`).
const RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
/// Cap on exponential retry backoff.
const RETRY_MAX_DELAY: Duration = Duration::from_secs(4);

/// Tracks UI video interest, desired subscriptions and retry timers for one active call attempt.
#[derive(Debug, Default)]
pub(crate) struct VideoSubscriptionPlanner {
    /// Whether the first `voice_answer` has been received for this attempt.
    first_answer_received: bool,
    /// Ordered, deduplicated UI video interest.
    interest: Vec<VideoTarget>,
    /// Targets we have sent `track_subscribe` for (and not yet unsubscribed).
    requested: BTreeSet<VideoTarget>,
    /// Per-target backoff deadlines after non-fatal rejections.
    retry_after: BTreeMap<VideoTarget, (u32, Instant)>,
    /// Earliest time the next `track_subscribe` may be emitted under the 5/s rate limit.
    next_subscribe_at: Option<Instant>,
}

impl VideoSubscriptionPlanner {
    /// Updates ordered UI video interest, preserving priority and deduplicating entries.
    pub(crate) fn set_interest(&mut self, targets: Vec<VideoTarget>) {
        let mut seen = BTreeSet::new();
        self.interest = targets
            .into_iter()
            .filter(|target| seen.insert(*target))
            .collect();
        self.retry_after.retain(|target, _| seen.contains(target));
    }

    /// Marks the first remote SDP answer received, unlocking `track_subscribe` commands.
    pub(crate) fn mark_first_answer_received(&mut self) {
        self.first_answer_received = true;
    }

    /// Clears backoff for every target currently confirmed in `voice_video_routes`.
    pub(crate) fn note_video_routes(&mut self, reducer: &VoiceReducer) {
        for route in reducer.video_routes().values() {
            let target = VideoTarget {
                participant: route.participant,
                kind: route.kind,
            };
            self.retry_after.remove(&target);
            self.requested.insert(target);
        }
    }

    /// Schedules bounded backoff for outstanding unrouted subscriptions after a non-fatal error.
    pub(crate) fn note_retryable_rejection(&mut self, reducer: &VoiceReducer, now: Instant) {
        let routed: BTreeSet<VideoTarget> = reducer
            .video_routes()
            .values()
            .map(|route| VideoTarget {
                participant: route.participant,
                kind: route.kind,
            })
            .collect();
        let unrouted: Vec<VideoTarget> = self
            .requested
            .iter()
            .copied()
            .filter(|target| !routed.contains(target))
            .collect();
        for target in unrouted {
            self.requested.remove(&target);
            let attempts = self
                .retry_after
                .get(&target)
                .map_or(0, |(count, _)| count.saturating_add(1));
            let shift = attempts.min(4);
            let delay = RETRY_BASE_DELAY
                .saturating_mul(1_u32 << shift)
                .min(RETRY_MAX_DELAY);
            self.retry_after.insert(target, (attempts, now + delay));
        }
    }

    /// Removes all tracking for a publisher who left the room (server already freed their slots).
    pub(crate) fn note_member_left(&mut self, user: Uuid) {
        self.requested.retain(|target| target.participant != user);
        self.retry_after
            .retain(|target, _| target.participant != user);
    }

    /// Computes immediate `track_unsubscribe` / `track_subscribe` commands to reconcile with `interest`.
    pub(crate) fn reconcile(
        &mut self,
        channel: Uuid,
        self_user: Uuid,
        reducer: &VoiceReducer,
        now: Instant,
    ) -> Vec<Command> {
        if !self.first_answer_received {
            return Vec::new();
        }
        let channel_str = channel.to_string();
        let mut commands = Vec::new();

        let desired: Vec<VideoTarget> = self
            .interest
            .iter()
            .copied()
            .filter(|target| {
                target.participant != self_user
                    && reducer.is_publishing(channel, target.participant, target.kind)
            })
            .collect();
        let desired_set: BTreeSet<VideoTarget> = desired.iter().copied().collect();

        // Unsubscribe anything we requested that is no longer wanted (only send wire unsubscribe
        // if the publisher is still publishing; stopped tracks are already released by the SFU).
        let stale: Vec<VideoTarget> = self
            .requested
            .iter()
            .copied()
            .filter(|target| !desired_set.contains(target))
            .collect();
        for target in stale {
            self.requested.remove(&target);
            self.retry_after.remove(&target);
            if reducer.is_publishing(channel, target.participant, target.kind) {
                commands.push(Command::TrackUnsubscribe {
                    channel_id: channel_str.clone(),
                    publisher_id: target.participant.to_string(),
                    kind: target.kind.to_api(),
                });
            }
        }

        // Subscribe to newly desired targets respecting backoff and the 5/s subscribe rate limit.
        for target in desired {
            if self.requested.contains(&target) {
                continue;
            }
            if let Some(&(_, deadline)) = self.retry_after.get(&target)
                && now < deadline
            {
                continue;
            }
            if let Some(next_at) = self.next_subscribe_at
                && now < next_at
            {
                break;
            }
            self.retry_after.remove(&target);
            self.requested.insert(target);
            self.next_subscribe_at = Some(now + SUBSCRIBE_MIN_INTERVAL);
            commands.push(Command::TrackSubscribe {
                channel_id: channel_str.clone(),
                publisher_id: target.participant.to_string(),
                kind: target.kind.to_api(),
            });
        }
        commands
    }

    /// Returns the earliest monotonic wakeup needed for retry backoff or rate-limit pacing.
    #[must_use]
    pub(crate) fn next_wakeup(
        &self,
        channel: Uuid,
        self_user: Uuid,
        reducer: &VoiceReducer,
    ) -> Option<Instant> {
        if !self.first_answer_received {
            return None;
        }
        let mut earliest: Option<Instant> = None;
        for &target in &self.interest {
            if target.participant == self_user
                || self.requested.contains(&target)
                || !reducer.is_publishing(channel, target.participant, target.kind)
            {
                continue;
            }
            let target_ready = self.retry_after.get(&target).map(|&(_, at)| at);
            let candidate = match (target_ready, self.next_subscribe_at) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            if let Some(at) = candidate {
                earliest = Some(earliest.map_or(at, |cur| cur.min(at)));
            }
        }
        earliest
    }
}
