//! Pins discovery parsing and failure classification without a server; the loopback
//! tests in `tests/loopback/discovery.rs` cover the requests themselves.

use base64::Engine;
use papo_api::wire::{Problem, Server};

use super::*;

fn wire(public: Option<bool>, icon_blob: Option<String>, icon_format: Option<&str>) -> Server {
    Server {
        name: Some("Lumen".into()),
        public,
        icon_blob,
        icon_format: icon_format.map(str::to_owned),
        ..Server::default()
    }
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn problem(status: u16, slug: &str) -> ClientError {
    ClientError::Api(papo_api::Error::Problem {
        status,
        problem: Box::new(Problem {
            r#type: format!("/errors/{slug}"),
            title: String::new(),
            status: i64::from(status),
            detail: None,
            instance: None,
        }),
    })
}

#[test]
fn discovery_keeps_the_name_access_and_icon() {
    let discovery =
        ServerDiscovery::from_wire(wire(Some(false), Some(encode(b"png")), Some(" png ")));
    assert_eq!(discovery.name.as_deref(), Some("Lumen"));
    assert!(!discovery.public);
    let icon = discovery.icon.unwrap();
    assert_eq!(icon.bytes, b"png");
    assert_eq!(icon.format.as_deref(), Some("png"));
}

#[test]
fn a_server_that_omits_the_access_flag_counts_as_public() {
    assert!(ServerDiscovery::from_wire(wire(None, None, None)).public);
}

#[test]
fn an_icon_larger_than_the_cap_is_dropped() {
    let at_cap = vec![7; MAX_ICON_BYTES];
    assert!(
        ServerDiscovery::from_wire(wire(None, Some(encode(&at_cap)), None))
            .icon
            .is_some()
    );
    let over = vec![7; MAX_ICON_BYTES + 1];
    assert!(
        ServerDiscovery::from_wire(wire(None, Some(encode(&over)), None))
            .icon
            .is_none()
    );
}

#[test]
fn a_malformed_or_blank_icon_is_dropped() {
    assert!(
        ServerDiscovery::from_wire(wire(None, Some("not base64!".into()), None))
            .icon
            .is_none()
    );
    assert!(
        ServerDiscovery::from_wire(wire(None, Some(String::new()), Some("png")))
            .icon
            .is_none()
    );
    let blank_format = ServerDiscovery::from_wire(wire(None, Some(encode(b"x")), Some("")));
    assert_eq!(blank_format.icon.unwrap().format, None);
}

#[test]
fn failures_fall_into_the_categories_clients_word() {
    use papo_api::Error as Api;
    let cases = [
        (problem(404, "not-found"), Some(DiscoveryFailure::NotSetUp)),
        (
            ClientError::Api(Api::Http { status: 404 }),
            Some(DiscoveryFailure::NotPapo),
        ),
        (
            ClientError::Api(Api::Decode),
            Some(DiscoveryFailure::NotPapo),
        ),
        (
            problem(403, "forbidden"),
            Some(DiscoveryFailure::Unreachable),
        ),
        (
            problem(404, "nothing-here"),
            Some(DiscoveryFailure::NotPapo),
        ),
        (
            problem(410, "not-found"),
            Some(DiscoveryFailure::Unreachable),
        ),
        (
            ClientError::Api(Api::Transport),
            Some(DiscoveryFailure::Unreachable),
        ),
        (
            ClientError::Api(Api::NotDispatched),
            Some(DiscoveryFailure::Unreachable),
        ),
        (ClientError::Timeout, Some(DiscoveryFailure::Unreachable)),
        (
            ClientError::Api(Api::Http { status: 502 }),
            Some(DiscoveryFailure::Unreachable),
        ),
        (
            problem(503, "internal"),
            Some(DiscoveryFailure::Unreachable),
        ),
        (
            problem(429, "rate-limit"),
            Some(DiscoveryFailure::Unreachable),
        ),
        (ClientError::Closed, None),
        (ClientError::Api(Api::Address), None),
    ];
    for (error, expected) in cases {
        assert_eq!(DiscoveryFailure::classify(&error), expected, "{error}");
    }
}
