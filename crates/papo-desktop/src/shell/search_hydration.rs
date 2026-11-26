//! Search results with their media and link cards (`docs/design.md` §10, Search; §15.1): a hit
//! carries only the message's text, so the results on screen are hydrated through the SDK
//! (`Session::hydrate_search_hit`, which walks real history pages), and a result drawn from its
//! whole message shows its attachments and cards as a pin does.
//!
//! Owns which results are asked for, how far each walks and what is kept: two at a time, each
//! for at most [`MAX_TURNS`] turns (the SDK's four pages a turn), at most [`MAX_KEPT`] kept
//! per search, the least recently asked let go first. A result that is not found in time
//! stays as the hit drew it. Drawing is `search_hits`'.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::Arc;

use papo_sdk::{AccountId, DetachedMessageUpdate, Message, Uuid};

use super::Shell;
use super::search::Found;

/// Hydrations running at once: the SDK admits two per account.
const MAX_IN_FLIGHT: usize = 2;

/// Turns one result may take, four history pages each, before it stays as its hit.
const MAX_TURNS: usize = 5;

/// Whole messages kept per search, as the SDK keeps 32 detached messages per account.
const MAX_KEPT: usize = 32;

/// Where one result's hydration stands.
#[derive(Clone)]
pub(crate) enum Hydration {
    /// Being looked for.
    Asked,
    /// Its whole message.
    Ready(Arc<Message>),
    /// Not found in time, or gone: the hit draws it.
    Gone,
}

/// One search's hydrated results.
#[derive(Default)]
pub(crate) struct Hydrated {
    entries: HashMap<Uuid, Hydration>,
    /// Asked order, oldest first, for letting go.
    order: VecDeque<Uuid>,
    /// The results on screen, as last reported.
    visible: Range<usize>,
}

impl Hydrated {
    /// Result `id`'s whole message, once it is known.
    pub(crate) fn message(&self, id: Uuid) -> Option<&Arc<Message>> {
        match self.entries.get(&id)? {
            Hydration::Ready(message) => Some(message),
            Hydration::Asked | Hydration::Gone => None,
        }
    }

    /// The whole messages of the results `ids` (those on screen), for the media they want.
    pub(crate) fn messages<'a>(
        &'a self,
        ids: impl IntoIterator<Item = Uuid> + 'a,
    ) -> impl Iterator<Item = &'a Arc<Message>> + 'a {
        ids.into_iter().filter_map(|id| self.message(id))
    }

    /// The results on screen, as last reported.
    pub(crate) fn visible(&self) -> Range<usize> {
        self.visible.clone()
    }

    fn in_flight(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry, Hydration::Asked))
            .count()
    }

    /// Marks `id` asked, letting the oldest settled entry go past [`MAX_KEPT`].
    fn ask(&mut self, id: Uuid) {
        self.entries.insert(id, Hydration::Asked);
        self.order.push_back(id);
        while self.order.len() > MAX_KEPT {
            let Some(oldest) = self
                .order
                .iter()
                .position(|id| !matches!(self.entries.get(id), Some(Hydration::Asked)))
            else {
                break;
            };
            if let Some(id) = self.order.remove(oldest) {
                self.entries.remove(&id);
            }
        }
    }
}

impl Shell {
    /// Results `range` are on screen: those not yet asked are hydrated, two at a time.
    pub(super) fn hydrate_visible_hits(&mut self, range: Range<usize>) {
        if let Some(search) = self.search.as_mut() {
            search.hydrated.visible = range;
        }
        self.hydrate_next_hits();
        // Results already whole ask for nothing else, but their emoji are on screen now.
        self.want_emoji();
    }

    /// Starts hydrating the next results on screen that were never asked, while slots are free.
    fn hydrate_next_hits(&mut self) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let Found::Ready(results) = &search.found else {
            return;
        };
        let range = search.hydrated.visible.clone();
        let wanted: Vec<_> = results
            .items()
            .iter()
            .skip(range.start)
            .take(range.len())
            .filter(|item| !search.hydrated.entries.contains_key(&item.id()))
            .map(|item| (item.id(), item.hit.clone()))
            .collect();
        let free = MAX_IN_FLIGHT.saturating_sub(search.hydrated.in_flight());
        let account = search.account;
        for (id, hit) in wanted.into_iter().take(free) {
            search.hydrated.ask(id);
            let updates = sdk.updates();
            sdk.run(move |client| async move {
                let found = match client.session(account) {
                    Ok(session) => hydrate(&session, &hit).await,
                    Err(_) => None,
                };
                updates.send(move |shell| shell.hit_hydrated(account, id, found));
            });
        }
    }

    /// Result `id` of `account`'s search was looked for: its whole message, or none. A whole
    /// message does not depend on the query, so it lands across a changed query; a search
    /// closed, or for another account, since takes nothing.
    fn hit_hydrated(&mut self, account: AccountId, id: Uuid, found: Option<Arc<Message>>) -> bool {
        let Some(search) = self
            .search
            .as_mut()
            .filter(|search| search.account == account)
        else {
            return false;
        };
        let Some(entry) = search.hydrated.entries.get_mut(&id) else {
            return false;
        };
        *entry = found.map_or(Hydration::Gone, Hydration::Ready);
        self.hydrate_next_hits();
        // Its media load as a pin's do.
        self.want_media();
        true
    }
}

/// `hit`'s whole message from history, within [`MAX_TURNS`] turns; none when it is not found
/// in time or the account or search went.
async fn hydrate(session: &papo_sdk::Session, hit: &papo_sdk::SearchHit) -> Option<Arc<Message>> {
    let request = session.hydrate_search_hit(hit).ok()?;
    for _ in 0..MAX_TURNS {
        match request.next_batch().await.ok()? {
            DetachedMessageUpdate::Ready(message) => return Some(message),
            DetachedMessageUpdate::Pending => {}
            DetachedMessageUpdate::Unavailable => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    //! What a search keeps.

    use super::{Hydrated, Hydration, MAX_KEPT};
    use papo_sdk::Uuid;

    #[test]
    fn a_search_keeps_its_latest_messages_and_never_drops_one_being_asked() {
        let mut hydrated = Hydrated::default();
        hydrated.ask(Uuid::from_u128(0));
        for n in 1..=MAX_KEPT as u128 + 5 {
            hydrated.ask(Uuid::from_u128(n));
            hydrated.entries.insert(Uuid::from_u128(n), Hydration::Gone);
        }
        assert!(hydrated.order.len() <= MAX_KEPT + 1);
        assert!(
            matches!(
                hydrated.entries.get(&Uuid::from_u128(0)),
                Some(Hydration::Asked)
            ),
            "the one still asked stays"
        );
        assert!(!hydrated.entries.contains_key(&Uuid::from_u128(1)));
    }
}
