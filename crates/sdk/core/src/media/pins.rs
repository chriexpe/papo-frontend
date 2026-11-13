//! In-flight encoded-file protection; reference authorization and bounded read admission live above.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// Admission happens under the media gate; releasing a pin never waits for that gate.
#[derive(Clone, Default)]
pub(super) struct ReadPins(Arc<Mutex<BTreeMap<[u8; 32], usize>>>);

pub(super) struct ReadPin {
    pins: ReadPins,
    digest: [u8; 32],
}
impl ReadPins {
    pub(super) fn pin(&self, digest: [u8; 32]) -> ReadPin {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(digest)
            .or_default() += 1;
        ReadPin {
            pins: self.clone(),
            digest,
        }
    }
    pub(super) fn contains(&self, digest: &[u8; 32]) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(digest)
    }
    pub(super) fn snapshot(&self) -> Vec<[u8; 32]> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .copied()
            .collect()
    }
}
impl Drop for ReadPin {
    fn drop(&mut self) {
        let mut pins = self
            .pins
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(count) = pins.get_mut(&self.digest) {
            *count -= 1;
            if *count == 0 {
                pins.remove(&self.digest);
            }
        }
    }
}
