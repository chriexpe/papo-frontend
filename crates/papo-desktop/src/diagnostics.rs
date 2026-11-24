//! What Settings › Diagnostics shows and does (`docs/design.md` §8): an
//! account's connection and its heartbeat's round trip, the media cache's
//! size, clearing it, and reconnecting.
//!
//! Owns reading those from the SDK and the jobs that act on them; the
//! connection, the measurement and the cache stay in the SDK. The rows are
//! `settings`', the shell carries the answers (`shell/settings_diagnostics`).

use papo_sdk::{AccountId, ClientHandle, ConnectionState, ConnectionStatus, RoundTrip};

/// An account's connection, as Diagnostics names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Link {
    /// Opening a connection.
    Connecting,
    /// Connected; the latest heartbeat's round trip in milliseconds, none
    /// until the first is answered.
    Online { round_trip_ms: Option<u32> },
    /// Not connected: no socket, or waiting to try again.
    Offline,
}

impl Link {
    /// The link `status` and `round_trip` say: a round trip counts only
    /// when the connection it measured is the one that is up.
    pub(crate) fn of(status: ConnectionStatus, round_trip: Option<RoundTrip>) -> Link {
        match status.state {
            ConnectionState::Connecting => Link::Connecting,
            ConnectionState::Online => Link::Online {
                round_trip_ms: round_trip
                    .filter(|measured| measured.generation == status.generation)
                    .map(|measured| {
                        u32::try_from(measured.round_trip.as_millis()).unwrap_or(u32::MAX)
                    }),
            },
            _ => Link::Offline,
        }
    }
}

/// The media cache's size, in bytes, as the SDK measured it; none when it
/// could not.
pub(crate) async fn media_bytes(client: ClientHandle) -> Option<u64> {
    match client.disk_usage().await {
        Ok(report) => Some(report.media_bytes),
        Err(error) => {
            log::warn!("the media cache could not be measured: {error}");
            None
        }
    }
}

/// Clears every signed-in account's media from the cache (`clear_cache`
/// removes the files no account refers to any more), then measures it
/// again; true when every account's clear worked.
pub(crate) async fn clear_media(client: ClientHandle) -> (bool, Option<u64>) {
    let registry = std::sync::Arc::clone(&client.subscribe_registry().borrow());
    let mut cleared = true;
    for account in &registry.accounts {
        let Ok(session) = client.session(account.id) else {
            continue;
        };
        if let Err(error) = session.media().clear_cache().await {
            log::warn!("an account's media cache was not cleared: {error}");
            cleared = false;
        }
    }
    (cleared, media_bytes(client).await)
}

/// Starts a new connection for `account`.
pub(crate) fn reconnect(client: &ClientHandle, account: AccountId) {
    match client
        .session(account)
        .and_then(|session| session.reconnect())
    {
        Ok(()) => log::info!("Diagnostics: reconnecting an account"),
        Err(error) => log::info!("Diagnostics: an account did not reconnect: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use papo_sdk::{ConnectionState, ConnectionStatus, Generation, RoundTrip};

    use super::Link;

    fn status(generation: u64, state: ConnectionState) -> ConnectionStatus {
        ConnectionStatus {
            generation: Generation::for_test(generation),
            state,
        }
    }

    #[test]
    fn a_round_trip_counts_only_for_the_connection_that_is_up() {
        let measured = Some(RoundTrip {
            generation: Generation::for_test(2),
            round_trip: Duration::from_millis(42),
        });
        assert_eq!(
            Link::of(status(2, ConnectionState::Online), measured),
            Link::Online {
                round_trip_ms: Some(42)
            }
        );
        assert_eq!(
            Link::of(status(3, ConnectionState::Online), measured),
            Link::Online {
                round_trip_ms: None
            },
            "an older connection's"
        );
        assert_eq!(
            Link::of(status(2, ConnectionState::Connecting), measured),
            Link::Connecting
        );
        assert_eq!(
            Link::of(status(2, ConnectionState::Offline), None),
            Link::Offline
        );
    }
}
