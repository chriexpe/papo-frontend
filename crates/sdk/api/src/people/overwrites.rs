//! Owns channel role-overwrite REST calls; permission evaluation belongs to the SDK.

use crate::{
    Error, RestClient, Uuid,
    wire::{
        ChannelPermissionsResponse, UpdateChannelPermissionsRequest,
        UpdateChannelPermissionsResponse,
    },
};
use reqwest::Method;

impl RestClient {
    /// Lists a channel's role-specific permission overwrites.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn channel_permissions(
        &self,
        channel: Uuid,
    ) -> Result<ChannelPermissionsResponse, Error> {
        self.json(self.request(Method::GET, &format!("channels/{channel}/permissions"))?)
            .await
    }

    /// Replaces a role's permissions for a channel.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn update_channel_permissions(
        &self,
        channel: Uuid,
        role: Uuid,
        request: &UpdateChannelPermissionsRequest,
    ) -> Result<UpdateChannelPermissionsResponse, Error> {
        self.json(
            self.request(
                Method::PUT,
                &format!("channels/{channel}/permissions/{role}"),
            )?
            .json(request),
        )
        .await
    }

    /// Removes a channel overwrite without deleting its role.
    /// # Errors
    /// Returns transport or backend failures.
    pub async fn delete_channel_role(&self, channel: Uuid, role: Uuid) -> Result<(), Error> {
        self.response(self.request(Method::DELETE, &format!("channels/{channel}/role/{role}"))?)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    //! Verifies channel-overwrite response decoding.

    use super::*;

    #[test]
    fn updated_permissions_decode_false_distinct_from_missing() {
        let value: UpdateChannelPermissionsResponse = serde_json::from_str(
            r#"{"channel_id":"c","role_id":"r","permissions":{"read_channel":false}}"#,
        )
        .unwrap();
        let permissions = value.permissions.unwrap();
        assert_eq!(permissions.read_channel, Some(false));
        assert_eq!(permissions.send_messages, None);
    }
}
