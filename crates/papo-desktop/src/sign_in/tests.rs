//! The flow's state machine without a window or an SDK: events in, screens
//! and effects out. SDK answers are fed in as the events `requests` would
//! post. The account steps' tests are in `tests/account_steps.rs`.

use papo_sdk::{ServerAddress, ServerId};

use super::*;

mod account_steps;

/// No effects.
const NOTHING: [Effect; 0] = [];

fn server_id() -> ServerId {
    ServerAddress::parse("https://papo.example.invalid")
        .unwrap()
        .id()
}

fn step(flow: &mut SignIn, fields: &mut Fields, event: Event) -> Vec<Effect> {
    flow.step(fields, event).into_iter().collect()
}

/// The server step with `typed` in the address field.
fn typed(address: &str) -> (SignIn, Fields) {
    let mut fields = Fields::default();
    let mut flow = SignIn::new(Host::Window, &mut fields);
    fields.address.replace_text(address);
    step(&mut flow, &mut fields, Event::Edited);
    (flow, fields)
}

fn discovery(public: bool) -> Discovery {
    Discovery {
        name: Some("Lumen".to_owned()),
        public,
        icon: None,
    }
}

/// The account steps of a server just added.
fn added(public: bool) -> (SignIn, Fields) {
    let (mut flow, mut fields) = typed("papo.example.invalid");
    step(&mut flow, &mut fields, Event::Submit);
    let probe = Probe::Added {
        id: server_id(),
        discovery: discovery(public),
    };
    step(&mut flow, &mut fields, Event::Probed(probe));
    (flow, fields)
}

fn form(flow: &SignIn) -> Form {
    match flow.screen() {
        Screen::Account { form, .. } => *form,
        Screen::Address { .. } => panic!("on the server step"),
    }
}

fn line(flow: &SignIn) -> Option<Line> {
    match flow.screen() {
        Screen::Account {
            phase: Phase::Editing(line),
            ..
        }
        | Screen::Address {
            phase: Phase::Editing(line),
            ..
        } => line.clone(),
        Screen::Account { .. } | Screen::Address { .. } => None,
    }
}

fn sign_in_typed(flow: &mut SignIn, fields: &mut Fields) -> Vec<Effect> {
    fields.username.replace_text(" ana ");
    fields.password.replace_text("Secret!pass");
    step(flow, fields, Event::Edited);
    step(flow, fields, Event::Submit)
}

#[test]
fn connect_probes_the_address_with_https_added() {
    let (mut flow, mut fields) = typed("papo.example.invalid");
    let effects = step(&mut flow, &mut fields, Event::Submit);
    assert_eq!(
        effects,
        [Effect::Probe {
            address: "https://papo.example.invalid".to_owned()
        }]
    );
    assert!(matches!(
        flow.screen(),
        Screen::Address {
            phase: Phase::Busy,
            ..
        }
    ));
    assert_eq!(
        step(&mut flow, &mut fields, Event::Submit),
        NOTHING,
        "a second Connect while probing sends nothing"
    );
}

#[test]
fn an_invalid_address_sends_nothing_and_says_why_on_connect() {
    let (mut flow, mut fields) = typed("ftp://papo.example.invalid");
    assert_eq!(
        flow.address_line(&fields),
        None,
        "no problem shows while the field has focus"
    );
    assert_eq!(step(&mut flow, &mut fields, Event::Submit), NOTHING);
    assert_eq!(
        flow.address_line(&fields),
        Some(Line::Address(AddressProblem::Scheme))
    );
}

#[test]
fn an_address_problem_shows_once_the_field_loses_focus() {
    let (flow, mut fields) = typed("papo.example.invalid/?x=1");
    fields.address.focused = false;
    assert_eq!(
        flow.address_line(&fields),
        Some(Line::Address(AddressProblem::Query))
    );
}

#[test]
fn plain_http_off_the_local_network_warns_but_connects() {
    let (mut flow, mut fields) = typed("http://papo.example.invalid");
    let effects = step(&mut flow, &mut fields, Event::Submit);
    assert_eq!(effects.len(), 1);
    assert_eq!(flow.address_line(&fields), Some(Line::Unencrypted));
    assert_eq!(Line::Unencrypted.tone(), Tone::Warning);
}

#[test]
fn a_failed_probe_says_why_and_offers_to_try_again_when_it_may_help() {
    let (mut flow, mut fields) = typed("papo.example.invalid");
    step(&mut flow, &mut fields, Event::Submit);
    let failed = Probe::Failed(ProbeFailure::Unreachable);
    step(&mut flow, &mut fields, Event::Probed(failed));
    let shown = line(&flow).unwrap();
    assert_eq!(shown, Line::Unreachable);
    assert_eq!(shown.action(), Some(LineAction::TryAgain));
    assert_eq!(Line::NotSetUp.action(), None);
    assert_eq!(
        step(&mut flow, &mut fields, Event::Submit).len(),
        1,
        "Try again probes again"
    );
}

#[test]
fn editing_the_address_clears_the_line() {
    let (mut flow, mut fields) = typed("papo.example.invalid");
    step(&mut flow, &mut fields, Event::Submit);
    let failed = Probe::Failed(ProbeFailure::NotPapo);
    step(&mut flow, &mut fields, Event::Probed(failed));
    fields.address.replace_text("other.example.invalid");
    step(&mut flow, &mut fields, Event::Edited);
    assert_eq!(flow.address_line(&fields), None);
}
