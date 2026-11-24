//! The Background colour field in the shell, headless (`docs/design.md`
//! §4): a colour drags from where it sat and saves when it lets go, a
//! press on empty space adds one, and Try again after a failed add saves
//! the colour that was added.

use rune::prefs::Prefs;

use super::navigation::click;
use super::settings::{open, rect, saved};
use super::*;
use crate::appearance::AppearanceStore;
use crate::settings::RowId;

#[test]
fn a_colour_on_the_field_drags_from_where_it_is_and_saves_when_it_lets_go() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &RowId::Background.key());
    let before = saved(&rt).background.colours()[0];
    let [x, y, w, h] = rect(&rt, "settings-colour-0");
    let (from_x, from_y) = (x + w / 2.0, y + h / 2.0);
    let [fx, fy, _, fh] = rect(&rt, "settings-colour-field");
    // To the field's far left, past the disc: full vividness, hue 180°.
    let (to_x, to_y) = (fx + 8.0, fy + fh / 2.0);
    rt.pointer_down(from_x, from_y);
    rt.frame(FRAME_S);
    rt.pointer_move(f32::midpoint(from_x, to_x), f32::midpoint(from_y, to_y));
    rt.frame(FRAME_S);
    rt.pointer_move(to_x, to_y);
    rt.frame(FRAME_S);
    assert_eq!(
        saved(&rt).background.colours()[0],
        before,
        "nothing saved mid-drag"
    );
    let drawn = rt.state.shown_appearance().background.colours()[0];
    assert!(
        (drawn.hue_degrees - 180.0).abs() < 3.0,
        "drawn as it drags: {drawn:?}"
    );
    rt.pointer_up(to_x, to_y);
    rt.frame(FRAME_S);
    let after = saved(&rt).background.colours().to_vec();
    assert_eq!(after.len(), 1, "a drag moves the colour, never adds one");
    assert!((after[0].hue_degrees - 180.0).abs() < 3.0, "{after:?}");
    assert!((after[0].vividness - 1.0).abs() < 1e-3);
}

#[test]
fn a_press_on_the_empty_field_adds_a_colour_there() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &RowId::Background.key());
    let [fx, fy, fw, fh] = rect(&rt, "settings-colour-field");
    // Right of the centre: hue 0°.
    let (px, py) = (fx + fw / 2.0 + 50.0, fy + fh / 2.0);
    rt.pointer_down(px, py);
    rt.frame(FRAME_S);
    rt.pointer_up(px, py);
    rt.frame(FRAME_S);
    let colours = saved(&rt).background.colours().to_vec();
    assert_eq!(colours.len(), 2, "{colours:?}");
    assert!(
        colours[1].hue_degrees < 3.0 || colours[1].hue_degrees > 357.0,
        "{colours:?}"
    );
}

#[test]
fn try_again_after_a_failed_add_saves_the_added_colour() {
    let mut rt = app();
    // A settings file under a regular file can never be written.
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("file");
    std::fs::write(&blocker, b"").unwrap();
    let failing = blocker.join("prefs.json");
    rt.update(move |shell, _tx| {
        shell.appearance = AppearanceStore::from_prefs(Prefs::at(failing));
    });
    open(&mut rt);
    click(&mut rt, &RowId::Background.key());
    let [fx, fy, fw, fh] = rect(&rt, "settings-colour-field");
    let (px, py) = (fx + fw / 2.0 + 50.0, fy + fh / 2.0);
    rt.pointer_down(px, py);
    rt.frame(FRAME_S);
    rt.pointer_up(px, py);
    rt.frame(FRAME_S);
    assert_eq!(saved(&rt).background.colours().len(), 1, "not saved");
    assert!(rt.find("settings-retry").is_some());
    // The disk works again: Try again saves the colour that was added.
    let working = directory.path().join("prefs.json");
    rt.update(move |shell, _tx| {
        shell.appearance = AppearanceStore::from_prefs(Prefs::at(working));
    });
    click(&mut rt, "settings-retry");
    assert_eq!(saved(&rt).background.colours().len(), 2, "the added colour");
    assert!(rt.find("settings-retry").is_none(), "the danger line goes");
}
