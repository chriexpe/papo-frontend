//! Owns acknowledged server configuration changes; authorization and validation remain on the server.

use super::{OperationError, OperationReceipt, Operations, Refresh};
pub use papo_api::wire::{PatchServerRequest, ReplaceServerRequest, Server};

impl Operations {
    /// Changes only supplied server settings. A refresh failure is an accepted receipt warning.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn patch_server(
        &self,
        request: PatchServerRequest,
    ) -> Result<OperationReceipt<Server>, OperationError> {
        self.mutate(
            move |rest| async move { rest.patch_server(&request).await },
            Refresh::Server,
        )
        .await
    }

    /// Replaces all backend server settings, including password, visibility and icon.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn replace_server(
        &self,
        request: ReplaceServerRequest,
    ) -> Result<OperationReceipt<Server>, OperationError> {
        self.mutate(
            move |rest| async move { rest.replace_server(&request).await },
            Refresh::Server,
        )
        .await
    }
}
