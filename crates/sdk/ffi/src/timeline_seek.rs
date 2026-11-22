//! Cancellable hourglass catch-up seek handle and timeline config mapping across FFI.
//! Live and focused `FfiTimeline` operations live in `timeline.rs`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use papo_sdk_ui::{CatchUpSeek, TimelineConfig, UtcOffset};

use crate::{
    client::FfiClient,
    timeline::FfiTimeline,
    timeline_errors::{FfiHistoryClosedReason, FfiTimelineError},
    timeline_items::{FfiTimelineConfig, FfiUtcOffsetProvider},
};

/// Cancelable hourglass seek operation returned by `FfiTimeline::seek_catch_up`.
#[derive(uniffi::Object)]
pub struct FfiCatchUpSeek {
    /// Parent client handle for scoped execution.
    pub(crate) client: Arc<FfiClient>,
    /// Inner one-shot `CatchUpSeek` handle (`None` once consumed by `finish`).
    pub(crate) inner: Mutex<Option<CatchUpSeek>>,
}

#[uniffi::export]
impl FfiCatchUpSeek {
    /// Cancels hourglass prefix pagination immediately without erasing catch-up interval.
    pub fn cancel(&self) {
        if let Some(seek) = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            seek.cancel();
        }
    }

    /// Resolves unloaded prefix and opens focused timeline window at catch-up start target.
    ///
    /// # Errors
    /// Returns `Cancelled`, `FocusMissing`, or history errors.
    pub async fn finish(&self) -> Result<Arc<FfiTimeline>, FfiTimelineError> {
        let seek = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or(FfiTimelineError::Cancelled)?;
        let client = Arc::clone(&self.client);
        self.client
            .run_scoped(|| FfiTimelineError::Cancelled, async move {
                let inner = seek.finish().await?;
                Ok(Arc::new(FfiTimeline { client, inner }))
            })
            .await
    }
}

/// Converts FFI timeline config + foreign offset provider into `TimelineConfig`.
pub(crate) fn to_timeline_config(
    config: FfiTimelineConfig,
    offset_provider: Arc<dyn FfiUtcOffsetProvider>,
) -> Result<TimelineConfig, FfiTimelineError> {
    let cap =
        usize::try_from(config.subscriber_capacity).map_err(|_| FfiTimelineError::InvalidConfig)?;
    if config.page_size == 0 || config.page_size > 100 || cap == 0 {
        return Err(FfiTimelineError::InvalidConfig);
    }
    Ok(TimelineConfig {
        page_size: config.page_size,
        subscriber_capacity: cap,
        offset_provider: Arc::new(move |ts_us: i64| {
            let secs = catch_unwind(AssertUnwindSafe(|| {
                offset_provider.offset_at_seconds_east(ts_us)
            }))
            .unwrap_or(0);
            UtcOffset::from_seconds_east(secs).unwrap_or_default()
        }),
    })
}

/// Maps helper errors on timeline opening cleanly.
pub(crate) fn timeline_closed_err() -> FfiTimelineError {
    FfiTimelineError::Closed {
        reason: FfiHistoryClosedReason::AccountClosed,
    }
}
