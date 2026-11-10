//! Owns fixture member pagination and on-demand profile batches; leaves authorization and account snapshots outside.

use std::collections::{BTreeMap, BTreeSet};

use papo_api::{
    Uuid,
    wire::{
        MemberListResponse, ProfileBatchRequest, ProfileBatchResponse, UserProfile, UserSummary,
    },
};
use serde_json::json;

use crate::{
    resources::Resources,
    response::{Reply, problem},
};

impl Resources {
    pub(crate) fn members(&self, query: &str) -> Reply {
        let parameters: BTreeMap<_, _> = url::form_urlencoded::parse(query.as_bytes()).collect();
        let order = parameters
            .get("order")
            .map_or("asc", |value| value.as_ref());
        if !matches!(order, "asc" | "desc") {
            return problem(400, "invalid-param", "/users");
        }
        let since = parameters.get("since").map(AsRef::as_ref);
        let last_id = parameters.get("last_id").map_or("", |value| value.as_ref());
        let mut profiles: Vec<_> = self
            .profiles
            .values()
            .filter(|profile| {
                since.is_none_or(|since| {
                    let created_at = profile.created_at.as_deref().unwrap_or_default();
                    let id = profile.id.as_deref().unwrap_or_default();
                    if last_id.is_empty() {
                        if order == "asc" {
                            created_at > since
                        } else {
                            created_at < since
                        }
                    } else if order == "asc" {
                        (created_at, id) > (since, last_id)
                    } else {
                        (created_at, id) < (since, last_id)
                    }
                })
            })
            .collect();
        profiles.sort_by(|left, right| {
            (&left.created_at, &left.id).cmp(&(&right.created_at, &right.id))
        });
        if order == "desc" {
            profiles.reverse();
        }
        let has_more = profiles.len() > 100;
        let users = profiles.into_iter().take(100).map(summary).collect();
        Reply::json(
            200,
            json!(MemberListResponse {
                users: Some(users),
                has_more: Some(has_more)
            }),
        )
    }

    pub(crate) fn profile_batch(&self, body: &[u8], path: &str) -> Reply {
        let Some(ids) = batch_ids(body, 50, false) else {
            return problem(400, "invalid-param", path);
        };
        let profiles = ids
            .iter()
            .filter_map(|id| self.profiles.get(id))
            .cloned()
            .collect();
        Reply::json(
            200,
            json!(ProfileBatchResponse {
                profiles: Some(profiles)
            }),
        )
    }

    pub(crate) fn summary_batch(&self, body: &[u8], path: &str) -> Reply {
        let Some(ids) = batch_ids(body, 1000, true) else {
            return problem(400, "invalid-param", path);
        };
        let users: Vec<_> = ids
            .iter()
            .filter_map(|id| self.profiles.get(id))
            .map(summary)
            .collect();
        Reply::json(200, json!(users))
    }
}

fn batch_ids(body: &[u8], limit: usize, validate_uuid: bool) -> Option<Vec<String>> {
    let request: ProfileBatchRequest = serde_json::from_slice(body).ok()?;
    if request.ids.is_empty()
        || request.ids.len() > limit
        || request.ids.iter().any(String::is_empty)
    {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    for id in request.ids {
        let id = if validate_uuid {
            Uuid::parse_str(&id).ok()?.to_string()
        } else {
            id
        };
        if seen.insert(id.clone()) {
            ids.push(id);
        }
    }
    Some(ids)
}

fn summary(profile: &UserProfile) -> UserSummary {
    UserSummary {
        id: profile.id.clone(),
        username: profile.username.clone(),
        nickname: profile.nickname.clone(),
        banned: Some(false),
        status: profile.status.clone(),
        status_message: profile.status_message.clone(),
        typing: profile.typing.clone(),
        status_updated_at: profile.status_updated_at.clone(),
        created_at: profile.created_at.clone(),
        roles: profile.roles.clone(),
    }
}

#[cfg(test)]
mod tests {
    //! Owns clock-free pagination and batch validation tests; leaves TCP transport to integration tests.

    use super::*;

    #[test]
    fn profile_batches_deduplicate_in_request_order_and_skip_unknown_ids() {
        let mut resources = Resources::default();
        resources.register("one", "first");
        resources.register("two", "second");
        let reply = resources.profile_batch(
            br#"{"ids":["two","missing","one","two"]}"#,
            "/users/profile_batch",
        );
        let batch: ProfileBatchResponse = serde_json::from_value(reply.body.unwrap()).unwrap();
        assert_eq!(
            batch
                .profiles
                .unwrap()
                .iter()
                .map(|profile| profile.id.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["two", "one"]
        );
    }

    #[test]
    fn profile_batches_reject_empty_oversized_and_malformed_requests() {
        let resources = Resources::default();
        for body in [
            json!({"ids": []}),
            json!({"ids": [""]}),
            json!({"ids": vec!["one"; 51]}),
            json!({}),
        ] {
            assert_eq!(
                resources
                    .profile_batch(&serde_json::to_vec(&body).unwrap(), "/users/profile_batch")
                    .status,
                400
            );
        }
    }

    #[test]
    fn member_pages_use_exact_equal_timestamp_id_boundary_in_both_directions() {
        let mut resources = Resources::default();
        for id in 0..101 {
            resources.register(&format!("{id:03}"), "member");
        }
        let page: MemberListResponse =
            serde_json::from_value(resources.members("").body.unwrap()).unwrap();
        assert_eq!(
            (page.users.unwrap().len(), page.has_more),
            (100, Some(true))
        );
        let query = format!("since={}&last_id=099", crate::resources::CREATED_AT);
        let terminal: MemberListResponse =
            serde_json::from_value(resources.members(&query).body.unwrap()).unwrap();
        assert_eq!(terminal.users.unwrap()[0].id.as_deref(), Some("100"));
        let query = format!(
            "since={}&last_id=001&order=desc",
            crate::resources::CREATED_AT
        );
        let reverse: MemberListResponse =
            serde_json::from_value(resources.members(&query).body.unwrap()).unwrap();
        assert_eq!(reverse.users.unwrap()[0].id.as_deref(), Some("000"));
    }
}
