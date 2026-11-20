//! Headless tests of the shell view: the production view in Rune's
//! `Runtime`, with no window, display or GPU. They prove behaviour, not
//! pixels or the platform's input method. No SDK client is open: requests the
//! screens send are dropped, and the tests post the SDK's answers themselves.

use papo_sdk::{AccountId, ServerAddress, ServerId, Uuid};
use rune::{Env, Ime, Key, KeyEvent, Mods, Runtime};

use super::{COMPOSER_KEY, Shell, view};
use crate::account_watch::{Accounts, Credentials, SignedOut};
use crate::sign_in::{AlreadyAdded, Discovery, Event, Probe, SavedAccounts};

const SIZE_PT: (f32, f32) = (1000.0, 700.0);
const FRAME_S: f64 = 1.0 / 60.0;

/// The shell once the SDK said which accounts are saved.
fn runtime_with(accounts: Accounts) -> Runtime<Shell> {
    let mut shell = Shell::default();
    shell.accounts_changed(accounts);
    let env = Env {
        system_dark: true,
        scale: 1.0,
    };
    let mut rt = Runtime::new(shell, view, SIZE_PT, env);
    rt.frame(0.0);
    rt
}

/// The app with one account signed in.
fn app() -> Runtime<Shell> {
    runtime_with(Accounts {
        saved: 1,
        signed_out: Vec::new(),
    })
}

/// First run: no account saved.
fn first_run() -> Runtime<Shell> {
    runtime_with(Accounts::default())
}

fn server_id() -> ServerId {
    ServerAddress::parse("https://papo.example.invalid")
        .unwrap()
        .id()
}

/// A saved account whose session expired.
fn expired() -> SignedOut {
    SignedOut {
        id: AccountId::new(server_id(), Uuid::nil()),
        credentials: Credentials::Kept,
        address: "https://papo.example.invalid".to_owned(),
        username: "ana".to_owned(),
    }
}

/// The summary with one working account and `expired` signed out.
fn one_expired(rt: &mut Runtime<Shell>) {
    rt.update(|shell, _tx| {
        shell.accounts_changed(Accounts {
            saved: 2,
            signed_out: vec![expired()],
        });
    });
    rt.frame(FRAME_S);
}

fn press(rt: &mut Runtime<Shell>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    rt.frame(FRAME_S);
}

fn type_text(rt: &mut Runtime<Shell>, text: &str) {
    press(rt, Key::Text(text.to_owned()));
}

fn post(rt: &mut Runtime<Shell>, event: Event) {
    rt.update(|shell, _tx| {
        shell.sign_in_event(event);
    });
    rt.frame(FRAME_S);
}

fn tree(rt: &Runtime<Shell>) -> String {
    rt.accessibility_json().to_string()
}

fn settles(rt: &mut Runtime<Shell>) -> bool {
    for _ in 0..120 {
        rt.frame(FRAME_S);
    }
    !rt.frame(FRAME_S) && rt.wake_deadline().is_none()
}

/// Probes an address on the first-run screen and answers that it is a new
/// public server named Lumen.
fn signed_in_screen() -> Runtime<Shell> {
    let mut rt = first_run();
    type_text(&mut rt, "papo.example.invalid");
    press(&mut rt, Key::Enter);
    let probe = Probe::Added {
        id: server_id(),
        discovery: Discovery {
            name: Some("Lumen".to_owned()),
            public: true,
            icon: None,
        },
    };
    post(&mut rt, Event::Probed(probe));
    rt
}

#[test]
fn the_composer_has_focus_at_start_and_takes_typing() {
    let mut rt = app();
    assert!(rt.has_focus());
    type_text(&mut rt, "olá");
    assert_eq!(rt.state.composer.text, "olá");
}

#[test]
fn an_input_method_composition_commits_into_the_composer() {
    let mut rt = app();
    rt.ime(Ime::Preedit("にほん".to_owned(), Some((9, 9)), Vec::new()));
    rt.frame(FRAME_S);
    assert_eq!(
        rt.state.composer.text, "",
        "a preedit is drawn, not inserted"
    );
    rt.ime(Ime::Commit("日本".to_owned()));
    rt.frame(FRAME_S);
    assert_eq!(rt.state.composer.text, "日本");
}

#[test]
fn a_cancelled_composition_leaves_the_text_as_it_was() {
    let mut rt = app();
    type_text(&mut rt, "a");
    rt.ime(Ime::Preedit("か".to_owned(), Some((3, 3)), Vec::new()));
    rt.frame(FRAME_S);
    rt.ime(Ime::Preedit(String::new(), None, Vec::new()));
    rt.frame(FRAME_S);
    assert_eq!(rt.state.composer.text, "a");
}

#[test]
fn assistive_technology_finds_a_labelled_text_field() {
    let rt = app();
    let tree = tree(&rt);
    assert!(
        tree.contains(&format!("\"{COMPOSER_KEY}\"")),
        "the composer is in the tree: {tree}"
    );
    assert!(
        tree.contains("\"Message\""),
        "the composer is named: {tree}"
    );
}

#[test]
fn a_settled_window_asks_for_no_more_frames() {
    let mut rt = app();
    assert!(
        settles(&mut rt),
        "an idle window with a focused field keeps drawing"
    );
}

#[test]
fn nothing_shows_before_the_sdk_says_which_accounts_are_saved() {
    let env = Env {
        system_dark: false,
        scale: 1.0,
    };
    let mut rt = Runtime::new(Shell::default(), view, SIZE_PT, env);
    rt.frame(0.0);
    assert!(rt.find(COMPOSER_KEY).is_none());
    assert!(rt.find("sign-in").is_none());
}

#[test]
fn first_run_fills_the_window_with_the_server_step() {
    let mut rt = first_run();
    assert!(rt.find("sign-in").is_some());
    assert!(rt.find("sign-in-popup").is_none(), "no popup on first run");
    assert!(rt.find(COMPOSER_KEY).is_none());
    type_text(&mut rt, "papo");
    assert_eq!(
        rt.state.fields.address.text, "papo",
        "the address has focus"
    );
    let tree = tree(&rt);
    for name in ["Add a server", "Address", "Connect"] {
        assert!(tree.contains(&format!("\"{name}\"")), "{name}: {tree}");
    }
}

#[test]
fn return_connects_and_the_button_says_so_while_it_waits() {
    let mut rt = first_run();
    type_text(&mut rt, "papo.example.invalid");
    press(&mut rt, Key::Enter);
    assert!(
        tree(&rt).contains("\"Connecting…\""),
        "the busy label: {}",
        tree(&rt)
    );
    assert!(
        rt.wake_deadline().is_some() || rt.frame(FRAME_S),
        "the spinner turns while it waits"
    );
}

#[test]
fn an_address_problem_shows_when_the_field_loses_focus() {
    let mut rt = first_run();
    type_text(&mut rt, "ftp://papo.example.invalid");
    assert!(!tree(&rt).contains("Use http:// or https://."));
    press(&mut rt, Key::Tab);
    assert!(
        tree(&rt).contains("Use http:// or https://."),
        "{}",
        tree(&rt)
    );
}

#[test]
fn a_new_public_server_asks_for_the_account_and_names_itself() {
    let mut rt = signed_in_screen();
    let tree = tree(&rt);
    for name in ["Lumen", "Username", "Password", "Sign in", "Create account"] {
        assert!(tree.contains(&format!("\"{name}\"")), "{name}: {tree}");
    }
    type_text(&mut rt, "ana");
    assert_eq!(
        rt.state.fields.username.text, "ana",
        "the username has focus"
    );
    assert!(settles(&mut rt), "the account step settles once answered");
}

#[test]
fn signing_in_closes_the_screen_and_names_the_server() {
    let mut rt = signed_in_screen();
    post(&mut rt, Event::SignedIn(Ok(())));
    assert!(rt.find("sign-in").is_none());
    assert!(rt.find(COMPOSER_KEY).is_some());
    assert!(tree(&rt).contains("Connected to Lumen"), "{}", tree(&rt));
    rt.accessibility_action("connected-dismiss", rune::accesskit::Action::Click, None);
    rt.frame(FRAME_S);
    assert!(rt.find("connected").is_none());
}

#[test]
fn the_popup_closes_on_escape_and_forgets_what_was_typed() {
    let mut rt = app();
    rt.update(|shell, _tx| shell.add_server());
    rt.frame(FRAME_S);
    assert!(rt.find("sign-in-popup").is_some());
    assert!(
        tree(&rt).contains("\"Close\""),
        "the close button is named: {}",
        tree(&rt)
    );
    type_text(&mut rt, "papo.example.invalid");
    press(&mut rt, Key::Escape);
    assert!(rt.find("sign-in-popup").is_none());
    assert_eq!(rt.state.fields.address.text, "");
}

#[test]
fn an_expired_session_asks_to_sign_in_again_as_the_same_person() {
    let account = expired();
    let rt = runtime_with(Accounts {
        saved: 1,
        signed_out: vec![account],
    });
    assert!(rt.find("sign-in-popup").is_none(), "nothing else to show");
    assert_eq!(rt.state.fields.username.text, "ana");
    let tree = tree(&rt);
    assert!(tree.contains("Session expired. Sign in again."), "{tree}");
    assert!(!tree.contains("\"Create account\""), "{tree}");
    assert!(!tree.contains("\"Change server\""), "{tree}");
}

#[test]
fn a_session_that_expires_while_another_account_works_opens_in_the_popup() {
    let mut rt = app();
    one_expired(&mut rt);
    assert!(rt.find("sign-in-popup").is_some());
}

#[test]
fn a_settled_server_step_asks_for_no_more_frames() {
    let mut rt = first_run();
    assert!(settles(&mut rt), "an idle server step keeps drawing");
}

#[test]
fn a_closed_sign_in_again_stays_closed_while_the_account_stays_signed_out() {
    let mut rt = app();
    one_expired(&mut rt);
    press(&mut rt, Key::Escape);
    assert!(rt.find("sign-in-popup").is_none());
    one_expired(&mut rt);
    assert!(rt.find("sign-in-popup").is_none(), "it came straight back");
}

#[test]
fn a_session_that_expires_behind_another_screen_is_offered_once_it_closes() {
    let mut rt = app();
    rt.update(|shell, _tx| shell.add_server());
    rt.frame(FRAME_S);
    one_expired(&mut rt);
    assert!(
        !tree(&rt).contains("Session expired"),
        "the open screen stays"
    );
    press(&mut rt, Key::Escape);
    assert!(
        tree(&rt).contains("Session expired. Sign in again."),
        "{}",
        tree(&rt)
    );
}

#[test]
fn open_on_an_already_added_server_signs_its_signed_out_account_in_again() {
    let mut rt = app();
    rt.update(|shell, _tx| shell.add_server());
    rt.frame(FRAME_S);
    type_text(&mut rt, "papo.example.invalid");
    press(&mut rt, Key::Enter);
    let already = AlreadyAdded {
        id: server_id(),
        name: "Lumen".to_owned(),
        address: "papo.example.invalid".to_owned(),
        saved: SavedAccounts::SignedOut(expired()),
        public: true,
        icon: None,
    };
    post(&mut rt, Event::Probed(Probe::AlreadyAdded(already)));
    assert!(
        tree(&rt).contains("You already added Lumen."),
        "{}",
        tree(&rt)
    );
    rt.accessibility_action("feedback-open", rune::accesskit::Action::Click, None);
    rt.frame(FRAME_S);
    let tree = tree(&rt);
    assert!(rt.find("sign-in-popup").is_some(), "still in the popup");
    assert!(tree.contains("Session expired. Sign in again."), "{tree}");
    assert!(!tree.contains("\"Change server\""), "{tree}");
    assert_eq!(rt.state.fields.username.text, "ana");
}
