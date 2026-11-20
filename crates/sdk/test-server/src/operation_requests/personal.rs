//! Personal mutation fixtures update explicit resources; denial scenarios are scripted, not a permission engine.

use papo_api::wire::{DirectConversation, UserSummary};
use serde_json::{Value, json};

use crate::{
    resources::CREATED_AT,
    response::{Reply, problem},
    server::State,
};

pub(super) fn relevant(method: &str, path: &str) -> bool {
    (method == "PUT" && path.starts_with("/users/"))
        || (method == "POST"
            && (path == "/dms" || path.ends_with("/block") || path.ends_with("/settings")))
        || (method == "DELETE" && (path.starts_with("/dms/") || path.ends_with("/block")))
        || (method == "GET" && path.starts_with("/dms/"))
}

pub(super) fn handle(
    state: &mut State,
    method: &str,
    path: &str,
    parts: &[&str],
    user: &str,
    body: &Value,
) -> Option<Reply> {
    match (method, parts) {
        ("PUT", ["users", "settings"]) => {
            let version = state
                .operations
                .settings
                .get(user)
                .and_then(|s| s["version"].as_u64())
                .unwrap_or(1)
                .saturating_add(1);
            let settings = json!({"version":version,"config":body["config"]});
            state
                .operations
                .settings
                .insert(user.into(), settings.clone());
            Some(Reply::json(
                200,
                json!({"user_id":user,"version":version,"config":settings["config"],"updated_at":CREATED_AT}),
            ))
        }
        ("POST", ["channels", channel, "user", target, "settings"]) => {
            if *target != user {
                return Some(problem(403, "forbidden", path));
            }
            let exists = state.resources.channels.contains_key(*channel)
                || state
                    .resources
                    .dms
                    .values()
                    .flatten()
                    .any(|dm| dm.id == *channel);
            if !exists {
                return Some(problem(404, "not-found", path));
            }
            let Ok(setting) = serde_json::from_value::<papo_api::wire::NotificationSettings>(
                body["notification_settings"].clone(),
            ) else {
                return Some(problem(400, "invalid-param", path));
            };
            state
                .resources
                .channel_settings
                .insert((user.to_owned(), (*channel).to_owned()), setting.clone());
            Some(Reply::json(
                200,
                json!({
                    "user_id": user,
                    "channel_id": channel,
                    "notification_settings": setting,
                    "updated_at": CREATED_AT,
                }),
            ))
        }
        ("PUT", ["users", target]) => Some(profile(state, path, target, user, None, body)),
        ("PUT", ["users", target, action @ ("status" | "avatar" | "banner")]) => {
            Some(profile(state, path, target, user, Some(action), body))
        }
        ("POST", ["users", target, "block"]) => Some(block(state, path, user, target, true)),
        ("DELETE", ["users", target, "block"]) => Some(block(state, path, user, target, false)),
        ("POST", ["dms"]) => Some(open_dm(state, path, user, body)),
        ("GET", ["dms", id]) => Some(
            state
                .resources
                .dms
                .get(user)
                .and_then(|dms| dms.iter().find(|dm| dm.id == *id))
                .map_or_else(
                    || problem(404, "not-found", path),
                    |dm| Reply::json(200, json!(dm)),
                ),
        ),
        ("DELETE", ["dms", id]) => {
            let dms = state.resources.dms.entry(user.into()).or_default();
            if !dms.iter().any(|dm| dm.id == *id) {
                return Some(problem(404, "not-found", path));
            }
            dms.retain(|dm| dm.id != *id);
            Some(Reply::empty())
        }
        _ => None,
    }
}

fn profile(
    state: &mut State,
    path: &str,
    target: &str,
    user: &str,
    action: Option<&str>,
    body: &Value,
) -> Reply {
    if target != user {
        return problem(403, "forbidden", path);
    }
    let Some(profile) = state.resources.profiles.get_mut(target) else {
        return problem(404, "not-found", path);
    };
    match action {
        None => {
            let (Some(nickname), Some(status), Some(description)) = (
                body["nickname"].as_str(),
                body["status"].as_str(),
                body["description"].as_str(),
            ) else {
                return problem(400, "invalid-param", path);
            };
            profile.nickname = nickname.into();
            profile.status_message = Some(status.into());
            profile.description = Some(description.into());
            if let Some(typing) = body.get("typing") {
                profile.typing = typing
                    .as_str()
                    .map_or(papo_api::NullableText::Null, Into::into);
            }
        }
        Some("status") => {
            if !body["status"].is_null()
                && !matches!(body["status"].as_str(), Some("away" | "busy"))
            {
                return problem(400, "invalid-param", path);
            }
            profile.status = body["status"].as_str().map(str::to_owned);
        }
        Some("avatar") => {
            profile.avatar_blob = body["avatar"].as_str().map(str::to_owned);
            profile.avatar_format = body["avatar_format"].as_str().map(str::to_owned);
        }
        Some("banner") => {
            profile.banner_media = body["banner"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
        }
        _ => return problem(404, "not-found", path),
    }
    Reply::json(200, json!({"response":"ok"}))
}

fn summary(state: &State, user: &str) -> Option<UserSummary> {
    let profile = state.resources.profiles.get(user)?;
    Some(UserSummary {
        id: profile.id.clone(),
        username: profile.username.clone(),
        nickname: profile.nickname.clone(),
        roles: profile.roles.clone(),
        created_at: profile.created_at.clone(),
        ..UserSummary::default()
    })
}

fn block(state: &mut State, path: &str, user: &str, target: &str, blocked: bool) -> Reply {
    if user == target {
        return problem(400, "invalid-param", path);
    }
    if blocked {
        let Some(target_summary) = summary(state, target) else {
            return problem(404, "not-found", path);
        };
        let users = state.resources.blocks.entry(user.into()).or_default();
        if !users.iter().any(|s| s.id.as_deref() == Some(target)) {
            users.push(target_summary);
        }
    } else {
        state
            .resources
            .blocks
            .entry(user.into())
            .or_default()
            .retain(|s| s.id.as_deref() != Some(target));
    }
    Reply::empty()
}

fn open_dm(state: &mut State, path: &str, user: &str, body: &Value) -> Reply {
    let Some(target) = body["user_id"].as_str() else {
        return problem(400, "invalid-param", path);
    };
    if user == target {
        return problem(400, "invalid-param", path);
    }
    let Some(target_summary) = summary(state, target) else {
        return problem(404, "not-found", path);
    };
    if let Some(dm) = state
        .resources
        .dms
        .get(user)
        .and_then(|dms| dms.iter().find(|dm| dm.user.id.as_deref() == Some(target)))
    {
        return Reply::json(200, json!(dm));
    }
    let id = state.operations.id();
    let dm = DirectConversation {
        id,
        user: target_summary,
        created_at: CREATED_AT.into(),
        ..DirectConversation::default()
    };
    state
        .resources
        .dms
        .entry(user.into())
        .or_default()
        .push(dm.clone());
    Reply::json(201, json!(dm))
}
