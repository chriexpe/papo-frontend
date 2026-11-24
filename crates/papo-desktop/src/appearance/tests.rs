//! The Appearance settings' defaults, saving and reading back.

use rune::prefs::Prefs;

use serde_json::json;

use super::{AppearanceStore, ShapeValue, stored, text_size_step};
use crate::design::{Accent, Background, Theme};

#[test]
fn the_defaults_are_the_designs() {
    let store = AppearanceStore::read(&Prefs::memory());
    let appearance = store.appearance();
    assert_eq!(
        appearance.theme, None,
        "the system's theme until one is picked"
    );
    assert_eq!(appearance.theme_or(true), Theme::Graphite);
    assert_eq!(appearance.theme_or(false), Theme::Light);
    assert_eq!(appearance.background, Background::default());
    assert_eq!(appearance.accent, Accent::Blue);
    assert_eq!(appearance.shape, super::Shape::default());
    assert!(
        (appearance.glass - 0.5).abs() < f32::EPSILON,
        "every theme starts frosted, OLED too"
    );
}

#[test]
fn a_change_shows_at_once_and_its_write_is_read_back() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prefs.json");
    let mut store = AppearanceStore::read(&Prefs::at(&path));
    let mut changed = *store.appearance();
    changed.theme = Some(Theme::Oled);
    changed.accent = Accent::Teal;
    changed.grain = 0.3;
    changed.shape = changed.shape.with(ShapeValue::TextSize, 16.0);
    changed.glass = 0.9;
    let write = store.change(changed).expect("a write");
    assert_eq!(*store.appearance(), changed, "shown before it is written");
    write.run().unwrap();
    assert_eq!(store.written(changed, true), None, "nothing waits");
    let again = AppearanceStore::read(&Prefs::at(&path));
    assert_eq!(*again.appearance(), changed);
}

#[test]
fn changes_during_a_write_wait_for_it_and_only_the_last_is_written() {
    let mut store = AppearanceStore::read(&Prefs::memory());
    let mut first = *store.appearance();
    first.grain = 0.1;
    let mut second = first;
    second.grain = 0.2;
    let mut third = first;
    third.grain = 0.3;
    let running = store.change(first).expect("written at once");
    assert_eq!(store.change(second), None, "waits for the running write");
    assert_eq!(store.change(third), None, "replaces the one waiting");
    let next = store
        .written(running.appearance(), true)
        .expect("the last one");
    assert_eq!(next.appearance(), third);
    assert_eq!(store.written(third, true), None);
}

#[test]
fn a_failed_write_takes_the_settings_back_and_drops_what_waited() {
    let mut store = AppearanceStore::read(&Prefs::memory());
    let before = *store.appearance();
    let mut first = before;
    first.grain = 0.4;
    let mut second = before;
    second.grain = 0.6;
    let running = store.change(first).unwrap();
    assert_eq!(store.change(second), None);
    assert_eq!(
        store.written(running.appearance(), false),
        None,
        "nothing more"
    );
    assert_eq!(*store.appearance(), before, "back to what the file holds");
}

#[test]
fn a_wrong_value_in_the_file_takes_its_default() {
    let mut prefs = Prefs::memory();
    prefs
        .set(
            stored::KEY,
            &json!({
                "theme": "sepia",
                "accent": 3,
                "shape": [99.0, -1.0, 1.0, 16.0, 1.0],
                "grain": "coarse",
                "messages": ["roomy", true, false],
            }),
        )
        .unwrap();
    let store = AppearanceStore::read(&prefs);
    let appearance = store.appearance();
    assert_eq!(appearance.theme, None);
    assert_eq!(appearance.accent, Accent::Blue);
    assert_eq!(appearance.shape.text_pt, 16.0, "held to its range");
    assert_eq!(appearance.shape.roundness, 0.0);
    assert_eq!(appearance.grain, 0.0);
    assert_eq!(appearance.messages, super::MessageLayout::default());
}

#[test]
fn a_number_too_large_for_the_settings_takes_its_default() {
    let mut prefs = Prefs::memory();
    prefs
        .set(
            stored::KEY,
            &json!({
                "background": [[1e39, 0.5]],
                "strength": 0.4,
                "glass": 1e39,
            }),
        )
        .unwrap();
    let store = AppearanceStore::read(&prefs);
    let appearance = store.appearance();
    assert!(
        appearance
            .background
            .colours()
            .iter()
            .all(|colour| colour.hue_degrees.is_finite()),
        "{:?}",
        appearance.background
    );
    assert!((appearance.glass - 0.5).abs() < f32::EPSILON);
}

#[test]
fn ctrl_plus_and_minus_step_whole_points_within_the_range() {
    assert_eq!(text_size_step(14.0, true), 15.0);
    assert_eq!(text_size_step(14.4, true), 15.0);
    assert_eq!(text_size_step(14.4, false), 14.0);
    assert_eq!(text_size_step(16.0, true), 16.0);
    assert_eq!(text_size_step(12.0, false), 12.0);
}
