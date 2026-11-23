//! Watches each signed-in account for what the shell draws live beyond a
//! timeline: its outgoing queue (a chip waits while its change is in
//! flight; a refused reaction is said and dropped), its quick reactions,
//! ranked by the SDK from the account's own use (`quick_reactions`), and
//! its people's activity (presence marks, status phrases, who types where).
//!
//! Owns only the watch loops and what they post. The queue, the usage
//! counts, the ranking and the activity stay in the SDK; what the shell
//! draws and says from them is the shell's.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;

use futures_util::StreamExt;
use papo_sdk::{AccountId, ClientHandle, OutgoingSnapshot, Session};
use papo_sdk_ui::{
    AccountActivity, ActivityService, ExpressionLibraryService, ExpressionLibraryUpdate,
    ExpressionLibraryView, quick_reactions,
};
use rune::Tx;
use tokio::task::{AbortHandle, JoinSet};

use crate::sdk_bridge::Updates;

/// How many quick reactions an account keeps: the message menu's strip.
pub(crate) const QUICK_REACTIONS: usize = 8;

/// What the shell hears about one account.
pub(crate) enum Posted {
    /// Its outgoing queue now.
    Outgoing(Arc<OutgoingSnapshot>),
    /// Its quick reactions now, most used first.
    Quick(Vec<String>),
    /// Its people's activity now.
    People(Arc<AccountActivity>),
    /// The account's session closed: nothing more comes for it.
    Closed,
}

/// Applies a post on the UI thread; returns whether the view changed.
pub(crate) type Apply<S> = fn(&mut S, AccountId, Posted, &mut Tx) -> bool;

/// Runs until the client closes: one watch per open session, started as
/// sessions open and dropped as they close (or with this watch). A watch
/// that ended by itself starts again only when the sessions change, so one
/// that cannot subscribe never spins.
pub(crate) async fn run<S: 'static>(client: ClientHandle, updates: Updates<S>, apply: Apply<S>) {
    let mut sessions = client.subscribe_session_changes();
    let mut tasks = JoinSet::new();
    let mut running: HashMap<AccountId, AbortHandle> = HashMap::new();
    loop {
        sessions.borrow_and_update();
        let registry = Arc::clone(&client.subscribe_registry().borrow());
        let open: Vec<Session> = registry
            .accounts
            .iter()
            .filter_map(|account| client.session(account.id).ok())
            .collect();
        running.retain(|account, task| {
            let kept = open.iter().any(|session| session.account() == *account);
            if !kept {
                task.abort();
                let account = *account;
                updates.send_tx(move |state, tx| apply(state, account, Posted::Closed, tx));
            }
            kept && !task.is_finished()
        });
        for session in open {
            if let Entry::Vacant(slot) = running.entry(session.account()) {
                slot.insert(tasks.spawn(account_watch(session, updates.clone(), apply)));
            }
        }
        loop {
            tokio::select! {
                changed = sessions.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    break;
                }
                // Reaped, not restarted: see above.
                Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
            }
        }
    }
}

/// One account's queue, library and activity, until its session closes.
async fn account_watch<S: 'static>(session: Session, updates: Updates<S>, apply: Apply<S>) {
    let account = session.account();
    let post =
        move |posted: Posted| updates.send_tx(move |state, tx| apply(state, account, posted, tx));
    let mut outgoing = match session.outgoing().subscribe().await {
        Ok(outgoing) => outgoing,
        Err(error) => {
            log::debug!("an account's outgoing queue is not watched: {error}");
            return;
        }
    };
    post(Posted::Outgoing(Arc::clone(&outgoing.borrow_and_update())));
    let people = ActivityService::new(session.clone()).subscribe();
    post(Posted::People(Arc::new(people.snapshot)));
    let mut people = people.stream;
    let library = ExpressionLibraryService::new(session.clone())
        .subscribe()
        .await;
    let mut library = match library {
        Ok(library) => {
            // A library that starts in a fault still has the defaults.
            let quick =
                ranked(&library.baseline).unwrap_or_else(|| quick_reactions(&[], QUICK_REACTIONS));
            post(Posted::Quick(quick));
            Some(library)
        }
        Err(error) => {
            // Without its counts, the account's quick reactions are the
            // defaults (`quick_reactions` of no use).
            log::debug!("an account's reaction use is not watched: {error}");
            post(Posted::Quick(quick_reactions(&[], QUICK_REACTIONS)));
            None
        }
    };
    loop {
        tokio::select! {
            changed = outgoing.changed() => {
                if changed.is_err() {
                    post(Posted::Closed);
                    return;
                }
                post(Posted::Outgoing(Arc::clone(&outgoing.borrow_and_update())));
            }
            // The stream ends with the session, which the queue reports.
            Some(activity) = people.next() => post(Posted::People(Arc::new(activity))),
            update = next_library(library.as_mut()) => match update {
                ExpressionLibraryUpdate::Closed => library = None,
                update => {
                    if let Some(quick) = ranked(&update) {
                        post(Posted::Quick(quick));
                    }
                }
            },
        }
    }
}

/// The next library update; never, once the library is gone.
async fn next_library(
    library: Option<&mut papo_sdk_ui::ExpressionLibrarySubscription>,
) -> ExpressionLibraryUpdate {
    match library {
        Some(library) => library.next().await,
        None => std::future::pending().await,
    }
}

/// The quick reactions an update ranks; none for a fault, which keeps the
/// last ranking.
fn ranked(update: &ExpressionLibraryUpdate) -> Option<Vec<String>> {
    match update {
        ExpressionLibraryUpdate::Reset(view) => Some(ranking(view)),
        _ => None,
    }
}

fn ranking(view: &ExpressionLibraryView) -> Vec<String> {
    quick_reactions(&view.reaction_usage, QUICK_REACTIONS)
}
