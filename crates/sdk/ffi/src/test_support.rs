//! Deterministic `FakeMediaFactory` test harness exposed only when `feature = "test-support"` is enabled.
//! Production Android APK and release libraries never enable this module.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use papo_sdk::{
    FakeMediaFactory, FakeMediaLeaseHandle, FakeStartBehavior, FakeVideoDestination, MediaCommand,
    MediaStartError, VideoDestinationId,
};

use crate::{
    call_enums::FfiVideoKind,
    calls::FfiCallHandle,
    client::{FfiClient, FfiClientConfig},
    errors::FfiClientError,
    ids::checked_list_index,
    operation_errors::FfiCallError,
    secret::FfiSecretStore,
};

/// FFI-exportable deterministic `FakeMediaFactory` controller for JVM & Rust call integration tests.
#[derive(uniffi::Object)]
pub struct FfiFakeMediaFactory {
    /// Underlying fake media factory injected into `FfiClient::open_with_media_engine`.
    factory: FakeMediaFactory,
    /// Tracks bound fake video destinations by ID for revocation assertions.
    destinations: Mutex<BTreeMap<u64, Arc<FakeVideoDestination>>>,
}

impl FfiFakeMediaFactory {
    /// Exposes underlying `FakeMediaFactory` for Rust integration tests.
    #[must_use]
    pub fn inner(&self) -> &FakeMediaFactory {
        &self.factory
    }

    /// Returns all created lease handles in start order.
    #[must_use]
    pub fn leases(&self) -> Vec<Arc<FakeMediaLeaseHandle>> {
        self.factory.leases()
    }
}

#[uniffi::export]
impl FfiFakeMediaFactory {
    /// Creates a new fake media engine factory with auto-drain enabled.
    #[uniffi::constructor]
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            factory: FakeMediaFactory::new(),
            destinations: Mutex::new(BTreeMap::new()),
        })
    }

    /// Controls whether leases automatically emit `Drained` when stopped.
    pub fn set_auto_drain(&self, auto_drain: bool) {
        self.factory.set_auto_drain(auto_drain);
    }

    /// Scripts the next `start()` call to fail with `DeviceUnavailable`.
    pub fn fail_next_device_unavailable(&self) {
        self.factory
            .push_start(FakeStartBehavior::Fail(MediaStartError::DeviceUnavailable));
    }

    /// Total `start()` invocations observed across calls.
    #[must_use]
    pub fn start_count(&self) -> u32 {
        checked_list_index(self.factory.start_count()).unwrap_or(u32::MAX)
    }

    /// Number of started lease handles recorded so far.
    #[must_use]
    pub fn lease_count(&self) -> u32 {
        checked_list_index(self.factory.leases().len()).unwrap_or(u32::MAX)
    }

    /// Attempt ID recorded on the most recently started lease, if any.
    #[must_use]
    pub fn latest_lease_attempt(&self) -> Option<u64> {
        self.factory.latest_lease().map(|l| l.attempt.raw())
    }

    /// True if the lease at `index` has received `stop()`.
    #[must_use]
    pub fn is_lease_stopped(&self, index: u32) -> bool {
        self.factory
            .leases()
            .get(index as usize)
            .is_some_and(|l| l.is_stopped())
    }

    /// Emits `MediaTransportState::Connected` on the latest started lease.
    pub fn emit_latest_connected(&self) {
        if let Some(lease) = self.factory.latest_lease() {
            lease.emit(papo_sdk::MediaReportKind::TransportState(
                papo_sdk::MediaTransportState::Connected,
            ));
        }
    }

    /// Binds a `FakeVideoDestination(destination_id)` through `FfiCallHandle::bind_video_destination`
    /// so JVM and Rust tests can verify exact instance binding and revocation.
    ///
    /// # Errors
    /// Returns [`FfiCallError`] if `participant_user_id` is malformed or the attempt ended.
    pub fn bind_fake_video_destination(
        &self,
        call_handle: Arc<FfiCallHandle>,
        participant_user_id: String,
        kind: FfiVideoKind,
        destination_id: u64,
    ) -> Result<(), FfiCallError> {
        let dest = Arc::new(FakeVideoDestination::new(VideoDestinationId::from_raw(
            destination_id,
        )));
        self.destinations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(destination_id, Arc::clone(&dest));
        call_handle.bind_video_destination(participant_user_id, kind, dest)
    }

    /// True if a `SetVideoSlotPlan` command sent to `lease_index` contains `destination_id`.
    #[must_use]
    pub fn lease_received_video_destination(&self, lease_index: u32, destination_id: u64) -> bool {
        let target = VideoDestinationId::from_raw(destination_id);
        let Some(lease) = self.factory.leases().into_iter().nth(lease_index as usize) else {
            return false;
        };
        lease.commands().into_iter().any(|cmd| match cmd {
            MediaCommand::SetVideoSlotPlan { slots, .. } => slots
                .iter()
                .any(|slot| slot.destination.as_ref().is_some_and(|d| d.id() == target)),
            _ => false,
        })
    }

    /// True once `revoke()` was invoked on the bound fake destination `destination_id`.
    #[must_use]
    pub fn is_fake_video_revoked(&self, destination_id: u64) -> bool {
        self.destinations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&destination_id)
            .is_some_and(|d| d.is_revoked())
    }
}

/// Opens `FfiClient` injected with a controllable `FakeMediaFactory` for host integration tests.
///
/// # Errors
/// Returns configuration, profile lock, or worker startup failures.
#[uniffi::export]
pub fn open_ffi_client_with_fake_media(
    config: FfiClientConfig,
    secret_store: Arc<dyn FfiSecretStore>,
    media: Arc<FfiFakeMediaFactory>,
) -> Result<Arc<FfiClient>, FfiClientError> {
    let media_engine: Arc<dyn papo_sdk::MediaEngineFactory> = Arc::new(media.factory.clone());
    FfiClient::open_with_media_engine(config, secret_store, Some(media_engine))
}
