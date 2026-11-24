//! The panel's own rules: which page a sub-page belongs to, the search's
//! folding, and what a row shows.

use super::rows::{Context, Kind, page};
use super::search::{fold, results};
use super::{PageId, RowId};
use crate::appearance::Appearance;
use crate::conversation_settings::ConversationSettings;
use crate::design::Density;

static CONVERSATIONS: std::sync::LazyLock<ConversationSettings> =
    std::sync::LazyLock::new(ConversationSettings::default);

fn context(appearance: &Appearance) -> Context<'_> {
    Context {
        appearance,
        conversations: &CONVERSATIONS,
        system_dark: true,
        system_font_known: false,
    }
}

#[test]
fn sub_pages_go_back_to_appearance() {
    assert_eq!(PageId::Background.parent(), Some(PageId::Appearance));
    assert_eq!(PageId::Shape.nav(), PageId::Appearance);
    assert_eq!(PageId::Appearance.parent(), None);
}

#[test]
fn the_search_ignores_case_and_accents() {
    assert_eq!(fold("Aparência"), "aparencia");
    assert_eq!(fold("ESPAÇAMENTO"), "espacamento");
}

#[test]
fn the_search_finds_values_as_well_as_titles() {
    let appearance = Appearance::default();
    let cx = context(&appearance);
    // "Balanced" is Shape and type's value on the Appearance page, and the
    // starting point on its own page.
    let found = results("balanced", &cx);
    let pages: Vec<PageId> = found.iter().map(|(page, _)| *page).collect();
    assert_eq!(pages, vec![PageId::Appearance, PageId::Shape]);
}

#[test]
fn a_row_that_cannot_work_is_left_out() {
    let appearance = Appearance::default();
    let ids: Vec<RowId> = page(PageId::Appearance, &context(&appearance))
        .into_iter()
        .flat_map(|group| group.rows)
        .map(|row| row.id)
        .collect();
    assert!(!ids.contains(&RowId::SystemFont), "no system font named");
    assert!(
        !ids.contains(&RowId::Stacked),
        "Stack only at Extra compact"
    );
    let mut extra = Appearance::default();
    extra.messages.density = Density::ExtraCompact;
    let ids: Vec<RowId> = page(PageId::Appearance, &context(&extra))
        .into_iter()
        .flat_map(|group| group.rows)
        .map(|row| row.id)
        .collect();
    assert!(ids.contains(&RowId::Stacked));
}

#[test]
fn glass_is_a_slider_from_solid_to_liquid() {
    let appearance = Appearance::default();
    let glass = page(PageId::Appearance, &context(&appearance))
        .into_iter()
        .flat_map(|group| group.rows)
        .find(|row| row.id == RowId::Glass)
        .map(|row| row.kind);
    let Some(Kind::Slider(slider)) = glass else {
        panic!("a slider");
    };
    assert_eq!((slider.min, slider.max), (0.0, 1.0));
    assert!(
        (slider.value - 0.5).abs() < f32::EPSILON,
        "frosted at first"
    );
}

#[test]
fn a_drag_draws_from_its_press_and_ends_without_a_draft() {
    use super::{Held, HeldEvent};
    use crate::design::BackgroundColour;
    let origin = BackgroundColour {
        hue_degrees: 30.0,
        vividness: 0.5,
    };
    let drawn = Appearance::default();
    let pressed = Held::Idle.step(HeldEvent::ColourPressed(origin));
    assert_eq!(pressed.origin(), Some(origin));
    assert_eq!(pressed.draft(), None, "nothing drawn before it moves");
    let moving = pressed.step(HeldEvent::Drew(drawn));
    assert_eq!(moving.origin(), Some(origin), "it moves from where it sat");
    assert_eq!(moving.draft(), Some(&drawn));
    let slider = Held::Idle.step(HeldEvent::Drew(drawn));
    assert_eq!(slider.origin(), None, "a slider has no colour to move");
    assert_eq!(moving.step(HeldEvent::Ended), Held::Idle);
}
