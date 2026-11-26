//! Listing the server's emoji and publishing the catalog: one watch per session, one listing at a
//! time, a listing per connection that comes online, and a throttled listing when an emoji's image
//! is missing. Durable reads and writes live in `persistence`.

use super::{CustomEmoji, EmojiCatalog, EmojiCatalogError, EmojiCatalogUpdate, MAX_CUSTOM_EMOJI};
use crate::{ClientError, ConnectionState, Generation, MediaSource, Session};
use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::{sync::watch, time::Instant};

/// A missing image lists the server's emoji at most this often: the list carries every image, so
/// a cleared media cache costs one listing, not one per emoji drawn.
const MISSING_IMAGE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Catalog revisions, process-wide: parsed content is shared across sessions by account
/// (`papo-sdk-ui`'s content cache), so a session signed in again never reuses a number.
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

/// The session's catalog state.
pub(crate) struct Shared {
    updates: watch::Sender<EmojiCatalogUpdate>,
    started: tokio::sync::OnceCell<()>,
    /// Listings finished, successful or not. Read before waiting for [`Shared::listing`], so a
    /// caller that waited behind a listing knows one ended while it waited.
    finished: AtomicU64,
    /// Held for a whole listing, so listings never overlap.
    listing: tokio::sync::Mutex<Listings>,
}

#[derive(Default)]
struct Listings {
    /// The connection generation of the last listing that succeeded.
    listed: Option<Generation>,
    /// Whether the most recent listing succeeded.
    last_succeeded: bool,
    /// When the last listing for a missing image started.
    last_for_missing: Option<Instant>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            // Revision 0, empty: until the kept catalog or a listing replaces it.
            updates: watch::channel(EmojiCatalogUpdate::Ready(Arc::default())).0,
            started: tokio::sync::OnceCell::new(),
            finished: AtomicU64::new(0),
            listing: tokio::sync::Mutex::new(Listings::default()),
        }
    }
}

impl Shared {
    pub(super) fn subscribe(&self) -> watch::Receiver<EmojiCatalogUpdate> {
        self.updates.subscribe()
    }

    /// Tells every subscriber the account is gone; nothing is published after it.
    pub(crate) fn close(&self) {
        self.updates.send_replace(EmojiCatalogUpdate::Closed);
    }

    /// Whether the catalog names emoji `id` at image revision `revision`.
    fn names(&self, id: crate::Uuid, revision: &str) -> bool {
        match &*self.updates.borrow() {
            EmojiCatalogUpdate::Ready(catalog) => catalog
                .get(id)
                .is_some_and(|emoji| emoji.created_at == revision),
            _ => false,
        }
    }
}

/// Reads the kept catalog and starts the connection watcher, once per session. Never waits for
/// a listing: the kept catalog is published only if no listing published first.
pub(super) async fn start(session: &Session) {
    session
        .inner
        .emoji_catalog
        .started
        .get_or_init(|| async {
            let loaded = super::persistence::load(session.clone()).await;
            publish_if(session, |current| {
                // Only over the initial empty catalog: a listing that finished first is newer.
                let initial = matches!(
                    current,
                    EmojiCatalogUpdate::Ready(catalog) if catalog.revision == 0
                );
                initial.then(|| match loaded {
                    Ok(emojis) => EmojiCatalogUpdate::Ready(Arc::new(EmojiCatalog {
                        revision: next_revision(),
                        emojis,
                    })),
                    Err(_) => EmojiCatalogUpdate::Fault,
                })
            });
            let token = session.inner.tasks.token();
            let watcher = session.clone();
            session.client.inner.runtime.spawn(async move {
                let _token = token;
                watch_connections(&watcher).await;
            });
        })
        .await;
}

/// Lists once for every connection generation that comes online.
async fn watch_connections(session: &Session) {
    let mut connection = session.subscribe_connection();
    loop {
        let online = {
            let status = connection.borrow_and_update();
            (status.state == ConnectionState::Online).then_some(status.generation)
        };
        if let Some(generation) = online {
            // A failure keeps the catalog it had; the next connection, a missing image or a
            // client's own refresh lists again.
            let _ = refresh_for(session, Some(generation)).await;
        }
        tokio::select! {
            biased;
            () = session.inner.root.cancelled() => break,
            changed = connection.changed() => if changed.is_err() { break; },
        }
    }
    session.inner.emoji_catalog.close();
}

/// Lists now, or takes the answer of a listing that succeeded while this waited.
pub(super) async fn refresh(session: &Session) -> Result<(), EmojiCatalogError> {
    refresh_for(session, None).await
}

/// Lists, unless a listing already succeeded: on connection generation `generation` or later,
/// or (with none) one that finished while this waited.
async fn refresh_for(
    session: &Session,
    generation: Option<Generation>,
) -> Result<(), EmojiCatalogError> {
    let shared = &session.inner.emoji_catalog;
    let seen = shared.finished.load(Ordering::Acquire);
    let mut listings = shared.listing.lock().await;
    let joined = shared.finished.load(Ordering::Acquire) != seen;
    let covered = match (generation, listings.listed) {
        (Some(wanted), Some(listed)) => listed >= wanted,
        (None, Some(_)) => joined && listings.last_succeeded,
        (_, None) => false,
    };
    if covered {
        return Ok(());
    }
    list_and_publish(session, &mut listings).await
}

/// Lists for emoji `source`'s image, which the media store lacks, when the catalog names it at
/// that revision, at most once per [`MISSING_IMAGE_INTERVAL`]. A listing that finished while
/// this waited counts too; the caller looks in the store once more either way.
pub(crate) async fn refresh_for_missing_image(session: &Session, source: &MediaSource) {
    let MediaSource::Emoji { id, revision } = source else {
        return;
    };
    let shared = &session.inner.emoji_catalog;
    // An emoji the catalog doesn't name (deleted, or a stale revision) has no image to list.
    if !shared.names(*id, revision) {
        return;
    }
    let seen = shared.finished.load(Ordering::Acquire);
    let mut listings = shared.listing.lock().await;
    if shared.finished.load(Ordering::Acquire) != seen {
        return;
    }
    let now = Instant::now();
    if listings
        .last_for_missing
        .is_some_and(|last| now.duration_since(last) < MISSING_IMAGE_INTERVAL)
    {
        return;
    }
    listings.last_for_missing = Some(now);
    let _ = list_and_publish(session, &mut listings).await;
}

async fn list_and_publish(
    session: &Session,
    listings: &mut Listings,
) -> Result<(), EmojiCatalogError> {
    let result = list_and_keep(session).await;
    listings.last_succeeded = result.is_ok();
    if let Ok(generation) = result {
        listings.listed = Some(generation);
    }
    session
        .inner
        .emoji_catalog
        .finished
        .fetch_add(1, Ordering::AcqRel);
    result.map(|_| ())
}

/// Lists, keeps the result and publishes it when it changed (or the kept one was unreadable).
/// Returns the connection generation it listed on.
async fn list_and_keep(session: &Session) -> Result<Generation, EmojiCatalogError> {
    let (generation, emojis) = list(session).await?;
    let changed = super::persistence::replace(session.clone(), emojis.clone()).await?;
    publish_if(session, |current| {
        let unreadable = matches!(current, EmojiCatalogUpdate::Fault);
        (changed || unreadable).then(|| {
            EmojiCatalogUpdate::Ready(Arc::new(EmojiCatalog {
                revision: next_revision(),
                emojis,
            }))
        })
    });
    Ok(generation)
}

/// Every page of the server's list, all from one connection generation, each emoji once.
async fn list(session: &Session) -> Result<(Generation, Vec<CustomEmoji>), EmojiCatalogError> {
    let media = session.media();
    let mut emojis = Vec::new();
    let mut ids = HashSet::new();
    let mut cursor = None;
    let mut generation = None;
    loop {
        let page = media.emojis(cursor).await?;
        let listed_on = *generation.get_or_insert(page.generation);
        if listed_on != page.generation {
            return Err(ClientError::Superseded.into());
        }
        // A server that repeats an emoji lists it once.
        emojis.extend(
            page.value
                .emojis
                .into_iter()
                .filter(|emoji| ids.insert(emoji.id))
                .map(|emoji| CustomEmoji {
                    id: emoji.id,
                    name: emoji.name,
                    format: emoji.format,
                    created_at: emoji.created_at,
                }),
        );
        if emojis.len() > MAX_CUSTOM_EMOJI {
            return Err(EmojiCatalogError::TooMany);
        }
        match page.value.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Ok((listed_on, emojis)),
        }
    }
}

fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// Publishes what `next` makes of the current update, if anything; never after `Closed` or on a
/// closed session.
fn publish_if(
    session: &Session,
    next: impl FnOnce(&EmojiCatalogUpdate) -> Option<EmojiCatalogUpdate>,
) {
    if session.is_closed() {
        return;
    }
    session
        .inner
        .emoji_catalog
        .updates
        .send_if_modified(|current| {
            if matches!(current, EmojiCatalogUpdate::Closed) {
                return false;
            }
            match next(current) {
                Some(update) => {
                    *current = update;
                    true
                }
                None => false,
            }
        });
}
