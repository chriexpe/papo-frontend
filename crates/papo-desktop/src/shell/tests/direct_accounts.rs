//! Direct's account pill (`docs/design.md` §10, Direct messages),
//! headless: it shows the account as a server's pill does, its ⇄ carries
//! what every account has unread and opens every server's accounts, each
//! with its badge and the shown one ticked; Escape closes the list and
//! picking a row gives the keyboard back to ⇄.

use std::collections::BTreeMap;

use papo_sdk::UnreadCount;

use super::navigation::{ana, click, entry, focused, named};
use super::*;
use crate::account_switch::{DIRECT_ACCOUNTS_KEY, DIRECT_ACCOUNTS_LIST_KEY, account_key};
use crate::badges::Badge;
use crate::server_list::{AccountEntry, ServerList};

fn bia() -> AccountEntry {
    AccountEntry {
        id: AccountId::new(named("Lumen"), Uuid::from_u128(2)),
        name: "Bia".to_owned(),
        username: "bia".to_owned(),
    }
}

/// Direct, with Ana shown on Lumen and Mirante, Bia saved on Lumen with
/// two mentions waiting.
fn direct_with_accounts() -> Runtime<Shell> {
    let mut rt = app();
    let mut lumen = entry("Lumen", &["general"]);
    lumen.accounts.push(bia());
    let mirante = entry("Mirante", &["geral"]);
    let badges = BTreeMap::from([(bia().id, Badge::new(Some(true), UnreadCount::Exact(2)))]);
    rt.update(move |shell, tx| {
        shell.servers_changed(
            ServerList {
                servers: vec![lumen, mirante],
                direct: Badge::default(),
                account_badges: badges,
            },
            tx,
        );
        shell.select_direct(tx);
    });
    rt.frame(FRAME_S);
    rt
}

fn label_of(rt: &Runtime<Shell>, key: &str) -> String {
    rt.elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key))
        .and_then(|element| element.label)
        .unwrap_or_else(|| panic!("no {key}"))
}

#[test]
fn directs_pill_is_the_account_with_a_switch_carrying_every_accounts_unread() {
    let rt = direct_with_accounts();
    assert!(
        rt.elements()
            .iter()
            .any(|element| element.text.as_deref() == Some("Ana")),
        "the account, as a server's pill shows it"
    );
    let label = label_of(&rt, DIRECT_ACCOUNTS_KEY);
    assert_eq!(label, "Switch account, 2 mentions");
}

#[test]
fn the_pill_lists_every_servers_accounts_with_their_badges() {
    let mut rt = direct_with_accounts();
    click(&mut rt, DIRECT_ACCOUNTS_KEY);
    assert!(rt.find(DIRECT_ACCOUNTS_LIST_KEY).is_some());
    assert_eq!(focused(&rt), Some(account_key(ana("Lumen").id)));
    assert!(
        rt.find(&account_key(ana("Mirante").id)).is_some(),
        "Mirante's too"
    );
    let bia_label = label_of(&rt, &account_key(bia().id));
    assert!(bia_label.contains('2'), "Bia's mentions: {bia_label}");
}

#[test]
fn escape_closes_the_list_and_a_pick_gives_the_keyboard_back_to_the_pill() {
    let mut rt = direct_with_accounts();
    click(&mut rt, DIRECT_ACCOUNTS_KEY);
    press(&mut rt, Key::Escape);
    assert!(rt.find(DIRECT_ACCOUNTS_LIST_KEY).is_none());
    click(&mut rt, DIRECT_ACCOUNTS_KEY);
    click(&mut rt, &account_key(bia().id));
    assert!(rt.find(DIRECT_ACCOUNTS_LIST_KEY).is_none());
    assert_eq!(focused(&rt).as_deref(), Some(DIRECT_ACCOUNTS_KEY));
}
