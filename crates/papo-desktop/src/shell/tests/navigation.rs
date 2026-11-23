//! The rail and the channel pane, headless: the server list is posted as the
//! SDK's watch would post it.

use rune::accesskit::Action;

use super::*;
use crate::badges::Badge;
use crate::rail::{ADD_KEY, tile_key};
use crate::server_list::{AccountEntry, ChannelEntry, ServerEntry, ServerList};

pub(super) fn named(name: &str) -> ServerId {
    ServerAddress::parse(&format!("https://{name}.example.invalid"))
        .unwrap()
        .id()
}

pub(super) fn ana(server: &str) -> AccountEntry {
    AccountEntry {
        id: AccountId::new(named(server), Uuid::nil()),
        name: "Ana".to_owned(),
        username: "ana".to_owned(),
    }
}

pub(super) fn entry(name: &str, channels: &[&str]) -> ServerEntry {
    ServerEntry {
        id: named(name),
        name: name.to_owned(),
        address: format!("{name}.example.invalid"),
        host: None,
        owner: Some("nadia".to_owned()),
        icon: None,
        account: Some(ana(name)),
        accounts: vec![ana(name)],
        text_channels: channels
            .iter()
            .map(|channel| ChannelEntry {
                id: format!("{name}-{channel}"),
                name: (*channel).to_owned(),
                badge: Badge::default(),
            })
            .collect(),
        voice_channels: vec![ChannelEntry {
            id: format!("{name}-hangout"),
            name: "hangout".to_owned(),
            badge: Badge::default(),
        }],
        direct_badges: std::collections::BTreeMap::new(),
        badge: Badge::default(),
    }
}

/// The app with two servers in the rail.
pub(super) fn with_servers() -> Runtime<Shell> {
    let mut rt = app();
    rt.update(|shell, tx| {
        shell.servers_changed(
            ServerList {
                servers: vec![
                    entry("Lumen", &["general", "media"]),
                    entry("Mirante", &["avisos"]),
                ],
                direct: Badge::default(),
            },
            tx,
        );
    });
    rt.frame(FRAME_S);
    rt
}

pub(super) fn focused(rt: &Runtime<Shell>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|element| element.focused)
        .and_then(|element| element.key)
}

pub(super) fn click(rt: &mut Runtime<Shell>, key: &str) {
    rt.accessibility_action(key, Action::Click, None);
    rt.frame(FRAME_S);
}

#[test]
fn the_rail_names_each_server_in_order_and_ends_with_add() {
    let rt = with_servers();
    let tree = tree(&rt);
    let lumen = tree.find("\"Lumen\"").unwrap();
    let mirante = tree.find("\"Mirante\"").unwrap();
    assert!(lumen < mirante, "saved order: {tree}");
    for name in ["Servers", "Add server", "Channels", "Server options"] {
        assert!(tree.contains(&format!("\"{name}\"")), "{name}: {tree}");
    }
    assert!(rt.find(ADD_KEY).is_some());
}

#[test]
fn the_selected_server_s_channels_and_account_fill_the_pane() {
    let rt = with_servers();
    let tree = tree(&rt);
    for name in ["general", "media", "hangout", "Ana", "@ana", "by nadia"] {
        assert!(tree.contains(name), "{name}: {tree}");
    }
    assert!(!tree.contains("avisos"), "another server's channel: {tree}");
}

#[test]
fn pressing_a_tile_shows_that_server_s_channels() {
    let mut rt = with_servers();
    click(&mut rt, &tile_key(named("Mirante")));
    let tree = tree(&rt);
    assert!(tree.contains("avisos"), "{tree}");
    assert!(!tree.contains("\"general\""), "{tree}");
}

#[test]
fn pressing_a_channel_selects_it() {
    let mut rt = with_servers();
    assert_eq!(rt.state.nav.selected_channel(), Some("Lumen-general"));
    click(&mut rt, "channel-Lumen-media");
    assert_eq!(rt.state.nav.selected_channel(), Some("Lumen-media"));
}

#[test]
fn tab_goes_rail_then_add_then_the_server_pill_then_channels() {
    let mut rt = with_servers();
    rt.accessibility_action(&tile_key(named("Lumen")), Action::Focus, None);
    rt.frame(FRAME_S);
    let mut order = vec![focused(&rt).unwrap()];
    for _ in 0..4 {
        press(&mut rt, Key::Tab);
        order.push(focused(&rt).unwrap());
    }
    assert_eq!(
        order,
        [
            tile_key(named("Lumen")),
            ADD_KEY.to_owned(),
            "switch-account".to_owned(),
            "server-options".to_owned(),
            "channel-Lumen-general".to_owned(),
        ]
    );
}

#[test]
fn arrows_move_between_servers_selecting_each_and_between_channels() {
    let mut rt = with_servers();
    rt.accessibility_action(&tile_key(named("Lumen")), Action::Focus, None);
    rt.frame(FRAME_S);
    press(&mut rt, Key::Down);
    assert_eq!(focused(&rt), Some(tile_key(named("Mirante"))));
    assert_eq!(
        rt.state.nav.selected().unwrap().name,
        "Mirante",
        "a tab selects as it takes focus"
    );
    rt.accessibility_action("channel-Mirante-avisos", Action::Focus, None);
    rt.frame(FRAME_S);
    press(&mut rt, Key::Down);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("channel-Mirante-hangout"),
        "on into the voice group"
    );
}

#[test]
fn the_server_menu_asks_before_removing() {
    let mut rt = with_servers();
    click(&mut rt, "server-options");
    assert!(tree(&rt).contains("\"Remove server\""), "{}", tree(&rt));
    click(&mut rt, "menu-remove-server");
    assert!(rt.find("menu").is_none(), "the menu closes on a pick");
    assert!(tree(&rt).contains("Remove Lumen?"), "{}", tree(&rt));
    assert!(tree(&rt).contains("You'll be signed out on this device."));
    press(&mut rt, Key::Escape);
    assert!(
        rt.find("remove-server").is_none(),
        "Escape keeps the server"
    );
    assert_eq!(rt.state.nav.servers().servers.len(), 2);
}

#[test]
fn the_menu_key_on_a_tile_opens_its_menu_and_escape_closes_it() {
    let mut rt = with_servers();
    rt.accessibility_action(&tile_key(named("Mirante")), Action::Focus, None);
    rt.frame(FRAME_S);
    press(&mut rt, Key::ContextMenu);
    assert!(rt.find("menu").is_some());
    press(&mut rt, Key::Escape);
    assert!(rt.find("menu").is_none());
}

#[test]
fn add_opens_the_add_server_popup() {
    let mut rt = with_servers();
    click(&mut rt, ADD_KEY);
    assert!(rt.find("sign-in-popup").is_some());
}

#[test]
fn a_settled_app_with_servers_asks_for_no_more_frames() {
    let mut rt = with_servers();
    assert!(settles(&mut rt), "an idle rail keeps drawing");
}

fn center(rt: &Runtime<Shell>, key: &str) -> (f32, f32) {
    let element = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key))
        .unwrap();
    let [x, y, w, h] = element.rect;
    (x + w / 2.0, y + h / 2.0)
}

fn rail_names(rt: &Runtime<Shell>) -> Vec<String> {
    rt.state
        .nav
        .shown()
        .iter()
        .map(|server| server.name.clone())
        .collect()
}

#[test]
fn dragging_a_tile_previews_its_place_and_escape_puts_it_back() {
    let mut rt = with_servers();
    let (x, y) = center(&rt, &tile_key(named("Lumen")));
    rt.pointer_down(x, y);
    rt.frame(FRAME_S);
    for step in 1..=6 {
        #[expect(clippy::cast_precision_loss, reason = "a few small steps")]
        rt.pointer_move(x, y + step as f32 * 10.0);
        rt.frame(FRAME_S);
    }
    assert_eq!(rail_names(&rt), ["Mirante", "Lumen"], "the drop preview");
    press(&mut rt, Key::Escape);
    rt.pointer_up(x, y + 60.0);
    rt.frame(FRAME_S);
    assert_eq!(
        rail_names(&rt),
        ["Lumen", "Mirante"],
        "a cancelled drag moves nothing"
    );
}

#[test]
fn alt_down_moves_the_focused_server_instead_of_the_focus() {
    let mut rt = with_servers();
    rt.accessibility_action(&tile_key(named("Lumen")), Action::Focus, None);
    rt.frame(FRAME_S);
    rt.key(KeyEvent {
        key: Key::Down,
        mods: Mods {
            alt: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
    assert_eq!(
        focused(&rt),
        Some(tile_key(named("Lumen"))),
        "Alt+↓ is not an arrow"
    );
    assert_eq!(rt.state.nav.selected().unwrap().name, "Lumen");
    assert_eq!(
        rt.state.nav.selected_channel(),
        Some("Lumen-general"),
        "a focused tile moves itself, not the channel"
    );
}

#[test]
fn a_sign_in_screen_closes_the_menu_and_the_confirmation_behind_it() {
    let mut rt = with_servers();
    click(&mut rt, "server-options");
    assert!(rt.find("menu").is_some());
    rt.update(|shell, _tx| shell.add_server());
    rt.frame(FRAME_S);
    assert!(rt.find("menu").is_none());
    assert!(rt.find("sign-in-popup").is_some());
}

#[test]
fn return_in_the_removal_confirmation_keeps_the_server() {
    let mut rt = with_servers();
    click(&mut rt, "server-options");
    click(&mut rt, "menu-remove-server");
    assert_eq!(focused(&rt).as_deref(), Some("remove-cancel"));
    press(&mut rt, Key::Enter);
    assert!(rt.find("remove-server").is_none());
    assert_eq!(rt.state.nav.servers().servers.len(), 2);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("server-options"),
        "back to its opener"
    );
}

/// The app with Lumen carrying 120 mentions, its "media" channel 3 or more,
/// its "general" channel unread, and Mirante unknown.
fn with_badges() -> Runtime<Shell> {
    use papo_sdk::UnreadCount;
    let mut lumen = entry("Lumen", &["general", "media", "random"]);
    lumen.badge = Badge::new(Some(true), UnreadCount::Exact(120));
    lumen.text_channels[0].badge = Badge::new(Some(true), UnreadCount::Exact(0));
    lumen.text_channels[1].badge = Badge::new(None, UnreadCount::AtLeast(3));
    let mut mirante = entry("Mirante", &["avisos"]);
    mirante.badge = Badge::new(None, UnreadCount::AtLeast(0));
    let mut rt = app();
    rt.update(|shell, tx| {
        shell.servers_changed(
            ServerList {
                servers: vec![lumen, mirante],
                direct: Badge::default(),
            },
            tx,
        );
    });
    rt.frame(FRAME_S);
    rt
}

#[test]
fn badges_are_in_the_names_assistive_technology_reads() {
    let rt = with_badges();
    let tree = tree(&rt);
    for name in [
        "\"Lumen, 120 mentions\"",
        "\"general, unread messages\"",
        "\"media, at least 3 mentions\"",
        "\"random\"",
        "\"Mirante\"",
    ] {
        assert!(tree.contains(name), "{name}: {tree}");
    }
}

#[test]
fn pills_show_only_the_numbers_the_sdk_gave() {
    let rt = with_badges();
    let texts: Vec<String> = rt
        .elements()
        .into_iter()
        .filter_map(|element| element.text)
        .collect();
    assert!(texts.iter().any(|text| text == "99+"), "{texts:?}");
    assert!(texts.iter().any(|text| text == "3+"), "{texts:?}");
    assert!(
        !texts.iter().any(|text| text == "120" || text == "3"),
        "{texts:?}"
    );
}

#[test]
fn a_settled_app_with_badges_asks_for_no_more_frames() {
    let mut rt = with_badges();
    assert!(settles(&mut rt), "badges keep the window drawing");
}

fn width(rt: &Runtime<Shell>, text: &str) -> f32 {
    rt.elements()
        .into_iter()
        .find(|element| element.text.as_deref() == Some(text))
        .map(|element| element.rect[2])
        .unwrap()
}

#[test]
fn the_selected_bar_wins_over_the_unread_bar() {
    let rt = with_badges();
    // Lumen is selected and unread: one bar beside it, the tall one.
    let bars: Vec<f32> = rt
        .elements()
        .into_iter()
        .filter(|element| {
            element.path.contains("rail-place-") && (element.rect[2] - 4.0).abs() < 0.01
        })
        .map(|element| element.rect[3])
        .collect();
    assert_eq!(bars, [24.0], "{bars:?}");
}

#[test]
fn short_counts_sit_in_a_circle_and_long_ones_in_a_pill() {
    let rt = with_badges();
    let circle = rt
        .elements()
        .into_iter()
        .find(|element| element.text.as_deref() == Some("3+"))
        .unwrap();
    assert!(width(&rt, "3+") <= 18.0, "the text fits the circle");
    assert!(circle.rect[3] <= 18.0);
    assert!(width(&rt, "99+") > 0.0);
}
