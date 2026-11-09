//! Pull-driven committed registry projection; SDK owns publication and watch closure.

use std::sync::Arc;

use eyeball_im::VectorDiff;
use futures_util::stream;
use papo_sdk::{RegistrySnapshot, ServerId};
use tokio::sync::watch;

use crate::list_diff;
use crate::server_list::{
    AccountListSubscription, AccountListUpdate, ServerListSnapshot, ServerListSubscription,
    ServerListUpdate,
};

/// Captures one registry snapshot without holding a watch borrow during projection.
pub(crate) fn subscribe(
    mut source: watch::Receiver<Arc<RegistrySnapshot>>,
) -> ServerListSubscription {
    let baseline = Arc::clone(&source.borrow_and_update());
    let version = baseline.version;
    let snapshot = project(&baseline);
    let changes = stream::unfold(
        (source, version, snapshot.clone()),
        |(mut source, version, previous)| async move {
            source.changed().await.ok()?;
            let current = Arc::clone(&source.borrow_and_update());
            let next = project(&current);
            let update = update(version, current.version, &previous, &next);
            Some((update, (source, current.version, next)))
        },
    );
    ServerListSubscription {
        version,
        snapshot,
        stream: Box::pin(changes),
    }
}

/// Captures one server's account list, retaining active flags as part of each item.
pub(crate) fn subscribe_accounts(
    mut source: watch::Receiver<Arc<RegistrySnapshot>>,
    server: ServerId,
) -> AccountListSubscription {
    let baseline = Arc::clone(&source.borrow_and_update());
    let version = baseline.version;
    let snapshot = accounts(&baseline, server);
    let changes = stream::unfold(
        (source, version, snapshot.clone()),
        move |(mut source, version, previous)| async move {
            source.changed().await.ok()?;
            let current = Arc::clone(&source.borrow_and_update());
            let next = accounts(&current, server);
            let diffs = if version.checked_add(1) == Some(current.version) {
                list_diff::between(&previous, &next, |account| account.id)
            } else {
                vec![VectorDiff::Reset {
                    values: next.clone(),
                }]
            };
            Some((
                AccountListUpdate {
                    version: current.version,
                    diffs,
                },
                (source, current.version, next),
            ))
        },
    );
    AccountListSubscription {
        version,
        snapshot,
        stream: Box::pin(changes),
    }
}

/// Projects only visible SDK records; filtering/removal policy belongs to SDK commits.
fn project(source: &RegistrySnapshot) -> ServerListSnapshot {
    ServerListSnapshot {
        servers: source.servers.iter().cloned().collect(),
        accounts: source.accounts.iter().cloned().collect(),
        removals: source.removals.iter().cloned().collect(),
    }
}

/// Projects account ownership without interpreting authentication or selection policy.
fn accounts(source: &RegistrySnapshot, server: ServerId) -> imbl::Vector<papo_sdk::SavedAccount> {
    source
        .accounts
        .iter()
        .filter(|account| account.id.server() == server)
        .cloned()
        .collect()
}

/// A gap invalidates incremental history for all lists, not just visibly changed lists.
fn update(
    previous_version: u64,
    version: u64,
    previous: &ServerListSnapshot,
    next: &ServerListSnapshot,
) -> ServerListUpdate {
    if previous_version.checked_add(1) != Some(version) {
        return ServerListUpdate {
            version,
            servers: vec![VectorDiff::Reset {
                values: next.servers.clone(),
            }],
            accounts: vec![VectorDiff::Reset {
                values: next.accounts.clone(),
            }],
            removals: vec![VectorDiff::Reset {
                values: next.removals.clone(),
            }],
        };
    }
    ServerListUpdate {
        version,
        servers: list_diff::between(&previous.servers, &next.servers, |server| {
            server.address.id()
        }),
        accounts: list_diff::between(&previous.accounts, &next.accounts, |account| account.id),
        removals: list_diff::between(&previous.removals, &next.removals, |job| job.id),
    }
}

#[cfg(test)]
mod tests;
