//! A channel's description (`docs/design.md` §8): the strip beside the
//! name for six seconds when the channel opens, again under the pointer,
//! never with the setting off, and the whole topic in a panel on a press.

use rune::Runtime;

use super::navigation::{click, entry, named};
use super::*;
use crate::badges::Badge;
use crate::server_list::ServerList;
use crate::shell::title::TITLE_PILL_KEY;

const STRIP_KEY: &str = "title-description";

/// The app on Lumen's #general, whose topic is `topic`.
fn on_general(topic: Option<&str>) -> Runtime<Shell> {
    let mut rt = app();
    let mut lumen = entry("Lumen", &["general"]);
    lumen.text_channels[0].topic = topic.map(str::to_owned);
    rt.update(move |shell, tx| {
        shell.servers_changed(
            ServerList {
                servers: vec![lumen],
                direct: Badge::default(),
                account_badges: std::collections::BTreeMap::new(),
            },
            tx,
        );
        shell.nav.select_server(named("Lumen"));
        shell.nav.select_channel("Lumen-general");
        // The header shows over a shown conversation.
        let key = crate::timeline::TimelineKey {
            account: AccountId::new(named("Lumen"), Uuid::nil()),
            channel: Uuid::from_u128(7),
        };
        shell.tabs.show(Some(key), false);
        *shell.tabs.shown_mut().unwrap() =
            crate::timeline::OpenTimeline::loaded(key, imbl::Vector::new());
    });
    rt.frame(FRAME_S);
    rt
}

fn opened(rt: &mut Runtime<Shell>) {
    rt.update(Shell::description_opened);
    rt.frame(FRAME_S);
}

#[test]
fn the_strip_shows_as_the_channel_opens_and_goes_after_six_seconds() {
    let mut rt = on_general(Some("Daily chat, be kind"));
    opened(&mut rt);
    assert!(rt.find(STRIP_KEY).is_some());
    assert!(tree(&rt).contains("Daily chat, be kind"));
    rt.update(|shell, tx| {
        shell.poll_description(tx.clock() + 6.5);
    });
    rt.frame(FRAME_S);
    assert!(rt.find(STRIP_KEY).is_none(), "the pill ends after the name");
    rt.update(|shell, _tx| shell.title_hovered(true));
    rt.frame(FRAME_S);
    assert!(rt.find(STRIP_KEY).is_some(), "the pointer brings it back");
}

#[test]
fn with_the_setting_off_the_strip_waits_for_the_pointer() {
    let mut rt = on_general(Some("Daily chat"));
    rt.update(|shell, _tx| {
        shell
            .conversations
            .set_for_test(|settings| settings.channel_description = false);
    });
    opened(&mut rt);
    assert!(rt.find(STRIP_KEY).is_none());
    rt.update(|shell, _tx| shell.title_hovered(true));
    rt.frame(FRAME_S);
    assert!(rt.find(STRIP_KEY).is_some());
}

#[test]
fn a_press_on_the_pill_opens_the_whole_topic() {
    let mut rt = on_general(Some("Daily chat, be kind"));
    click(&mut rt, TITLE_PILL_KEY);
    let topic = rt
        .state
        .nav
        .menu
        .as_ref()
        .and_then(|menu| menu.description.clone());
    assert_eq!(topic.as_deref(), Some("Daily chat, be kind"));
    assert!(rt.find(crate::menu::MENU_KEY).is_some());
}

#[test]
fn a_channel_without_a_topic_shows_and_opens_nothing() {
    let mut rt = on_general(None);
    opened(&mut rt);
    assert!(rt.find(STRIP_KEY).is_none());
    click(&mut rt, TITLE_PILL_KEY);
    assert!(rt.state.nav.menu.is_none());
}

fn rect_of(rt: &Runtime<Shell>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no {key}"))
        .rect
}

#[test]
fn a_long_topic_scrolls_in_a_strip_that_stops_before_the_actions_pill() {
    let long = "A channel for everything that happens around the place, ".repeat(3);
    let mut rt = on_general(Some(&long));
    opened(&mut rt);
    for _ in 0..3 {
        rt.frame(FRAME_S);
    }
    let strip = rect_of(&rt, STRIP_KEY);
    let actions = rect_of(&rt, "actions-pill");
    assert!(
        strip[0] + strip[2] < actions[0],
        "the pill stops before the actions pill: {strip:?} {actions:?}"
    );
    let measured = rt.state.description_width();
    assert!(
        measured.is_some_and(|width_pt| width_pt > strip[2]),
        "the topic is longer than the strip, so it scrolls: {measured:?} {strip:?}"
    );
}

#[test]
fn a_topic_with_line_breaks_shows_on_one_line() {
    let mut rt = on_general(Some("SUBJECT TO CHANGE.\n\nSecond paragraph"));
    opened(&mut rt);
    rt.frame(FRAME_S);
    let strip = rect_of(&rt, STRIP_KEY);
    let pill = rect_of(&rt, TITLE_PILL_KEY);
    assert!(
        strip[3] <= pill[3],
        "inside the pill's height: {strip:?} {pill:?}"
    );
    assert!(tree(&rt).contains("SUBJECT TO CHANGE. Second paragraph"));
}

#[test]
fn once_the_strip_has_gone_the_window_rests() {
    let long = "A channel for everything that happens around the place, ".repeat(3);
    let mut rt = on_general(Some(&long));
    opened(&mut rt);
    rt.update(|shell, tx| {
        shell.poll_description(tx.clock() + 6.5);
    });
    assert!(
        (0..120).any(|_| !rt.frame(FRAME_S)),
        "the scrolling stops with the strip"
    );
}

#[test]
fn the_panel_fits_the_topic_up_to_a_quarter_of_the_window_and_wraps_past_it() {
    let long = "A channel for everything that happens around the place, ".repeat(4);
    let mut rt = on_general(Some(&long));
    opened(&mut rt);
    rt.frame(FRAME_S);
    click(&mut rt, TITLE_PILL_KEY);
    rt.frame(FRAME_S);
    let header = rect_of(&rt, "conversation-header");
    let window_pt = header[0] + header[2];
    let width_pt = rt
        .state
        .nav
        .menu
        .as_ref()
        .map(|menu| menu.width_pt)
        .unwrap();
    assert!(
        width_pt <= (window_pt / 4.0).max(280.0) + 0.5,
        "{width_pt} in a {window_pt} window"
    );
    let menu = rect_of(&rt, crate::menu::MENU_KEY);
    assert!(
        (menu[2] - width_pt).abs() < 1.0,
        "as wide as opened: {menu:?}"
    );
    assert!(menu[3] > 60.0, "on several lines: {menu:?}");

    // A short topic gets a panel as narrow as the panel allows.
    let mut rt = on_general(Some("Daily chat"));
    opened(&mut rt);
    rt.frame(FRAME_S);
    click(&mut rt, TITLE_PILL_KEY);
    let width_pt = rt
        .state
        .nav
        .menu
        .as_ref()
        .map(|menu| menu.width_pt)
        .unwrap();
    assert!((width_pt - 280.0).abs() < 0.5, "{width_pt}");
}
