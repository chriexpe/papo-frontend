//! Navigation without a window or an SDK: server lists in, selection and
//! rail order out.

use papo_sdk::ServerAddress;

use super::*;
use crate::badges::Badge;
use crate::server_list::ChannelEntry;

fn id(name: &str) -> ServerId {
    ServerAddress::parse(&format!("https://{name}.example.invalid"))
        .unwrap()
        .id()
}

fn server(name: &str, channels: &[&str]) -> ServerEntry {
    ServerEntry {
        id: id(name),
        name: name.to_owned(),
        address: format!("{name}.example.invalid"),
        host: None,
        owner: None,
        icon: None,
        account: None,
        accounts: Vec::new(),
        text_channels: channels
            .iter()
            .map(|channel| ChannelEntry {
                id: format!("{name}-{channel}"),
                name: (*channel).to_owned(),
                topic: None,
                notification_mode: None,
                badge: Badge::default(),
            })
            .collect(),
        voice_channels: Vec::new(),
        direct_badges: std::collections::BTreeMap::new(),
        blocked: Vec::new(),
        own: None,
        member_count: 0,
        notification_policy: None,
        badge: Badge::default(),
    }
}

fn list(names: &[&str]) -> ServerList {
    ServerList {
        servers: names
            .iter()
            .map(|name| server(name, &["general", "media"]))
            .collect(),
        direct: Badge::default(),
        account_badges: std::collections::BTreeMap::new(),
    }
}

fn shown(nav: &Navigation) -> Vec<&str> {
    nav.shown()
        .iter()
        .map(|server| server.name.as_str())
        .collect()
}

fn navigation(names: &[&str]) -> Navigation {
    let mut nav = Navigation::default();
    nav.servers_changed(list(names));
    nav
}

#[test]
fn the_first_server_is_selected_until_another_is_picked() {
    let mut nav = navigation(&["lumen", "mirante"]);
    assert_eq!(nav.selected().unwrap().name, "lumen");
    nav.select_server(id("mirante"));
    assert_eq!(nav.selected().unwrap().name, "mirante");
    nav.select_server(id("unknown"));
    assert_eq!(nav.selected().unwrap().name, "mirante");
}

#[test]
fn a_removed_server_takes_its_selection_with_it() {
    let mut nav = navigation(&["lumen", "mirante"]);
    nav.select_server(id("mirante"));
    nav.removing = Some(id("mirante"));
    nav.servers_changed(list(&["lumen"]));
    assert_eq!(nav.selected().unwrap().name, "lumen");
    assert_eq!(nav.removing, None);
    nav.servers_changed(list(&[]));
    assert!(nav.selected().is_none());
}

#[test]
fn a_server_shows_its_first_text_channel_until_one_is_picked() {
    let mut nav = navigation(&["lumen", "mirante"]);
    assert_eq!(nav.selected_channel(), Some("lumen-general"));
    nav.select_channel("lumen-media");
    assert_eq!(nav.selected_channel(), Some("lumen-media"));
    nav.select_server(id("mirante"));
    assert_eq!(
        nav.selected_channel(),
        Some("mirante-general"),
        "per server"
    );
    nav.select_server(id("lumen"));
    assert_eq!(nav.selected_channel(), Some("lumen-media"));
}

#[test]
fn a_picked_channel_the_server_deleted_falls_back_to_the_first() {
    let mut nav = navigation(&["lumen"]);
    nav.select_channel("lumen-media");
    nav.servers_changed(ServerList {
        servers: vec![server("lumen", &["general"])],
        direct: Badge::default(),
        account_badges: std::collections::BTreeMap::new(),
    });
    assert_eq!(nav.selected_channel(), Some("lumen-general"));
}

#[test]
fn alt_arrows_move_a_server_and_stop_at_the_ends() {
    let mut nav = navigation(&["a", "b", "c"]);
    assert_eq!(nav.nudge(id("a"), -1), None, "already first");
    let order = nav.nudge(id("a"), 1).unwrap();
    assert_eq!(order, [id("b"), id("a"), id("c")]);
    assert_eq!(
        shown(&nav),
        ["b", "a", "c"],
        "shown before the SDK saves it"
    );
    assert_eq!(nav.nudge(id("c"), 1), None, "already last");
}

#[test]
fn the_order_sent_shows_until_the_registry_has_it() {
    let mut nav = navigation(&["a", "b", "c"]);
    nav.nudge(id("c"), -1);
    nav.servers_changed(list(&["a", "b", "c"]));
    assert_eq!(
        shown(&nav),
        ["a", "c", "b"],
        "a stale list does not undo it"
    );
    nav.servers_changed(list(&["a", "c", "b"]));
    assert_eq!(shown(&nav), ["a", "c", "b"]);
    nav.servers_changed(list(&["c", "a", "b"]));
    assert_eq!(shown(&nav), ["c", "a", "b"], "later saved orders win");
}

#[test]
fn a_refused_reorder_shows_the_saved_order_again() {
    let mut nav = navigation(&["a", "b"]);
    nav.nudge(id("a"), 1);
    nav.reorder_failed();
    assert_eq!(shown(&nav), ["a", "b"]);
}

#[test]
fn a_dragged_tile_previews_where_it_lands_and_stays_under_the_pointer() {
    let mut nav = navigation(&["a", "b", "c"]);
    nav.drag_moved(id("a"), RAIL_STEP_PT * 1.4);
    assert_eq!(shown(&nav), ["b", "a", "c"]);
    let offset = nav.drag_offset(id("a"));
    assert!((offset - RAIL_STEP_PT * 0.4).abs() < 0.01, "{offset}");
    assert!(nav.drag_offset(id("b")).abs() < f32::EPSILON);
    nav.drag_moved(id("a"), RAIL_STEP_PT * 9.0);
    assert_eq!(shown(&nav), ["b", "c", "a"], "clamped to the last place");
    let order = nav.drag_ended().unwrap();
    assert_eq!(order, [id("b"), id("c"), id("a")]);
    assert!(nav.drag_offset(id("a")).abs() < f32::EPSILON);
}

#[test]
fn a_drag_that_lands_where_it_started_sends_nothing() {
    let mut nav = navigation(&["a", "b"]);
    nav.drag_moved(id("b"), -RAIL_STEP_PT * 0.3);
    assert_eq!(nav.drag_ended(), None);
}

#[test]
fn a_cancelled_drag_sends_nothing_and_shows_the_saved_order() {
    let mut nav = navigation(&["a", "b"]);
    nav.drag_moved(id("a"), RAIL_STEP_PT);
    nav.drag_cancelled();
    assert_eq!(shown(&nav), ["a", "b"]);
    assert_eq!(nav.drag_ended(), None);
}

#[test]
fn the_open_accounts_close_when_another_server_is_selected_or_theirs_goes() {
    let mut nav = Navigation::default();
    nav.servers_changed(list(&["Lumen", "Mirante"]));
    nav.open_accounts();
    assert!(nav.accounts_open());
    nav.select_server(id("Mirante"));
    nav.select_server(id("Lumen"));
    assert!(!nav.accounts_open(), "they stay closed coming back");
    nav.open_accounts();
    nav.servers_changed(list(&["Mirante"]));
    assert!(!nav.accounts_open());
    nav.select_server(id("Mirante"));
    assert!(!nav.accounts_open());
}

/// One server with `channels`, selected, the first channel showing.
fn one_server(channels: &[&str]) -> Navigation {
    let mut nav = Navigation::default();
    nav.servers_changed(ServerList {
        servers: vec![server("lumen", channels)],
        direct: Badge::default(),
        account_badges: std::collections::BTreeMap::new(),
    });
    nav
}

#[test]
fn back_skips_a_channel_that_went_and_a_pick_clears_what_was_ahead() {
    let mut nav = one_server(&["general", "media", "chat"]);
    nav.select_channel("lumen-media");
    nav.select_channel("lumen-chat");
    nav.servers_changed(ServerList {
        servers: vec![server("lumen", &["general", "chat"])],
        direct: Badge::default(),
        account_badges: std::collections::BTreeMap::new(),
    });
    assert_eq!(
        nav.step_history(false).as_deref(),
        Some("lumen-general"),
        "media went"
    );
    assert_eq!(nav.step_history(true).as_deref(), Some("lumen-chat"));
    nav.step_history(false);
    nav.select_channel("lumen-chat");
    assert_eq!(
        nav.step_history(true),
        None,
        "the pick cleared what was ahead"
    );
}

#[test]
fn the_history_keeps_at_most_fifty_channels_behind() {
    let names: Vec<String> = (0..60).map(|n| format!("c{n}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut nav = one_server(&refs);
    for name in &names {
        nav.select_channel(&format!("lumen-{name}"));
    }
    let mut steps = 0;
    while nav.step_history(false).is_some() {
        steps += 1;
    }
    assert_eq!(steps, 50);
}

#[test]
fn another_account_on_the_server_starts_its_history_anew() {
    let mut nav = one_server(&["general", "media"]);
    nav.select_channel("lumen-media");
    let mut switched = server("lumen", &["general", "media"]);
    switched.account = Some(crate::server_list::AccountEntry {
        id: AccountId::new(id("lumen"), papo_sdk::Uuid::from_u128(9)),
        name: "Bia".to_owned(),
        username: "bia".to_owned(),
    });
    nav.servers_changed(ServerList {
        servers: vec![switched],
        direct: Badge::default(),
        account_badges: std::collections::BTreeMap::new(),
    });
    assert_eq!(nav.step_history(false), None);
}

#[test]
fn no_other_unread_channel_or_a_voice_room_selected_steps_nowhere() {
    let mut nav = one_server(&["general"]);
    assert_eq!(
        nav.unread_channel_beside(1),
        None,
        "one channel, nothing else unread"
    );
    let mut lumen = server("lumen", &["general", "media"]);
    lumen.text_channels[1].badge = Badge::Unread;
    lumen.voice_channels.push(ChannelEntry {
        id: "lumen-hangout".to_owned(),
        name: "hangout".to_owned(),
        topic: None,
        notification_mode: None,
        badge: Badge::default(),
    });
    nav.servers_changed(ServerList {
        servers: vec![lumen],
        direct: Badge::default(),
        account_badges: std::collections::BTreeMap::new(),
    });
    assert_eq!(
        nav.unread_channel_beside(-1),
        Some("lumen-media"),
        "round the start"
    );
    nav.select_channel("lumen-hangout");
    assert_eq!(nav.unread_channel_beside(1), None);
}
