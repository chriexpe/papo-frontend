//! Owns notification, push-device and read-marker REST calls; notification policy belongs to the SDK.

use crate::{
    Cursor, Error, RestClient, Uuid, routes,
    wire::{
        ChannelUserSetting, MarkNotificationsReadRequest, MarkNotificationsReadResponse,
        NotificationList, PushDeviceRequest, ReadChannelRequest, UpdateChannelUserSettingRequest,
    },
};
impl RestClient {
    /// Lists a user's notifications in backend order.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn user_notifications(
        &self,
        user: Uuid,
        cursor: Option<&Cursor<'_>>,
    ) -> Result<NotificationList, Error> {
        self.user_notifications_ordered(user, cursor, crate::HistoryOrder::Desc)
            .await
    }

    /// Lists notifications in explicit backend tuple order, including an initial ascending page.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn user_notifications_ordered(
        &self,
        user: Uuid,
        cursor: Option<&Cursor<'_>>,
        order: crate::HistoryOrder,
    ) -> Result<NotificationList, Error> {
        let mut parameters = cursor.map_or_else(Vec::new, Cursor::parameters);
        parameters.retain(|(key, _)| *key != routes::ORDER);
        parameters.push((routes::ORDER, order.as_str().to_owned()));
        self.json(
            self.route_request(
                &routes::GET_USERS_USER_ID_NOTIFICATIONS,
                &[user.to_string()],
            )?
            .query(&parameters),
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
            self.route_request(
                &routes::PUT_USERS_USER_ID_READ_NOTIFICATION,
                &[user.to_string()],
            )?
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
        self.post(
            &routes::POST_CHANNELS_CHANNEL_ID_USER_USER_ID_SETTINGS,
            &[channel.to_string(), user.to_string()],
            request,
        )
        .await
    }

    /// Advances the channel read marker through the given message.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn read_channel(&self, channel: Uuid, message: Uuid) -> Result<(), Error> {
        self.response(
            self.route_request(
                &routes::PUT_CHANNELS_CHANNEL_ID_READ,
                &[channel.to_string()],
            )?
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
            self.route_request(&routes::PUT_USERS_ME_PUSH_DEVICE, &[])?
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
            self.route_request(&routes::DELETE_USERS_ME_PUSH_DEVICE, &[])?
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
