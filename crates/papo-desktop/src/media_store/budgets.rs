//! How much received media keeps (`docs/papo-desktop.md` §5): which GIFs animate within their
//! budget, and which stills and animations no view draws stay warm, in decoded bytes, most
//! recently drawn first. The rest of the rules are `media_store`'s.

use std::collections::HashSet;
use std::sync::Arc;

use super::{MediaKey, MediaStore, Progress, decoded_bytes};

/// Decoded bytes of stills no view draws that are kept for a quick return.
const STILL_WARM_BYTES: u64 = 48 * 1024 * 1024;

/// Decoded bytes of animations that no longer animate, kept for a quick return.
const ANIMATED_WARM_BYTES: u64 = 64 * 1024 * 1024;

/// Decoded bytes all animating GIFs may take together; past it, the later ones show still.
const ANIMATING_BYTES: u64 = 256 * 1024 * 1024;

/// How many decoded bytes each set may keep.
#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::struct_field_names,
    reason = "each budget is in bytes, which its name carries"
)]
pub(crate) struct Budgets {
    pub(crate) still_warm_bytes: u64,
    pub(crate) animated_warm_bytes: u64,
    pub(crate) animating_bytes: u64,
}

impl Default for Budgets {
    fn default() -> Budgets {
        Budgets {
            still_warm_bytes: STILL_WARM_BYTES,
            animated_warm_bytes: ANIMATED_WARM_BYTES,
            animating_bytes: ANIMATING_BYTES,
        }
    }
}

impl MediaStore {
    /// Which showing GIFs animate: those with an animation, in order, while they fit.
    pub(super) fn settle_animating(&mut self) {
        let mut total = 0;
        let mut animating = HashSet::new();
        for key in &self.showing {
            let has_animation = self
                .view
                .drawn
                .get(key)
                .is_some_and(|drawn| drawn.animated.is_some());
            let bytes = self
                .progress
                .get(key)
                .map_or(0, |progress| progress.animated_bytes);
            if has_animation && total + bytes <= self.budgets.animating_bytes {
                total += bytes;
                animating.insert(*key);
            }
        }
        if *self.view.animating != animating {
            self.view.animating = Arc::new(animating);
        }
    }

    /// Releases what no view draws past the warm budgets, the least recently drawn first.
    pub(super) fn prune(&mut self) {
        let mut idle: Vec<(u64, MediaKey)> = self
            .progress
            .iter()
            .filter(|(key, _)| !self.wanted.contains(key))
            .map(|(key, progress)| (progress.used, *key))
            .collect();
        // Most recent first: they keep their place in the warm set.
        idle.sort_unstable_by_key(|(used, _)| std::cmp::Reverse(*used));
        let mut still_bytes = 0;
        let mut animated_bytes = 0;
        let mut release_still = Vec::new();
        let mut release_animation = Vec::new();
        for (_, key) in idle {
            // Never drawn, or nothing to draw (it failed): forget it, so it starts again if it
            // returns.
            let Some(still) = self
                .view
                .drawn
                .get(&key)
                .and_then(|drawn| drawn.still.as_ref())
            else {
                release_still.push(key);
                continue;
            };
            let bytes = decoded_bytes(still, 1);
            if still_bytes + bytes <= self.budgets.still_warm_bytes {
                still_bytes += bytes;
            } else {
                release_still.push(key);
            }
        }
        // Idle animations stay warm most recently drawn first.
        let mut animated: Vec<(&MediaKey, &Progress)> = self.progress.iter().collect();
        animated.sort_unstable_by_key(|(_, progress)| std::cmp::Reverse(progress.used));
        for (key, progress) in animated {
            let has_animation = self
                .view
                .drawn
                .get(key)
                .is_some_and(|drawn| drawn.animated.is_some());
            if !has_animation || self.view.animating.contains(key) {
                continue;
            }
            if animated_bytes + progress.animated_bytes <= self.budgets.animated_warm_bytes {
                animated_bytes += progress.animated_bytes;
            } else {
                release_animation.push(*key);
            }
        }
        if release_still.is_empty() && release_animation.is_empty() {
            return;
        }
        let drawn = Arc::make_mut(&mut self.view.drawn);
        for key in release_animation {
            if let Some(entry) = drawn.get_mut(&key) {
                entry.animated = None;
            }
        }
        for key in release_still {
            drawn.remove(&key);
            self.progress.remove(&key);
        }
    }
}
