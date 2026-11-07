//! Frozen identity vectors and validation contracts; no transport is exercised here.

use super::{ServerAddress, ServerAddressError};

#[test]
fn server_id_v1_golden_vectors_never_change() {
    // Reserved domains and documentation addresses are synthetic, never live servers.
    let vectors = [
        (
            "HTTPS://EXAMPLE.invalid/",
            "https://example.invalid",
            "1d090adb04ff7cd601bb93480cff000d",
        ),
        (
            "https://bücher.invalid/",
            "https://xn--bcher-kva.invalid",
            "e9e7b79903d78c3e82c9bc2d45368ca0",
        ),
        (
            "https://example.invalid:443/",
            "https://example.invalid",
            "1d090adb04ff7cd601bb93480cff000d",
        ),
        (
            "http://example.invalid:80/",
            "http://example.invalid",
            "e13e2cf80c0ba7da17c171e20face7e0",
        ),
        (
            "https://example.invalid:8443/",
            "https://example.invalid:8443",
            "fdb7dc47898463ff18088daed4706cc8",
        ),
        (
            "https://example.invalid/Chat/Room/",
            "https://example.invalid/Chat/Room",
            "db30d4a2ed41a13e85223842af1b4602",
        ),
        (
            "https://example.invalid/A%2fb/%7EUser/",
            "https://example.invalid/A%2fb/%7EUser",
            "4dcb8ad5307ef9df37269ebbfb1ca086",
        ),
        (
            "https://[2001:0DB8:0:0::1]:443/",
            "https://[2001:db8::1]",
            "e77c46e3bacdab2900f1dab3097a86c6",
        ),
        (
            "http://127.1/",
            "http://127.0.0.1",
            "57d72eadecbc488f57c6623ded4d6cbb",
        ),
        (
            "https://example.invalid/path//",
            "https://example.invalid/path/",
            "8699e839ec26badb184a438588cef166",
        ),
    ];
    for (input, canonical, id) in vectors {
        let address = ServerAddress::parse(input).unwrap();
        assert_eq!(address.as_str(), canonical);
        assert_eq!(address.id().to_string(), id);
    }
}

#[test]
fn path_case_distinguishes_servers() {
    let upper = ServerAddress::parse("https://example.invalid/Chat").unwrap();
    let lower = ServerAddress::parse("https://example.invalid/chat").unwrap();
    assert_ne!(upper.id(), lower.id());
}

#[test]
fn userinfo_is_rejected_even_when_empty_or_erased_by_parser() {
    for input in [
        "https://@example.invalid",
        "https://:@example.invalid",
        "https://user@example.invalid",
        "https://user:password@example.invalid",
        "https:\\@example.invalid",
        "https:/\n/@example.invalid",
    ] {
        assert!(matches!(
            ServerAddress::parse(input),
            Err(ServerAddressError::Userinfo)
        ));
    }
}

#[test]
fn at_sign_in_path_is_not_userinfo() {
    assert!(ServerAddress::parse("https://example.invalid/@name").is_ok());
}

#[test]
fn queries_and_fragments_are_rejected_including_empty_ones() {
    for input in [
        "https://example.invalid?",
        "https://example.invalid?key=value",
    ] {
        assert!(matches!(
            ServerAddress::parse(input),
            Err(ServerAddressError::Query)
        ));
    }
    for input in [
        "https://example.invalid#",
        "https://example.invalid#fragment",
    ] {
        assert!(matches!(
            ServerAddress::parse(input),
            Err(ServerAddressError::Fragment)
        ));
    }
}

#[test]
fn non_http_schemes_and_malformed_urls_are_rejected() {
    for input in [
        "ftp://example.invalid",
        "file:///path",
        "data:text/plain,hi",
    ] {
        assert!(matches!(
            ServerAddress::parse(input),
            Err(ServerAddressError::UnsupportedScheme)
        ));
    }
    for input in ["", "/relative", "https://", "https://[broken]"] {
        assert!(matches!(
            ServerAddress::parse(input),
            Err(ServerAddressError::InvalidUrl(_))
        ));
    }
}

#[test]
fn debug_output_redacts_address() {
    let address = ServerAddress::parse("https://example.invalid/Private").unwrap();
    assert_eq!(format!("{address:?}"), "ServerAddress([redacted])");
}

#[test]
fn digest_bytes_match_lowercase_hex_display() {
    let id = ServerAddress::parse("https://example.invalid")
        .unwrap()
        .id();
    assert_eq!(
        *id.as_bytes(),
        [
            0x1d, 0x09, 0x0a, 0xdb, 0x04, 0xff, 0x7c, 0xd6, 0x01, 0xbb, 0x93, 0x48, 0x0c, 0xff,
            0x00, 0x0d
        ]
    );
}
