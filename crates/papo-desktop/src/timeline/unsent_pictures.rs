//! Your unsent message's pictures (`docs/design.md` §14.3, §14.4): one in
//! the media box at its own shape and never larger than itself, or an
//! album; each picture waiting or uploading under a scrim with the ring and
//! ✕ at its centre.
//!
//! Pure: drawn from the items `unsent_attachments` hands it.

use papo_sdk::Uuid;
use rune::{Color, El, SurfaceFit, col, el, row, text};

use super::TimelineKey;
use super::unsent_attachments::{Item, Upload, cancel_button, progress_label};
use super::view::TimelineHost;
use crate::composer::tray::TileKind;
use crate::design::{
    ALBUM_ASPECT, ALBUM_CELLS, ALBUM_GAP_PT, ALBUM_MORE_ALPHA, Look, MEDIA_BADGE_ALPHA,
    MEDIA_BADGE_PAD_PT, MEDIA_BADGE_RADIUS, MEDIA_MAX_PT, MEDIA_UNKNOWN_ASPECT, PILL_OUTLINE_PT,
    RADIUS_CARD, SPACE_MD, TYPE_MEDIA_BADGE, TYPE_TITLE, UPLOAD_RING_PT, UPLOAD_RING_TRACK_ALPHA,
    UPLOAD_SCRIM_ALPHA,
};
use crate::progress_ring::ring;
use crate::strings;
use crate::unsent_media::UnsentFile;

/// One picture in the media box, fitted to its own shape (16:9 and
/// letterboxed when the shape is unknown), never larger than the box nor
/// than the picture itself.
pub(super) fn single<S: TimelineHost>(
    item: &Item<'_>,
    cancel: Option<(TimelineKey, Uuid)>,
    max_height_pt: f32,
    look: &Look,
) -> El {
    let known = item.file.aspect;
    let aspect = known.unwrap_or(MEDIA_UNKNOWN_ASPECT);
    let fit = if known.is_some() {
        SurfaceFit::Fill
    } else {
        SurfaceFit::Fit
    };
    let (mut max_w, mut max_h) = (MEDIA_MAX_PT.0, MEDIA_MAX_PT.1.min(max_height_pt));
    if let Some((width, height)) = item.file.size_px {
        // A pixel stands for a point, as everywhere a picture is shown.
        #[expect(clippy::cast_precision_loss, reason = "picture sides")]
        let (width, height) = (width as f32, height as f32);
        max_w = max_w.min(width);
        max_h = max_h.min(height);
    }
    let mut cell = el()
        .aspect_fit(aspect)
        .max_w(max_w)
        .max_h(max_h)
        .radius(look.radius(RADIUS_CARD))
        .clip_rounded(look.radius(RADIUS_CARD));
    cell = picture(cell, item.file, fit, look);
    cell = with_badge(cell, item.file, look);
    with_upload::<S>(cell, item, cancel, look).child(inner_edge(look))
}

/// Two or more pictures as one album (§14.4): side by side at 3 : 2 for
/// two; at 4 : 3, one tall beside two stacked for three, two by two for
/// four or more (1 2 above 3 4), "+N" over the fourth when more follow.
pub(super) fn album<S: TimelineHost>(
    items: &[&Item<'_>],
    cancel: Option<(TimelineKey, Uuid)>,
    look: &Look,
) -> El {
    let aspect = if items.len() == 2 {
        ALBUM_ASPECT.0
    } else {
        ALBUM_ASPECT.1
    };
    let more = items.len().saturating_sub(ALBUM_CELLS);
    let shown = &items[..items.len().min(ALBUM_CELLS)];
    let last = shown.len() - 1;
    let mut cells: Vec<El> = shown
        .iter()
        .enumerate()
        .map(|(place, item)| {
            let more = if place == last { more } else { 0 };
            album_cell::<S>(item, more, cancel, look)
        })
        .collect();
    let line = |cells: Vec<El>, along: El| along.grow(1.0).gap(ALBUM_GAP_PT).children(cells);
    let body = match cells.len() {
        2 => line(cells, row()),
        3 => {
            let right = cells.split_off(1);
            line(cells, row()).child(line(right, col()).min_w(0.0).h_full())
        }
        _ => {
            let below = cells.split_off(2);
            col()
                .gap(ALBUM_GAP_PT)
                .child(line(cells, row()).w_full().min_h(0.0))
                .child(line(below, row()).w_full().min_h(0.0))
        }
    };
    el().w_full()
        .max_w(MEDIA_MAX_PT.0)
        .aspect_ratio(aspect)
        .radius(look.radius(RADIUS_CARD))
        .clip_rounded(look.radius(RADIUS_CARD))
        .role("group")
        .label(strings::media_album(items.len()))
        .child(body.w_full().h_full())
        .child(inner_edge(look))
}

/// One album cell: its picture, the GIF badge, "+N" when `more` follow,
/// and over all of it the upload's scrim, ring and ✕.
fn album_cell<S: TimelineHost>(
    item: &Item<'_>,
    more: usize,
    cancel: Option<(TimelineKey, Uuid)>,
    look: &Look,
) -> El {
    let mut cell = el().grow(1.0).min_w(0.0).min_h(0.0);
    cell = picture(cell, item.file, SurfaceFit::Fill, look);
    cell = with_badge(cell, item.file, look);
    if more > 0 {
        cell = cell.child(more_mark(more, look));
    }
    with_upload::<S>(cell, item, cancel, look)
}

/// `cell` showing `file`'s picture, `surface.hover` until one decodes and
/// `surface.code` behind a letterboxed one.
fn picture(cell: El, file: &UnsentFile, fit: SurfaceFit, look: &Look) -> El {
    let behind = if fit == SurfaceFit::Fit {
        look.palette.surface_code
    } else {
        look.palette.surface_hover
    };
    let cell = cell.fill(behind);
    match file.shown() {
        Some(image) if image.error().is_none() => cell.pixels_fit(image.pixels(), fit),
        _ => cell,
    }
}

/// The "GIF" badge at a GIF's top left.
fn with_badge(cell: El, file: &UnsentFile, look: &Look) -> El {
    if file.kind != TileKind::Gif {
        return cell;
    }
    let inset = look.space(SPACE_MD);
    cell.child(
        el().pin(Some(inset), Some(inset), None, None).child(
            text(strings::media_gif())
                .font(look.font(TYPE_MEDIA_BADGE))
                .color(Color::WHITE)
                .px(MEDIA_BADGE_PAD_PT.0)
                .py(MEDIA_BADGE_PAD_PT.1)
                .radius(look.radius(MEDIA_BADGE_RADIUS))
                .fill(Color::BLACK.alpha(MEDIA_BADGE_ALPHA))
                .a11y_hidden(),
        ),
    )
}

/// The 1 pt `line.subtle` edge inside media, so a white picture on Light
/// still has one.
fn inner_edge(look: &Look) -> El {
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .radius(look.radius(RADIUS_CARD))
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .a11y_hidden()
}

/// "+N" over the album's last cell, on black at 60 %.
fn more_mark(more: usize, look: &Look) -> El {
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .fill(Color::BLACK.alpha(ALBUM_MORE_ALPHA))
        .child(
            text(strings::media_more(more))
                .font(look.font(TYPE_TITLE))
                .color(Color::WHITE)
                .a11y_hidden(),
        )
}

/// A picture still waiting or uploading: a 40 % black scrim, and the ring
/// at its centre with ✕ inside, which cancels the message's upload.
fn with_upload<S: TimelineHost>(
    cell: El,
    item: &Item<'_>,
    cancel: Option<(TimelineKey, Uuid)>,
    look: &Look,
) -> El {
    let done = match item.upload {
        Upload::Done => return cell,
        Upload::Waiting => None,
        Upload::Sending { share, .. } => Some(share),
    };
    let mut middle = el()
        .size(UPLOAD_RING_PT, UPLOAD_RING_PT)
        .center()
        .role("progressbar")
        .label(progress_label(item))
        .child(el().absolute(0.0, 0.0).child(ring(
            done,
            UPLOAD_RING_PT,
            Color::WHITE.alpha(UPLOAD_RING_TRACK_ALPHA),
            Color::WHITE,
        )));
    if let Some((key, local)) = cancel {
        middle = middle.child(cancel_button::<S>(
            key,
            local,
            item.index,
            Color::WHITE,
            look,
        ));
    }
    cell.child(
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .center()
            .fill(Color::BLACK.alpha(UPLOAD_SCRIM_ALPHA))
            .child(middle),
    )
}
