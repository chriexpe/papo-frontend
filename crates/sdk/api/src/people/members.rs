//! Owns member listing and profile wire calls; media downloads belong to the media transport.

use crate::{
    Error, RestClient, Uuid,
    wire::{
        GenericResponse, MemberListResponse, ProfileBatchRequest, ProfileBatchResponse,
        UpdateAvatarRequest, UpdateBannerRequest, UpdateStatusRequest, UpdateUserRequest,
        UserProfile, UserSummary, UserSummaryBatchRequest,
    },
};
use reqwest::Method;

/// Member pagination; `since` is the exact backend timestamp string.
#[derive(Default)]
pub struct MemberQuery<'a> {
    /// Timestamp of the cursor, if any.
    pub since: Option<&'a str>,
    /// ID paired with `since` for an exact cursor.
    pub last_id: Option<Uuid>,
    /// Backend ordering direction (`asc` or `desc`).
    pub order: Option<&'a str>,
}

impl MemberQuery<'_> {
    fn parameters(&self) -> Vec<(&'static str, String)> {
        let mut parameters = Vec::new();
        if let Some(since) = self.since {
            parameters.push(("since", since.to_owned()));
        }
        if let Some(last_id) = self.last_id {
            parameters.push(("last_id", last_id.to_string()));
        }
        if let Some(order) = self.order {
            parameters.push(("order", order.to_owned()));
        }
        parameters
    }
}

impl RestClient {
    /// Lists a page of members without fetching full profiles.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn members(&self, query: &MemberQuery<'_>) -> Result<MemberListResponse, Error> {
        self.json(
            self.request(Method::GET, "users")?
                .query(&query.parameters()),
        )
        .await
    }

    /// Fetches a single member profile.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn profile(&self, user: Uuid) -> Result<UserProfile, Error> {
        self.json(self.request(Method::GET, &format!("users/{user}/profile"))?)
            .await
    }

    /// Fetches up to 50 profiles; the backend validates the request.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn profiles(
        &self,
        request: &ProfileBatchRequest,
    ) -> Result<ProfileBatchResponse, Error> {
        self.post("users/profile_batch", request).await
    }

    /// Fetches up to 1000 lightweight summaries; the backend validates the request.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn user_summaries(
        &self,
        request: &UserSummaryBatchRequest,
    ) -> Result<Vec<UserSummary>, Error> {
        self.post("users/user_summary_batch", request).await
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
            self.request(Method::PUT, &format!("users/{user}"))?
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
            self.request(Method::PUT, &format!("users/{user}/status"))?
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
            self.request(Method::PUT, &format!("users/{user}/avatar"))?
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
            self.request(Method::PUT, &format!("users/{user}/banner"))?
                .json(request),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    //! Verifies member cursors and profile response decoding.

    use super::*;

    #[test]
    fn member_cursor_keeps_timestamp_precision() {
        let query = MemberQuery {
            since: Some("2024-01-02T03:04:05.123456Z"),
            order: Some("desc"),
            ..MemberQuery::default()
        };
        let client = RestClient::new("https://example.invalid/base").unwrap();
        let request = client
            .request(Method::GET, "users")
            .unwrap()
            .query(&query.parameters())
            .build()
            .unwrap();
        let pairs: Vec<_> = request.url().query_pairs().collect();
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "since" && value == "2024-01-02T03:04:05.123456Z")
        );
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "order" && value == "desc")
        );
        assert!(!pairs.iter().any(|(key, _)| key == "last_id"));
    }

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
