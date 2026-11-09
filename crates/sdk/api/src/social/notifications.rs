//! Owns notification, push-device and read-marker REST calls; notification policy belongs to the SDK.

use super::Page;
use crate::{
    Error, RestClient, Uuid,
    wire::{
        ChannelUserSetting, MarkNotificationsReadRequest, MarkNotificationsReadResponse,
        NotificationList, PushDeviceRequest, ReadChannelRequest, UpdateChannelUserSettingRequest,
    },
};
use reqwest::Method;
impl RestClient {
    /// Lists a user's notifications in backend order.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn user_notifications(
        &self,
        user: Uuid,
        page: &Page<'_>,
    ) -> Result<NotificationList, Error> {
        self.json(
            self.request(Method::GET, &format!("users/{user}/notifications"))?
                .query(&page.parameters()),
        )
        .await
    }

    /// Marks the supplied notification IDs read.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn mark_notifications_read(
        &self,
        user: Uuid,
        request: &MarkNotificationsReadRequest,
    ) -> Result<MarkNotificationsReadResponse, Error> {
        self.json(
            self.request(Method::PUT, &format!("users/{user}/read_notification"))?
                .json(request),
        )
        .await
    }

    /// Updates a user's notification setting for a channel.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn update_channel_user_setting(
        &self,
        channel: Uuid,
        user: Uuid,
        request: &UpdateChannelUserSettingRequest,
    ) -> Result<ChannelUserSetting, Error> {
        self.post(&format!("channels/{channel}/user/{user}/settings"), request)
            .await
    }

    /// Advances the channel read marker through the given message.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn read_channel(&self, channel: Uuid, message: Uuid) -> Result<(), Error> {
        self.response(
            self.request(Method::PUT, &format!("channels/{channel}/read"))?
                .json(&ReadChannelRequest {
                    message_id: message.to_string(),
                }),
        )
        .await
        .map(|_| ())
    }

    /// Registers or updates a push device for the authenticated user.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn register_push_device(&self, request: &PushDeviceRequest) -> Result<(), Error> {
        self.response(
            self.request(Method::PUT, "users/me/push-device")?
                .json(request),
        )
        .await
        .map(|_| ())
    }

    /// Removes the authenticated user's push device identified by its token.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn delete_push_device(&self, request: &PushDeviceRequest) -> Result<(), Error> {
        self.response(
            self.request(Method::DELETE, "users/me/push-device")?
                .json(request),
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    //! Owns inline read-marker shape tests.
    use super::*;
    #[test]
    fn read_marker_uses_message_id_field() {
        assert_eq!(
            serde_json::to_value(ReadChannelRequest {
                message_id: Uuid::nil().to_string()
            })
            .unwrap(),
            serde_json::json!({"message_id": "00000000-0000-0000-0000-000000000000"})
        );
    }
}
