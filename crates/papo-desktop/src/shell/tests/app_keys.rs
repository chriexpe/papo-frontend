//! Keys that work from anywhere (`docs/design.md` §12), headless: Ctrl+1–9
//! and the tab keys between servers, Alt+⇧↑/↓ between unread channels,
//! Alt+←/→ back and forward through a server's channels, and F6 between
//! the window's regions.

use rune::{Key, KeyEvent, Mods, Runtime};

use super::navigation::{entry, focused, named};
use super::*;
use crate::badges::Badge;
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
