//! The live list of direct conversations (`docs/design.md` §10, Direct
//! messages): the SDK's list across the accounts the rail shows (each
//! server's shown account, whose badges the Direct tile counts), newest
//! activity first, without people this account blocks.
//!
//! Owns only the watch that posts it. The list, its order and what it
//! leaves out are the SDK's (`DirectMessageService`); a row's server, badge,
//! presence and typing are the shell's to add when it draws.

use std::sync::Arc;

use futures_util::StreamExt;
use imbl::Vector;
use papo_sdk::{AccountId, ClientHandle, RegistrySnapshot, Session};
use papo_sdk_ui::{DirectMessageItem, DirectMessageService};

use crate::sdk_bridge::Updates;
use crate::server_list::shown_account;

/// The rows, newest activity first.
pub(crate) type DirectRows = Vector<Arc<DirectMessageItem>>;

/// Runs until the client closes: posts the list now and on every change,
/// subscribing again whenever the sessions or the shown accounts change.
pub(crate) async fn run<S: 'static>(
    client: ClientHandle,
    updates: Updates<S>,
    apply: fn(&mut S, DirectRows, &mut rune::Tx) -> bool,
) {
    let mut sessions = client.subscribe_session_changes();
    let mut registry_changes = client.subscribe_registry();
    loop {
        sessions.borrow_and_update();
        let accounts = shown_accounts(&registry_changes.borrow_and_update());
        let open: Vec<Session> = accounts
            .iter()
            .filter_map(|account| client.session(*account).ok())
            .collect();
        let subscription = DirectMessageService::new(open).subscribe();
        let mut rows = subscription.snapshot;
        let mut list_updates = subscription.stream;
        let post = |rows: &DirectRows| {
            let rows = rows.clone();
            updates.send_tx(move |state, tx| apply(state, rows, tx));
        };
        post(&rows);
        loop {
            tokio::select! {
                // A session opened or closed: the service is bound to the
                // sessions it got.
                changed = sessions.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    break;
                }
                // A registry change matters only when it changes which
                // account a server shows (a switch, an added server).
                changed = registry_changes.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if shown_accounts(&registry_changes.borrow_and_update()) != accounts {
                        break;
                    }
                }
                update = list_updates.next() => {
                    let Some(update) = update else {
                        // Every session closed: wait for the next one.
                        list_updates = Box::pin(futures_util::stream::pending());
                        continue;
                    };
                    for diff in update.diffs {
                        diff.apply(&mut rows);
                    }
                    post(&rows);
                }
            }
        }
    }
}

/// Each server's shown account, in the rail's order.
fn shown_accounts(registry: &RegistrySnapshot) -> Vec<AccountId> {
    registry
        .servers
        .iter()
        .filter_map(|server| shown_account(&registry.accounts, server.address.id()))
        .map(|account| account.id)
        .collect()
}
