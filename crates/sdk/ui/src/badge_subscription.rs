//! Pull-driven badge watch fan-in; SDK owns session lifecycle and all count computation.

#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, pin::Pin, sync::Arc};

use futures_util::{
    Stream, StreamExt,
    stream::{self, SelectAll},
};
use papo_sdk::{AccountId, ClientHandle, ReadingSnapshot, RegistrySnapshot, ResourceSnapshot};
use tokio::sync::watch;

use crate::{
    badge_projection::{self, AccountImage},
    badges::{BadgeSubscription, BadgeUpdate},
};

enum Event {
    Closed,
    Registry(Arc<RegistrySnapshot>),
    Sessions,
    Resources(AccountId, Option<Arc<ResourceSnapshot>>),
    Reading(AccountId, Option<Arc<ReadingSnapshot>>),
}

type Source = Pin<Box<dyn Stream<Item = Event> + Send>>;

struct Sources {
    handle: ClientHandle,
    registry: Arc<RegistrySnapshot>,
    images: BTreeMap<AccountId, AccountImage>,
    controls: SelectAll<Source>,
    accounts: SelectAll<Source>,
}

pub(crate) fn subscribe(handle: ClientHandle) -> BadgeSubscription {
    let mut registry_rx = handle.subscribe_registry();
    let mut sessions = handle.subscribe_sessions();
    // Mark control baselines seen before querying capabilities. Subsequent lifecycle changes stay pending.
    let registry = Arc::clone(&registry_rx.borrow_and_update());
    sessions.borrow_and_update();
    let mut controls = SelectAll::<Source>::new();
    controls.push(watch_source(registry_rx, Event::Registry));
    controls.push(watch_source(sessions, |_| Event::Sessions));
    let mut state = Sources {
        handle,
        registry,
        images: BTreeMap::new(),
        controls,
        accounts: SelectAll::new(),
    };
    state.rebind();
    from_sources(state)
}

fn from_sources(state: Sources) -> BadgeSubscription {
    let snapshot = badge_projection::project(&state.registry, &state.images);
    let previous = snapshot.clone();
    let changes = stream::unfold(
        (state, previous, 0_u64),
        |(mut state, mut previous, mut version)| async move {
            loop {
                let event = tokio::select! {
                    biased;
                    event = state.controls.next() => event?,
                    event = state.accounts.next(), if !state.accounts.is_empty() => event?,
                };
                match event {
                    Event::Closed => return None,
                    Event::Registry(registry) => {
                        state.registry = registry;
                        state.rebind();
                    }
                    Event::Sessions => state.rebind(),
                    Event::Resources(account, Some(resources)) => {
                        if let Some(image) = state.images.get_mut(&account) {
                            image.resources = resources;
                        }
                    }
                    Event::Reading(account, Some(reading)) => {
                        if let Some(image) = state.images.get_mut(&account) {
                            image.reading = reading;
                        }
                    }
                    Event::Resources(account, None) | Event::Reading(account, None) => {
                        state.images.remove(&account);
                    }
                }
                let snapshot = badge_projection::project(&state.registry, &state.images);
                if snapshot == previous {
                    continue;
                }
                version = version.checked_add(1)?;
                previous = snapshot.clone();
                return Some((
                    BadgeUpdate { version, snapshot },
                    (state, previous, version),
                ));
            }
        },
    );
    BadgeSubscription {
        version: 0,
        snapshot,
        stream: Box::pin(changes.fuse()),
    }
}

impl Sources {
    fn rebind(&mut self) {
        self.accounts = SelectAll::new();
        self.images.clear();
        for account in &self.registry.accounts {
            let Ok(session) = self.handle.session(account.id) else {
                continue;
            };
            let id = account.id;
            let mut resources = session.subscribe_resources();
            let mut reading = session.subscribe_reading();
            self.images.insert(
                id,
                AccountImage {
                    resources: Arc::clone(&resources.borrow_and_update()),
                    reading: Arc::clone(&reading.borrow_and_update()),
                },
            );
            self.accounts.push(account_source(resources, move |image| {
                Event::Resources(id, image)
            }));
            self.accounts.push(account_source(reading, move |image| {
                Event::Reading(id, image)
            }));
        }
    }
}

fn watch_source<T: Clone + Send + Sync + 'static>(
    receiver: watch::Receiver<T>,
    event: impl Fn(T) -> Event + Send + 'static,
) -> Source {
    Box::pin(stream::unfold(
        Some((receiver, event)),
        |state| async move {
            let (mut receiver, event) = state?;
            if receiver.changed().await.is_err() {
                return Some((Event::Closed, None));
            }
            let value = receiver.borrow_and_update().clone();
            Some((event(value), Some((receiver, event))))
        },
    ))
}

fn account_source<T: Send + Sync + 'static>(
    receiver: watch::Receiver<Arc<T>>,
    event: impl Fn(Option<Arc<T>>) -> Event + Send + 'static,
) -> Source {
    Box::pin(stream::unfold(
        Some((receiver, event)),
        |state| async move {
            let (mut receiver, event) = state?;
            if receiver.changed().await.is_err() {
                return Some((event(None), None));
            }
            let value = Arc::clone(&receiver.borrow_and_update());
            Some((event(Some(value)), Some((receiver, event))))
        },
    ))
}
