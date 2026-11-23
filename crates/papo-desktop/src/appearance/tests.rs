//! The Appearance settings' defaults, saving and reading back.

use rune::prefs::Prefs;

use super::{AppearanceStore, ShapeValue, key, text_size_step};
use crate::design::{Accent, Background, Theme};

#[test]
fn the_defaults_are_the_designs() {
    let store = AppearanceStore::read(Prefs::memory());
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
    assert!(!appearance.glass(Theme::Oled), "OLED's pills start solid");
    for theme in [Theme::Graphite, Theme::Light, Theme::SoftLight] {
        assert!(appearance.glass(theme), "{theme:?} starts as glass");
    }
}

#[test]
fn a_saved_change_is_kept_and_read_back() {
    let mut store = AppearanceStore::read(Prefs::memory());
    let mut changed = *store.appearance();
    changed.theme = Some(Theme::Oled);
    changed.accent = Accent::Teal;
    changed.grain = 0.3;
    changed.shape = changed.shape.with(ShapeValue::TextSize, 16.0);
    store.set(changed).unwrap();
    let again = AppearanceStore::read(store.prefs);
    assert_eq!(*again.appearance(), changed);
}

#[test]
fn a_wrong_value_in_the_file_takes_its_default() {
    let mut prefs = Prefs::memory();
    prefs.set(key::THEME, "sepia").unwrap();
    prefs.set(key::ACCENT, &3).unwrap();
    prefs
        .set(key::SHAPE, &[99.0, -1.0, 1.0, 16.0, 1.0])
        .unwrap();
    let store = AppearanceStore::read(prefs);
    assert_eq!(store.appearance().theme, None);
    assert_eq!(store.appearance().accent, Accent::Blue);
    assert_eq!(store.appearance().shape.text_pt, 16.0, "held to its range");
    assert_eq!(store.appearance().shape.roundness, 0.0);
}

#[test]
fn ctrl_plus_and_minus_step_whole_points_within_the_range() {
    assert_eq!(text_size_step(14.0, true), 15.0);
    assert_eq!(text_size_step(14.4, true), 15.0);
    assert_eq!(text_size_step(14.4, false), 14.0);
    assert_eq!(text_size_step(16.0, true), 16.0);
    assert_eq!(text_size_step(12.0, false), 12.0);
}
