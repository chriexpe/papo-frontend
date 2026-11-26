//! Custom emoji drawn in a line of text (`docs/design.md` §9, Emoji): the image in place of its
//! `:name:`, the square a loading one reserves, room kept clear in a hidden spoiler. What each
//! emoji draws as is the store's.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use rune::{Color, El, InlineImage, Pixels, TextStyle};

use super::EmojiLook;
use crate::design::{EMOJI_LOADING_RADIUS, Look, emoji_box};

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
    mut el: El,
    offset: usize,
    emoji: impl IntoIterator<Item = (Range<usize>, EmojiLook)>,
    font: TextStyle,
    ui: &Look,
) -> El {
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
