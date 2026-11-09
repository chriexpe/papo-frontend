//! Owns server configuration and channel-order REST calls; validation belongs to the backend.

use crate::{
    Error, RestClient, Uuid,
    wire::{
        ChangeChannelPositionRequest, Channel, PatchServerRequest, ReplaceServerRequest, Server,
    },
};
use reqwest::Method;

impl RestClient {
    /// Replaces all server settings.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn replace_server(&self, request: &ReplaceServerRequest) -> Result<Server, Error> {
        self.json(self.request(Method::PUT, "server")?.json(request))
            .await
    }

    /// Updates only the provided server settings.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn patch_server(&self, request: &PatchServerRequest) -> Result<Server, Error> {
        self.json(self.request(Method::PATCH, "server")?.json(request))
            .await
    }

    /// Changes a channel's position using the backend's old and new positions.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn change_channel_position(
        &self,
        channel: Uuid,
        request: &ChangeChannelPositionRequest,
    ) -> Result<Channel, Error> {
        self.json(
            self.request(Method::PUT, &format!("channels/{channel}/change_position"))?
                .json(request),
        )
        .await
    }
}
