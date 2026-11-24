//! The account's pages (`docs/design.md` §8): the nav's subject tile and
//! group, Profile's text rows edited in place (only once the profile is
//! read back), the blocked list, Change password's checks, and Sign out
//! asking first.

use rune::{Key, KeyEvent, Mods, Runtime};

use super::navigation::{click, entry, named};
use super::settings::open;
use super::*;
use crate::badges::Badge;
use crate::own_profile::OwnProfile;
use crate::server_list::{BlockedPerson, ServerList};
use crate::settings::password_form::{NEW_PASSWORD_KEY, REPEAT_PASSWORD_KEY};
use crate::settings::text_row::field_key;
use crate::settings::{PageId, PasswordProblem, RowId};

/// The app with Lumen's account signed in: its profile `known` or not, and
/// one blocked person.
fn signed_in(known: bool) -> Runtime<Shell> {
    let mut rt = app();
    let mut lumen = entry("Lumen", &["general"]);
    lumen.own = Some(OwnProfile {
        nickname: Some("Ana".to_owned()),
        username: Some("ana".to_owned()),
        status: Some("on call".to_owned()),
        known,
        ..OwnProfile::default()
    });
    lumen.blocked = vec![BlockedPerson {
        id: Uuid::from_u128(9),
        name: Some("Rafa".to_owned()),
    }];
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
    });
    rt.frame(FRAME_S);
    open(&mut rt);
    rt
}

fn page(rt: &mut Runtime<Shell>, page: PageId) {
    rt.update(move |shell, tx| shell.settings_page(page, tx));
    rt.frame(FRAME_S);
}

#[test]
fn the_nav_leads_with_the_account_and_its_group() {
    let rt = signed_in(true);
    assert!(
        rt.find(&format!("settings-nav-{:?}", PageId::Profile))
            .is_some()
    );
    assert!(
        rt.find(&format!("settings-nav-{:?}", PageId::Privacy))
            .is_some()
    );
    let shown = tree(&rt);
    assert!(shown.contains("ACCOUNT ON LUMEN"), "{shown}");
}

#[test]
fn without_an_account_the_nav_has_only_the_app_pages() {
    let mut rt = app();
    open(&mut rt);
    assert!(
        rt.find(&format!("settings-nav-{:?}", PageId::Profile))
            .is_none()
    );
    assert!(
        rt.find(&format!("settings-nav-{:?}", PageId::Appearance))
            .is_some()
    );
}

#[test]
fn a_text_row_edits_in_place_and_escape_puts_it_back() {
    let mut rt = signed_in(true);
    page(&mut rt, PageId::Profile);
    assert!(
        rt.find("settings-profile-preview").is_some(),
        "read-only preview"
    );
    click(&mut rt, &RowId::Status.key());
    let editing = rt.state.settings().and_then(|panel| panel.editing.as_ref());
    assert_eq!(
        editing.map(|(row, edit)| (*row, edit.text.clone())),
        Some((RowId::Status, "on call".to_owned()))
    );
    assert!(rt.find(&field_key(RowId::Status)).is_some());
    assert!(tree(&rt).contains("7/64"), "the count");
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(FRAME_S);
    assert!(
        rt.state
            .settings()
            .and_then(|panel| panel.editing.as_ref())
            .is_none()
    );
    assert!(rt.state.settings().is_some(), "the panel stays");
}

#[test]
fn text_rows_wait_until_the_profile_is_read_back() {
    let mut rt = signed_in(false);
    page(&mut rt, PageId::Profile);
    click(&mut rt, &RowId::Nickname.key());
    assert!(
        rt.state
            .settings()
            .and_then(|panel| panel.editing.as_ref())
            .is_none()
    );
}

#[test]
fn a_text_too_long_stays_in_its_field() {
    let mut rt = signed_in(true);
    page(&mut rt, PageId::Profile);
    click(&mut rt, &RowId::Nickname.key());
    rt.update(|shell, _tx| *shell.settings_text_mut() = rune::TextEdit::new("a".repeat(40)));
    rt.frame(FRAME_S);
    click(&mut rt, &format!("{}-save", RowId::Nickname.key()));
    assert!(
        rt.state
            .settings()
            .and_then(|panel| panel.editing.as_ref())
            .is_some()
    );
}

#[test]
fn the_blocked_list_names_each_person_with_unblock() {
    let mut rt = signed_in(true);
    page(&mut rt, PageId::Blocked);
    let shown = tree(&rt);
    assert!(shown.contains("Rafa"), "{shown}");
    assert!(
        rt.find(&format!(
            "{}-unblock",
            RowId::BlockedPerson(Uuid::from_u128(9)).key()
        ))
        .is_some()
    );
}

fn set_passwords(rt: &mut Runtime<Shell>, new: &str, repeat: &str) {
    let (new, repeat) = (new.to_owned(), repeat.to_owned());
    rt.update(move |shell, _tx| {
        *shell.settings_new_password_mut() = rune::TextEdit::new(&new);
        *shell.settings_repeat_password_mut() = rune::TextEdit::new(&repeat);
    });
    rt.frame(FRAME_S);
    click(rt, "settings-password-save");
}

#[test]
fn change_password_checks_the_rules_and_the_repeat_before_sending() {
    let mut rt = signed_in(true);
    page(&mut rt, PageId::Password);
    assert!(rt.find(NEW_PASSWORD_KEY).is_some() && rt.find(REPEAT_PASSWORD_KEY).is_some());
    set_passwords(&mut rt, "short", "short");
    let problem =
        |rt: &Runtime<Shell>| rt.state.settings().and_then(|panel| panel.password.problem);
    assert_eq!(problem(&rt), Some(PasswordProblem::Weak));
    set_passwords(&mut rt, "Longer#pass1", "Longer#pass2");
    assert_eq!(problem(&rt), Some(PasswordProblem::Mismatch));
    assert!(tree(&rt).contains("The passwords don't match."));
    set_passwords(&mut rt, "Longer#pass1", "Longer#pass1");
    assert_eq!(problem(&rt), None);
}

#[test]
fn sign_out_asks_in_its_row_first() {
    let mut rt = signed_in(true);
    page(&mut rt, PageId::Privacy);
    click(&mut rt, &RowId::SignOut.key());
    assert!(tree(&rt).contains("Sign out of Lumen?"));
    assert!(
        rt.state.settings().is_some(),
        "nothing happens until confirmed"
    );
}

/// Whether `rt` stops asking for frames within two seconds.
fn rests(rt: &mut Runtime<Shell>) -> bool {
    (0..120).any(|_| !rt.frame(FRAME_S))
}

#[test]
fn profile_and_diagnostics_at_rest_ask_for_no_frames() {
    let mut rt = signed_in(true);
    page(&mut rt, PageId::Profile);
    assert!(rests(&mut rt), "Profile rests");
    page(&mut rt, PageId::Diagnostics);
    assert!(rests(&mut rt), "Diagnostics rests");
}

#[test]
fn change_password_stays_signed_in_unless_unticked_and_says_the_rest_sign_out() {
    let mut rt = signed_in(true);
    page(&mut rt, PageId::Password);
    let stay = |rt: &Runtime<Shell>| rt.state.settings().map(|panel| panel.password.stay);
    assert_eq!(stay(&rt), Some(true), "on at first");
    let shown = tree(&rt);
    assert!(shown.contains("Stay signed in on this device"), "{shown}");
    assert!(
        shown.contains("Your other sessions will be signed out."),
        "{shown}"
    );
    click(&mut rt, "settings-password-stay");
    assert_eq!(stay(&rt), Some(false));
}
