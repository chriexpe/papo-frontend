//! The panel's own rules: which page a sub-page belongs to, the search's
//! folding, and what a row shows.

use super::rows::{Context, DiagnosticsFacts, Kind, page};
use super::search::{fold, results};
use super::{PageId, RowId};
use crate::appearance::Appearance;
use crate::conversation_settings::ConversationSettings;
use crate::design::Density;
use crate::general_settings::GeneralSettings;

static CONVERSATIONS: std::sync::LazyLock<ConversationSettings> =
    std::sync::LazyLock::new(ConversationSettings::default);
static NOTIFICATIONS: std::sync::LazyLock<crate::notification_settings::NotificationPrefs> =
    std::sync::LazyLock::new(crate::notification_settings::NotificationPrefs::default);
static GENERAL: std::sync::LazyLock<GeneralSettings> =
    std::sync::LazyLock::new(GeneralSettings::default);
static TRUSTED: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
static PLAYERS: std::sync::LazyLock<crate::embedded_players::PlayerSettings> =
    std::sync::LazyLock::new(crate::embedded_players::PlayerSettings::default);
static SITES: std::collections::BTreeSet<papo_sdk_ui::PlayerProvider> =
    std::collections::BTreeSet::new();
static DIAGNOSTICS: std::sync::LazyLock<DiagnosticsFacts> =
    std::sync::LazyLock::new(DiagnosticsFacts::default);

fn context(appearance: &Appearance) -> Context<'_> {
    Context {
        appearance,
        conversations: &CONVERSATIONS,
        notifications: &NOTIFICATIONS,
        general: &GENERAL,
        trusted: &TRUSTED,
        players: &PLAYERS,
        sites_with_data: &SITES,
        diagnostics: &DIAGNOSTICS,
        account: None,
        system_dark: true,
        system_font_known: false,
        reduced_motion: false,
        pictures: None,
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

/// Diagnostics' rows for `facts`, by row.
fn diagnostics_rows(facts: &DiagnosticsFacts) -> Vec<super::rows::Row> {
    let appearance = Appearance::default();
    let cx = Context {
        diagnostics: facts,
        ..context(&appearance)
    };
    page(PageId::Diagnostics, &cx)
        .into_iter()
        .flat_map(|group| group.rows)
        .collect()
}

fn sub_of(rows: &[super::rows::Row], id: RowId) -> Option<String> {
    rows.iter()
        .find(|row| row.id == id)
        .and_then(|row| row.sub.clone())
}

#[test]
fn connected_names_the_round_trip_once_a_heartbeat_was_answered() {
    use crate::diagnostics::Link;
    let mut facts = DiagnosticsFacts {
        server: Some("Lumen".to_owned()),
        link: Some(Link::Online {
            round_trip_ms: Some(42),
        }),
        ..DiagnosticsFacts::default()
    };
    let rows = diagnostics_rows(&facts);
    assert_eq!(
        sub_of(&rows, RowId::Connected).as_deref(),
        Some("42 ms to Lumen")
    );
    facts.link = Some(Link::Online {
        round_trip_ms: None,
    });
    let rows = diagnostics_rows(&facts);
    assert_eq!(
        sub_of(&rows, RowId::Connected).as_deref(),
        Some("Connected to Lumen")
    );
    facts.link = Some(Link::Offline);
    let rows = diagnostics_rows(&facts);
    let connected = rows.iter().find(|row| row.id == RowId::Connected).unwrap();
    assert_eq!(connected.title, "Not connected");
}

#[test]
fn without_an_account_there_is_no_connection_to_show() {
    let rows = diagnostics_rows(&DiagnosticsFacts::default());
    let ids: Vec<RowId> = rows.iter().map(|row| row.id).collect();
    assert!(!ids.contains(&RowId::Connected) && !ids.contains(&RowId::Reconnect));
    assert!(ids.contains(&RowId::ClearCache) && ids.contains(&RowId::Version));
    assert_eq!(sub_of(&rows, RowId::Cache), None, "not measured yet");
}

#[test]
fn the_media_cache_shows_its_measured_size() {
    let facts = DiagnosticsFacts {
        cache_bytes: Some(13_002_342),
        ..DiagnosticsFacts::default()
    };
    let rows = diagnostics_rows(&facts);
    assert_eq!(sub_of(&rows, RowId::Cache).as_deref(), Some("12.4 MB used"));
    assert_eq!(crate::strings::size(1000), "1 KB");
    assert_eq!(crate::strings::size(2_684_354_560), "2.5 GB");
    // The SDK's limits are binary: they read as round numbers.
    assert_eq!(
        crate::strings::size(papo_sdk::MAX_ATTACHMENT_BYTES),
        "100 MB"
    );
}

#[test]
fn clear_media_cache_asks_in_its_row_first() {
    let facts = DiagnosticsFacts {
        asking: Some(RowId::ClearCache),
        ..DiagnosticsFacts::default()
    };
    let rows = diagnostics_rows(&facts);
    let clear = rows.iter().find(|row| row.id == RowId::ClearCache).unwrap();
    let Kind::Destructive {
        question, asking, ..
    } = &clear.kind
    else {
        panic!("a destructive row");
    };
    assert!(*asking);
    assert_eq!(question, "Clear the cache?");
}

#[test]
fn notifications_page_hides_sub_toggles_when_master_is_off() {
    use super::rows::{account::AccountFacts, notifications_summary};
    use papo_sdk::{AccountNotificationPolicy, Generation};

    let appearance = Appearance::default();
    let policy_on = AccountNotificationPolicy {
        generation: Generation::for_test(1),
        enabled: true,
        mentions: true,
        message_preview: true,
        sound: false,
    };
    let account_on = AccountFacts {
        server: "Lumen".to_owned(),
        handle: "ana".to_owned(),
        notification_policy: Some(policy_on),
        ..AccountFacts::default()
    };
    let cx_on = Context {
        account: Some(&account_on),
        ..context(&appearance)
    };
    assert_eq!(notifications_summary(&cx_on), "On · tray counter");
    let rows_on: Vec<RowId> = page(PageId::Notifications, &cx_on)
        .into_iter()
        .flat_map(|group| group.rows)
        .map(|row| row.id)
        .collect();
    assert!(rows_on.contains(&RowId::NotifyMessages));
    assert!(rows_on.contains(&RowId::NotifyMentions));
    assert!(rows_on.contains(&RowId::NotifyPreview));
    assert!(rows_on.contains(&RowId::NotifySound));
    assert!(rows_on.contains(&RowId::NotifySyncNote));
    assert!(rows_on.contains(&RowId::TrayCounter));
    assert!(rows_on.contains(&RowId::ChannelNotifications));

    let account_off = AccountFacts {
        notification_policy: Some(AccountNotificationPolicy {
            enabled: false,
            ..policy_on
        }),
        ..account_on
    };
    let cx_off = Context {
        account: Some(&account_off),
        ..context(&appearance)
    };
    assert_eq!(notifications_summary(&cx_off), "Off");
    let rows_off: Vec<RowId> = page(PageId::Notifications, &cx_off)
        .into_iter()
        .flat_map(|group| group.rows)
        .map(|row| row.id)
        .collect();
    assert!(rows_off.contains(&RowId::NotifyMessages));
    assert!(!rows_off.contains(&RowId::NotifyMentions));
    assert!(!rows_off.contains(&RowId::NotifyPreview));
    assert!(!rows_off.contains(&RowId::NotifySound));
    assert!(rows_off.contains(&RowId::TrayCounter));
}
