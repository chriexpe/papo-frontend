//! Bounded in-flight reaction/pin replay when the gateway omits channel identity.
//! Only canonical page payloads establish ownership; this journal never creates channel rows.

use std::sync::{Arc, Mutex};

use papo_api::gateway::KnownEvent;

use crate::Generation;

use super::{HistoryError, model::Model};

const CAPACITY: usize = 1024;

#[derive(Default)]
pub(super) struct Journal {
    inner: Mutex<Pending>,
}

#[derive(Default)]
struct Pending {
    generation: Option<Generation>,
    active: usize,
    events: Vec<KnownEvent>,
    overflowed: bool,
}

pub(super) struct Request {
    journal: Arc<Journal>,
    generation: Generation,
    start: usize,
}

impl Journal {
    pub fn request(self: &Arc<Self>, generation: Generation) -> Request {
        let mut pending = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending
            .generation
            .is_some_and(|current| current > generation)
        {
            return Request {
                journal: Arc::clone(self),
                generation,
                start: 0,
            };
        }
        if pending.generation != Some(generation) || pending.active == 0 {
            *pending = Pending {
                generation: Some(generation),
                ..Pending::default()
            };
        }
        pending.active += 1;
        Request {
            journal: Arc::clone(self),
            generation,
            start: pending.events.len(),
        }
    }

    pub fn record(&self, generation: Generation, event: &KnownEvent) {
        let mut pending = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.generation == Some(generation) && pending.active > 0 {
            if pending.events.len() == CAPACITY {
                pending.overflowed = true;
            } else {
                pending.events.push(event.clone());
            }
        }
    }
}

impl Request {
    /// Replay only IDs whose ownership is now proved by this model. Holding the channel
    /// state lock orders replay with normal live mutation; absolute totals are idempotent.
    pub fn replay(&self, model: &mut Model) -> Result<(), HistoryError> {
        let pending = self
            .journal
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.generation != Some(self.generation) {
            return Err(crate::ClientError::Superseded.into());
        }
        if pending.overflowed {
            // Refuse a stale page instead of silently losing a mutation under load.
            return Err(HistoryError::InvalidPage);
        }
        for event in &pending.events[self.start..] {
            if let Some(id) = message_id(event)?
                && model.entries.contains_key(&id)
            {
                model.event(event, self.generation.get())?;
            }
        }
        Ok(())
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        let mut pending = self
            .journal
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.generation == Some(self.generation) {
            pending.active -= 1;
            if pending.active == 0 {
                pending.events.clear();
                pending.overflowed = false;
            }
        }
    }
}

pub(super) fn message_id(event: &KnownEvent) -> Result<Option<[u8; 16]>, HistoryError> {
    match event {
        KnownEvent::ReactUpdate(event) => Model::id(&event.message_id).map(Some),
        KnownEvent::MessagePin(event) => Model::id(&event.message_id).map(Some),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests;
