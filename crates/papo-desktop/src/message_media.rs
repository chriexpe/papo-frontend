//! Media in a message (`docs/design.md` §14.4, §14.6): pictures in the media box, one alone or
//! an album, and other files as cards. Your unsent messages draw theirs from the local files
//! with each upload's progress (`timeline::unsent_attachments`); received ones from the
//! server's (`received`). Both build on the pieces here and on the SDK's box rules, so a message
//! keeps its shape from unsent to sent and a row never changes height as its media arrive.
//!
//! Pure views. What the pictures are (decoded, loading, failed) is `media_store`'s; what a press
//! does is the host's.

use papo_sdk_ui::{AlbumLayout, MediaBox, MediaFit};
use rune::{Color, Curve, Easing, El, Image, SurfaceFit, col, el, icon, row, text};

use crate::attaching::extension_label;
use crate::design::{
    ALBUM_MORE_ALPHA, EXTENSION_BLOCK_PT, EXTENSION_BLOCK_RADIUS, FILE_CARD_BAR_PT,
    FILE_CARD_MAX_PT, FILE_CARD_PAD_PT, HIDDEN_BLUR_PT, HIDDEN_ICON_PT, HIDDEN_MARK_PT, Look,
    MEDIA_BADGE_ALPHA, MEDIA_BADGE_PAD_PT, MEDIA_BADGE_RADIUS, MEDIA_FAILED_ICON_PT,
    PILL_OUTLINE_PT, RADIUS_CARD, SPACE_MD, SPACE_SM, TYPE_BODY_MEDIUM, TYPE_CAPTION,
    TYPE_EXTENSION, TYPE_MEDIA_BADGE, TYPE_TITLE, VIEWER_GROW_S,
};
use crate::icons::{EYE_OFF, IMAGE_OFF};
use crate::strings;

pub(crate) mod audio;
mod file_card_view;
mod picture_cell;
pub(crate) mod received;
pub(crate) mod video;
pub(crate) mod video_bar;

pub(crate) use video_bar::glass;

/// A 4 pt track `share` filled (a seek bar, a volume), for the viewer's video bar too.
pub(crate) fn video_track(share: f32) -> El {
    video_bar::track(share, None)
}

/// A single picture's box: `boxed`'s shape at most `boxed`'s size, narrower only when the
/// column is, at `radius.card`. The caller adds its picture, its marks and [`inner_edge`].
pub(crate) fn single_box(boxed: MediaBox, look: &Look) -> El {
    el().aspect_fit(boxed.width_pt / boxed.height_pt)
        .max_w(boxed.width_pt)
        .max_h(boxed.height_pt)
        .radius(look.radius(RADIUS_CARD))
        .clip_rounded(look.radius(RADIUS_CARD))
}

/// An album of `cells` (the shown pictures, in order) in `boxed`, laid out as `layout` with
/// the SDK's gap, at `radius.card` outside only; spoken "Album, 5 items".
pub(crate) fn album(
    layout: AlbumLayout,
    boxed: MediaBox,
    mut cells: Vec<El>,
    count: usize,
    look: &Look,
) -> El {
    let gap = papo_sdk_ui::ALBUM_GAP_PT;
    let line = |cells: Vec<El>, along: El| along.gap(gap).children(cells);
    let body = match layout {
        AlbumLayout::Two => line(
            cells
                .into_iter()
                .map(|cell| cell.grow(1.0).min_w(0.0))
                .collect(),
            row(),
        ),
        AlbumLayout::Three => {
            let right: Vec<El> = cells
                .split_off(1)
                .into_iter()
                .map(|cell| cell.grow(1.0).min_h(0.0))
                .collect();
            // The tall one takes two thirds of the width, as the SDK's cells say.
            let tall: Vec<El> = cells
                .into_iter()
                .map(|cell| cell.grow(2.0).min_w(0.0))
                .collect();
            line(tall, row()).child(line(right, col()).grow(1.0).min_w(0.0).h_full())
        }
        AlbumLayout::Four { .. } => {
            let below: Vec<El> = cells
                .split_off(2)
                .into_iter()
                .map(|cell| cell.grow(1.0).min_w(0.0))
                .collect();
            let above: Vec<El> = cells
                .into_iter()
                .map(|cell| cell.grow(1.0).min_w(0.0))
                .collect();
            col()
                .gap(gap)
                .child(line(above, row()).w_full().grow(1.0).min_h(0.0))
                .child(line(below, row()).w_full().grow(1.0).min_h(0.0))
        }
    };
    single_box(boxed, look)
        .role("group")
        .label(strings::media_album(count))
        .child(body.w_full().h_full())
        .child(inner_edge(look))
}

/// How a picture sits in its box beyond its pixels.
#[derive(Clone, Debug, Default)]
pub(crate) struct Layer {
    /// Sensitive and not revealed yet: blurred, with the `eye-off` mark over it (a spoiler).
    pub(crate) hidden: bool,
    /// The viewer shows it now: the box stays, the picture is the viewer's.
    pub(crate) in_viewer: bool,
    /// The id the viewer's picture shares with it, so the viewer grows out of this box and
    /// shrinks back into it.
    pub(crate) matched: Option<String>,
}

/// `cell` painted with `image` (when decoded) as `fit` says, on a layer of its own: `surface.hover`
/// while it loads, `surface.code` behind a letterboxed one.
pub(crate) fn painted(
    cell: El,
    image: Option<&Image>,
    fit: MediaFit,
    layer: Layer,
    look: &Look,
) -> El {
    let (behind, surface_fit) = match fit {
        MediaFit::Fill => (look.palette.surface_hover, SurfaceFit::Fill),
        MediaFit::Letterbox => (look.palette.surface_code, SurfaceFit::Fit),
    };
    let mut cell = cell.fill(behind);
    let image = image.filter(|image| image.is_ready() && image.error().is_none());
    if let Some(image) = image.filter(|_| !layer.in_viewer) {
        let mut picture = el()
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .pixels_fit(image.pixels(), surface_fit)
            .a11y_hidden();
        if let Some(id) = layer.matched {
            picture = grows_with_viewer(picture, id);
        }
        if layer.hidden {
            // The blur reaches past the picture: its own clip keeps it in the box.
            picture = el()
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .clip_rounded(0.0)
                .child(picture.blur(HIDDEN_BLUR_PT));
        }
        cell = cell.child(picture);
    }
    if layer.hidden {
        cell = cell.child(hidden_mark());
    }
    cell
}

/// `picture` shared with the viewer as `id`: opening grows the viewer's out of it, closing
/// shrinks it back (§14.8, 180 ms; Rune leaves both still under reduced motion).
pub(crate) fn grows_with_viewer(picture: El, id: String) -> El {
    picture
        .matched(id)
        .matched_motion(Curve::tween(VIEWER_GROW_S, Easing::EASE_OUT))
}

/// The `eye-off` mark at hidden media's centre, white on the badges' 60 % black.
pub(crate) fn hidden_mark() -> El {
    col()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .a11y_hidden()
        .child(
            el().size(HIDDEN_MARK_PT, HIDDEN_MARK_PT)
                .radius(HIDDEN_MARK_PT / 2.0)
                .center()
                .fill(Color::BLACK.alpha(MEDIA_BADGE_ALPHA))
                .child(icon(&EYE_OFF, HIDDEN_ICON_PT).color(Color::WHITE)),
        )
}

/// The "GIF" badge at a GIF's top left, in the badge style (white mono on 60 % black).
pub(crate) fn gif_badge(look: &Look) -> El {
    let inset = look.space(SPACE_MD);
    el().pin(Some(inset), Some(inset), None, None)
        .child(badge(strings::media_gif().to_owned(), look))
}

/// A video's length at its bottom left, in the badge style.
pub(crate) fn length_badge(words: String, look: &Look) -> El {
    el().pin(
        Some(crate::design::VIDEO_MARK_INSET_PT),
        None,
        None,
        Some(crate::design::VIDEO_MARK_INSET_PT),
    )
    .child(badge(words, look))
}

/// A badge: white mono on 60 % black.
pub(crate) fn badge(words: String, look: &Look) -> El {
    text(words)
        .font(look.font(TYPE_MEDIA_BADGE))
        .color(Color::WHITE)
        .px(MEDIA_BADGE_PAD_PT.0)
        .py(MEDIA_BADGE_PAD_PT.1)
        .radius(look.radius(MEDIA_BADGE_RADIUS))
        .fill(Color::BLACK.alpha(MEDIA_BADGE_ALPHA))
        .a11y_hidden()
}

/// The 1 pt `line.subtle` edge inside media, so a white picture on Light still has one.
pub(crate) fn inner_edge(look: &Look) -> El {
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .radius(look.radius(RADIUS_CARD))
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .a11y_hidden()
}

/// "+N" over an album's last cell, on black at 60 %.
pub(crate) fn more_mark(more: usize, look: &Look) -> El {
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

/// A picture that could not load (§14.4): the `image-off` icon, "Couldn't load the image" in
/// `text.muted`, and `retry`, inside the reserved box.
pub(crate) fn failed(retry: El, look: &Look) -> El {
    col()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .gap(look.space(SPACE_SM))
        .child(icon(&IMAGE_OFF, MEDIA_FAILED_ICON_PT).color(look.palette.text_muted))
        .child(
            text(strings::media_load_failed())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        )
        .child(retry)
}

/// What a file card says: its name (the extension kept) and the line under it.
pub(crate) struct FileCard<'a> {
    pub(crate) name: &'a str,
    /// "17.8 MB", "4.1 of 17.8 MB", "17.8 MB · Saved", or a failure; none while the size is
    /// unknown.
    pub(crate) size_line: Option<El>,
    /// Its spoken name: "File, notes.pdf, 2.4 MB".
    pub(crate) label: String,
}

/// A size line in mono `text.muted`.
pub(crate) fn size_line(words: String, look: &Look) -> El {
    text(words)
        .font(look.font(TYPE_CAPTION).mono())
        .color(look.palette.text_muted)
}

/// A file as a card (§14.6): the extension block, the name in one line (its extension kept),
/// the size line, `ends` at the end, and while `bar` the accent bar along its bottom with its
/// spoken name.
pub(crate) fn file_card(
    card: FileCard<'_>,
    ends: Vec<El>,
    bar: Option<(f32, String)>,
    look: &Look,
) -> El {
    let extension = el()
        .size(EXTENSION_BLOCK_PT, EXTENSION_BLOCK_PT)
        .center()
        .radius(look.radius(EXTENSION_BLOCK_RADIUS))
        .fill(look.palette.surface_code)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .child(
            text(extension_label(card.name))
                .font(look.font(TYPE_EXTENSION))
                .color(look.palette.text_secondary)
                .a11y_hidden(),
        );
    let words = col()
        .grow(1.0)
        .min_w(0.0)
        .child(
            text(card.name)
                .font(look.font(TYPE_BODY_MEDIUM))
                .color(look.palette.text_primary)
                .truncate_middle(),
        )
        .children(card.size_line);
    let mut built = row()
        .w_full()
        .max_w(FILE_CARD_MAX_PT)
        .pl(FILE_CARD_PAD_PT.0)
        .pr(FILE_CARD_PAD_PT.1)
        .py(FILE_CARD_PAD_PT.1)
        .gap(look.space(SPACE_MD))
        .items_center()
        .radius(look.radius(RADIUS_CARD))
        .clip_rounded(look.radius(RADIUS_CARD))
        .fill(look.palette.surface_sidebar)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .role("group")
        .label(card.label)
        .child(extension)
        .child(words)
        .children(ends);
    if let Some((share, label)) = bar {
        built = built.child(
            // From the left edge, along the bottom.
            el().pin(Some(0.0), None, None, Some(0.0))
                .w_frac(share.clamp(0.0, 1.0))
                .h(FILE_CARD_BAR_PT)
                .fill(look.palette.accent)
                .role("progressbar")
                .label(label),
        );
    }
    built
}
