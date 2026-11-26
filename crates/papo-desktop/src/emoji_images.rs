//! Custom emoji images as content draws them (`docs/design.md` §9, Emoji): each emoji's image
//! decoded once, to fit the largest box it is drawn in (jumbo at the window's scale), and kept
//! while a view still wants it, up to [`MAX_KEPT`] emoji.
//!
//! Pure: it decides which images to fetch and keeps what landed. The bytes are the SDK's
//! (`MediaService::fetch` on the emoji's source; the catalog lists them into the store), the
//! fetching and decoding run off the UI thread in `shell::emoji_images`, and the drawing is
//! `timeline::content`'s and the composer's.

use std::collections::HashMap;
use std::sync::Arc;

use std::cell::RefCell;

use papo_sdk::{AccountId, CustomEmoji, MediaSource, Uuid};
use rune::{Color, InlineImage, Pixels, TextStyle};

use crate::design::{EMOJI_LOADING_RADIUS, Look, emoji_box};

/// The most decoded emoji kept: at most `DECODE_MAX_PX`² × 4 bytes each, so about 26 MiB.
const MAX_KEPT: usize = 256;

/// The longest side an emoji is decoded to, in device pixels: jumbo (3.4 × the largest text
/// size, 16 pt) at a scale of two and a half.
pub(crate) const DECODE_MAX_PX: u32 = 160;

/// One account's emoji.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct EmojiKey {
    pub(crate) account: AccountId,
    pub(crate) id: Uuid,
}

/// What a view draws for an emoji now.
#[derive(Clone, Debug)]
pub(crate) enum EmojiLook {
    /// Not here yet: its square is reserved.
    Loading,
    /// Its image, decoded; `aspect` is width over height.
    Ready { pixels: Pixels, aspect: f32 },
    /// It could not be fetched or decoded: its `:name:` shows as text (§9).
    Failed,
}

impl PartialEq for EmojiLook {
    /// The same look: two images are the same when they are the same pixels.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (EmojiLook::Loading, EmojiLook::Loading) | (EmojiLook::Failed, EmojiLook::Failed) => {
                true
            }
            (
                EmojiLook::Ready { pixels, aspect },
                EmojiLook::Ready {
                    pixels: other,
                    aspect: other_aspect,
                },
            ) => pixels.signature() == other.signature() && aspect == other_aspect,
            _ => false,
        }
    }
}

// An aspect is a finite ratio of two sides, never NaN, so equality is total.
impl Eq for EmojiLook {}

impl std::hash::Hash for EmojiLook {
    /// Hashes as [`PartialEq`] compares, so a look can be part of a row's `memo` version.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        if let EmojiLook::Ready { pixels, aspect } = self {
            pixels.signature().hash(state);
            aspect.to_bits().hash(state);
        }
    }
}

/// A fetch to run: emoji `key`'s image from `source`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EmojiJob {
    pub(crate) key: EmojiKey,
    pub(crate) source: MediaSource,
}

struct Entry {
    /// The image's revision (the emoji's creation time), so a re-created emoji is fetched anew.
    revision: String,
    /// The last tick a view wanted it.
    used: u64,
}

/// What every emoji draws as now: a cheap snapshot views read.
#[derive(Clone, Default)]
pub(crate) struct EmojiLooks(Arc<HashMap<EmojiKey, EmojiLook>>);

impl EmojiLooks {
    /// What emoji `key` draws as; loading until asked and landed.
    pub(crate) fn look(&self, key: EmojiKey) -> EmojiLook {
        self.0.get(&key).cloned().unwrap_or(EmojiLook::Loading)
    }

    /// What account `account`'s emoji at `emoji` (ranges of a text) draw as.
    pub(crate) fn of(
        &self,
        account: AccountId,
        emoji: &[(std::ops::Range<usize>, Uuid)],
    ) -> Vec<(std::ops::Range<usize>, EmojiLook)> {
        emoji
            .iter()
            .map(|(range, id)| (range.clone(), self.look(EmojiKey { account, id: *id })))
            .collect()
    }
}

/// The decoded emoji images.
#[derive(Default)]
pub(crate) struct EmojiImages {
    entries: HashMap<EmojiKey, Entry>,
    looks: EmojiLooks,
    tick: u64,
}

impl EmojiImages {
    /// What every emoji draws as now.
    pub(crate) fn looks(&self) -> EmojiLooks {
        self.looks.clone()
    }

    /// The views want `emojis` of `account` now. Returns the fetches to start: one for each
    /// emoji not yet asked for, or asked for at an older revision.
    pub(crate) fn want<'a>(
        &mut self,
        account: AccountId,
        emojis: impl IntoIterator<Item = &'a CustomEmoji>,
    ) -> Vec<EmojiJob> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let mut jobs = Vec::new();
        for emoji in emojis {
            let key = EmojiKey {
                account,
                id: emoji.id,
            };
            match self.entries.get_mut(&key) {
                Some(entry) if entry.revision == emoji.created_at => entry.used = tick,
                _ => {
                    self.entries.insert(
                        key,
                        Entry {
                            revision: emoji.created_at.clone(),
                            used: tick,
                        },
                    );
                    Arc::make_mut(&mut self.looks.0).remove(&key);
                    jobs.push(EmojiJob {
                        key,
                        source: emoji.source(),
                    });
                }
            }
        }
        self.evict();
        jobs
    }

    /// Job `job` ended with `image` (its pixels and their aspect), or none when it failed.
    /// Returns whether a view draws something else now.
    pub(crate) fn landed(&mut self, job: &EmojiJob, image: Option<(Pixels, f32)>) -> bool {
        let MediaSource::Emoji { revision, .. } = &job.source else {
            return false;
        };
        let Some(entry) = self.entries.get_mut(&job.key) else {
            // Evicted while it was fetched.
            return false;
        };
        if entry.revision != *revision {
            return false;
        }
        let look = match image {
            Some((pixels, aspect)) => EmojiLook::Ready { pixels, aspect },
            None => EmojiLook::Failed,
        };
        Arc::make_mut(&mut self.looks.0).insert(job.key, look);
        true
    }

    /// Account `account` is gone: its images go too.
    pub(crate) fn forget(&mut self, account: AccountId) {
        self.entries.retain(|key, _| key.account != account);
        Arc::make_mut(&mut self.looks.0).retain(|key, _| key.account != account);
    }

    /// Keeps the [`MAX_KEPT`] most recently wanted, never one still loading.
    fn evict(&mut self) {
        let over = self.entries.len().saturating_sub(MAX_KEPT);
        if over == 0 {
            return;
        }
        let mut settled: Vec<(u64, EmojiKey)> = self
            .entries
            .iter()
            .filter(|(key, _)| self.looks.0.contains_key(key))
            .map(|(key, entry)| (entry.used, *key))
            .collect();
        settled.sort_unstable_by_key(|(used, _)| *used);
        let looks = Arc::make_mut(&mut self.looks.0);
        for (_, key) in settled.into_iter().take(over) {
            self.entries.remove(&key);
            looks.remove(&key);
        }
    }
}

/// Where an emoji is drawn in a line of text.
#[derive(Clone, Copy)]
pub(crate) struct EmojiPlace {
    /// The line's font.
    pub(crate) font: TextStyle,
    /// A jumbo message: the emoji is the text size, not the line's height.
    pub(crate) jumbo: bool,
    /// In a hidden spoiler: it keeps its room and shows nothing.
    pub(crate) hidden: bool,
}

/// What draws in place of an emoji's `:name:` (§9, Emoji): its image once it landed, its
/// square reserved in `surface.hover` while it loads; none when it failed, so the name shows.
pub(crate) fn inline_image(look: &EmojiLook, place: EmojiPlace, ui: &Look) -> Option<InlineImage> {
    let (height, below) = emoji_box(place.font, place.jumbo);
    let (pixels, aspect) = match look {
        EmojiLook::Failed => return None,
        EmojiLook::Loading => (
            loading_square(
                ui.palette.surface_hover,
                height,
                ui.radius(EMOJI_LOADING_RADIUS),
            ),
            1.0,
        ),
        EmojiLook::Ready { pixels, aspect } => (pixels.clone(), *aspect),
    };
    let pixels = if place.hidden { clear() } else { pixels };
    Some(InlineImage::new(
        pixels,
        height * aspect,
        height - below,
        below,
    ))
}

/// `el`, a line of text in `font`, with the custom emoji at `emoji` (byte ranges of its text,
/// moved by `offset`, and how each draws) set over their `:name:`.
pub(crate) fn with_images(
    mut el: rune::El,
    offset: usize,
    emoji: impl IntoIterator<Item = (std::ops::Range<usize>, EmojiLook)>,
    font: TextStyle,
    ui: &Look,
) -> rune::El {
    let place = EmojiPlace {
        font,
        jumbo: false,
        hidden: false,
    };
    for (range, look) in emoji {
        if let Some(image) = inline_image(&look, place, ui) {
            el = el.inline_image(range.start + offset..range.end + offset, image);
        }
    }
    el
}

thread_local! {
    /// The loading squares drawn so far, by colour, size and radius.
    static LOADING: RefCell<HashMap<(u32, u32, u32), Pixels>> = RefCell::new(HashMap::new());
    /// One clear pixel, scaled to any box.
    static CLEAR: Pixels = {
        let pixels = Pixels::new();
        pixels.set(1, 1, vec![0; 4]);
        pixels
    };
}

/// Pixels a point a loading square is drawn at, so it stays sharp when scaled.
const LOADING_PX_PER_PT: f32 = 4.0;

/// A loading emoji's square: `ink`, `side_pt` points, its corners rounded at `radius_pt`.
fn loading_square(ink: Color, side_pt: f32, radius_pt: f32) -> Pixels {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=512 pixels"
    )]
    let side = (side_pt * LOADING_PX_PER_PT).round().clamp(1.0, 512.0) as u32;
    let rgba = srgb8(ink);
    let key = (u32::from_be_bytes(rgba), side, radius_pt.to_bits());
    if let Some(pixels) = LOADING.with_borrow(|kept| kept.get(&key).cloned()) {
        return pixels;
    }
    let pixels = Pixels::new();
    pixels.set(
        side,
        side,
        rounded_square(side, radius_pt * LOADING_PX_PER_PT, rgba),
    );
    LOADING.with_borrow_mut(|kept| kept.insert(key, pixels.clone()));
    pixels
}

fn clear() -> Pixels {
    CLEAR.with(Clone::clone)
}

/// `color` as straight sRGB bytes, as [`Pixels`] hold them.
fn srgb8(color: Color) -> [u8; 4] {
    let [r, g, b, a] = color.premul_srgb(1.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=255"
    )]
    let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let straight = |channel: f32| if a > 0.0 { channel / a } else { 0.0 };
    [
        byte(straight(r)),
        byte(straight(g)),
        byte(straight(b)),
        byte(a),
    ]
}

/// A `side`-pixel square of `rgba` with corners rounded at `radius` pixels, edges smoothed.
fn rounded_square(side: u32, radius: f32, rgba: [u8; 4]) -> Vec<u8> {
    #[expect(clippy::cast_precision_loss, reason = "at most 512 pixels a side")]
    let side_f = side as f32;
    let radius = radius.clamp(0.0, side_f / 2.0);
    let mut out = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            #[expect(clippy::cast_precision_loss, reason = "at most 512 pixels a side")]
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            // How far outside the nearest corner's circle, in pixels, as coverage.
            let cx = px.clamp(radius, side_f - radius);
            let cy = py.clamp(radius, side_f - radius);
            let outside = ((px - cx).hypot(py - cy) - radius).clamp(-0.5, 0.5);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "an alpha in 0..=255"
            )]
            let alpha = (f32::from(rgba[3]) * (0.5 - outside)).round() as u8;
            out.extend_from_slice(&[rgba[0], rgba[1], rgba[2], alpha]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> AccountId {
        let server = papo_sdk::ServerAddress::parse("https://emoji.invalid")
            .unwrap()
            .id();
        AccountId::new(server, Uuid::from_u128(1))
    }

    fn emoji(id: u128, created_at: &str) -> CustomEmoji {
        CustomEmoji {
            id: Uuid::from_u128(id),
            name: format!("e{id}"),
            format: "png".into(),
            created_at: created_at.into(),
        }
    }

    fn pixels() -> Pixels {
        let pixels = Pixels::new();
        pixels.set(1, 1, vec![0; 4]);
        pixels
    }

    #[test]
    fn an_emoji_is_fetched_once_and_draws_when_it_lands() {
        let mut images = EmojiImages::default();
        let gato = emoji(1, "t1");
        let jobs = images.want(account(), [&gato]);
        assert_eq!(jobs.len(), 1);
        assert!(images.want(account(), [&gato]).is_empty(), "asked once");
        let key = jobs[0].key;
        assert_eq!(images.looks().look(key), EmojiLook::Loading);
        assert!(images.landed(&jobs[0], Some((pixels(), 1.0))));
        assert!(matches!(images.looks().look(key), EmojiLook::Ready { .. }));
    }

    #[test]
    fn a_failed_emoji_shows_its_name_and_a_recreated_one_is_fetched_again() {
        let mut images = EmojiImages::default();
        let jobs = images.want(account(), [&emoji(1, "t1")]);
        images.landed(&jobs[0], None);
        assert_eq!(images.looks().look(jobs[0].key), EmojiLook::Failed);
        let again = images.want(account(), [&emoji(1, "t2")]);
        assert_eq!(again.len(), 1, "a new revision is a new image");
        assert!(
            !images.landed(&jobs[0], Some((pixels(), 1.0))),
            "the old fetch is stale"
        );
        assert_eq!(images.looks().look(jobs[0].key), EmojiLook::Loading);
    }

    #[test]
    fn the_least_recently_wanted_settled_images_go_first() {
        let mut images = EmojiImages::default();
        let all: Vec<CustomEmoji> = (0..=MAX_KEPT as u128).map(|id| emoji(id, "t")).collect();
        for one in &all[..MAX_KEPT] {
            let jobs = images.want(account(), [one]);
            images.landed(&jobs[0], Some((pixels(), 1.0)));
        }
        let first = EmojiKey {
            account: account(),
            id: all[0].id,
        };
        images.want(account(), [&all[MAX_KEPT]]);
        assert_eq!(images.entries.len(), MAX_KEPT);
        assert_eq!(
            images.looks().look(first),
            EmojiLook::Loading,
            "evicted: asked again when wanted"
        );
    }

    /// The height of a line of body text holding `:gato:`, drawn as `look` when given.
    fn line_height(look: Option<EmojiLook>, jumbo: bool, scale: f32) -> f32 {
        use rune::{Cx, El, Env, Runtime, col, text};
        struct State(Option<EmojiLook>, bool);
        fn view(state: &State, _: &mut Cx<State>) -> El {
            let ui = Look::new(crate::design::Theme::Graphite);
            let font = ui.font(if state.1 {
                crate::design::TYPE_JUMBO
            } else {
                crate::design::TYPE_BODY
            });
            let mut line = text("oi :gato: tudo").key("line").font(font);
            if let Some(look) = &state.0 {
                let place = EmojiPlace {
                    font,
                    jumbo: state.1,
                    hidden: false,
                };
                if let Some(image) = inline_image(look, place, &ui) {
                    line = line.inline_image(3..9, image);
                }
            }
            col().key("root").child(line)
        }
        let env = Env {
            system_dark: true,
            scale,
        };
        let mut rt = Runtime::with_text(
            State(look, jumbo),
            view,
            (600.0, 200.0),
            env,
            crate::fonts::text_system(),
        );
        rt.frame(0.0);
        rt.elements()
            .into_iter()
            .find(|element| element.path.ends_with("line"))
            .unwrap()
            .rect[3]
    }

    #[test]
    fn an_emoji_never_grows_its_line_loading_or_drawn() {
        let image = pixels_of(64, 64);
        for (jumbo, scale) in [false, true]
            .into_iter()
            .flat_map(|jumbo| [1.0, 1.25, 1.5, 2.0, 3.0].map(|scale| (jumbo, scale)))
        {
            let plain = line_height(None, jumbo, scale);
            for look in [
                EmojiLook::Loading,
                EmojiLook::Ready {
                    pixels: image.clone(),
                    aspect: 1.0,
                },
                EmojiLook::Ready {
                    pixels: image.clone(),
                    aspect: 2.5,
                },
            ] {
                let with = line_height(Some(look.clone()), jumbo, scale);
                assert!(
                    (with - plain).abs() < 0.01,
                    "jumbo {jumbo} at {scale}×, {look:?}: {with} against {plain}"
                );
            }
        }
    }

    fn pixels_of(width: u32, height: u32) -> Pixels {
        let pixels = Pixels::new();
        pixels.set(width, height, vec![255; (width * height * 4) as usize]);
        pixels
    }
}
