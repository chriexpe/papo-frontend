//! Exercises public secret-byte validation offline; cookie expiry may consult the system clock here.

use papo_api::{CredentialSnapshot, Error, RestClient};
use serde_json::{Value, json};

fn value() -> String {
    (b'A'..=b'Z').map(char::from).collect()
}

fn encoded(base: &str, cookies: Vec<String>) -> Vec<u8> {
    let cookies = Value::Array(cookies.into_iter().map(Value::String).collect());
    serde_json::to_vec(&json!({ "version": 1, "base": base, "cookies": cookies })).unwrap()
}

fn cookies(snapshot: &CredentialSnapshot) -> Vec<String> {
    let envelope: Value = serde_json::from_slice(snapshot.expose_bytes()).unwrap();
    serde_json::from_value(envelope["cookies"].clone()).unwrap()
}

#[test]
fn session_and_persistent_cookie_scopes_survive_public_roundtrip() {
    let base = "https://example.invalid/api/";
    let session = format!(
        "Auth={}; HttpOnly; SameSite=Strict; Secure; Path=/",
        value()
    );
    let persistent = format!(
        "gate={}; Path=/api/private; Domain=example.invalid; Expires=Thu, 01 Jan 2099 00:00:00 GMT",
        value()
    );
    let input = vec![session, persistent];
    let snapshot = CredentialSnapshot::import(&encoded(base, input.clone())).unwrap();
    let client = RestClient::from_snapshot(base, &snapshot).unwrap();
    let output = cookies(&client.credential_snapshot().unwrap());
    assert_eq!(output.len(), 2);
    assert!(input.iter().all(|cookie| output.contains(cookie)));
    assert!(
        output
            .iter()
            .any(|cookie| !cookie.contains("Expires=") && !cookie.contains("Domain="))
    );
}

#[test]
fn invalid_cookie_octets_scope_attributes_and_duplicate_keys_are_rejected() {
    let base = "https://example.invalid/api/";
    for raw in [
        "Auth=x\r\nInjected=y",
        "bad name=x; Path=/",
        "Auth=x y; Path=/",
        "Auth=x; Domain=other.invalid; Path=/",
        "Auth=x; Path=/; Max-Age=300",
        "Auth=x; Path=/; Expires=invalid",
        "Auth=x; Path=/; Unknown=yes",
        "Auth=x; Path=relative",
        "Auth=x; Path=/; Secure; Secure",
        "Auth=x; Path=/\u{202e}",
    ] {
        assert!(matches!(
            CredentialSnapshot::import(&encoded(base, vec![raw.into()])),
            Err(Error::Credentials)
        ));
    }
    let raw = format!("Auth={}; Path=/", value());
    assert!(CredentialSnapshot::import(&encoded(base, vec![raw.clone(), raw])).is_err());
    assert!(
        CredentialSnapshot::import(&encoded(
            base,
            vec![format!("Auth={}; Path=/", "A".repeat(8192))]
        ))
        .is_err()
    );
}

#[test]
fn expired_snapshots_import_but_restore_without_expired_credentials() {
    let base = "https://example.invalid/";
    let snapshot = CredentialSnapshot::import(&encoded(
        base,
        vec![format!(
            "Auth={}; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
            value()
        )],
    ))
    .unwrap();
    let client = RestClient::from_snapshot(base, &snapshot).unwrap();
    assert!(cookies(&client.credential_snapshot().unwrap()).is_empty());
}

#[test]
fn restores_reject_origin_port_and_base_path_changes() {
    let snapshot =
        CredentialSnapshot::import(&encoded("https://example.invalid/api/", vec![])).unwrap();
    for target in [
        "http://example.invalid/api/",
        "https://other.invalid/api/",
        "https://example.invalid:8443/api/",
        "https://example.invalid/other/",
    ] {
        assert!(matches!(
            RestClient::from_snapshot(target, &snapshot),
            Err(Error::Credentials)
        ));
    }
}

#[test]
fn loopback_restore_rejects_non_loopback_without_a_request() {
    let base = "http://example.invalid/";
    let snapshot = CredentialSnapshot::import(&encoded(base, vec![])).unwrap();
    assert!(matches!(
        RestClient::loopback_from_snapshot(base, &snapshot),
        Err(Error::Address)
    ));
}

#[test]
fn public_import_enforces_version_count_size_and_unknown_field_bounds() {
    let base = "https://example.invalid/";
    let mut envelope = json!({ "version": 2, "base": base, "cookies": [] });
    assert!(CredentialSnapshot::import(&serde_json::to_vec(&envelope).unwrap()).is_err());
    envelope["version"] = 1.into();
    envelope["extra"] = true.into();
    assert!(CredentialSnapshot::import(&serde_json::to_vec(&envelope).unwrap()).is_err());
    let too_many = (0..65).map(|index| format!("c{index}=A; Path=/")).collect();
    assert!(CredentialSnapshot::import(&encoded(base, too_many)).is_err());
    let too_large = (0..64)
        .map(|index| format!("c{index}={}; Path=/", "A".repeat(2048)))
        .collect();
    assert!(CredentialSnapshot::import(&encoded(base, too_large)).is_err());
}

#[test]
fn malformed_public_import_never_formats_credential_input() {
    let error = CredentialSnapshot::import(value().as_bytes())
        .err()
        .unwrap();
    assert_eq!(format!("{error:?}"), "invalid API credential snapshot");
    assert!(std::error::Error::source(&error).is_none());
}
