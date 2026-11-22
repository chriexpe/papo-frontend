//! Shared push notification device registration and revocation helpers across FFI.
//! Policy, settings, inbox, and delivery listeners live in `notifications.rs`.

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::FfiAccountId,
    notification_records::{FfiPushDevice, FfiPushRegistration, map_push_device, map_push_reg},
    operation_errors::FfiNotificationError,
};

#[uniffi::export]
impl FfiClient {
    /// Registers a push device token cleanly, rotating prior ownership on the same server first.
    ///
    /// # Errors
    /// Returns [`FfiNotificationError`].
    pub async fn register_push(
        &self,
        account: FfiAccountId,
        device: FfiPushDevice,
    ) -> Result<FfiPushRegistration, FfiNotificationError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiNotificationError::Session { error: err })?;
        let sdk_device = map_push_device(device);
        self.run_scoped(
            || FfiNotificationError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                session
                    .notifications()
                    .register_push(sdk_device)
                    .await
                    .map(map_push_reg)
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Removes active push token registration for `account`.
    ///
    /// # Errors
    /// Returns [`FfiNotificationError`].
    pub async fn unregister_push(
        &self,
        account: FfiAccountId,
    ) -> Result<FfiPushRegistration, FfiNotificationError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiNotificationError::Session { error: err })?;
        self.run_scoped(
            || FfiNotificationError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                session
                    .notifications()
                    .unregister_push()
                    .await
                    .map(map_push_reg)
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Removes an explicitly supplied host push token after restart or uncertain ownership.
    ///
    /// # Errors
    /// Returns [`FfiNotificationError`].
    pub async fn unregister_push_device(
        &self,
        account: FfiAccountId,
        device: FfiPushDevice,
    ) -> Result<FfiPushRegistration, FfiNotificationError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiNotificationError::Session { error: err })?;
        let sdk_device = map_push_device(device);
        self.run_scoped(
            || FfiNotificationError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                session
                    .notifications()
                    .unregister_push_device(sdk_device)
                    .await
                    .map(map_push_reg)
                    .map_err(Into::into)
            },
        )
        .await
    }
}
