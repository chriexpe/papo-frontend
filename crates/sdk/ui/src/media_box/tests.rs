//! Media boxes and album layouts from synthetic sizes, settings and attachments.

use super::*;

fn medium() -> MediaBounds {
    MediaBounds::new(MediaSizeSetting::Medium, f32::INFINITY, f32::INFINITY)
}

fn sized(width_pt: f32, height_pt: f32, fit: MediaFit) -> MediaBox {
    MediaBox {
        width_pt,
        height_pt,
        fit,
    }
}

fn picture(mime: &str) -> Attachment {
    Attachment {
        mime_type: Some(mime.to_owned()),
        size_bytes: Some(1000),
        ..Attachment::default()
    }
}

#[test]
fn a_known_picture_keeps_its_shape_and_is_never_enlarged() {
    // §14.9's mock: a 4032 × 3024 photo in Medium is 448 × 336.
    assert_eq!(
        media_box(MediaSize::exact(4032, 3024), medium()),
        sized(448.0, 336.0, MediaFit::Fill)
    );
    // A tall one stops at the max height.
    assert_eq!(
        media_box(MediaSize::exact(3024, 4032), medium()),
        sized(300.0, 400.0, MediaFit::Fill)
    );
    // A small one is shown at its own size.
    assert_eq!(
        media_box(MediaSize::exact(96, 96), medium()),
        sized(96.0, 96.0, MediaFit::Fill)
    );
}

#[test]
fn a_shape_fills_the_bounds() {
    // Someone else's 250 × 141 GIF reads as its 128 × 72 thumbnail: it fills the width.
    assert_eq!(
        media_box(
            MediaSize::Shape {
                width_px: 128,
                height_px: 72
            },
            medium()
        ),
        sized(448.0, 252.0, MediaFit::Fill)
    );
}

#[test]
fn an_unknown_size_is_a_letterboxed_16_by_9_box_at_the_max_width() {
    assert_eq!(
        media_box(MediaSize::Unknown, medium()),
        sized(448.0, 252.0, MediaFit::Letterbox)
    );
    // A short host: the height wins and the width follows the shape.
    let short = MediaBounds::new(MediaSizeSetting::Medium, f32::INFINITY, 180.0);
    assert_eq!(
        media_box(MediaSize::Unknown, short),
        sized(320.0, 180.0, MediaFit::Letterbox)
    );
    assert_eq!(MediaSize::exact(0, 10), MediaSize::Unknown);
}

#[test]
fn bounds_take_the_smallest_of_the_setting_the_column_and_the_host() {
    let bounds = MediaBounds::new(MediaSizeSetting::Large, 500.0, 0.55 * 600.0);
    assert_eq!(bounds.max_width_pt, 500.0);
    assert_eq!(bounds.max_height_pt, 330.0);
    let bounds = MediaBounds::new(MediaSizeSetting::Small, 900.0, 900.0);
    assert_eq!((bounds.max_width_pt, bounds.max_height_pt), (320.0, 280.0));
    // A limit that is not a number limits nothing; a side is never below one point.
    let bounds = MediaBounds::new(MediaSizeSetting::Small, f32::NAN, -5.0);
    assert_eq!((bounds.max_width_pt, bounds.max_height_pt), (320.0, 1.0));
}

#[test]
fn the_presets_and_custom_sizes() {
    assert_eq!(MediaSizeSetting::default(), MediaSizeSetting::Medium);
    assert_eq!(MediaSizeSetting::Small.max_size_pt(), (320, 280));
    assert_eq!(MediaSizeSetting::Medium.max_size_pt(), (448, 400));
    assert_eq!(MediaSizeSetting::Large.max_size_pt(), (600, 520));
    assert_eq!(MediaSizeSetting::custom(500, 300).max_size_pt(), (504, 304));
    assert_eq!(MediaSizeSetting::custom(10, 9999).max_size_pt(), (240, 640));
    // A stored Custom off its range or step is brought back.
    let stored = MediaSizeSetting::Custom {
        max_width_pt: 721,
        max_height_pt: 203,
    };
    assert_eq!(stored.max_size_pt(), (720, 200));
}

#[test]
fn boxes_are_whole_points() {
    let bounds = MediaBounds::new(MediaSizeSetting::Medium, 447.6, f32::INFINITY);
    let fitted = media_box(MediaSize::exact(3151, 2102), bounds);
    assert_eq!(fitted.width_pt, fitted.width_pt.round());
    assert_eq!(fitted.height_pt, fitted.height_pt.round());
    assert!(fitted.width_pt <= 447.0);
}

#[test]
fn a_received_attachment_s_size() {
    let mut video = picture("video/mp4");
    video.width = Some(480);
    video.height = Some(860);
    assert_eq!(MediaSize::of(&video), MediaSize::exact(480, 860));

    let mut photo = picture("image/jpeg");
    photo.thumbnail_width = Some(1024);
    photo.thumbnail_height = Some(664);
    assert_eq!(
        MediaSize::of(&photo),
        MediaSize::Shape {
            width_px: 1024,
            height_px: 664
        }
    );

    // Under the thumbnail step the server keeps the picture as it is.
    let mut icon = picture("image/webp");
    icon.thumbnail_width = Some(48);
    icon.thumbnail_height = Some(48);
    assert_eq!(MediaSize::of(&icon), MediaSize::exact(48, 48));

    let mut half = picture("image/png");
    half.thumbnail_width = Some(128);
    half.thumbnail_height = Some(0);
    assert_eq!(MediaSize::of(&half), MediaSize::Unknown);
    assert_eq!(MediaSize::of(&picture("image/jpeg")), MediaSize::Unknown);

    // A file is never sized from a thumbnail.
    let mut file = picture("application/pdf");
    file.thumbnail_width = Some(64);
    file.thumbnail_height = Some(64);
    assert_eq!(MediaSize::of(&file), MediaSize::Unknown);
}

#[test]
fn album_layouts_by_count() {
    assert_eq!(AlbumLayout::for_count(1), None);
    assert_eq!(AlbumLayout::for_count(2), Some(AlbumLayout::Two));
    assert_eq!(AlbumLayout::for_count(3), Some(AlbumLayout::Three));
    assert_eq!(
        AlbumLayout::for_count(4),
        Some(AlbumLayout::Four { more: 0 })
    );
    let seven = AlbumLayout::for_count(7).expect("an album");
    assert_eq!((seven.shown(), seven.more()), (4, 3));
}

#[test]
fn an_album_takes_the_max_width_at_its_shape() {
    assert_eq!(
        AlbumLayout::Two.album_box(medium()),
        sized(448.0, 299.0, MediaFit::Fill)
    );
    assert_eq!(
        AlbumLayout::Three.album_box(medium()),
        sized(448.0, 336.0, MediaFit::Fill)
    );
    // A short timeline narrows it to keep the shape.
    let short = MediaBounds::new(MediaSizeSetting::Medium, f32::INFINITY, 240.0);
    assert_eq!(
        AlbumLayout::Four { more: 0 }.album_box(short),
        sized(320.0, 240.0, MediaFit::Fill)
    );
}

#[test]
fn album_cells_tile_the_box_two_points_apart() {
    let album = sized(448.0, 336.0, MediaFit::Fill);
    let three = AlbumLayout::Three.cells(album);
    assert_eq!(three.len(), 3);
    assert_eq!(three[0].x_pt, 0.0);
    assert_eq!(three[0].height_pt, 336.0);
    assert_eq!(three[1].x_pt, three[0].width_pt + ALBUM_GAP_PT);
    assert!((three[0].width_pt - 2.0 * three[1].width_pt).abs() < 0.01);
    assert_eq!(three[2].y_pt, three[1].height_pt + ALBUM_GAP_PT);
    assert_eq!(three[2].y_pt + three[2].height_pt, 336.0);

    let four = AlbumLayout::Four { more: 3 }.cells(album);
    assert_eq!(four.len(), 4);
    assert_eq!(four[3].x_pt + four[3].width_pt, 448.0);
    assert_eq!(four[3].y_pt + four[3].height_pt, 336.0);

    let two = AlbumLayout::Two.cells(album);
    assert_eq!(two[1].x_pt + two[1].width_pt, 448.0);
}

#[test]
fn bounds_built_by_hand_are_made_sane() {
    let wild = MediaBounds {
        max_width_pt: f32::INFINITY,
        max_height_pt: f32::NAN,
    };
    let boxed = media_box(MediaSize::Unknown, wild);
    assert!(boxed.width_pt.is_finite() && boxed.height_pt.is_finite());
    let tiny = AlbumLayout::Four { more: 0 }.album_box(MediaBounds {
        max_width_pt: 1.0,
        max_height_pt: 1.0,
    });
    for cell in (AlbumLayout::Four { more: 0 }).cells(tiny) {
        assert!(cell.width_pt >= 0.0 && cell.height_pt >= 0.0);
    }
}
