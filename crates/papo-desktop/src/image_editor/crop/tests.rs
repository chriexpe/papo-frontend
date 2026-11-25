//! The frame, the cover rule and the pixels kept.
use super::*;

const STAGE: (f32, f32) = (600.0, 320.0);

fn photo() -> Crop {
    Crop::new((3000, 2000), STAGE, Shape::Original)
}

#[test]
fn an_untouched_picture_keeps_everything() {
    let crop = photo();
    assert_eq!(crop.rect(), None);
    assert!(crop.untouched());
    let frame = crop.frame();
    assert!((frame.h - (320.0 - 2.0 * FRAME_CLEAR_PT)).abs() < 0.01);
}

#[test]
fn a_square_keeps_the_middle_square() {
    let mut crop = photo();
    crop.set_shape(Shape::Square);
    assert_eq!(
        crop.rect(),
        Some(CropRect {
            x: 500,
            y: 0,
            width: 2000,
            height: 2000
        })
    );
}

#[test]
fn the_picture_never_leaves_the_frame_uncovered() {
    let mut crop = photo();
    crop.set_shape(Shape::Square);
    crop.pan(10_000.0, 0.0);
    assert_eq!(crop.rect().map(|rect| rect.x), Some(0));
    crop.pan(-10_000.0, -10_000.0);
    assert_eq!(crop.rect().map(|rect| rect.x), Some(1000));
}

#[test]
fn zoom_keeps_a_smaller_part_and_stays_in_range() {
    let mut crop = photo();
    crop.set_zoom(2.0);
    let rect = crop.rect().expect("a part");
    assert_eq!((rect.width, rect.height), (1500, 1000));
    crop.set_zoom(9.0);
    assert!((crop.zoom - ZOOM_RANGE.1).abs() < f32::EPSILON);
}

#[test]
fn a_turn_swaps_the_sides_the_crop_is_measured_in() {
    let mut crop = photo();
    crop.rotate();
    assert_eq!(crop.turns, 1);
    assert!(crop.rect().is_none(), "the whole turned picture");
    assert!(!crop.untouched(), "a turn is a change");
    crop.set_shape(Shape::Square);
    assert_eq!(
        crop.rect(),
        Some(CropRect {
            x: 0,
            y: 500,
            width: 2000,
            height: 2000
        })
    );
}

#[test]
fn a_profile_banner_is_three_to_one() {
    let crop = Crop::new((3000, 2000), STAGE, Shape::Fixed(3.0));
    let rect = crop.rect().expect("a band");
    assert_eq!((rect.width, rect.height), (3000, 1000));
}

#[test]
fn a_saved_crop_comes_back_as_it_was() {
    for (shape, zoom, pan) in [
        (Shape::Square, 1.0, (40.0, 0.0)),
        (Shape::Original, 2.5, (-60.0, 30.0)),
    ] {
        let mut crop = photo();
        crop.rotate();
        crop.set_shape(shape);
        crop.set_zoom(zoom);
        crop.pan(pan.0, pan.1);
        let saved = crop.rect();
        let back = Crop::restored((3000, 2000), STAGE, None, crop.turns, saved);
        assert_eq!(back.shape, shape);
        assert_eq!(back.rect(), saved, "{shape:?}");
    }
}

#[test]
fn a_saved_odd_crop_comes_back_free() {
    let rect = CropRect {
        x: 100,
        y: 100,
        width: 1200,
        height: 400,
    };
    let back = Crop::restored((3000, 2000), STAGE, None, 0, Some(rect));
    assert_eq!(back.shape, Shape::Free);
    assert_eq!(back.rect(), Some(rect));
}

#[test]
fn extreme_shapes_never_panic() {
    for size in [(4000, 3), (3, 4000), (0, 0), (1, 1)] {
        let mut crop = Crop::new(size, STAGE, Shape::Free);
        let start = crop.frame();
        crop.resize(Corner::TopLeft, start, 10_000.0, 10_000.0);
        crop.pan(-10_000.0, 10_000.0);
        crop.set_zoom(f32::NAN);
        let _ = crop.rect();
    }
}

#[test]
fn a_free_corner_stops_at_the_smallest_frame() {
    let mut crop = photo();
    crop.set_shape(Shape::Free);
    let start = crop.frame();
    crop.resize(Corner::BottomRight, start, -10_000.0, -10_000.0);
    let frame = crop.frame();
    assert!((frame.w - FREE_MIN_PT).abs() < 0.01 && (frame.h - FREE_MIN_PT).abs() < 0.01);
}

#[test]
fn a_free_corner_cuts_the_side_it_is_dragged_on() {
    let mut crop = photo();
    crop.set_shape(Shape::Free);
    let start = crop.frame();
    let scale = crop.geometry().scale;
    crop.resize(Corner::BottomRight, start, -100.0, -100.0);
    let rect = crop.rect().expect("a part");
    // The picture keeps its scale and place: the right and bottom go.
    assert!((crop.geometry().scale - scale).abs() < 1e-4);
    assert_eq!((rect.x, rect.y), (0, 0));
    assert!(rect.width < 3000 && rect.height < 2000);
}

#[test]
fn zoom_keeps_what_the_frame_centres() {
    let mut crop = photo();
    crop.set_shape(Shape::Square);
    crop.pan(150.0, 0.0);
    let before = crop.rect().expect("a part");
    crop.set_zoom(2.0);
    let after = crop.rect().expect("a part");
    let centre = |rect: CropRect| rect.x + rect.width / 2;
    assert!(centre(before).abs_diff(centre(after)) <= 1);
}

#[test]
fn a_crop_a_pixel_off_a_shape_comes_back_free_and_unchanged() {
    let rect = CropRect {
        x: 0,
        y: 0,
        width: 1500,
        height: 1005,
    };
    let back = Crop::restored((3000, 2000), STAGE, None, 0, Some(rect));
    assert_eq!(back.shape, Shape::Free);
    assert_eq!(back.rect(), Some(rect));
}

#[test]
fn a_part_never_passes_the_picture() {
    for pan in [-10_000.0, 10_000.0] {
        let mut crop = Crop::new((3001, 2001), STAGE, Shape::Square);
        crop.set_zoom(1.7);
        crop.pan(pan, pan);
        let rect = crop.rect().expect("a part");
        assert!(
            rect.x + rect.width <= 3001 && rect.y + rect.height <= 2001,
            "{rect:?}"
        );
    }
}
