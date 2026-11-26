//! Watches each signed-in account for what the shell draws live beyond a
//! timeline: its outgoing queue (a chip waits while its change is in
//! flight; a refused reaction is said and dropped), its count of emoji used
//! (which the SDK ranks into the quick reactions and Frequently used), its
//! people's activity (presence marks, status phrases, who types where), its
//! custom emoji catalog, and its connection with the heartbeat's round trip
//! (Settings › Diagnostics).
//!
//! Owns only the watch loops and what they post. The queue, the usage
//! counts, the ranking and the activity stay in the SDK; what the shell
//! draws and says from them is the shell's.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::sync::Arc;

use futures_util::StreamExt;
use papo_sdk::{
    AccountId, ClientHandle, EmojiCatalog, EmojiCatalogUpdate, OutgoingSnapshot, Session,
};
use papo_sdk_ui::{
    AccountActivity, ActivityService, ExpressionLibraryService, ExpressionLibraryUpdate,
};
use rune::Tx;
use tokio::task::{AbortHandle, JoinSet};

use crate::diagnostics::Link;
use crate::sdk_bridge::Updates;

/// What the shell hears about one account.
pub(crate) enum Posted {
    /// Its outgoing queue now.
    Outgoing(Arc<OutgoingSnapshot>),
    /// Its emoji count now, most used first.
    Usage(Vec<(String, u64)>),
    /// Its favorite GIFs now, by GIPHY's ID: their stars are filled.
    FavoriteGifs(HashSet<String>),
    /// Its people's activity now.
    People(Arc<AccountActivity>),
    /// Its custom emoji catalog now.
    Emojis(Arc<EmojiCatalog>),
    /// Its connection now.
    Link(Link),
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

/// One account's queue, library, activity and connection, until its
/// session closes.
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
    let mut connection = session.subscribe_connection();
    let mut round_trip = session.subscribe_round_trip();
    let link = |connection: &mut tokio::sync::watch::Receiver<_>,
                round_trip: &mut tokio::sync::watch::Receiver<_>| {
        Link::of(
            *connection.borrow_and_update(),
            *round_trip.borrow_and_update(),
        )
    };
    post(Posted::Link(link(&mut connection, &mut round_trip)));
    // A watch that closes stops being read; the queue says the session
    // closed.
    let mut watching_link = true;
    let people = ActivityService::new(session.clone()).subscribe();
    post(Posted::People(Arc::new(people.snapshot)));
    let mut people = people.stream;
    let library = ExpressionLibraryService::new(session.clone())
        .subscribe()
        .await;
    let mut library = match library {
        Ok(library) => {
            // A library that starts in a fault ranks no use: the defaults.
            post(Posted::Usage(usage(&library.baseline).unwrap_or_default()));
            post(Posted::FavoriteGifs(
                favorite_gifs(&library.baseline).unwrap_or_default(),
            ));
            Some(library)
        }
        Err(error) => {
            // Without its counts, the account's quick reactions are the
            // defaults (`quick_reactions` of no use).
            log::debug!("an account's reaction use is not watched: {error}");
            post(Posted::Usage(Vec::new()));
            None
        }
    };
    // Without a catalog, `:name:` stays text.
    let mut emojis = session.emoji_catalog().subscribe().await.ok();
    if let Some(catalog) = emojis.as_mut().and_then(ready) {
        post(Posted::Emojis(catalog));
    }
    loop {
        tokio::select! {
            changed = next_change(emojis.as_mut()) => {
                if changed.is_err() {
                    emojis = None;
                } else if let Some(catalog) = emojis.as_mut().and_then(ready) {
                    post(Posted::Emojis(catalog));
                }
            }
            changed = outgoing.changed() => {
                if changed.is_err() {
                    post(Posted::Closed);
                    return;
                }
                post(Posted::Outgoing(Arc::clone(&outgoing.borrow_and_update())));
            }
            // The stream ends with the session, which the queue reports.
            Some(activity) = people.next() => post(Posted::People(Arc::new(activity))),
            changed = connection.changed(), if watching_link => {
                watching_link = changed.is_ok();
                post(Posted::Link(link(&mut connection, &mut round_trip)));
            }
            changed = round_trip.changed(), if watching_link => {
                watching_link = changed.is_ok();
                post(Posted::Link(link(&mut connection, &mut round_trip)));
            }
            update = next_library(library.as_mut()) => match update {
                ExpressionLibraryUpdate::Closed => library = None,
                update => {
                    if let Some(count) = usage(&update) {
                        post(Posted::Usage(count));
                    }
                    if let Some(favorites) = favorite_gifs(&update) {
                        post(Posted::FavoriteGifs(favorites));
                    }
                }
            },
        }
    }
}

/// The catalog a watch holds now; none while it can't be read (the last one stays drawn).
fn ready(
    updates: &mut tokio::sync::watch::Receiver<EmojiCatalogUpdate>,
) -> Option<Arc<EmojiCatalog>> {
    match &*updates.borrow_and_update() {
        EmojiCatalogUpdate::Ready(catalog) => Some(Arc::clone(catalog)),
        _ => None,
    }
}

/// The next change of an optional watch; never, without one.
async fn next_change<T>(
    updates: Option<&mut tokio::sync::watch::Receiver<T>>,
) -> Result<(), tokio::sync::watch::error::RecvError> {
    match updates {
        Some(updates) => updates.changed().await,
        None => std::future::pending().await,
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

/// The favorite GIFs an update holds, by GIPHY's ID; none for a fault, which keeps the last.
fn favorite_gifs(update: &ExpressionLibraryUpdate) -> Option<HashSet<String>> {
    match update {
        ExpressionLibraryUpdate::Reset(view) => Some(
            view.favorites
                .iter()
                .filter_map(|item| match &item.id {
                    papo_sdk_ui::ExpressionId::Gif(id) => Some(id.clone()),
                    papo_sdk_ui::ExpressionId::Sticker(_) => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

/// The emoji count an update holds; none for a fault, which keeps the last one.
fn usage(update: &ExpressionLibraryUpdate) -> Option<Vec<(String, u64)>> {
    match update {
        ExpressionLibraryUpdate::Reset(view) => Some(view.emoji_usage.clone()),
        _ => None,
    }
}
