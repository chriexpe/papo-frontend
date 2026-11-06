//! Event-driven startup cache hydration on the session owner, never on GPUI.
//!
//! Core chooses the page size and completes reads on its existing cache worker.
//! One in-flight read and one completion slot bound each server's work. A slow
//! read never parks the actor; replacement and shutdown need not await it.
//! Core owns paging/freshness. Session/account/channel/epoch reject stale pages.

use super::SessionIdentity;
use async_channel::{Receiver, Sender};
use papo_core::{cache::CachedMessagePage, runtime::ServerRuntime, state::Store};

/// One cache read may be outstanding per server, including across replacement.
const COMPLETION_CAPACITY: usize = 1;
type PageResult = Result<CachedMessagePage, String>;
type CompletePage = Box<dyn FnOnce(PageResult) + Send>;

/// Inputs captured before a cache read; core invalidates the epoch on reset.
struct CacheRequest {
    account: String,
    channel: String,
    epoch: u64,
}

impl CacheRequest {
    fn matches(&self, store: &Store) -> bool {
        self.account == store.me
            && self.channel == store.selected_channel
            && self.epoch == store.cache_restore_epoch()
    }

    fn apply(self, store: &mut Store, page: CachedMessagePage, stopped: bool) {
        if !stopped && self.matches(store) && page.channel_id == self.channel {
            store.restore_cached_page(page, false);
        }
    }
}

struct Completion {
    identity: SessionIdentity,
    request: CacheRequest,
    result: PageResult,
}

/// One asynchronous read and its bounded mailbox; no waiter thread or timer.
pub(super) struct Hydration {
    pending: bool,
    completed: Receiver<Completion>,
    reply: Sender<Completion>,
}

impl Default for Hydration {
    fn default() -> Self {
        let (reply, completed) = async_channel::bounded(COMPLETION_CAPACITY);
        Self {
            pending: false,
            completed,
            reply,
        }
    }
}

impl Hydration {
    /// Applies any completed page, then requests at most one eligible cache page.
    /// Neither operation waits for disk, a receipt, or the foreground.
    pub(super) fn step(
        &mut self,
        runtime: &mut ServerRuntime,
        identity: &SessionIdentity,
        wake: &Sender<()>,
    ) {
        self.receive(&mut runtime.store, identity);
        self.request(&mut runtime.store, identity, wake, |channel, completed| {
            runtime.cache.load_channel_page_with_callback(
                &runtime.server_key,
                channel,
                None,
                completed,
            )
        });
    }

    fn receive(&mut self, store: &mut Store, identity: &SessionIdentity) {
        let Ok(completion) = self.completed.try_recv() else {
            return;
        };
        self.pending = false;
        if !completion.identity.matches(identity) {
            return;
        }
        if !completion.request.matches(store) {
            if completion.request.account == store.me
                && completion.request.epoch == store.cache_restore_epoch()
            {
                store.cancel_cached_page(&completion.request.channel);
            }
            return;
        }
        match completion.result {
            Ok(page) if page.channel_id == completion.request.channel => {
                completion.request.apply(store, page, false);
            }
            _ => {
                store.cached_page_failed(&completion.request.channel, false);
                log::warn!("session: startup cache page could not be loaded");
            }
        }
    }

    /// Captures a ticket and submits I/O. Injection keeps the delayed-read test clock-free.
    fn request(
        &mut self,
        store: &mut Store,
        identity: &SessionIdentity,
        wake: &Sender<()>,
        enqueue: impl FnOnce(&str, CompletePage) -> bool,
    ) {
        if self.pending {
            return;
        }
        let Some(channel) = store.channel_needing_cache() else {
            return;
        };
        let request = CacheRequest {
            account: store.me.clone(),
            channel: channel.clone(),
            epoch: store.cache_restore_epoch(),
        };
        store.mark_cache_loading(&channel);
        self.pending = true;
        let identity = identity.clone();
        let reply = self.reply.clone();
        let wake = wake.clone();
        if !enqueue(
            &channel,
            Box::new(move |result| {
                // Only one read is outstanding, so this mailbox cannot overflow.
                // A closed receiver means the actor stopped; discard without waiting.
                let _ = reply.try_send(Completion {
                    identity,
                    request,
                    result,
                });
                let _ = wake.try_send(());
            }),
        ) {
            self.pending = false;
            store.cached_page_failed(&channel, false);
            log::warn!("session: startup cache page could not be queued");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CacheRequest;
    use papo_core::{
        cache::{CachedMessagePage, CachedServerMetadata},
        state::Store,
    };

    #[test]
    fn cache_receipt_for_a_replaced_account_or_selection_is_dropped() {
        let mut store = Store::default();
        store.me = "account-1".into();
        store.selected_channel = "channel-1".into();
        let request = CacheRequest {
            account: store.me.clone(),
            channel: store.selected_channel.clone(),
            epoch: store.cache_restore_epoch(),
        };
        store.me = "account-2".into();
        assert!(!request.matches(&store));
        store.me = "account-1".into();
        store.selected_channel = "channel-2".into();
        assert!(!request.matches(&store));
    }

    #[test]
    fn cache_receipt_cannot_mark_a_stopped_or_reset_session_hydrated() {
        let mut store = Store::default();
        store.restore_cached_metadata(CachedServerMetadata {
            cached_channels: ["channel-1".into()].into(),
            ..Default::default()
        });
        store.selected_channel = "channel-1".into();
        let request = CacheRequest {
            account: store.me.clone(),
            channel: store.selected_channel.clone(),
            epoch: store.cache_restore_epoch(),
        };
        let page = CachedMessagePage {
            channel_id: "channel-1".into(),
            ..Default::default()
        };
        request.apply(&mut store, page, true);
        assert_eq!(store.channel_needing_cache().as_deref(), Some("channel-1"));
        let request = CacheRequest {
            account: store.me.clone(),
            channel: store.selected_channel.clone(),
            epoch: store.cache_restore_epoch(),
        };
        store.apply(papo_core::api::net::Update::Session(None));
        assert!(!request.matches(&store));
    }
    #[test]
    fn hydration_uses_core_restore_without_claiming_server_freshness() {
        let mut store = Store::default();
        store.restore_cached_metadata(CachedServerMetadata {
            cached_channels: ["channel-1".into()].into(),
            ..Default::default()
        });
        store.selected_channel = "channel-1".into();
        let request = CacheRequest {
            account: store.me.clone(),
            channel: store.selected_channel.clone(),
            epoch: store.cache_restore_epoch(),
        };
        let message = serde_json::from_value(serde_json::json!({"id":"message-1","channel_id":"channel-1","author_id":"account-1","content":"","created_at":"2000-01-01T00:00:00Z"})).unwrap();
        let page = CachedMessagePage {
            channel_id: "channel-1".into(),
            messages: vec![papo_core::cache::CachedMessage::from_api(&message)],
            ..Default::default()
        };
        request.apply(&mut store, page, false);
        assert_eq!(store.messages_in("channel-1").count(), 1);
        assert!(store.channel_needing_cache().is_none());
        assert_ne!(
            store.timeline_status("channel-1"),
            papo_core::state::TimelineStatus::Fresh
        );
    }

    #[test]
    fn unavailable_cache_does_not_retry_the_first_page_on_every_wake() {
        use papo_core::{
            api::net::Wake, cache::ClientDb, notification::NotificationCoordinator,
            runtime::ServerRuntime, storage::MemorySecretStore,
        };
        use std::sync::Arc;
        let cache = Arc::new(ClientDb::open(None));
        let notification = Arc::new(NotificationCoordinator::new(Arc::clone(&cache), None));
        let mut store = Store::default();
        store.restore_cached_metadata(CachedServerMetadata {
            cached_channels: ["channel-1".into()].into(),
            ..Default::default()
        });
        store.selected_channel = "channel-1".into();
        let mut runtime = ServerRuntime::open_with_store(
            "http://127.0.0.1:9".into(),
            store,
            Wake::noop(),
            Arc::new(MemorySecretStore::default()),
            cache,
            notification,
        );
        let (wake, _) = async_channel::bounded(1);
        let identity = super::SessionIdentity::new("server-1".into(), None);
        super::Hydration::default().step(&mut runtime, &identity, &wake);
        assert!(runtime.store.channel_needing_cache().is_none());
        runtime.net.shutdown();
    }
    #[test]
    fn stalled_page_returns_to_the_actor_and_keeps_only_one_read_in_flight() {
        let mut store = Store::default();
        store.restore_cached_metadata(CachedServerMetadata {
            cached_channels: ["channel-1".into()].into(),
            ..Default::default()
        });
        store.selected_channel = "channel-1".into();
        let identity = super::SessionIdentity::new("server-1".into(), None);
        let (wake, _) = async_channel::bounded(1);
        let mut hydration = super::Hydration::default();
        let mut held = None;
        hydration.request(&mut store, &identity, &wake, |_, completed| {
            held = Some(completed);
            true
        });
        assert!(hydration.pending);
        assert_eq!(hydration.completed.capacity(), Some(1));
        // This is the actor's submission path: it returns while the disk read
        // is deliberately unfinished, so normal core transitions can continue.
        store.apply(papo_core::api::net::Update::Connection(
            papo_core::api::ws::Connection::Online,
        ));
        assert_eq!(store.connection, papo_core::api::ws::Connection::Online);
        hydration.request(&mut store, &identity, &wake, |_, _| {
            panic!("second read while the first is pending")
        });
        assert!(hydration.completed.is_empty());
        // Teardown has no receipt deadline and no waiter to join. A later
        // completion observes the closed mailbox and drops its page.
        drop(hydration);
        held.unwrap()(Ok(CachedMessagePage {
            channel_id: "channel-1".into(),
            ..Default::default()
        }));
    }

    #[test]
    fn completed_page_from_a_replaced_same_account_incarnation_is_dropped() {
        let mut store = Store::default();
        store.restore_cached_metadata(CachedServerMetadata {
            cached_channels: ["channel-1".into()].into(),
            ..Default::default()
        });
        store.selected_channel = "channel-1".into();
        let old = super::SessionIdentity::new("server-1".into(), None);
        let new = super::SessionIdentity::new("server-1".into(), None);
        let (wake, _) = async_channel::bounded(1);
        let mut hydration = super::Hydration::default();
        let mut held = None;
        hydration.request(&mut store, &old, &wake, |_, completed| {
            held = Some(completed);
            true
        });
        held.unwrap()(Ok(CachedMessagePage {
            channel_id: "channel-1".into(),
            ..Default::default()
        }));
        let mut replacement = Store::default();
        replacement.restore_cached_metadata(CachedServerMetadata {
            cached_channels: ["channel-1".into()].into(),
            ..Default::default()
        });
        replacement.selected_channel = "channel-1".into();
        assert_eq!(
            store.cache_restore_epoch(),
            replacement.cache_restore_epoch()
        );
        hydration.receive(&mut replacement, &new);
        assert!(!hydration.pending);
        assert_eq!(
            replacement.channel_needing_cache().as_deref(),
            Some("channel-1")
        );
    }
    #[test]
    fn switching_channels_discards_the_page_but_releases_its_loading_mark() {
        let mut store = Store::default();
        store.restore_cached_metadata(CachedServerMetadata {
            cached_channels: ["channel-1".into()].into(),
            ..Default::default()
        });
        store.selected_channel = "channel-1".into();
        let identity = super::SessionIdentity::new("server-1".into(), None);
        let (wake, _) = async_channel::bounded(1);
        let mut hydration = super::Hydration::default();
        let mut held = None;
        hydration.request(&mut store, &identity, &wake, |_, completed| {
            held = Some(completed);
            true
        });
        store.selected_channel = "channel-2".into();
        held.unwrap()(Ok(CachedMessagePage {
            channel_id: "channel-1".into(),
            ..Default::default()
        }));
        hydration.receive(&mut store, &identity);
        store.selected_channel = "channel-1".into();
        assert_eq!(store.channel_needing_cache().as_deref(), Some("channel-1"));
    }
}
