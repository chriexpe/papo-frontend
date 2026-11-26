//! Received media as it loads (`docs/design.md` §14.4, `docs/papo-desktop.md` §5): for each
//! attachment a view draws, its still picture (the server's thumbnail, then the original when
//! the thumbnail is smaller than the drawn size), a GIF's animation while it shows, and what is
//! kept warm once it no longer shows, bounded in decoded bytes.
//!
//! Pure: it decides which fetches to start and keeps what landed. The bytes are the SDK's cache
//! (`MediaService::fetch`), the fetching and decoding run off the UI thread in
//! `shell::received_media`, and the drawing is `message_media`'s. Views read [`MediaView`], a
//! cheap snapshot. The budgets and what stays warm are `budgets`'.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use rune::Image;

mod budgets;
mod view;
pub(crate) use budgets::Budgets;
pub(crate) use view::{
    Fetch, Job, MediaKey, MediaOf, MediaView, Want, attachment_id, decoded_bytes,
};

/// Fetches running at once. The SDK admits two media fetches per profile; more would only
/// queue there, where a fetch for a row already scrolled away can't be dropped.
const MAX_IN_FLIGHT: usize = 2;

/// How far an attachment's still has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Still {
    /// Nothing asked yet.
    Unasked,
    /// The thumbnail is coming and the original will follow.
    ThumbnailThenOriginal,
    /// The thumbnail is coming and is enough.
    ThumbnailOnly,
    /// The original is coming (after a thumbnail, or without one).
    Original,
    /// Done, or failed.
    Settled,
}

/// One attachment's progress and when it was last drawn.
struct Progress {
    still: Still,
    want: Want,
    /// The last tick a view wanted it.
    used: u64,
    /// The animation's decoded bytes, once it landed.
    animated_bytes: u64,
    /// The box's longer side in device pixels the still was asked for: a box much larger
    /// asks again.
    asked_px: f32,
    /// The animation could not be fetched or decoded: it is not asked for again.
    animation_failed: bool,
}

/// How much larger a box must grow before its picture is fetched and decoded again.
const REDECODE_GROWTH: f32 = 1.25;

/// Received media's state.
#[derive(Default)]
pub(crate) struct MediaStore {
    budgets: Budgets,
    view: MediaView,
    progress: HashMap<MediaKey, Progress>,
    /// What the shown views want now.
    wanted: HashSet<MediaKey>,
    /// GIFs the views would animate now (at least half visible, or hovered with the setting off).
    showing: Vec<MediaKey>,
    queue: VecDeque<Job>,
    in_flight: HashSet<(MediaKey, Fetch)>,
    tick: u64,
    last_version: u64,
}

impl MediaStore {
    /// A store keeping at most `budgets`.
    #[cfg(test)]
    pub(crate) fn with_budgets(budgets: Budgets) -> MediaStore {
        MediaStore {
            budgets,
            ..MediaStore::default()
        }
    }

    /// What views read.
    pub(crate) fn view(&self) -> MediaView {
        self.view.clone()
    }

    /// The shown views draw `wants` now (visible and near rows); anything else is no longer
    /// wanted. Starts what each needs and returns the fetches to run.
    pub(crate) fn want(&mut self, wants: impl IntoIterator<Item = Want>) -> Vec<Job> {
        self.tick += 1;
        let tick = self.tick;
        self.wanted.clear();
        for want in wants {
            self.wanted.insert(want.key);
            let progress = self.progress.entry(want.key).or_insert(Progress {
                still: Still::Unasked,
                want,
                used: tick,
                animated_bytes: 0,
                asked_px: 0.0,
                animation_failed: false,
            });
            progress.used = tick;
            progress.want = want;
            // A larger Image size, window or scale: the picture is asked for at the new size,
            // the one it has drawing until then.
            if progress.still == Still::Settled && want.box_px > progress.asked_px * REDECODE_GROWTH
            {
                progress.still = Still::Unasked;
            }
            if progress.still == Still::Unasked {
                progress.asked_px = want.box_px;
                progress.still = first_step(&want);
                let fetch = match progress.still {
                    Still::Original => Fetch::Original,
                    _ => Fetch::Thumbnail,
                };
                self.queue.push_back(Job {
                    key: want.key,
                    fetch,
                    box_pt: want.box_pt,
                });
            }
        }
        self.queue_animations();
        self.prune();
        self.next_jobs()
    }

    /// GIFs `showing` are at least half visible (or hovered with Animate GIFs off), most
    /// visible first: those that fit the animating budget animate. Returns the fetches to run.
    pub(crate) fn show_gifs(&mut self, showing: Vec<MediaKey>) -> Vec<Job> {
        // Reported on every scrolled frame: the same GIFs showing with nothing to ask is
        // nothing to do.
        let queued = self.queue.len();
        let same = showing == self.showing;
        self.showing = showing;
        self.queue_animations();
        if same && self.queue.len() == queued {
            return Vec::new();
        }
        self.settle_animating();
        self.prune();
        self.next_jobs()
    }

    /// Asks for the animation of each showing GIF that has none yet, is wanted, and has not
    /// failed: whichever of its row and its place came first.
    fn queue_animations(&mut self) {
        for key in self.showing.clone() {
            let Some(progress) = self.progress.get(&key) else {
                continue;
            };
            let has_animation = self
                .view
                .drawn
                .get(&key)
                .is_some_and(|drawn| drawn.animated.is_some());
            let asked = self.in_flight.contains(&(key, Fetch::Animation))
                || self
                    .queue
                    .iter()
                    .any(|job| job.key == key && job.fetch == Fetch::Animation);
            if !has_animation && !asked && !progress.animation_failed {
                self.queue.push_back(Job {
                    key,
                    fetch: Fetch::Animation,
                    box_pt: 0.0,
                });
            }
        }
    }

    /// Jobs that were handed out but never started (the SDK was not there): their slots are
    /// free again and their pictures are asked for anew.
    pub(crate) fn dropped(&mut self, jobs: &[Job]) {
        for job in jobs {
            self.in_flight.remove(&(job.key, job.fetch));
            if let Some(progress) = self.progress.get_mut(&job.key)
                && job.fetch != Fetch::Animation
                && progress.still != Still::Settled
            {
                progress.still = Still::Unasked;
            }
        }
    }

    /// Fetch `job` ended with `image` (none when it failed); `animated_bytes` is an animation's
    /// decoded size. Returns the fetches to run next.
    pub(crate) fn landed(
        &mut self,
        job: Job,
        image: Option<Image>,
        animated_bytes: u64,
    ) -> Vec<Job> {
        self.in_flight.remove(&(job.key, job.fetch));
        let Some(progress) = self.progress.get_mut(&job.key) else {
            // Pruned meanwhile: nothing draws it.
            return self.next_jobs();
        };
        let failed = image.as_ref().is_none_or(|image| image.error().is_some());
        let mut follow = None;
        match job.fetch {
            Fetch::Thumbnail => {
                if failed || progress.still == Still::ThumbnailThenOriginal {
                    // A failed thumbnail falls back on the original.
                    progress.still = Still::Original;
                    follow = Some(Job {
                        key: job.key,
                        fetch: Fetch::Original,
                        box_pt: progress.want.box_pt,
                    });
                } else {
                    progress.still = Still::Settled;
                }
            }
            Fetch::Original => progress.still = Still::Settled,
            Fetch::Animation => {
                progress.animated_bytes = animated_bytes;
                progress.animation_failed = failed;
            }
        }
        self.last_version += 1;
        let version = self.last_version;
        let drawn = Arc::make_mut(&mut self.view.drawn)
            .entry(job.key)
            .or_default();
        drawn.version = version;
        match (job.fetch, image.filter(|_| !failed)) {
            (Fetch::Animation, Some(image)) => drawn.animated = Some(image),
            (_, Some(image)) => drawn.still = Some(image),
            // A failed thumbnail waits for the original; a failed animation leaves the still.
            (Fetch::Thumbnail | Fetch::Animation, None) => {}
            (Fetch::Original, None) => drawn.failed = drawn.still.is_none(),
        }
        if let Some(follow) = follow {
            self.queue.push_back(follow);
        }
        self.settle_animating();
        self.prune();
        self.next_jobs()
    }

    /// Retry on a picture that failed: it is asked for again.
    pub(crate) fn retry(&mut self, key: MediaKey) -> Vec<Job> {
        let Some(progress) = self.progress.get_mut(&key) else {
            return Vec::new();
        };
        progress.still = Still::Unasked;
        let want = progress.want;
        if let Some(drawn) = Arc::make_mut(&mut self.view.drawn).get_mut(&key) {
            self.last_version += 1;
            drawn.failed = false;
            drawn.version = self.last_version;
        }
        let wants: Vec<Want> = self
            .wanted
            .iter()
            .filter_map(|key| self.progress.get(key).map(|progress| progress.want))
            .chain((!self.wanted.contains(&key)).then_some(want))
            .collect();
        self.want(wants)
    }

    /// The fetches to start now: queued ones still wanted, up to [`MAX_IN_FLIGHT`].
    fn next_jobs(&mut self) -> Vec<Job> {
        let mut jobs = Vec::new();
        let mut kept = VecDeque::new();
        while let Some(job) = self.queue.pop_front() {
            let still_wanted = match job.fetch {
                Fetch::Animation => self.showing.contains(&job.key),
                Fetch::Thumbnail | Fetch::Original => self.wanted.contains(&job.key),
            };
            if !still_wanted {
                // Its row left before the fetch started: it starts again if it returns.
                if let Some(progress) = self.progress.get_mut(&job.key)
                    && job.fetch != Fetch::Animation
                    && progress.still != Still::Settled
                {
                    progress.still = Still::Unasked;
                }
                continue;
            }
            // The same fetch already runs (it was asked again after being let go): it lands.
            if self.in_flight.contains(&(job.key, job.fetch))
                || jobs
                    .iter()
                    .any(|other: &Job| other.key == job.key && other.fetch == job.fetch)
            {
                continue;
            }
            if self.in_flight.len() + jobs.len() >= MAX_IN_FLIGHT {
                kept.push_back(job);
                continue;
            }
            jobs.push(job);
        }
        self.queue = kept;
        for job in &jobs {
            self.in_flight.insert((job.key, job.fetch));
        }
        jobs
    }
}

/// The first fetch for `want`: its thumbnail when it has one (then the original when the
/// thumbnail is smaller than the drawn size), else the original.
fn first_step(want: &Want) -> Still {
    match want.thumbnail_px {
        None => Still::Original,
        Some((width, height)) => {
            #[expect(clippy::cast_precision_loss, reason = "thumbnail sides are small")]
            let thumbnail_px = width.max(height) as f32;
            if thumbnail_px >= want.box_px {
                Still::ThumbnailOnly
            } else {
                Still::ThumbnailThenOriginal
            }
        }
    }
}

#[cfg(test)]
mod tests;
