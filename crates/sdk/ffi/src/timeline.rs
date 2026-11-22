//! Observable channel timeline windows (`live`, `live_at`, `focused_at`, `focused`),
//! pagination, gap filling, visible-range reporting, and reading overlays across FFI.

use std::sync::{Arc, Mutex};

use papo_sdk::Generation;
use papo_sdk_ui::{Timeline, TimelineConfig, TimelineEvent, TimelineReadingEvent};

use crate::{
    client::FfiClient,
    diffs::translate_diffs,
    ids::{FfiAccountId, FfiHistoryCursor, parse_uuid, parse_uuid_list},
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription},
    timeline_errors::{FfiHistoryClosedReason, FfiTimelineError},
    timeline_items::{
        FfiHistoryGap, FfiTimelineConfig, FfiTimelineDiff, FfiTimelineDirection,
        FfiTimelineListener, FfiTimelinePage, FfiTimelineReadingListener, FfiTimelineReadingState,
        FfiTimelineSnapshot, FfiTimelineUpdate, FfiUtcOffsetProvider, FfiVisibilityContinuity,
    },
    timeline_seek::{
        FfiCatchUpSeek, timeline_closed_err, timeline_session_err, to_timeline_config,
    },
};

/// Timeline window handle supporting subscriptions, pagination, visibility reporting, and catch-up.
#[derive(uniffi::Object)]
pub struct FfiTimeline {
    /// Parent client runtime and shutdown tracker.
    pub(crate) client: Arc<FfiClient>,
    /// Authoritative UI timeline window handle.
    pub(crate) inner: Timeline,
}

#[uniffi::export]
impl FfiTimeline {
    /// Captures baseline and subscribes to ordered window item diffs on a serial callback thread.
    ///
    /// # Errors
    /// Returns terminal closure, invalid cursor/config, or worker errors.
    pub fn subscribe(
        &self,
        listener: Arc<dyn FfiTimelineListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiTimelineError> {
        let sub = self.inner.subscribe()?;
        let baseline = FfiTimelineSnapshot {
            version: sub.version,
            generation: sub.generation.get(),
            items: sub.items.iter().map(Into::into).collect(),
        };
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.client.runtime,
            &self.client.tasks,
            &self.client.shutdown,
            self.client.callbacks.clone(),
            baseline,
            sub.stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |event| match event {
                TimelineEvent::Update(upd) => {
                    let Ok(diffs) = translate_diffs(upd.diffs, |d| {
                        FfiTimelineDiff::from_sdk(d, |i| (&i).into())
                    }) else {
                        return false;
                    };
                    l_upd.on_update(FfiTimelineUpdate {
                        version: upd.version,
                        generation: upd.generation.get(),
                        diffs,
                    });
                    true
                }
                TimelineEvent::Closed(reason) => {
                    l_upd.on_closed(reason.into());
                    false
                }
                _ => false,
            },
            move || l_end.on_closed(FfiHistoryClosedReason::AccountClosed),
        )
        .map_err(|_| FfiTimelineError::Closed {
            reason: FfiHistoryClosedReason::AccountClosed,
        })
    }

    /// Paginates older or newer history within this window without acknowledging read state.
    ///
    /// # Errors
    /// Returns busy, closure, generation, transport, or storage failures.
    pub async fn paginate(
        &self,
        direction: FfiTimelineDirection,
    ) -> Result<FfiTimelinePage, FfiTimelineError> {
        let timeline = self.inner.clone();
        self.client
            .run_scoped(
                || FfiTimelineError::Closed {
                    reason: FfiHistoryClosedReason::AccountClosed,
                },
                async move {
                    timeline
                        .paginate(direction.into())
                        .await
                        .map(Into::into)
                        .map_err(Into::into)
                },
            )
            .await
    }

    /// Fills an explicit displayed coverage gap by exact cursor.
    ///
    /// # Errors
    /// Returns stale snapshot if gap disappeared, busy, closure, or transport failures.
    pub async fn fill_gap(&self, gap: FfiHistoryGap) -> Result<FfiTimelinePage, FfiTimelineError> {
        let timeline = self.inner.clone();
        self.client
            .run_scoped(
                || FfiTimelineError::Closed {
                    reason: FfiHistoryClosedReason::AccountClosed,
                },
                async move {
                    timeline
                        .fill_gap(&gap.into())
                        .await
                        .map(Into::into)
                        .map_err(Into::into)
                },
            )
            .await
    }

    /// Reports strictly visible message IDs validated against displayed `version` and `generation`.
    ///
    /// # Errors
    /// Returns `StaleSnapshot`, invalid IDs, lifecycle, or durable commit errors.
    pub async fn report_visible(
        &self,
        version: u64,
        generation: u64,
        visible_message_ids: Vec<String>,
        continuity: FfiVisibilityContinuity,
    ) -> Result<(), FfiTimelineError> {
        let ids =
            parse_uuid_list(visible_message_ids).map_err(|_| FfiTimelineError::InvalidArgument)?;
        let timeline = self.inner.clone();
        self.client
            .run_scoped(
                || FfiTimelineError::Closed {
                    reason: FfiHistoryClosedReason::AccountClosed,
                },
                async move {
                    timeline
                        .report_visible(
                            version,
                            Generation::from_raw(generation),
                            &ids,
                            continuity.into(),
                        )
                        .await
                        .map_err(Into::into)
                },
            )
            .await
    }

    /// Subscribes to independent catch-up boundaries and live behind-head arrivals.
    ///
    /// # Errors
    /// Returns closure or worker start errors.
    pub fn subscribe_reading(
        &self,
        listener: Arc<dyn FfiTimelineReadingListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiTimelineError> {
        let sub = self.inner.subscribe_reading()?;
        let baseline = FfiTimelineReadingState::from(&sub.state);
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.client.runtime,
            &self.client.tasks,
            &self.client.shutdown,
            self.client.callbacks.clone(),
            baseline,
            sub.stream,
            move |state| {
                l_snap.on_snapshot(state);
                true
            },
            move |event| match event {
                TimelineReadingEvent::Update(state) => {
                    l_upd.on_update((&state).into());
                    true
                }
                TimelineReadingEvent::Closed(reason) => {
                    l_upd.on_closed(reason.into());
                    false
                }
                _ => false,
            },
            move || l_end.on_closed(FfiHistoryClosedReason::AccountClosed),
        )
        .map_err(|_| FfiTimelineError::Closed {
            reason: FfiHistoryClosedReason::AccountClosed,
        })
    }

    /// Returns latest catch-up & arrivals overlay state synchronously.
    ///
    /// # Errors
    /// Returns closure or cursor failures.
    pub fn reading_state(&self) -> Result<FfiTimelineReadingState, FfiTimelineError> {
        let state = self.inner.reading_state()?;
        Ok((&state).into())
    }

    /// Clears active catch-up interval with FULL durable commit.
    ///
    /// # Errors
    /// Returns durable storage or closure failures.
    pub async fn clear_catch_up(&self) -> Result<(), FfiTimelineError> {
        let timeline = self.inner.clone();
        self.client
            .run_scoped(
                || FfiTimelineError::Closed {
                    reason: FfiHistoryClosedReason::AccountClosed,
                },
                async move { timeline.clear_catch_up().await.map_err(Into::into) },
            )
            .await
    }

    /// Starts cancellable hourglass catch-up seek for unloaded prefix before focusing start target.
    ///
    /// # Errors
    /// Returns closure or history failures.
    pub fn seek_catch_up(&self) -> Result<Arc<FfiCatchUpSeek>, FfiTimelineError> {
        let seek = self.inner.seek_catch_up(TimelineConfig::default())?;
        Ok(Arc::new(FfiCatchUpSeek {
            client: Arc::clone(&self.client),
            inner: Mutex::new(Some(seek)),
        }))
    }
}

#[uniffi::export]
impl FfiClient {
    /// Opens a live channel timeline window following new incoming messages.
    ///
    /// # Errors
    /// Returns [`FfiTimelineError`].
    pub async fn open_live_timeline(
        self: &Arc<Self>,
        account: FfiAccountId,
        channel_id: String,
        config: FfiTimelineConfig,
        offset_provider: Arc<dyn FfiUtcOffsetProvider>,
    ) -> Result<Arc<FfiTimeline>, FfiTimelineError> {
        let session = self.session_by_id(&account).map_err(timeline_session_err)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiTimelineError::InvalidArgument)?;
        let sdk_cfg = to_timeline_config(config, offset_provider)?;
        let client = Arc::clone(self);
        self.run_scoped(timeline_closed_err, async move {
            let inner = Timeline::live(session, channel, sdk_cfg).await?;
            Ok(Arc::new(FfiTimeline { client, inner }))
        })
        .await
    }

    /// Opens a live timeline window spanning back to cached `anchor_id` (up to 20 pages).
    ///
    /// # Errors
    /// Returns `AnchorTooFar`, `AnchorDeleted`, `FocusMissing`, or lifecycle errors.
    pub async fn open_live_timeline_at(
        self: &Arc<Self>,
        account: FfiAccountId,
        channel_id: String,
        anchor_id: String,
        config: FfiTimelineConfig,
        offset_provider: Arc<dyn FfiUtcOffsetProvider>,
    ) -> Result<Arc<FfiTimeline>, FfiTimelineError> {
        let session = self.session_by_id(&account).map_err(timeline_session_err)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiTimelineError::InvalidArgument)?;
        let anchor = parse_uuid(&anchor_id).map_err(|_| FfiTimelineError::InvalidArgument)?;
        let sdk_cfg = to_timeline_config(config, offset_provider)?;
        let client = Arc::clone(self);
        self.run_scoped(timeline_closed_err, async move {
            let inner = Timeline::live_at(session, channel, anchor, sdk_cfg).await?;
            Ok(Arc::new(FfiTimeline { client, inner }))
        })
        .await
    }

    /// Opens a bounded focused neighborhood around cached `anchor_id` regardless of depth.
    ///
    /// # Errors
    /// Returns `AnchorDeleted`, `FocusMissing`, or lifecycle errors.
    pub async fn open_focused_timeline_at(
        self: &Arc<Self>,
        account: FfiAccountId,
        channel_id: String,
        anchor_id: String,
        config: FfiTimelineConfig,
        offset_provider: Arc<dyn FfiUtcOffsetProvider>,
    ) -> Result<Arc<FfiTimeline>, FfiTimelineError> {
        let session = self.session_by_id(&account).map_err(timeline_session_err)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiTimelineError::InvalidArgument)?;
        let anchor = parse_uuid(&anchor_id).map_err(|_| FfiTimelineError::InvalidArgument)?;
        let sdk_cfg = to_timeline_config(config, offset_provider)?;
        let client = Arc::clone(self);
        self.run_scoped(timeline_closed_err, async move {
            let inner = Timeline::focused_at(session, channel, anchor, sdk_cfg).await?;
            Ok(Arc::new(FfiTimeline { client, inner }))
        })
        .await
    }

    /// Opens an independent focused neighborhood around a pinned/replied/searched exact cursor.
    ///
    /// # Errors
    /// Returns `FocusMissing`, `InvalidCursor`, or lifecycle errors.
    pub async fn open_focused_timeline(
        self: &Arc<Self>,
        account: FfiAccountId,
        channel_id: String,
        cursor: FfiHistoryCursor,
        config: FfiTimelineConfig,
        offset_provider: Arc<dyn FfiUtcOffsetProvider>,
    ) -> Result<Arc<FfiTimeline>, FfiTimelineError> {
        let session = self.session_by_id(&account).map_err(timeline_session_err)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiTimelineError::InvalidArgument)?;
        let sdk_cfg = to_timeline_config(config, offset_provider)?;
        let client = Arc::clone(self);
        self.run_scoped(timeline_closed_err, async move {
            let inner = Timeline::focused(session, channel, cursor.into(), sdk_cfg).await?;
            Ok(Arc::new(FfiTimeline { client, inner }))
        })
        .await
    }
}
