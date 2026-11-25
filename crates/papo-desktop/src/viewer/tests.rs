//! The viewer's steps, zoom and pan, from a synthetic album.

use super::*;
use papo_sdk::{AccountId, ServerAddress, Uuid};

fn at(index: usize) -> MediaAt {
    MediaAt {
        surface: crate::message_media::received::MediaSurface::Timeline,
        account: AccountId::new(
            ServerAddress::parse("https://lumen.example.invalid")
                .unwrap()
                .id(),
            Uuid::from_u128(1),
        ),
        channel: Uuid::from_u128(2),
        message: Uuid::from_u128(3),
        index,
    }
}

fn album(count: usize, opened: usize) -> Viewer {
    let items = (0..count)
        .map(|index| ViewerItem {
            index,
            key: MediaKey {
                account: at(0).account,
                attachment: Uuid::from_u128(10 + index as u128),
            },
            name: format!("{index}.jpg"),
            gif: false,
        })
        .collect();
    Viewer::new(at(opened), items, "Ana".into(), "Today, 21:30".into())
}

const PHOTO: [f32; 2] = [4032.0, 3024.0];
const ROOM: [f32; 2] = [1000.0, 500.0];

#[test]
fn it_opens_on_the_pressed_item_and_steps_without_wrapping() {
    let mut viewer = album(3, 1);
    assert_eq!(viewer.current, 1);
    assert!(viewer.step(1));
    assert!(!viewer.step(1), "the last has no next");
    assert!(viewer.step(-1) && viewer.step(-1));
    assert!(!viewer.step(-1), "the first has no previous");
}

#[test]
fn it_opens_fitted_and_a_small_picture_stays_at_its_size() {
    let viewer = album(1, 0);
    let placed = viewer.placed(PHOTO, ROOM);
    assert!((placed.size[1] - 500.0).abs() < 0.01);
    assert!(!placed.zoomed);
    assert_eq!(viewer.placed([200.0, 100.0], ROOM).size, [200.0, 100.0]);
}

#[test]
fn the_wheel_zooms_toward_the_pointer_between_fit_and_800() {
    let mut viewer = album(1, 0);
    let fit = viewer.scale(PHOTO, ROOM);
    viewer.wheel(1.0, false, [0.0, 0.0], PHOTO, ROOM);
    assert!((viewer.scale(PHOTO, ROOM) - fit * 1.25).abs() < 1e-5);
    // Wider than the room now: the point under the pointer stays under it, so the centre
    // moves away from it.
    viewer.wheel(1.0, false, [0.0, 0.0], PHOTO, ROOM);
    viewer.wheel(1.0, false, [0.0, 0.0], PHOTO, ROOM);
    viewer.wheel(1.0, false, [200.0, 0.0], PHOTO, ROOM);
    let zoomed = viewer.placed(PHOTO, ROOM);
    assert!(zoomed.size[0] > ROOM[0]);
    assert!(zoomed.offset[0] < 0.0);
    for _ in 0..100 {
        viewer.wheel(1.0, false, [0.0, 0.0], PHOTO, ROOM);
    }
    assert_eq!(viewer.percent(PHOTO, ROOM), 800);
    for _ in 0..100 {
        viewer.wheel(-1.0, false, [0.0, 0.0], PHOTO, ROOM);
    }
    assert!(!viewer.placed(PHOTO, ROOM).zoomed, "back to fit");
    assert_eq!(viewer.placed(PHOTO, ROOM).offset, [0.0, 0.0]);
}

#[test]
fn a_drag_pans_only_a_zoomed_picture_and_never_past_its_edge() {
    let mut viewer = album(1, 0);
    viewer.drag([10.0, 10.0], true, PHOTO, ROOM);
    viewer.drag([500.0, 10.0], false, PHOTO, ROOM);
    assert_eq!(
        viewer.placed(PHOTO, ROOM).offset,
        [0.0, 0.0],
        "fitted: no pan"
    );
    viewer.double_click([0.0, 0.0], PHOTO, ROOM);
    assert_eq!(viewer.percent(PHOTO, ROOM), 100);
    viewer.drag([0.0, 0.0], true, PHOTO, ROOM);
    viewer.drag([100_000.0, 0.0], false, PHOTO, ROOM);
    let placed = viewer.placed(PHOTO, ROOM);
    assert!((placed.offset[0] - (4032.0 - 1000.0) / 2.0).abs() < 0.01);
    viewer.drag_end();
    viewer.double_click([0.0, 0.0], PHOTO, ROOM);
    assert!(
        !viewer.placed(PHOTO, ROOM).zoomed,
        "a second double click fits"
    );
}

#[test]
fn stepping_fits_and_forgets_the_full_picture() {
    let mut viewer = album(2, 0);
    viewer.zoom_step(true, PHOTO, ROOM);
    viewer.asked_s = Some(1.0);
    assert!(viewer.step(1));
    assert!(!viewer.placed(PHOTO, ROOM).zoomed);
    assert!(viewer.full.is_none() && viewer.asked_s.is_none());
}
