//! Bounded 1-live + 1-retiring session registry tracked by real drain rather than timeouts.

use crate::writer::{LeaseCheck, TileSnapshot, TileWriter, lock};
use std::{
    fmt::Write as _,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::oneshot;

pub(crate) const WAITING: i32 = 0;
pub(crate) const LIVE: i32 = 1;
pub(crate) const FAILED: i32 = -1;
pub(crate) const REFUSED_CAPACITY: i32 = -2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttachError {
    CapacityFull,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DrainedSummary {
    pub generation: u64,
    pub posts_ok: u64,
    pub posts_wrong_color: u64,
    pub posts_stale_dropped: u64,
    pub late_posts_after_cancel: u64,
}

pub(crate) struct SessionHandle<L: LeaseCheck> {
    generation: u64,
    writers: Vec<Arc<TileWriter<L>>>,
    cancel_tx: Mutex<Option<oneshot::Sender<()>>>,
    canceled: AtomicBool,
    media_drained: AtomicBool,
}

impl<L: LeaseCheck> SessionHandle<L> {
    pub(crate) fn new(
        generation: u64,
        writers: Vec<Arc<TileWriter<L>>>,
        cancel_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            generation,
            writers,
            cancel_tx: Mutex::new(Some(cancel_tx)),
            canceled: AtomicBool::new(false),
            media_drained: AtomicBool::new(false),
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }

    pub(crate) fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
        if let Some(tx) = lock(&self.cancel_tx).take() {
            let _ = tx.send(());
        }
        for writer in &self.writers {
            writer.cancel();
        }
    }

    pub(crate) fn mark_media_drained(&self) {
        self.media_drained.store(true, Ordering::Release);
    }

    pub(crate) fn is_media_drained(&self) -> bool {
        self.media_drained.load(Ordering::Acquire)
    }

    pub(crate) fn is_writers_drained(&self) -> bool {
        self.writers.iter().all(|writer| writer.is_drained())
    }

    pub(crate) fn wait_writers_drained(&self) {
        for writer in &self.writers {
            writer.wait_drained();
        }
    }

    pub(crate) fn is_fully_drained(&self) -> bool {
        self.is_media_drained() && self.is_writers_drained()
    }

    pub(crate) fn tile_snapshots(&self) -> Vec<TileSnapshot> {
        self.writers
            .iter()
            .enumerate()
            .map(|(index, writer)| writer.snapshot(index))
            .collect()
    }
}

pub(crate) struct SessionRegistry<L: LeaseCheck> {
    live: Option<Arc<SessionHandle<L>>>,
    retiring: Option<Arc<SessionHandle<L>>>,
    attaching: bool,
    next_gen: u64,
    total_drained_sessions: u64,
    last_drained: Option<DrainedSummary>,
    capacity_refusals: u64,
}

impl<L: LeaseCheck> SessionRegistry<L> {
    pub(crate) const fn new() -> Self {
        Self {
            live: None,
            retiring: None,
            attaching: false,
            next_gen: 1,
            total_drained_sessions: 0,
            last_drained: None,
            capacity_refusals: 0,
        }
    }

    pub(crate) fn reap(&mut self) {
        if self
            .retiring
            .as_ref()
            .is_some_and(|session| session.is_fully_drained())
            && let Some(done) = self.retiring.take()
        {
            self.record_drained(&done);
        }
        if self
            .live
            .as_ref()
            .is_some_and(|session| session.is_canceled())
        {
            let drained = self
                .live
                .as_ref()
                .is_some_and(|session| session.is_fully_drained());
            if drained && let Some(done) = self.live.take() {
                self.record_drained(&done);
            } else if self.retiring.is_none() {
                self.retiring = self.live.take();
            }
        }
    }

    fn record_drained(&mut self, session: &SessionHandle<L>) {
        self.total_drained_sessions += 1;
        let snaps = session.tile_snapshots();
        let mut summary = DrainedSummary {
            generation: session.generation(),
            ..DrainedSummary::default()
        };
        for snap in snaps {
            summary.posts_ok += snap.posts_ok;
            summary.posts_wrong_color += snap.posts_wrong_color;
            summary.posts_stale_dropped += snap.posts_stale_dropped;
            summary.late_posts_after_cancel += snap.late_posts_after_cancel;
        }
        self.last_drained = Some(summary);
    }

    pub(crate) fn begin_attach(&mut self) -> Result<u64, AttachError> {
        self.reap();
        if self.attaching || (self.live.is_some() && self.retiring.is_some()) {
            self.capacity_refusals += 1;
            return Err(AttachError::CapacityFull);
        }
        self.attaching = true;
        let generation = self.next_gen;
        self.next_gen += 1;
        Ok(generation)
    }

    pub(crate) fn abort_attach(&mut self) {
        self.attaching = false;
    }

    pub(crate) fn finish_attach(&mut self, session: Arc<SessionHandle<L>>) {
        self.reap();
        self.attaching = false;
        if let Some(old) = self.live.take() {
            old.cancel();
            if old.is_fully_drained() {
                self.record_drained(&old);
            } else {
                // begin_attach refuses when live and retiring are both occupied, and attaches serialize on `attaching`.
                self.retiring = Some(old);
            }
        }
        self.live = Some(session);
    }

    pub(crate) fn detach(&mut self, generation: Option<u64>) {
        self.reap();
        let matches = self
            .live
            .as_ref()
            .is_some_and(|live| generation.is_none_or(|g| live.generation() == g));
        if matches && let Some(live) = self.live.take() {
            live.cancel();
            if live.is_fully_drained() {
                self.record_drained(&live);
            } else if self.retiring.is_none() {
                self.retiring = Some(live);
            } else {
                // Both retiring and canceled-live stay bounded until retiring finishes draining.
                self.live = Some(live);
            }
        }
    }

    pub(crate) fn find_generation(&mut self, generation: u64) -> Option<Arc<SessionHandle<L>>> {
        self.reap();
        if let Some(live) = &self.live
            && live.generation() == generation
        {
            return Some(Arc::clone(live));
        }
        if let Some(retiring) = &self.retiring
            && retiring.generation() == generation
        {
            return Some(Arc::clone(retiring));
        }
        None
    }

    pub(crate) fn arm_stall_live(&mut self) -> bool {
        self.reap();
        let Some(live) = &self.live else {
            return false;
        };
        if live.is_canceled() {
            return false;
        }
        live.writers.first().is_some_and(|writer| writer.arm())
    }

    pub(crate) fn release_all_stalls(&mut self) {
        self.reap();
        for session in [self.live.as_ref(), self.retiring.as_ref()]
            .into_iter()
            .flatten()
        {
            for writer in &session.writers {
                writer.release();
            }
        }
    }

    pub(crate) fn overall_status(&mut self) -> i32 {
        self.reap();
        let Some(live) = &self.live else {
            return WAITING;
        };
        if live.is_canceled() {
            return WAITING;
        }
        let snaps = live.tile_snapshots();
        if snaps.iter().any(|snap| snap.status == FAILED) {
            return FAILED;
        }
        if !snaps.is_empty() && snaps.iter().all(|snap| snap.status == LIVE) {
            LIVE
        } else {
            WAITING
        }
    }

    pub(crate) fn tile_status(&mut self, index: usize) -> i32 {
        self.reap();
        let Some(live) = &self.live else {
            return WAITING;
        };
        if live.is_canceled() {
            return WAITING;
        }
        live.writers
            .get(index)
            .map_or(WAITING, |writer| writer.snapshot(index).status)
    }

    pub(crate) fn telemetry_json(&mut self, fence_accepted: u64, fence_rejected: u64) -> String {
        let overall_status = self.overall_status();
        let live_generation = self
            .live
            .as_ref()
            .filter(|s| !s.is_canceled())
            .map_or(0, |s| s.generation());
        let retiring_generation = self.retiring.as_ref().map_or(0, |s| s.generation());
        let retiring_snaps = self
            .retiring
            .as_ref()
            .map_or_else(Vec::new, |s| s.tile_snapshots());
        let retiring_stalled = retiring_snaps.iter().any(|t| t.stalled);
        let tiles = self
            .live
            .as_ref()
            .filter(|s| !s.is_canceled())
            .map_or_else(Vec::new, |s| s.tile_snapshots());

        let mut out = String::from("{");
        let _ = write!(
            out,
            "\"overall_status\":{overall_status},\"live_generation\":{live_generation},\
             \"retiring_generation\":{retiring_generation},\"total_drained_sessions\":{},\
             \"capacity_refusals\":{},\"fence_accepted_packets\":{fence_accepted},\
             \"fence_rejected_packets\":{fence_rejected},\"retiring_stalled\":{retiring_stalled},\
             \"tiles\":[",
            self.total_drained_sessions, self.capacity_refusals,
        );
        for (i, tile) in tiles.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"index\":{},\"expected\":\"{}\",\"status\":{},\"stalled\":{},\"drained\":{},\
                 \"posts_ok\":{},\"posts_wrong_color\":{},\"posts_stale_dropped\":{},\
                 \"late_posts_after_cancel\":{},\"last_track_id\":\"{}\",\
                 \"last_assignment_id\":{},\"last_rgb\":[{},{},{}]}}",
                tile.index,
                tile.expected.as_str(),
                tile.status,
                tile.stalled,
                tile.drained,
                tile.posts_ok,
                tile.posts_wrong_color,
                tile.posts_stale_dropped,
                tile.late_posts_after_cancel,
                escape_json(&tile.last_track_id),
                tile.last_assignment_id,
                tile.last_rgb[0],
                tile.last_rgb[1],
                tile.last_rgb[2],
            );
        }
        out.push_str("],\"last_drained\":");
        if let Some(done) = &self.last_drained {
            let _ = write!(
                out,
                "{{\"generation\":{},\"posts_ok\":{},\"posts_wrong_color\":{},\
                 \"posts_stale_dropped\":{},\"late_posts_after_cancel\":{}}}",
                done.generation,
                done.posts_ok,
                done.posts_wrong_color,
                done.posts_stale_dropped,
                done.late_posts_after_cancel,
            );
        } else {
            out.push_str("null");
        }
        out.push('}');
        out
    }
}

fn escape_json(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(escaped, "\\u{:04x}", c as u32);
            }
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
