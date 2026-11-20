//! The account steps: unlock, sign in, create account, already added, and
//! signing in again.

use papo_sdk::{AccountId, Uuid};

use super::*;
use crate::account_watch::{Credentials, SignedOut};

#[test]
fn a_public_server_goes_to_sign_in_and_a_private_one_to_its_password() {
    let (public, fields) = added(true);
    assert_eq!(form(&public), Form::Account(AccountForm::SignIn));
    assert!(fields.username.focused);
    let (private, fields) = added(false);
    assert_eq!(
        form(&private),
        Form::Unlock {
            then: AccountForm::SignIn,
            lapsed: false
        }
    );
    assert!(fields.server_password.focused);
}

#[test]
fn the_server_icon_is_decoded_by_the_shell() {
    let (mut flow, mut fields) = typed("papo.example.invalid");
    step(&mut flow, &mut fields, Event::Submit);
    let probe = Probe::Added {
        id: server_id(),
        discovery: Discovery {
            icon: Some(vec![1, 2, 3]),
            ..discovery(true)
        },
    };
    let effects = step(&mut flow, &mut fields, Event::Probed(probe));
    assert_eq!(effects, [Effect::LoadIcon(vec![1, 2, 3])]);
}

#[test]
fn sign_in_sends_the_trimmed_username_and_the_password_as_typed() {
    let (mut flow, mut fields) = added(true);
    let effects = sign_in_typed(&mut flow, &mut fields);
    assert_eq!(
        effects,
        [Effect::SignIn {
            username: "ana".to_owned(),
            password: "Secret!pass".to_owned()
        }]
    );
    let effects = step(&mut flow, &mut fields, Event::SignedIn(Ok(())));
    assert_eq!(
        effects,
        [Effect::Finish {
            server: "Lumen".to_owned()
        }]
    );
}

#[test]
fn sign_in_waits_for_both_fields() {
    let (mut flow, mut fields) = added(true);
    fields.username.replace_text("ana");
    assert_eq!(step(&mut flow, &mut fields, Event::Submit), NOTHING);
}

#[test]
fn a_password_request_before_any_unlock_goes_to_unlock_with_no_message() {
    let (mut flow, mut fields) = added(true);
    sign_in_typed(&mut flow, &mut fields);
    let answer = Event::SignedIn(Err(Failure::AccessRequired));
    step(&mut flow, &mut fields, answer);
    assert_eq!(
        form(&flow),
        Form::Unlock {
            then: AccountForm::SignIn,
            lapsed: false
        }
    );
    assert_eq!(line(&flow), None);
}

#[test]
fn access_that_lapses_after_an_unlock_says_so_and_keeps_the_username() {
    let (mut flow, mut fields) = added(false);
    fields.server_password.replace_text("server-pass");
    step(&mut flow, &mut fields, Event::Edited);
    let effects = step(&mut flow, &mut fields, Event::Submit);
    assert_eq!(
        effects,
        [Effect::Unlock {
            password: "server-pass".to_owned()
        }]
    );
    step(&mut flow, &mut fields, Event::Unlocked(Ok(())));
    assert_eq!(form(&flow), Form::Account(AccountForm::SignIn));
    assert_eq!(
        fields.server_password.text, "",
        "the server password is not kept"
    );
    sign_in_typed(&mut flow, &mut fields);
    let answer = Event::SignedIn(Err(Failure::AccessRequired));
    step(&mut flow, &mut fields, answer);
    assert_eq!(
        form(&flow),
        Form::Unlock {
            then: AccountForm::SignIn,
            lapsed: true
        }
    );
    assert_eq!(fields.username.text, " ana ");
}

#[test]
fn a_wrong_server_password_is_not_wrong_credentials() {
    let (mut flow, mut fields) = added(false);
    fields.server_password.replace_text("nope");
    step(&mut flow, &mut fields, Event::Submit);
    let answer = Event::Unlocked(Err(Failure::WrongCredentials));
    step(&mut flow, &mut fields, answer);
    assert_eq!(line(&flow), Some(Line::WrongServerPassword));
}

#[test]
fn each_sign_in_failure_has_its_line() {
    let cases = [
        (Failure::WrongCredentials, Line::WrongCredentials),
        (Failure::Banned, Line::Banned),
        (Failure::RateLimited, Line::RateLimited),
        (Failure::NoResponse, Line::NoResponse),
        (Failure::Keyring, Line::Keyring),
        (Failure::Other, Line::Failed),
    ];
    for (failure, expected) in cases {
        let (mut flow, mut fields) = added(true);
        sign_in_typed(&mut flow, &mut fields);
        step(&mut flow, &mut fields, Event::SignedIn(Err(failure)));
        assert_eq!(line(&flow), Some(expected), "{failure:?}");
    }
    assert_eq!(Line::RateLimited.tone(), Tone::Warning);
    assert_eq!(Line::NoResponse.action(), Some(LineAction::TryAgain));
}

#[test]
fn change_server_removes_the_server_and_returns_to_the_address() {
    let (mut flow, mut fields) = added(true);
    fields.password.replace_text("typed");
    let effects = step(&mut flow, &mut fields, Event::ChangeServer);
    assert_eq!(effects, [Effect::RemoveServer(server_id())]);
    assert!(matches!(flow.screen(), Screen::Address { .. }));
    assert!(fields.address.focused);
    assert_eq!(fields.password.text, "");
}

#[test]
fn create_account_takes_rules_the_server_will_accept() {
    let (mut flow, mut fields) = added(true);
    step(
        &mut flow,
        &mut fields,
        Event::ShowForm(AccountForm::CreateAccount),
    );
    assert_eq!(form(&flow), Form::Account(AccountForm::CreateAccount));
    fields.new_username.replace_text("ana");
    fields.new_password.replace_text("weakpass");
    assert_eq!(step(&mut flow, &mut fields, Event::Submit), NOTHING);
    fields.new_password.replace_text("Strong!pass");
    let effects = step(&mut flow, &mut fields, Event::Submit);
    assert_eq!(
        effects,
        [Effect::CreateAccount {
            username: "ana".to_owned(),
            password: "Strong!pass".to_owned()
        }]
    );
}

#[test]
fn a_taken_name_stays_on_create_account() {
    let (mut flow, mut fields) = added(true);
    step(
        &mut flow,
        &mut fields,
        Event::ShowForm(AccountForm::CreateAccount),
    );
    fields.new_username.replace_text("ana");
    fields.new_password.replace_text("Strong!pass");
    step(&mut flow, &mut fields, Event::Submit);
    let answer = Event::Created(Created::NotRegistered(Failure::NameTaken));
    step(&mut flow, &mut fields, answer);
    assert_eq!(form(&flow), Form::Account(AccountForm::CreateAccount));
    assert_eq!(line(&flow), Some(Line::NameTaken));
}

#[test]
fn an_account_created_but_not_signed_in_moves_to_sign_in_with_its_credentials() {
    let (mut flow, mut fields) = added(true);
    step(
        &mut flow,
        &mut fields,
        Event::ShowForm(AccountForm::CreateAccount),
    );
    fields.new_username.replace_text("ana");
    fields.new_password.replace_text("Strong!pass");
    step(&mut flow, &mut fields, Event::Submit);
    let answer = Event::Created(Created::NotSignedIn(Failure::NoResponse));
    step(&mut flow, &mut fields, answer);
    assert_eq!(form(&flow), Form::Account(AccountForm::SignIn));
    assert_eq!(fields.username.text, "ana");
    assert_eq!(fields.password.text, "Strong!pass");
    assert_eq!(line(&flow), Some(Line::NoResponse));
}

fn already_added(saved: SavedAccounts) -> AlreadyAdded {
    AlreadyAdded {
        id: server_id(),
        name: "Lumen".to_owned(),
        address: "papo.example.invalid".to_owned(),
        saved,
        public: true,
        icon: None,
    }
}

/// The server step's answer that the server is saved, and Open pressed.
fn open_already_added(saved: SavedAccounts) -> (SignIn, Fields, Vec<Effect>) {
    let (mut flow, mut fields) = typed("papo.example.invalid");
    step(&mut flow, &mut fields, Event::Submit);
    let probe = Probe::AlreadyAdded(already_added(saved));
    step(&mut flow, &mut fields, Event::Probed(probe));
    assert_eq!(line(&flow).unwrap().action(), Some(LineAction::Open));
    let effects = step(&mut flow, &mut fields, Event::Open);
    (flow, fields, effects)
}

#[test]
fn open_on_a_server_with_a_working_account_shows_the_app() {
    let (_, _, effects) = open_already_added(SavedAccounts::Working);
    assert_eq!(effects, [Effect::OpenServer(server_id())]);
}

#[test]
fn open_on_a_server_without_an_account_shows_its_account_steps() {
    let (flow, _, effects) = open_already_added(SavedAccounts::None);
    assert_eq!(effects, NOTHING);
    assert_eq!(form(&flow), Form::Account(AccountForm::SignIn));
}

#[test]
fn open_on_a_server_whose_account_is_signed_out_signs_it_in_again() {
    let account = SignedOut {
        id: AccountId::new(server_id(), Uuid::nil()),
        credentials: Credentials::Missing,
        address: "https://papo.example.invalid".to_owned(),
        username: "ana".to_owned(),
    };
    let (mut flow, mut fields, effects) =
        open_already_added(SavedAccounts::SignedOut(account.clone()));
    assert_eq!(effects, NOTHING);
    let Screen::Account { server, form, .. } = flow.screen() else {
        panic!("on the server step");
    };
    assert_eq!(*form, Form::Account(AccountForm::SignIn));
    assert_eq!(server.name, "Lumen");
    assert_eq!(
        server.renewing,
        Some(Renewing {
            account: account.id,
            credentials: Credentials::Missing
        })
    );
    assert_eq!(fields.username.text, "ana");
    assert!(fields.password.focused);
    assert_eq!(
        step(&mut flow, &mut fields, Event::ChangeServer),
        NOTHING,
        "the same account: no Change server"
    );
}

#[test]
fn signing_in_again_fills_the_username_and_keeps_to_the_same_account() {
    let account = SignedOut {
        id: AccountId::new(server_id(), Uuid::nil()),
        credentials: Credentials::Kept,
        address: "https://papo.example.invalid".to_owned(),
        username: "ana".to_owned(),
    };
    let mut fields = Fields::default();
    let (mut flow, discover) = SignIn::renewing(Host::Window, &account, &mut fields);
    assert_eq!(
        discover,
        Effect::Discover {
            address: "https://papo.example.invalid".to_owned()
        }
    );
    assert_eq!(fields.username.text, "ana");
    assert!(fields.password.focused);
    assert_eq!(
        step(&mut flow, &mut fields, Event::ChangeServer),
        NOTHING,
        "no Change server for the same account"
    );
    step(
        &mut flow,
        &mut fields,
        Event::ShowForm(AccountForm::CreateAccount),
    );
    assert_eq!(form(&flow), Form::Account(AccountForm::SignIn));
    step(
        &mut flow,
        &mut fields,
        Event::Discovered(Some(discovery(true))),
    );
    let Screen::Account { server, .. } = flow.screen() else {
        panic!("on the server step");
    };
    assert_eq!(server.name, "Lumen");
}

#[test]
fn edits_past_the_limits_are_cut() {
    let (mut flow, mut fields) = added(true);
    fields
        .username
        .replace_text("a".repeat(USERNAME_MAX_CHARS + 4));
    fields
        .password
        .replace_text("ç".repeat(PASSWORD_MAX_CHARS + 1));
    step(&mut flow, &mut fields, Event::Edited);
    assert_eq!(fields.username.text.chars().count(), USERNAME_MAX_CHARS);
    assert_eq!(fields.password.text.chars().count(), PASSWORD_MAX_CHARS);
}

#[test]
fn the_password_rules_need_length_an_uppercase_letter_and_a_symbol() {
    assert!(password_rules_met("Ábcdefg!"));
    assert!(!password_rules_met("Abcdefgh"), "no symbol");
    assert!(!password_rules_met("abcdefg!"), "no uppercase letter");
    assert!(!password_rules_met("Abcde!"), "too short");
    assert!(!password_rules_met("Abcdefg "), "a space is not a symbol");
}

#[test]
fn closing_an_account_step_leaves_its_server_for_removal() {
    let (flow, _fields) = added(true);
    assert_eq!(flow.abandon(), Some(server_id()));
    let (flow, _fields) = typed("papo.example.invalid");
    assert_eq!(flow.abandon(), None, "no server saved yet");
}

#[test]
fn closing_a_sign_in_again_keeps_the_account_s_server() {
    let account = SignedOut {
        id: AccountId::new(server_id(), Uuid::nil()),
        credentials: Credentials::Missing,
        address: "https://papo.example.invalid".to_owned(),
        username: "ana".to_owned(),
    };
    let mut fields = Fields::default();
    let (flow, _) = SignIn::renewing(Host::Popup, &account, &mut fields);
    assert_eq!(flow.abandon(), None);
}
