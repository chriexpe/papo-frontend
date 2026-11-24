//! Keys that work from anywhere (`docs/design.md` §12), headless: Ctrl+1–9
//! and the tab keys between servers, Alt+⇧↑/↓ between unread channels,
//! Alt+←/→ back and forward through a server's channels, and F6 between
//! the window's regions.

use rune::{Key, KeyEvent, Mods, Runtime};

use super::composer::{channel_id, writing};
use super::navigation::{entry, focused, named};
use super::*;
use crate::badges::Badge;
use crate::composer::COMPOSER_KEY;
use crate::server_list::ServerList;

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

fn alt(shift: bool) -> Mods {
    Mods {
        alt: true,
        shift,
        ..Mods::default()
    }
}

fn selected(rt: &Runtime<Shell>) -> Option<String> {
    rt.state
        .navigation()
        .selected()
        .map(|server| server.name.clone())
}

fn channel(rt: &Runtime<Shell>) -> Option<String> {
    rt.state.navigation().selected_channel().map(str::to_owned)
}

/// Three servers; on Lumen, "media" and "dev" are unread.
fn three_servers() -> Runtime<Shell> {
    let mut rt = app();
    let mut lumen = entry("Lumen", &["general", "media", "chat", "dev"]);
    for unread in [1, 3] {
        lumen.text_channels[unread].badge = Badge::new(Some(true), papo_sdk::UnreadCount::Exact(0));
    }
    rt.update(move |shell, tx| {
        shell.servers_changed(
            ServerList {
                servers: vec![
                    lumen,
                    entry("Mirante", &["avisos"]),
                    entry("Orla", &["praia"]),
                ],
                direct: Badge::default(),
                account_badges: std::collections::BTreeMap::new(),
            },
            tx,
        );
        shell.select_server(named("Lumen"), tx);
    });
    rt.frame(FRAME_S);
    rt
}

#[test]
fn ctrl_and_a_digit_shows_the_server_in_that_place() {
    let mut rt = three_servers();
    key(&mut rt, Key::Command('3'), ctrl());
    assert_eq!(selected(&rt).as_deref(), Some("Orla"));
    key(&mut rt, Key::Command('9'), ctrl());
    assert_eq!(
        selected(&rt).as_deref(),
        Some("Orla"),
        "no ninth server: nothing moves"
    );
}

#[test]
fn the_tab_keys_move_between_servers_round_the_ends() {
    let mut rt = three_servers();
    key(&mut rt, Key::Tab, ctrl());
    assert_eq!(selected(&rt).as_deref(), Some("Mirante"));
    key(&mut rt, Key::PageUp, ctrl());
    key(&mut rt, Key::PageUp, ctrl());
    assert_eq!(selected(&rt).as_deref(), Some("Orla"), "round the start");
    key(
        &mut rt,
        Key::Tab,
        Mods {
            shift: true,
            ..ctrl()
        },
    );
    assert_eq!(selected(&rt).as_deref(), Some("Mirante"));
}

#[test]
fn alt_shift_and_an_arrow_moves_to_the_next_unread_channel_round_the_ends() {
    let mut rt = three_servers();
    key(&mut rt, Key::Down, alt(true));
    assert_eq!(channel(&rt).as_deref(), Some("Lumen-media"));
    key(&mut rt, Key::Down, alt(true));
    assert_eq!(channel(&rt).as_deref(), Some("Lumen-dev"));
    key(&mut rt, Key::Down, alt(true));
    assert_eq!(
        channel(&rt).as_deref(),
        Some("Lumen-media"),
        "round the end"
    );
}

#[test]
fn alt_and_an_arrow_goes_back_and_forward_through_the_servers_channels() {
    let mut rt = three_servers();
    rt.update(|shell, tx| {
        shell.select_channel("Lumen-chat", tx);
        shell.select_channel("Lumen-dev", tx);
    });
    key(&mut rt, Key::Left, alt(false));
    assert_eq!(channel(&rt).as_deref(), Some("Lumen-chat"));
    key(&mut rt, Key::Left, alt(false));
    assert_eq!(channel(&rt).as_deref(), Some("Lumen-general"));
    key(&mut rt, Key::Right, alt(false));
    assert_eq!(channel(&rt).as_deref(), Some("Lumen-chat"));
    rt.update(|shell, tx| shell.select_channel("Lumen-media", tx));
    key(&mut rt, Key::Right, alt(false));
    assert_eq!(
        channel(&rt).as_deref(),
        Some("Lumen-media"),
        "a pick clears what was ahead"
    );
}

#[test]
fn f6_moves_between_the_windows_regions() {
    let mut rt = three_servers();
    let mut seen = Vec::new();
    for _ in 0..4 {
        key(&mut rt, Key::Function(6), Mods::default());
        seen.push(focused(&rt));
    }
    seen.dedup();
    assert!(seen.len() >= 3, "F6 reached several regions: {seen:?}");
}

#[test]
fn ctrl_slash_or_f1_opens_the_shortcuts_sheet_and_escape_or_the_key_closes_it() {
    let mut rt = three_servers();
    key(&mut rt, Key::Command('/'), ctrl());
    assert!(rt.find(crate::shortcuts_sheet::SHORTCUTS_KEY).is_some());
    assert!(
        rt.elements()
            .iter()
            .any(|element| element.label.as_deref() == Some("Pinned messages: Ctrl P, none")),
        "each row names its keys"
    );
    key(&mut rt, Key::Escape, Mods::default());
    assert!(rt.find(crate::shortcuts_sheet::SHORTCUTS_KEY).is_none());
    key(&mut rt, Key::Function(1), Mods::default());
    assert!(rt.find(crate::shortcuts_sheet::SHORTCUTS_KEY).is_some());
    key(&mut rt, Key::Command('/'), ctrl());
    assert!(rt.find(crate::shortcuts_sheet::SHORTCUTS_KEY).is_none());
}

fn jump_rows(rt: &Runtime<Shell>) -> Vec<String> {
    rt.state
        .jump
        .as_ref()
        .map(|jump| {
            jump.found
                .iter()
                .map(|target| target.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn ctrl_k_lists_unread_places_first_and_a_jump_shows_the_channel_and_comes_first_next_time() {
    let mut rt = three_servers();
    // Real channel ids are UUIDs.
    let id = |n: u128| Uuid::from_u128(0x6100 + n).to_string();
    let mut servers = rt.state.navigation().servers().servers.clone();
    for (n, channel) in servers[0].text_channels.iter_mut().enumerate() {
        channel.id = id(n as u128);
    }
    rt.update(move |shell, tx| {
        shell.servers_changed(
            ServerList {
                servers,
                direct: Badge::default(),
                account_badges: std::collections::BTreeMap::new(),
            },
            tx,
        );
    });
    rt.frame(FRAME_S);
    key(&mut rt, Key::Command('k'), ctrl());
    assert_eq!(
        focused(&rt).as_deref(),
        Some(crate::shell::jump::JUMP_FIELD_KEY)
    );
    let rows = jump_rows(&rt);
    assert_eq!(&rows[..2], ["media", "dev"], "unread first: {rows:?}");
    key(&mut rt, Key::Down, Mods::default());
    key(&mut rt, Key::Enter, Mods::default());
    assert!(rt.state.jump.is_none());
    assert_eq!(channel(&rt), Some(id(3)));
    // The last jump comes first next time.
    key(&mut rt, Key::Command('k'), ctrl());
    assert_eq!(jump_rows(&rt).first().map(String::as_str), Some("dev"));
    key(&mut rt, Key::Escape, Mods::default());
    assert!(rt.state.jump.is_none());
}

#[test]
fn ctrl_k_opens_centred_near_the_top_of_the_window() {
    let mut rt = three_servers();
    key(&mut rt, Key::Command('k'), ctrl());
    let elements = rt.elements();
    let window = elements[0].rect;
    let card = elements
        .iter()
        .find(|element| {
            element.role == "dialog"
                && element.label.as_deref() == Some("Jump to a channel, DM, person or server")
        })
        .unwrap()
        .rect;
    let card_centre = card[0] + card[2] / 2.0;
    let window_centre = window[0] + window[2] / 2.0;
    assert!(
        (card_centre - window_centre).abs() < 1.0,
        "centred: {card:?} in {window:?}"
    );
    assert!(
        card[1] < window[1] + window[3] / 3.0,
        "near the top: {card:?} in {window:?}"
    );
}

#[test]
fn the_unread_and_history_keys_work_from_the_text_field() {
    let mut rt = writing("");
    let mut servers = rt.state.navigation().servers().servers.clone();
    servers[0].text_channels[1].badge = Badge::Unread;
    rt.update(move |shell, tx| {
        shell.servers_changed(
            ServerList {
                servers,
                direct: Badge::default(),
                account_badges: std::collections::BTreeMap::new(),
            },
            tx,
        );
        tx.focus(COMPOSER_KEY, true);
    });
    rt.frame(FRAME_S);
    let first = channel(&rt);
    key(&mut rt, Key::Down, alt(true));
    assert_eq!(
        channel(&rt),
        Some(channel_id(2)),
        "the editor's word move gave way"
    );
    key(&mut rt, Key::Left, alt(false));
    assert_eq!(channel(&rt), first, "back, from the field too");
}

#[test]
fn an_open_sheet_keeps_the_app_keys_for_itself() {
    let mut rt = three_servers();
    key(&mut rt, Key::Command('/'), ctrl());
    key(&mut rt, Key::Command('2'), ctrl());
    assert_eq!(selected(&rt).as_deref(), Some("Lumen"), "Ctrl+2 waited");
    key(&mut rt, Key::Command('k'), ctrl());
    assert!(rt.state.jump.is_none(), "no second overlay");
}

#[test]
fn ctrl_k_closes_on_a_press_or_a_scroll_outside_and_keeps_a_scroll_on_it() {
    let mut rt = three_servers();
    let field = super::super::jump::JUMP_FIELD_KEY;
    let field_rect = |rt: &Runtime<Shell>| {
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some(field))
            .map(|element| element.rect)
    };
    key(&mut rt, Key::Command('k'), ctrl());
    let [x, y, _, height] = field_rect(&rt).expect("Ctrl+K open");
    rt.pointer_move(x + 4.0, y + height / 2.0);
    rt.wheel_event(rune::WheelEvent::new(0.0, 40.0).at(x + 4.0, y + height / 2.0));
    // The fingers lift: the next scroll is a gesture of its own.
    rt.wheel_event(rune::WheelEvent::new(0.0, 0.0).phase(rune::GesturePhase::Ended));
    rt.frame(FRAME_S);
    assert!(rt.state.jump.is_some(), "a scroll on the card keeps it");
    // Well below the card, on the dimmed app.
    let (out_x, out_y) = (x + 4.0, y + 400.0);
    rt.pointer_move(out_x, out_y);
    rt.wheel_event(rune::WheelEvent::new(0.0, 40.0).at(out_x, out_y));
    rt.frame(FRAME_S);
    assert!(rt.state.jump.is_none(), "a scroll outside closes it");
    key(&mut rt, Key::Command('k'), ctrl());
    assert!(rt.state.jump.is_some());
    // The card's padding, beside the field, is the card's.
    let (pad_x, pad_y) = (x - 4.0, y + height / 2.0);
    rt.pointer_move(pad_x, pad_y);
    rt.pointer_down(pad_x, pad_y);
    rt.pointer_up(pad_x, pad_y);
    rt.frame(FRAME_S);
    assert!(rt.state.jump.is_some(), "a press on the card keeps it");
    rt.pointer_move(out_x, out_y);
    rt.pointer_down(out_x, out_y);
    rt.pointer_up(out_x, out_y);
    rt.frame(FRAME_S);
    assert!(rt.state.jump.is_none(), "a press outside closes it");
}
