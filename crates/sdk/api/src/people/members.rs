//! Owns member listing and profile wire calls; media downloads belong to the media transport.

use crate::{
    Cursor, Error, RestClient, Uuid, routes,
    wire::{
        GenericResponse, MemberListResponse, ProfileBatchRequest, ProfileBatchResponse,
        UpdateAvatarRequest, UpdateBannerRequest, UpdateStatusRequest, UpdateUserRequest,
        UserProfile, UserSummary, UserSummaryBatchRequest,
    },
};
impl RestClient {
    /// Lists a page of members without fetching full profiles.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn members(&self, cursor: Option<&Cursor<'_>>) -> Result<MemberListResponse, Error> {
        let parameters = cursor.map_or_else(Vec::new, Cursor::parameters);
        self.json(
            self.route_request(&routes::GET_USERS, &[])?
                .query(&parameters),
        )
        .await
    }

    /// Fetches a single member profile.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn profile(&self, user: Uuid) -> Result<UserProfile, Error> {
        self.json(self.route_request(&routes::GET_USERS_USER_ID_PROFILE, &[user.to_string()])?)
            .await
    }

    /// Fetches up to 50 profiles; the backend validates the request.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn profiles(
        &self,
        request: &ProfileBatchRequest,
    ) -> Result<ProfileBatchResponse, Error> {
        self.post(&routes::POST_USERS_PROFILE_BATCH, &[], request)
            .await
    }

    /// Fetches up to 1000 lightweight summaries; the backend validates the request.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn user_summaries(
        &self,
        request: &UserSummaryBatchRequest,
    ) -> Result<Vec<UserSummary>, Error> {
        self.post(&routes::POST_USERS_USER_SUMMARY_BATCH, &[], request)
            .await
    }

    /// Updates the authenticated user's profile.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn update_profile(
        &self,
        user: Uuid,
        request: &UpdateUserRequest,
    ) -> Result<GenericResponse, Error> {
        self.json(
            self.route_request(&routes::PUT_USERS_USER_ID, &[user.to_string()])?
                .json(request),
        )
        .await
    }

    /// Updates the authenticated user's persisted presence status.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn update_status(
        &self,
        user: Uuid,
        request: &UpdateStatusRequest,
    ) -> Result<GenericResponse, Error> {
        self.json(
            self.route_request(&routes::PUT_USERS_USER_ID_STATUS, &[user.to_string()])?
                .json(request),
        )
        .await
    }

    /// Updates the authenticated user's avatar using the backend's base64 format.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn update_avatar(
        &self,
        user: Uuid,
        request: &UpdateAvatarRequest,
    ) -> Result<GenericResponse, Error> {
        self.json(
            self.route_request(&routes::PUT_USERS_USER_ID_AVATAR, &[user.to_string()])?
                .json(request),
        )
        .await
    }

    /// Updates or removes the authenticated user's banner.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn update_banner(
        &self,
        user: Uuid,
        request: &UpdateBannerRequest,
    ) -> Result<GenericResponse, Error> {
        self.json(
            self.route_request(&routes::PUT_USERS_USER_ID_BANNER, &[user.to_string()])?
                .json(request),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    //! Verifies profile response decoding.

    use super::*;

    #[test]
    fn profile_accepts_backend_null_avatar_and_roles() {
        let profile: UserProfile =
            serde_json::from_str(r#"{"avatar_blob":null,"roles":null}"#).unwrap();
        assert!(profile.avatar_blob.is_none() && profile.roles.is_none());
    }

    #[test]
    fn nullable_response_lists_remain_absent() {
        let page: MemberListResponse = serde_json::from_str(r#"{"users":null}"#).unwrap();
        let batch: ProfileBatchResponse = serde_json::from_str(r#"{"profiles":null}"#).unwrap();
        assert!(page.users.is_none() && batch.profiles.is_none());
    }

    #[test]
    fn empty_batch_responses_decode() {
        let page: MemberListResponse =
            serde_json::from_str(r#"{"users":[],"has_more":false}"#).unwrap();
        let batch: ProfileBatchResponse = serde_json::from_str(r#"{"profiles":[]}"#).unwrap();
        assert!(
            page.users.unwrap().is_empty()
                && page.has_more == Some(false)
                && batch.profiles.unwrap().is_empty()
        );
    }
}
