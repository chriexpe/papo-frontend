//! Pure registry behavior and schema tests, using only synthetic addresses.
//!
//! These tests keep core URL policy and identity authoritative. They exercise
//! immutable replacements and strict loading without a UI, filesystem or secrets.

use serde_json::json;

use super::{Change, MAX_ADDRESS_BYTES, MAX_SERVERS, PersistenceError, ServerList, decode, encode};

/// Synthetic base addresses; no fixture contacts a backend.
const FIRST: &str = "https://example.invalid/first";
/// A second synthetic identity for ordering and selection tests.
const SECOND: &str = "https://example.invalid/second";

fn saved_pair() -> ServerList {
    ServerList::default()
        .apply(Change::Add { url: FIRST.into() })
        .unwrap()
        .apply(Change::Add { url: SECOND.into() })
        .unwrap()
}

fn selected_pair(server: &str) -> ServerList {
    saved_pair()
        .apply(Change::Select {
            server: Some(papo_core::server_key(server)),
        })
        .unwrap()
}

fn document(servers: Vec<String>, selected_server: Option<String>) -> Vec<u8> {
    serde_json::to_vec(&super::Document {
        version: 1,
        servers,
        selected_server,
    })
    .unwrap()
}

#[test]
fn empty_list_round_trips_with_explicit_null_selection() {
    let empty = ServerList::default();
    assert!(empty.servers().is_empty());
    assert_eq!(empty.selected_server(), None);
    assert!(decode(&encode(&empty).unwrap()).unwrap() == empty);
    let schema: serde_json::Value = serde_json::from_slice(&encode(&empty).unwrap()).unwrap();
    assert_eq!(
        schema,
        json!({"version": 1, "servers": [], "selected_server": null})
    );
}

#[test]
fn core_recovery_server_address_policy_canonicalizes_supported_addresses() {
    for raw in [
        " https://EXAMPLE.invalid:443/base/// ",
        "http://example.invalid:8080/base",
        "https://example.invalid/a/../base",
    ] {
        let list = ServerList::default()
            .apply(Change::Add { url: raw.into() })
            .unwrap();
        let canonical = papo_core::recovery::server_address(raw).unwrap();
        assert_eq!(list.servers()[0].url(), canonical);
        assert_eq!(list.servers()[0].key(), papo_core::server_key(&canonical));
        assert_eq!(list.selected_server(), None);
        assert!(decode(&document(vec![raw.into()], None)).unwrap() == list);
    }
}

#[test]
fn core_recovery_server_address_policy_rejects_unsafe_or_ambiguous_urls() {
    for raw in [
        "",
        "example.invalid",
        "https://",
        "ftp://example.invalid",
        "https://synthetic@example.invalid",
        "https://:synthetic@example.invalid",
        "https://example.invalid?",
        "https://example.invalid?synthetic=value",
        "https://example.invalid#",
        "https://example.invalid#synthetic",
    ] {
        assert!(matches!(
            ServerList::default().apply(Change::Add { url: raw.into() }),
            Err(PersistenceError::InvalidAddress)
        ));
        assert!(matches!(
            decode(&document(vec![raw.into()], None)),
            Err(PersistenceError::InvalidDocument)
        ));
    }
}

#[test]
fn duplicate_detection_uses_core_identity_even_for_different_path_case() {
    let original = ServerList::default()
        .apply(Change::Add {
            url: "https://example.invalid/Base".into(),
        })
        .unwrap();
    for raw in [
        "https://EXAMPLE.invalid/Base/",
        "https://example.invalid/base",
        "https://example.invalid:443/Base",
    ] {
        assert!(matches!(
            original.apply(Change::Add { url: raw.into() }),
            Err(PersistenceError::DuplicateServer)
        ));
    }
    assert_eq!(original.servers().len(), 1);
}

#[test]
fn adding_keeps_order_and_does_not_change_existing_selection() {
    let original = ServerList::default()
        .apply(Change::Add { url: FIRST.into() })
        .unwrap()
        .apply(Change::Select {
            server: Some(papo_core::server_key(FIRST)),
        })
        .unwrap();
    let replacement = original.apply(Change::Add { url: SECOND.into() }).unwrap();
    assert_eq!(replacement.servers()[0].url(), FIRST);
    assert_eq!(replacement.servers()[1].url(), SECOND);
    assert_eq!(replacement.selected_server(), original.selected_server());
    assert_eq!(original.servers().len(), 1);
}

#[test]
fn removing_an_unselected_server_preserves_selection_and_other_entries() {
    let original = selected_pair(SECOND);
    let removed = original
        .apply(Change::Remove {
            server: papo_core::server_key(FIRST),
        })
        .unwrap();
    assert_eq!(removed.servers().len(), 1);
    assert_eq!(removed.servers()[0].url(), SECOND);
    assert_eq!(removed.selected_server(), original.selected_server());
    assert_eq!(original.servers().len(), 2);
}

#[test]
fn removing_selected_server_clears_selection_without_choosing_another() {
    let original = selected_pair(FIRST);
    let removed = original
        .apply(Change::Remove {
            server: papo_core::server_key(FIRST),
        })
        .unwrap();
    assert_eq!(removed.selected_server(), None);
    assert_eq!(removed.servers()[0].url(), SECOND);
}

#[test]
fn unknown_removal_and_selection_are_errors_without_mutating_snapshot() {
    let original = saved_pair();
    let before = encode(&original).unwrap();
    for server in [
        FIRST.to_owned(),
        papo_core::server_key("https://example.invalid/unknown"),
    ] {
        assert!(matches!(
            original.apply(Change::Remove {
                server: server.clone()
            }),
            Err(PersistenceError::UnknownServer)
        ));
        assert!(matches!(
            original.apply(Change::Select {
                server: Some(server)
            }),
            Err(PersistenceError::UnknownServer)
        ));
    }
    assert_eq!(encode(&original).unwrap(), before);
}

#[test]
fn selecting_and_clearing_are_explicit_and_preserve_order() {
    let original = saved_pair();
    let key = papo_core::server_key(SECOND);
    let selected = original
        .apply(Change::Select {
            server: Some(key.clone()),
        })
        .unwrap();
    assert_eq!(selected.selected_server(), Some(key.as_str()));
    assert!(selected.servers() == original.servers());
    assert!(selected.apply(Change::Select { server: None }).unwrap() == original);
}

#[test]
fn reorder_is_an_exact_key_permutation_and_preserves_selection() {
    let original = selected_pair(FIRST);
    let reordered = original
        .apply(Change::Reorder {
            servers: vec![papo_core::server_key(SECOND), papo_core::server_key(FIRST)],
        })
        .unwrap();
    assert_eq!(reordered.servers()[0].url(), SECOND);
    assert_eq!(reordered.servers()[1].url(), FIRST);
    assert_eq!(reordered.selected_server(), original.selected_server());
    assert_eq!(original.servers()[0].url(), FIRST);
}

#[test]
fn invalid_reorders_leave_snapshot_unchanged() {
    let original = saved_pair();
    let before = encode(&original).unwrap();
    let first = papo_core::server_key(FIRST);
    let second = papo_core::server_key(SECOND);
    for servers in [
        vec![],
        vec![first.clone()],
        vec![first.clone(), first.clone()],
        vec![
            first.clone(),
            papo_core::server_key("https://example.invalid/unknown"),
        ],
        vec![first.clone(), second, first],
        vec![FIRST.into(), SECOND.into()],
    ] {
        assert!(matches!(
            original.apply(Change::Reorder { servers }),
            Err(PersistenceError::InvalidOrder)
        ));
    }
    assert_eq!(encode(&original).unwrap(), before);
}

#[test]
fn empty_reorder_and_clear_selection_are_valid_for_empty_list() {
    let empty = ServerList::default();
    assert!(empty.apply(Change::Reorder { servers: vec![] }).unwrap() == empty);
    assert!(empty.apply(Change::Select { server: None }).unwrap() == empty);
}

#[test]
fn loaded_duplicates_and_dangling_selection_are_not_silently_repaired() {
    for bytes in [
        document(vec![FIRST.into(), FIRST.to_uppercase()], None),
        document(vec![FIRST.into()], Some(papo_core::server_key(SECOND))),
        document(vec![], Some(papo_core::server_key(FIRST))),
        document(vec![FIRST.into()], Some(FIRST.into())),
    ] {
        assert!(matches!(
            decode(&bytes),
            Err(PersistenceError::InvalidDocument)
        ));
    }
}

#[test]
fn malformed_documents_missing_fields_and_wrong_types_are_rejected() {
    for raw in [
        "",
        "{",
        "null",
        "[]",
        "{}",
        "{\"version\":1}",
        r#"{"version":1,"servers":[]}"#,
        r#"{"version":1,"selected_server":null}"#,
        r#"{"version":"1","servers":[],"selected_server":null}"#,
        r#"{"version":1,"servers":null,"selected_server":null}"#,
        r#"{"version":1,"servers":[1],"selected_server":null}"#,
        r#"{"version":1,"servers":[],"selected_server":false}"#,
        r#"{"version":1,"version":1,"servers":[],"selected_server":null}"#,
        r#"{"version":1,"servers":[],"servers":[],"selected_server":null}"#,
        r#"{"version":1,"servers":[],"selected_server":null,"selected_server":null}"#,
        r#"{"version":1,"servers":[],"selected_server":null} {}"#,
    ] {
        assert!(matches!(
            decode(raw.as_bytes()),
            Err(PersistenceError::InvalidDocument)
        ));
    }
    assert!(matches!(
        decode(&[0xff]),
        Err(PersistenceError::InvalidDocument)
    ));
}

#[test]
fn unknown_fields_are_rejected_in_version_one() {
    for extra in ["token", "password", "account", "cache", "future"] {
        let mut value = json!({"version":1,"servers":[],"selected_server":null});
        value[extra] = json!(null);
        assert!(matches!(
            decode(&serde_json::to_vec(&value).unwrap()),
            Err(PersistenceError::InvalidDocument)
        ));
    }
}

#[test]
fn unsupported_version_is_checked_before_version_one_shape() {
    for raw in [
        r#"{"version":0,"servers":[],"selected_server":null}"#,
        r#"{"version":2,"servers":[],"selected_server":null}"#,
        r#"{"version":2,"future":{"shape":true}}"#,
        r#"{"version":18446744073709551615}"#,
    ] {
        assert!(matches!(
            decode(raw.as_bytes()),
            Err(PersistenceError::UnsupportedVersion)
        ));
    }
}

#[test]
fn round_trip_preserves_state_and_schema_contains_no_secret_fields() {
    let list = selected_pair(FIRST);
    let encoded = encode(&list).unwrap();
    assert!(decode(&encoded).unwrap() == list);
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        value,
        json!({
            "version":1, "servers":[FIRST, SECOND],
            "selected_server":papo_core::server_key(FIRST),
        })
    );
}

#[test]
fn address_limit_accepts_exact_boundary_and_rejects_raw_or_canonical_excess() {
    let prefix = "https://example.invalid/";
    let boundary = format!("{prefix}{}", "a".repeat(MAX_ADDRESS_BYTES - prefix.len()));
    let list = ServerList::default()
        .apply(Change::Add {
            url: boundary.clone(),
        })
        .unwrap();
    assert_eq!(list.servers()[0].url().len(), MAX_ADDRESS_BYTES);
    assert!(decode(&encode(&list).unwrap()).unwrap() == list);
    for raw in [
        format!("{boundary}a"),
        format!("{prefix}{}", "é".repeat(MAX_ADDRESS_BYTES / 3)),
    ] {
        assert!(matches!(
            ServerList::default().apply(Change::Add { url: raw.clone() }),
            Err(PersistenceError::TooLarge)
        ));
        assert!(matches!(
            decode(&document(vec![raw], None)),
            Err(PersistenceError::TooLarge)
        ));
    }
}

#[test]
fn server_count_limit_accepts_boundary_but_rejects_excess() {
    let urls: Vec<_> = (0..MAX_SERVERS)
        .map(|index| format!("https://example.invalid/{index}"))
        .collect();
    let list = decode(&document(urls.clone(), None)).unwrap();
    assert_eq!(list.servers().len(), MAX_SERVERS);
    assert!(matches!(
        list.apply(Change::Add { url: FIRST.into() }),
        Err(PersistenceError::TooLarge)
    ));
    let mut excess = urls;
    excess.push(FIRST.into());
    assert!(matches!(
        decode(&document(excess, None)),
        Err(PersistenceError::TooLarge)
    ));
    assert!(matches!(
        list.apply(Change::Reorder {
            servers: vec![String::new(); MAX_SERVERS + 1]
        }),
        Err(PersistenceError::TooLarge)
    ));
}

#[test]
fn encode_revalidates_internal_invariants_instead_of_emitting_bad_documents() {
    let mut list = saved_pair();
    list.selected_server = Some(papo_core::server_key("https://example.invalid/unknown"));
    assert!(matches!(
        encode(&list),
        Err(PersistenceError::InvalidDocument)
    ));
    list.selected_server = None;
    list.servers[0].key = papo_core::server_key(SECOND);
    assert!(matches!(
        encode(&list),
        Err(PersistenceError::InvalidDocument)
    ));
}
