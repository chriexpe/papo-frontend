//! The server step's address: what the person typed, checked locally before
//! anything is sent (`docs/design.md` §7, Address).
//!
//! Owns adding `https://` when no scheme is typed, sorting the SDK's parse
//! errors, and telling a plain-HTTP address on the open internet from a local
//! one. Parsing and the address's identity are the SDK's
//! (`ServerAddress::parse`); when the problems show is the screen's.

use std::net::Ipv4Addr;

use papo_sdk::{ServerAddress, ServerAddressError};

/// What the address field holds, checked.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum AddressCheck {
    /// Nothing but whitespace.
    Empty,
    /// The SDK would reject it; nothing may be sent.
    Invalid(AddressProblem),
    /// An address the SDK accepts.
    Valid {
        /// The canonical address to probe. Never logged.
        address: String,
        /// Plain HTTP to a host that is not local: allowed, with a warning.
        unencrypted: bool,
    },
}

impl std::fmt::Debug for AddressCheck {
    /// Leaves the address out of logs and test output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddressCheck::Empty => f.write_str("Empty"),
            AddressCheck::Invalid(problem) => f.debug_tuple("Invalid").field(problem).finish(),
            AddressCheck::Valid { unencrypted, .. } => f
                .debug_struct("Valid")
                .field("unencrypted", unencrypted)
                .finish_non_exhaustive(),
        }
    }
}

/// Why an address cannot be used, one per message of `docs/design.md` §7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddressProblem {
    /// A scheme other than `http` or `https`.
    Scheme,
    /// A username or password in the address.
    Userinfo,
    /// A `?` query.
    Query,
    /// A `#` fragment.
    Fragment,
    /// Not an address at all.
    Malformed,
}

/// Checks `typed`, adding `https://` when it names no scheme.
pub(crate) fn check(typed: &str) -> AddressCheck {
    let typed = typed.trim();
    if typed.is_empty() {
        return AddressCheck::Empty;
    }
    // Only `://` marks a scheme: `localhost:8080` is a host and a port.
    let full = if typed.contains("://") {
        typed.to_owned()
    } else {
        format!("https://{typed}")
    };
    match ServerAddress::parse(&full) {
        Ok(address) => {
            let address = address.as_str().to_owned();
            let unencrypted = address
                .strip_prefix("http://")
                .is_some_and(|rest| !is_local(host(rest)));
            AddressCheck::Valid {
                address,
                unencrypted,
            }
        }
        Err(error) => AddressCheck::Invalid(problem(&error)),
    }
}

/// How a server's address is shown under its name: without `https://`, which
/// is the default; any other scheme stays visible.
pub(crate) fn shown(address: &str) -> &str {
    address.strip_prefix("https://").unwrap_or(address)
}

fn problem(error: &ServerAddressError) -> AddressProblem {
    match error {
        ServerAddressError::UnsupportedScheme => AddressProblem::Scheme,
        ServerAddressError::Userinfo => AddressProblem::Userinfo,
        ServerAddressError::Query => AddressProblem::Query,
        ServerAddressError::Fragment => AddressProblem::Fragment,
        // `InvalidUrl`, and any rejection added later: not an address.
        _ => AddressProblem::Malformed,
    }
}

/// The host of a canonical address with its scheme removed: up to the port or
/// the path. An IPv6 literal keeps its brackets.
fn host(rest: &str) -> &str {
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.starts_with('[') {
        return authority.split_inclusive(']').next().unwrap_or(authority);
    }
    authority.split(':').next().unwrap_or(authority)
}

/// Whether plain HTTP to `host` stays on this machine or the local network,
/// as `docs/design.md` §7 lists them: `localhost`, `127.`, `10.`, `192.168.`
/// and `.local`.
fn is_local(host: &str) -> bool {
    let mdns = host
        .strip_suffix(".local")
        .is_some_and(|name| !name.is_empty());
    if host == "localhost" || mdns {
        return true;
    }
    host.parse::<Ipv4Addr>().is_ok_and(|ip| {
        let [first, second, ..] = ip.octets();
        first == 127 || first == 10 || (first == 192 && second == 168)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(typed: &str) -> (String, bool) {
        match check(typed) {
            AddressCheck::Valid {
                address,
                unencrypted,
            } => (address, unencrypted),
            other => panic!("{typed}: {other:?}"),
        }
    }

    #[test]
    fn an_address_without_a_scheme_gets_https() {
        let (address, unencrypted) = valid("  papo.example.com  ");
        assert_eq!(address, "https://papo.example.com");
        assert!(!unencrypted);
    }

    #[test]
    fn a_host_and_port_is_not_taken_for_a_scheme() {
        assert_eq!(valid("localhost:8080").0, "https://localhost:8080");
    }

    #[test]
    fn plain_http_warns_only_off_the_local_network() {
        assert!(valid("http://papo.example.com").1);
        assert!(valid("http://10.example.com").1, "a name is not an address");
        for local in [
            "http://localhost:3000",
            "http://127.0.0.1",
            "http://10.0.0.7:8080/papo",
            "http://192.168.1.20",
            "http://papo.local",
        ] {
            assert!(!valid(local).1, "{local}");
        }
    }

    #[test]
    fn each_parse_error_has_its_own_problem() {
        let cases = [
            ("ftp://papo.example.com", AddressProblem::Scheme),
            (
                "https://ana:secret@papo.example.com",
                AddressProblem::Userinfo,
            ),
            ("papo.example.com/?x=1", AddressProblem::Query),
            ("papo.example.com/#top", AddressProblem::Fragment),
            ("https://", AddressProblem::Malformed),
        ];
        for (typed, problem) in cases {
            assert_eq!(check(typed), AddressCheck::Invalid(problem), "{typed}");
        }
    }

    #[test]
    fn whitespace_is_nothing_typed() {
        assert_eq!(check("   "), AddressCheck::Empty);
    }

    #[test]
    fn the_default_scheme_is_not_shown() {
        assert_eq!(shown("https://papo.example.com"), "papo.example.com");
        assert_eq!(shown("http://papo.local"), "http://papo.local");
    }
}
