//! Bounded startup cache hydration on the session owner, never on GPUI.
//!
//! Core chooses the channel and page size. Cache receipts have no network
//! Wake, so the actor waits explicitly for one page, with the same ten-second
//! bound as core's synchronous metadata loads. Results are checked against
//! account, selection and restore epoch; shutdown never publishes the page.
//! Paging and freshness policy remain in Store.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use papo_core::{cache::CachedMessagePage, runtime::ServerRuntime, state::Store};

/// Matches `papo-core/src/cache/mod.rs`, `LOAD_TIMEOUT`, in seconds.
const CACHE_LOAD_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Restores at most one selected-channel page before the actor publishes it.
/// A failed read clears the loading state and logs no external input.
pub(super) fn hydrate_selected(runtime: &mut ServerRuntime, stopped: &AtomicBool) {
    let Some(channel) = runtime.store.channel_needing_cache() else {
        return;
    };
    let request = CacheRequest {
        account: runtime.store.me.clone(),
        channel,
        epoch: runtime.store.cache_restore_epoch(),
    };
    runtime.store.mark_cache_loading(&request.channel);
    let receipt =
        runtime
            .cache
            .load_channel_page_async(&runtime.server_key, &request.channel, None);
    let page = receipt
        .and_then(|receipt| receipt.recv_timeout(CACHE_LOAD_TIMEOUT).ok())
        .and_then(Result::ok);
    if let Some(page) = page {
        request.apply(&mut runtime.store, page, stopped.load(Ordering::Acquire));
    } else {
        runtime.store.cached_page_failed(&request.channel, false);
        log::warn!("session: startup cache page could not be loaded");
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
        use std::sync::{Arc, atomic::AtomicBool};
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
        super::hydrate_selected(&mut runtime, &AtomicBool::new(false));
        assert!(runtime.store.channel_needing_cache().is_none());
        runtime.net.shutdown();
    }
}
