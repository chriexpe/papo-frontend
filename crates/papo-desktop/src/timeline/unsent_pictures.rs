//! Your unsent message's pictures (`docs/design.md` §14.3, §14.4): one in the media box at the
//! size the SDK's rules give it, or an album; each picture waiting or uploading under a scrim
//! with the ring and ✕ at its centre. The box and album come from `message_media`, as a
//! received message's do, so the row keeps its height once sent.
//!
//! Pure: drawn from the items `unsent_attachments` hands it.

use papo_sdk::Uuid;
use papo_sdk_ui::{AlbumLayout, MediaBounds, MediaFit, media_box};
use rune::{Color, El, SurfaceFit, el};

use super::TimelineKey;
use super::unsent_attachments::{Item, Upload, cancel_button, progress_label};
use super::view::TimelineHost;
use crate::composer::tray::TileKind;
use crate::design::{Look, UPLOAD_RING_PT, UPLOAD_RING_TRACK_ALPHA, UPLOAD_SCRIM_ALPHA};
use crate::message_media::{
    Layer, album as album_frame, gif_badge, inner_edge, length_badge, more_mark, painted,
    single_box,
};
use crate::progress_ring::ring;
use crate::strings;

/// One picture in the media box, the SDK's box for what its size is known as.
pub(super) fn single<S: TimelineHost>(
    item: &Item<'_>,
    cancel: Option<(TimelineKey, Uuid)>,
    bounds: MediaBounds,
    look: &Look,
) -> El {
    let boxed = media_box(item.file.media_size, bounds);
    let cell = picture(single_box(boxed, look), item, boxed.fit, look);
    with_upload::<S>(cell, item, cancel, look).child(inner_edge(look))
}

/// Two or more pictures as one album (§14.4), as the SDK lays it out, "+N" over the last cell
/// when more follow.
pub(super) fn album<S: TimelineHost>(
    items: &[&Item<'_>],
    cancel: Option<(TimelineKey, Uuid)>,
    bounds: MediaBounds,
    look: &Look,
) -> El {
    let Some(layout) = AlbumLayout::for_count(items.len()) else {
        return el();
    };
    let last = layout.shown() - 1;
    let cells = items
        .iter()
        .take(layout.shown())
        .enumerate()
        .map(|(place, item)| {
            let mut cell = picture(el(), item, MediaFit::Fill, look);
            if place == last && layout.more() > 0 {
                cell = cell.child(more_mark(layout.more(), look));
            }
            with_upload::<S>(cell, item, cancel, look)
        })
        .collect();
    album_frame(layout, layout.album_box(bounds), cells, items.len(), look)
}

/// `cell` showing `item`'s picture, with the "GIF" badge on a GIF; a video's first frame with
/// its length.
fn picture(cell: El, item: &Item<'_>, fit: MediaFit, look: &Look) -> El {
    let cell = painted(cell, item.file.shown(), fit, Layer::default(), look);
    match (&item.file.kind, &item.file.video) {
        (TileKind::Gif, _) => cell.child(gif_badge(look)),
        (TileKind::Video, Some(video)) => {
            let surface_fit = match fit {
                MediaFit::Fill => SurfaceFit::Fill,
                MediaFit::Letterbox => SurfaceFit::Fit,
            };
            cell.child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .native_surface(&video.surface, surface_fit)
                    .a11y_hidden(),
            )
            .child(length_badge(
                video.duration_ms.map(strings::clock).unwrap_or_default(),
                look,
            ))
        }
        _ => cell,
    }
}

/// A picture still waiting or uploading: a 40 % black scrim, and the ring at its centre with
/// ✕ inside, which cancels the message's upload.
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
