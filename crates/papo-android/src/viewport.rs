//! Viewport observations reduce read state without projecting the message/resource graph.
use crate::bindings::{Backend, CoreError, NativeClient, ReadingView};
use crate::{Action, projection};

#[derive(Clone, Debug, uniffi::Record)]
pub struct ViewportSnapshot {
    pub owner_user_id: String,
    pub channel_id: String,
    pub channel_unread: bool,
    pub channel_mentions: i32,
    pub notification_count: u32,
    pub reading: ReadingView,
}

#[uniffi::export]
impl NativeClient {
    /// Every observation uses the normal reducer, persistence and receipt delivery.
    /// Transport wakes still drain updates through poll; this never consumes them.
    pub fn observe_viewport(
        &self,
        channel_id: String,
        ids: Vec<String>,
        at_latest: bool,
        owner_user_id: String,
    ) -> Result<Option<ViewportSnapshot>, CoreError> {
        let mut guard = self
            .backend
            .lock()
            .map_err(|_| "Native client lock failed".to_owned())?;
        let backend = guard
            .as_mut()
            .ok_or_else(|| "Client is closed".to_owned())?;
        let store = match backend {
            Backend::Live(client) => &client.runtime.store,
            Backend::Demo(client) => &client.store,
        };
        if store.me != owner_user_id || store.selected_channel != channel_id {
            return Ok(None);
        }
        backend.dispatch(Action::Visible {
            channel_id: channel_id.clone(),
            ids,
            at_latest,
            owner_user_id: owner_user_id.clone(),
        })?;
        let (store, url, seeking) = match backend {
            Backend::Live(client) => {
                client.sync_read_receipts();
                (
                    &client.runtime.store,
                    client.runtime.url.as_str(),
                    client.catching_up.is_some(),
                )
            }
            Backend::Demo(client) => (&client.store, "demo://papo", false),
        };
        let mut reading = projection::reading_view(store, url);
        reading.seeking_catch_up = seeking;
        reading.sync_pending |= seeking;
        let channel = store.channel(&channel_id);
        Ok(Some(ViewportSnapshot {
            owner_user_id,
            channel_id,
            channel_unread: channel.is_some_and(|channel| channel.unread),
            channel_mentions: channel.map_or(0, |channel| channel.mentions as i32),
            notification_count: store.pending_notification_count(),
            reading,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_viewport_matches_full_reduction_including_upward_catch_up() {
        let narrow = NativeClient::demo();
        let full = NativeClient::demo();
        // Reach the head across a gap, then read the gap upward and downward.
        for (ids, latest, break_span) in [
            (vec!["general-0"], false, false),
            (vec!["general-9"], true, true),
            (vec!["general-7", "general-8"], false, false),
            (vec!["general-5", "general-6"], false, false),
            (vec!["general-1", "general-2"], false, true),
            (vec!["general-3", "general-4"], false, false),
        ] {
            if break_span {
                for client in [&narrow, &full] {
                    client
                        .dispatch(Action::BreakReadSpan {
                            channel_id: "general".into(),
                        })
                        .unwrap();
                }
            }
            let ids: Vec<String> = ids.into_iter().map(str::to_owned).collect();
            let update = narrow
                .observe_viewport("general".into(), ids.clone(), latest, "me".into())
                .unwrap()
                .unwrap();
            full.dispatch(Action::Visible {
                channel_id: "general".into(),
                ids,
                at_latest: latest,
                owner_user_id: "me".into(),
            })
            .unwrap();
            let snapshot = full.poll().unwrap();
            assert_eq!(update.reading, snapshot.reading);
            let channel = snapshot
                .channels
                .iter()
                .find(|c| c.id == "general")
                .unwrap();
            assert_eq!(update.channel_unread, channel.unread);
            assert_eq!(update.channel_mentions, channel.mentions);
            assert_eq!(update.notification_count, snapshot.notification_count);
        }
        assert!(
            narrow
                .observe_viewport("general".into(), vec![], true, "stale-account".into())
                .unwrap()
                .is_none()
        );
        assert!(
            narrow
                .observe_viewport("media".into(), vec![], true, "me".into())
                .unwrap()
                .is_none()
        );
        assert_eq!(narrow.poll().unwrap().reading, full.poll().unwrap().reading);
    }
}
