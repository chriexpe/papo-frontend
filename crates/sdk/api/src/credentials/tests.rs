//! Tests structural envelope rejection without cookie parsing, networking, randomness or clock access.

use super::*;

fn encoded(base: &str, cookies: Vec<String>) -> Vec<u8> {
    serde_json::to_vec(&Envelope {
        version: 1,
        base: base.into(),
        cookies,
    })
    .unwrap()
}

#[test]
fn malformed_encoding_is_rejected_without_diagnostics() {
    for bytes in [Vec::new(), vec![0xff], b"{}".to_vec(), b"null".to_vec()] {
        assert!(matches!(decode_envelope(&bytes), Err(Error::Credentials)));
    }
    let error = decode_envelope(b"{").err().unwrap();
    assert_eq!(format!("{error:?}"), "invalid API credential snapshot");
    assert!(std::error::Error::source(&error).is_none());
}

#[test]
fn unknown_versions_and_unknown_fields_are_rejected() {
    let mut envelope: serde_json::Value =
        serde_json::from_slice(&encoded("https://example.invalid/", vec![])).unwrap();
    envelope["version"] = 2.into();
    assert!(decode_envelope(&serde_json::to_vec(&envelope).unwrap()).is_err());
    envelope["version"] = 1.into();
    envelope["extra"] = true.into();
    assert!(decode_envelope(&serde_json::to_vec(&envelope).unwrap()).is_err());
}

#[test]
fn oversized_envelopes_and_cookie_counts_are_rejected_before_cookie_parsing() {
    let cookies = (0..MAX_COOKIES)
        .map(|index| format!("c{index}={}; Path=/", "A".repeat(2048)))
        .collect();
    let bytes = encoded("https://example.invalid/", cookies);
    assert!(bytes.len() > MAX_SNAPSHOT_BYTES);
    assert!(matches!(decode_envelope(&bytes), Err(Error::Credentials)));
    let cookies = (0..=MAX_COOKIES)
        .map(|index| format!("c{index}=A; Path=/"))
        .collect();
    assert!(decode_envelope(&encoded("https://example.invalid/", cookies)).is_err());
}

#[test]
fn invalid_and_noncanonical_addresses_are_rejected_without_cookie_parsing() {
    for base in [
        "ftp://example.invalid/",
        "https://user@example.invalid/",
        "https://example.invalid/?query",
        "https://example.invalid/#fragment",
        "https://example.invalid/api",
        "https://EXAMPLE.invalid/",
    ] {
        assert!(matches!(
            decode_envelope(&encoded(base, vec![])),
            Err(Error::Credentials)
        ));
    }
}

#[test]
fn cookie_validation_is_not_part_of_the_clock_free_envelope_boundary() {
    let bytes = encoded("https://example.invalid/", vec!["not a cookie".into()]);
    let (base, cookies) = decode_envelope(&bytes).unwrap();
    assert_eq!(base.scheme(), "https");
    assert_eq!(cookies.len(), 1);
}
