//! The settings panel in the shell, headless (`docs/design.md` §8): Ctrl+,
//! and the account pill's ⚙ open it, Escape clears the search and then
//! closes it, rows save as they change (a drag only when it lets go), a
//! failed save keeps the old value and says so, and the keyboard reaches
//! every row kind.

use rune::accesskit::Action;
use rune::prefs::Prefs;
use rune::{Key, KeyEvent, Mods, Runtime};

use super::navigation::{click, focused, with_servers};
use super::*;
use crate::appearance::{AppearanceStore, ShapeValue};
use crate::design::{Accent, Density, Theme};
use crate::settings::{PageId, RowId, SEARCH_KEY, SETTINGS_BUTTON_KEY, SETTINGS_KEY};

fn key(rt: &mut Runtime<Shell>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(FRAME_S);
}

fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    }
}

pub(super) fn open(rt: &mut Runtime<Shell>) {
    key(rt, Key::Command(','), ctrl());
    assert!(rt.find(SETTINGS_KEY).is_some(), "Ctrl+, opens the panel");
}

pub(super) fn saved(rt: &Runtime<Shell>) -> crate::appearance::Appearance {
    *rt.state.appearance.appearance()
}

pub(super) fn rect(rt: &Runtime<Shell>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no {key}"))
        .rect
}

#[test]
fn ctrl_comma_opens_the_panel_on_appearance_with_the_search_focused_and_again_closes_it() {
    let mut rt = app();
    open(&mut rt);
    assert_eq!(
        rt.state
            .settings()
            .map(crate::settings::SettingsPanel::page),
        Some(PageId::Appearance)
    );
    assert_eq!(focused(&rt).as_deref(), Some(SEARCH_KEY));
    let tree = tree(&rt);
    for name in [
        "Settings",
        "Appearance",
        "Theme",
        "Background colour",
        "Accent",
        "Message spacing",
    ] {
        assert!(tree.contains(&format!("\"{name}")), "{name}: {tree}");
    }
    key(&mut rt, Key::Command(','), ctrl());
    assert!(rt.find(SETTINGS_KEY).is_none());
}

#[test]
fn the_account_pills_gear_opens_the_panel_and_escape_gives_it_the_keyboard_back() {
    let mut rt = with_servers();
    click(&mut rt, SETTINGS_BUTTON_KEY);
    assert!(rt.find(SETTINGS_KEY).is_some());
    key(&mut rt, Key::Escape, Mods::default());
    assert!(rt.find(SETTINGS_KEY).is_none());
    assert_eq!(focused(&rt).as_deref(), Some(SETTINGS_BUTTON_KEY));
}

#[test]
fn escape_clears_the_search_first_then_closes_the_panel() {
    let mut rt = app();
    open(&mut rt);
    type_text(&mut rt, "spacing");
    rt.frame(FRAME_S);
    assert!(
        tree(&rt).contains("Appearance › Message spacing"),
        "{}",
        tree(&rt)
    );
    key(&mut rt, Key::Escape, Mods::default());
    assert!(
        rt.find(SETTINGS_KEY).is_some(),
        "the search cleared, the panel stays"
    );
    assert_eq!(
        rt.state.settings().map(|panel| panel.query().to_owned()),
        Some(String::new())
    );
    key(&mut rt, Key::Escape, Mods::default());
    assert!(rt.find(SETTINGS_KEY).is_none());
}

#[test]
fn the_search_ignores_accents_and_says_when_nothing_matches() {
    let mut rt = app();
    open(&mut rt);
    type_text(&mut rt, "aparência");
    rt.frame(FRAME_S);
    let found = tree(&rt);
    rt.update(|shell, _tx| {
        if let Some(panel) = shell.settings.as_mut() {
            panel.search = rune::TextEdit::new("aparencia");
        }
    });
    rt.frame(FRAME_S);
    assert_eq!(tree(&rt).matches("›").count(), found.matches("›").count());
    rt.update(|shell, _tx| {
        if let Some(panel) = shell.settings.as_mut() {
            panel.search = rune::TextEdit::new("zzz");
        }
    });
    rt.frame(FRAME_S);
    assert!(tree(&rt).contains("No settings found"));
}

#[test]
fn a_switch_flips_at_once_and_is_saved() {
    let mut rt = app();
    open(&mut rt);
    assert!(saved(&rt).messages.grouped);
    click(&mut rt, &RowId::Grouped.key());
    assert!(!saved(&rt).messages.grouped, "saved off");
    assert_eq!(
        rt.state.shown_appearance().messages.grouped,
        saved(&rt).messages.grouped
    );
}

#[test]
fn stack_shows_only_at_extra_compact() {
    let mut rt = app();
    open(&mut rt);
    assert!(
        rt.find(&RowId::Stacked.key()).is_none(),
        "left out, not greyed"
    );
    rt.update(|shell, tx| {
        shell.settings_chosen(
            crate::shell::SettingChoice::Density(Density::ExtraCompact),
            tx,
        );
    });
    rt.frame(FRAME_S);
    assert_eq!(saved(&rt).messages.density, Density::ExtraCompact);
    assert!(rt.find(&RowId::Stacked.key()).is_some());
}

#[test]
fn a_choice_opens_its_menu_under_the_row_and_a_pick_saves_it() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &RowId::Theme.key());
    assert!(
        rt.find(crate::menu::MENU_KEY).is_some(),
        "the menu under the row"
    );
    let menu = tree(&rt);
    for theme in ["OLED", "Graphite", "Light", "Soft light"] {
        assert!(menu.contains(&format!("\"{theme}\"")), "{theme}: {menu}");
    }
    click(&mut rt, "settings-theme-Oled");
    assert_eq!(saved(&rt).theme, Some(Theme::Oled));
    assert!(rt.find(crate::menu::MENU_KEY).is_none());
    assert!(rt.find(SETTINGS_KEY).is_some(), "the panel stays");
}

#[test]
fn escape_closes_a_choices_menu_before_the_panel() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &RowId::Density.key());
    key(&mut rt, Key::Escape, Mods::default());
    assert!(rt.find(crate::menu::MENU_KEY).is_none());
    assert!(rt.find(SETTINGS_KEY).is_some());
}

#[test]
fn an_accent_swatch_saves_its_accent() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, "settings-accent-Teal");
    assert_eq!(saved(&rt).accent, Accent::Teal);
}

#[test]
fn a_sub_page_opens_with_back_and_back_returns() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &RowId::Shape.key());
    assert_eq!(
        rt.state
            .settings()
            .map(crate::settings::SettingsPanel::page),
        Some(PageId::Shape)
    );
    assert!(
        tree(&rt).contains("\"Text size, 14.0 px\""),
        "{}",
        tree(&rt)
    );
    click(&mut rt, "settings-back");
    assert_eq!(
        rt.state
            .settings()
            .map(crate::settings::SettingsPanel::page),
        Some(PageId::Appearance)
    );
}

#[test]
fn a_slider_drag_is_drawn_and_saved_only_when_it_lets_go() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &RowId::Shape.key());
    let slider = RowId::Slider(ShapeValue::Roundness).key();
    let [x, y, w, h] = rect(&rt, &slider);
    let before = saved(&rt).shape.roundness;
    rt.pointer_down(x + 14.0, y + h / 2.0);
    rt.frame(FRAME_S);
    rt.pointer_move(x + w - 14.0, y + h / 2.0);
    rt.frame(FRAME_S);
    assert_eq!(
        saved(&rt).shape.roundness,
        before,
        "nothing saved while it drags"
    );
    assert!(
        rt.state.shown_appearance().shape.roundness > 1.5,
        "drawn as it drags"
    );
    rt.pointer_up(x + w - 14.0, y + h / 2.0);
    rt.frame(FRAME_S);
    assert!(saved(&rt).shape.roundness > 1.5, "saved when it lets go");
}

#[test]
fn a_slider_drag_cancelled_by_escape_puts_the_value_back_and_saves_nothing() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &RowId::Shape.key());
    let slider = RowId::Slider(ShapeValue::Roundness).key();
    let [x, y, w, h] = rect(&rt, &slider);
    let before = saved(&rt).shape.roundness;
    rt.pointer_down(x + 14.0, y + h / 2.0);
    rt.frame(FRAME_S);
    rt.pointer_move(x + w - 14.0, y + h / 2.0);
    rt.frame(FRAME_S);
    assert!(rt.state.shown_appearance().shape.roundness > 1.5);
    key(&mut rt, Key::Escape, Mods::default());
    assert!(
        (saved(&rt).shape.roundness - before).abs() < f32::EPSILON,
        "nothing saved"
    );
    assert!(
        (rt.state.shown_appearance().shape.roundness - before).abs() < f32::EPSILON,
        "drawn as it was"
    );
    assert!(
        rt.find(SETTINGS_KEY).is_some(),
        "the Escape went to the drag"
    );
}

#[test]
fn a_sliders_arrow_keys_step_and_save() {
    let mut rt = app();
    open(&mut rt);
    click(&mut rt, &RowId::Shape.key());
    let slider = RowId::Slider(ShapeValue::TextSize).key();
    rt.accessibility_action(&slider, Action::Focus, None);
    rt.frame(FRAME_S);
    key(&mut rt, Key::Right, Mods::default());
    assert_eq!(saved(&rt).shape.text_pt, 15.0);
    key(&mut rt, Key::End, Mods::default());
    assert_eq!(saved(&rt).shape.text_pt, 16.0);
}

#[test]
fn a_failed_save_keeps_the_old_value_and_shows_one_danger_line_with_try_again() {
    let mut rt = app();
    // A settings file under a regular file can never be written.
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("file");
    std::fs::write(&blocker, b"").unwrap();
    rt.update(move |shell, _tx| {
        shell.appearance = AppearanceStore::from_prefs(Prefs::at(blocker.join("prefs.json")));
    });
    open(&mut rt);
    click(&mut rt, &RowId::Grouped.key());
    assert!(saved(&rt).messages.grouped, "the old value stays");
    let tree = tree(&rt);
    assert_eq!(
        tree.matches("\"role\":\"alert\"").count(),
        1,
        "one line: {tree}"
    );
    assert!(tree.contains("Couldn't save this setting."), "{tree}");
    assert!(rt.find("settings-retry").is_some());
}

#[test]
fn the_panel_at_rest_asks_for_no_frames() {
    let mut rt = app();
    open(&mut rt);
    for _ in 0..120 {
        if !rt.frame(FRAME_S) {
            return;
        }
    }
    panic!("the open panel never rests");
}

#[test]
fn use_the_system_font_shows_once_the_system_names_one_and_sets_the_text_in_it() {
    let mut rt = app();
    open(&mut rt);
    assert!(
        rt.find(&RowId::SystemFont.key()).is_none(),
        "left out while no system font is known"
    );
    rt.update(|shell, tx| {
        assert!(shell.system_font_found("Noto Sans".to_owned(), tx));
        assert!(
            !shell.system_font_found("Noto Sans".to_owned(), tx),
            "the same font again changes nothing"
        );
    });
    rt.frame(FRAME_S);
    assert!(rt.find(&RowId::SystemFont.key()).is_some());
    let family = |rt: &Runtime<Shell>| rt.state.look(true).font(crate::design::TYPE_BODY).family;
    assert_eq!(
        family(&rt),
        rune::Family::Sans,
        "Inter until the switch is on"
    );
    click(&mut rt, &RowId::SystemFont.key());
    assert!(saved(&rt).system_font);
    assert_eq!(family(&rt), rune::Family::Named("Noto Sans"));
}
