//! The Diagnostics page (`docs/design.md` §8): Connection (Connected, with
//! the heartbeat's round trip to the panel's server, and Reconnect), Cache
//! (the media cache's size and Clear, which asks first) and About (the
//! app's version).
//!
//! Owns the page's rows; the facts are `diagnostics`' and the SDK's.

use super::{Context, Group, Kind, Row};
use crate::diagnostics::Link;
use crate::settings::RowId;
use crate::strings;

/// The app's version, as its package says.
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(super) fn diagnostics(cx: &Context<'_>) -> Vec<Group> {
    let facts = cx.diagnostics;
    let mut connection = Vec::new();
    if let Some(server) = &facts.server {
        let (title, sub) = match facts.link {
            Some(Link::Online {
                round_trip_ms: Some(ms),
            }) => (
                strings::settings_connected(),
                strings::settings_connected_sub(&strings::integer(i64::from(ms)), server),
            ),
            Some(Link::Online {
                round_trip_ms: None,
            }) => (
                strings::settings_connected(),
                strings::settings_connected_to(server),
            ),
            Some(Link::Connecting) | None => (strings::settings_connecting(), server.clone()),
            Some(Link::Offline) => (strings::settings_offline(), server.clone()),
        };
        connection.push(Row {
            id: RowId::Connected,
            title: title.to_owned(),
            sub: Some(sub),
            kind: Kind::Info,
        });
        connection.push(Row {
            id: RowId::Reconnect,
            title: strings::settings_reconnect().to_owned(),
            sub: None,
            kind: Kind::Action,
        });
    }
    let mut groups = Vec::new();
    if !connection.is_empty() {
        groups.push(Group {
            title: Some(strings::settings_group_connection()),
            rows: connection,
        });
    }
    groups.push(Group {
        title: Some(strings::settings_group_cache()),
        rows: vec![
            Row {
                id: RowId::Cache,
                title: strings::settings_cache().to_owned(),
                sub: facts
                    .cache_bytes
                    .map(|bytes| strings::settings_cache_sub(&strings::size(bytes))),
                kind: Kind::Info,
            },
            Row {
                id: RowId::ClearCache,
                title: strings::settings_clear_cache().to_owned(),
                sub: Some(strings::settings_clear_cache_sub().to_owned()),
                kind: Kind::Destructive {
                    question: strings::settings_clear_cache_question().to_owned(),
                    confirm: strings::settings_clear_cache_confirm().to_owned(),
                    asking: facts.asking == Some(RowId::ClearCache),
                    busy: facts.clearing,
                },
            },
        ],
    });
    groups.push(Group {
        title: Some(strings::settings_group_about()),
        rows: vec![Row {
            id: RowId::Version,
            title: strings::settings_version().to_owned(),
            sub: None,
            kind: Kind::Value(VERSION.to_owned()),
        }],
    });
    groups
}
