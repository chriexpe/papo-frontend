//! Owns pure script matching and validation tests; leaves listeners to integration tests.

use super::*;
use crate::Failure;

#[test]
fn invalid_failure_status_is_refused_before_binding() {
    let config = Config {
        rest: vec![RestStep {
            method: "GET".into(),
            path: "/server".into(),
            delay_ms: 0,
            failure: Some(Failure {
                status: 200,
                slug: "internal".into(),
            }),
        }],
        ..Config::default()
    };
    assert!(matches!(validate(&config), Err(Error::Configuration)));
}

#[test]
fn script_waits_for_its_matching_request_and_is_consumed_once() {
    let mut state = State {
        auth: Auth::new(),
        presence: Presence::new(),
        resources: Resources::default(),
        history: crate::history::History::default(),
        member_requests: crate::MemberRequestCounts::default(),
        holds: Vec::new(),
        gateway_accepting: true,
        rest: vec![RestStep {
            method: "GET".into(),
            path: "/server".into(),
            delay_ms: 7,
            failure: None,
        }]
        .into(),
        base_url: String::new(),
        message_body: None,
    };
    assert!(take_step(&mut state, "POST", "/auth/login").is_none());
    assert_eq!(take_step(&mut state, "GET", "/server").unwrap().delay_ms, 7);
    assert!(take_step(&mut state, "GET", "/server").is_none());
}
