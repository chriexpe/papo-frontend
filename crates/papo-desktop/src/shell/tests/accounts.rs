//! The account switch on the server pill, headless: the server list is
//! posted as the SDK's watch would post it, with two accounts on Lumen.

use rune::accesskit::Action;

use super::navigation::{ana, click, entry, focused, named, with_servers};
use super::*;
use crate::account_switch::{ACCOUNTS_KEY, ADD_ACCOUNT_KEY, account_key};
use crate::channel_pane::SWITCH_ACCOUNT_KEY;
use crate::server_list::{AccountEntry, ServerList};
use crate::sign_in::{Screen, SignIn};

fn bia() -> AccountEntry {
    AccountEntry {
        id: AccountId::new(named("Lumen"), Uuid::from_u128(2)),
        name: "Bia".to_owned(),
        username: "bia".to_owned(),
    }
}

/// The app with Ana shown on Lumen and Bia saved there too.
fn two_accounts() -> Runtime<Shell> {
    let mut rt = app();
    let mut lumen = entry("Lumen", &["general"]);
    lumen.accounts.push(bia());
    rt.update(|shell, tx| {
        shell.servers_changed(
            ServerList {
                servers: vec![lumen],
                direct: crate::badges::Badge::default(),
            },
            tx,
        );
    });
    rt.frame(FRAME_S);
    rt
}

#[test]
fn with_two_accounts_the_switch_says_how_many() {
    let rt = two_accounts();
    assert!(
        tree(&rt).contains("Switch account, 2 accounts"),
        "{}",
        tree(&rt)
    );
    let single = with_servers();
    assert!(!tree(&single).contains("2 accounts"));
    assert!(tree(&single).contains("\"Switch account\""));
}

#[test]
fn the_switch_lists_each_account_with_the_shown_one_checked_and_focused() {
    let mut rt = two_accounts();
    click(&mut rt, SWITCH_ACCOUNT_KEY);
    assert!(rt.find(ACCOUNTS_KEY).is_some());
    let ana_key = account_key(ana("Lumen").id);
    assert_eq!(focused(&rt), Some(ana_key.clone()), "the keyboard on Ana");
    let json = tree(&rt);
    assert!(json.contains("Ana, @ana"), "{json}");
    assert!(json.contains("Bia, @bia"), "{json}");
    assert!(json.contains("Add account"), "{json}");
    let tree = rt.accessibility_json();
    let checked: Vec<_> = [ana_key, account_key(bia().id)]
        .iter()
        .map(|key| states(find(&tree, key).expect("a row")))
        .collect();
    assert!(checked[0].contains(&"checked"), "{json}");
    assert!(checked[1].contains(&"unchecked"), "{json}");
}

/// The node keyed `key` in the accessibility tree.
fn find<'a>(node: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    if node["key"] == key {
        return Some(node);
    }
    node["children"]
        .as_array()?
        .iter()
        .find_map(|child| find(child, key))
}

fn states(node: &serde_json::Value) -> Vec<&str> {
    node["states"].as_array().map_or(Vec::new(), |states| {
        states
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect()
    })
}

#[test]
fn escape_closes_the_switch_and_keeps_the_account() {
    let mut rt = two_accounts();
    click(&mut rt, SWITCH_ACCOUNT_KEY);
    press(&mut rt, Key::Escape);
    assert!(rt.find(ACCOUNTS_KEY).is_none());
    let shown = rt.state.nav.selected().unwrap().account.clone().unwrap();
    assert_eq!(shown.id, ana("Lumen").id);
}

#[test]
fn picking_an_account_closes_the_switch_and_returns_to_it() {
    let mut rt = two_accounts();
    click(&mut rt, SWITCH_ACCOUNT_KEY);
    click(&mut rt, &account_key(bia().id));
    assert!(rt.find(ACCOUNTS_KEY).is_none());
    assert_eq!(focused(&rt).as_deref(), Some(SWITCH_ACCOUNT_KEY));
    // Without an SDK nothing switches; the registry's snapshot would.
    let shown = rt.state.nav.selected().unwrap().account.clone().unwrap();
    assert_eq!(shown.id, ana("Lumen").id);
}

#[test]
fn add_account_opens_the_account_screen_for_this_server_without_change_server() {
    let mut rt = two_accounts();
    click(&mut rt, SWITCH_ACCOUNT_KEY);
    click(&mut rt, ADD_ACCOUNT_KEY);
    assert!(rt.find(ACCOUNTS_KEY).is_none());
    assert!(rt.find("sign-in-popup").is_some());
    let Some(Screen::Account { server, .. }) = rt.state.sign_in().map(SignIn::screen) else {
        panic!("the account step");
    };
    assert_eq!(server.id, named("Lumen"));
    assert!(server.adding);
    assert!(rt.find("change-server").is_none());
    assert!(rt.find("to-create-account").is_some());
}

#[test]
fn closing_add_account_keeps_the_server() {
    let mut rt = two_accounts();
    click(&mut rt, SWITCH_ACCOUNT_KEY);
    click(&mut rt, ADD_ACCOUNT_KEY);
    let left = rt.state.sign_in().and_then(SignIn::abandon);
    assert_eq!(left, None, "a saved server is never left to remove");
}

#[test]
fn the_gear_on_the_grown_pill_opens_the_server_menu_in_its_place() {
    let mut rt = two_accounts();
    click(&mut rt, SWITCH_ACCOUNT_KEY);
    click(&mut rt, "server-options");
    assert!(rt.find(ACCOUNTS_KEY).is_none());
    assert!(rt.find("menu").is_some());
}

#[test]
fn a_settled_switch_asks_for_no_more_frames() {
    let mut rt = two_accounts();
    click(&mut rt, SWITCH_ACCOUNT_KEY);
    assert!(settles(&mut rt), "an open switch keeps drawing");
    rt.accessibility_action(SWITCH_ACCOUNT_KEY, Action::Click, None);
    assert!(settles(&mut rt));
}

#[test]
fn picking_a_signed_out_account_opens_its_sign_in() {
    let mut rt = two_accounts();
    rt.update(|shell, _tx| {
        shell.accounts_changed(Accounts {
            saved: 2,
            signed_out: vec![SignedOut {
                id: bia().id,
                credentials: Credentials::Kept,
                address: "https://Lumen.example.invalid".to_owned(),
                username: "bia".to_owned(),
                signing_in: false,
            }],
        });
    });
    rt.frame(FRAME_S);
    // Offered once, and dismissed.
    press(&mut rt, Key::Escape);
    assert!(rt.find("sign-in-popup").is_none());
    click(&mut rt, SWITCH_ACCOUNT_KEY);
    click(&mut rt, &account_key(bia().id));
    assert!(rt.find("sign-in-popup").is_some(), "{}", tree(&rt));
    assert!(rt.state.sign_in().is_some_and(SignIn::is_renewing));
}

#[test]
fn escape_on_the_menu_from_the_grown_pill_returns_to_the_gear() {
    let mut rt = two_accounts();
    click(&mut rt, SWITCH_ACCOUNT_KEY);
    click(&mut rt, "server-options");
    press(&mut rt, Key::Escape);
    assert!(rt.find("menu").is_none());
    assert_eq!(focused(&rt).as_deref(), Some("server-options"));
}
