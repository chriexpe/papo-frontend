//! Channel/server/role mutation fixtures; permission denials are supplied by explicit HTTP scripts.

use papo_api::wire::{Channel, ChannelPermissionEntry, Role, RoleSummary, Server};
use serde_json::{Value, json};

use crate::{
    resources::CREATED_AT,
    response::{Reply, problem},
    server::State,
};

pub(super) fn relevant(method: &str, path: &str) -> bool {
    (matches!(method, "PUT" | "PATCH") && path == "/server")
        || (method == "POST" && matches!(path, "/channels" | "/roles"))
        || (matches!(method, "PUT" | "DELETE")
            && (path.starts_with("/channels/") || path.starts_with("/roles/")))
        || (method == "POST" && path.starts_with("/users/") && path.ends_with("/roles"))
        || (method == "DELETE" && path.starts_with("/users/") && path.contains("/roles/"))
}

pub(super) fn handle(
    state: &mut State,
    method: &str,
    path: &str,
    parts: &[&str],
    body: &Value,
) -> Option<Reply> {
    match (method, parts) {
        ("PUT" | "PATCH", ["server"]) => Some(server(state, path, body)),
        ("POST", ["channels"]) => {
            let id = state.operations.id();
            let channel = Channel {
                id: Some(id.clone()),
                name: body["name"].as_str().map(str::to_owned),
                r#type: Some(body["type"].as_str().unwrap_or("text").into()),
                topic: body["topic"].as_str().map(str::to_owned),
                parent_id: body["parent_id"].as_str().map(str::to_owned),
                position: i64::try_from(state.resources.channels.len()).ok(),
                // The pinned `GET /channels` always serializes an overwrite list.
                permissions: Some(Vec::new()),
                created_at: Some(CREATED_AT.into()),
                ..Channel::default()
            };
            state.resources.channels.insert(id, channel.clone());
            Some(Reply::json(201, json!(channel)))
        }
        ("PUT", ["channels", id]) => {
            let Some(channel) = state.resources.channels.get_mut(*id) else {
                return Some(problem(404, "not-found", path));
            };
            channel.name = body["name"].as_str().map(str::to_owned);
            channel.topic = body["topic"].as_str().map(str::to_owned);
            Some(Reply::json(200, json!(channel)))
        }
        ("DELETE", ["channels", id]) => Some(if state.resources.channels.remove(*id).is_some() {
            Reply::empty()
        } else {
            problem(404, "not-found", path)
        }),
        ("PUT", ["channels", id, "change_position"]) => {
            let Some(channel) = state.resources.channels.get_mut(*id) else {
                return Some(problem(404, "not-found", path));
            };
            channel.position = body["new_position"].as_i64();
            Some(Reply::json(200, json!(channel)))
        }
        ("PUT", ["channels", id, "permissions", role]) => {
            Some(overwrite(state, path, id, role, body))
        }
        ("DELETE", ["channels", id, "role", role]) => {
            let Some(channel) = state.resources.channels.get_mut(*id) else {
                return Some(problem(404, "not-found", path));
            };
            channel
                .permissions
                .get_or_insert_default()
                .retain(|entry| entry.role_id.as_deref() != Some(*role));
            Some(Reply::empty())
        }
        ("POST", ["roles"]) => Some(create_role(state, path, body)),
        ("PUT", ["roles", id]) => Some(update_role(state, path, id, body)),
        ("DELETE", ["roles", id]) => {
            state
                .resources
                .roles
                .retain(|role| role.id.as_deref() != Some(*id));
            for profile in state.resources.profiles.values_mut() {
                profile
                    .roles
                    .get_or_insert_default()
                    .retain(|role| role.id.as_deref() != Some(*id));
            }
            Some(Reply::empty())
        }
        ("POST", ["users", user, "roles"]) => Some(assign_role(state, path, user, body)),
        ("DELETE", ["users", user, "roles", role]) => {
            let Some(profile) = state.resources.profiles.get_mut(*user) else {
                return Some(problem(404, "not-found", path));
            };
            profile
                .roles
                .get_or_insert_default()
                .retain(|r| r.id.as_deref() != Some(*role));
            Some(Reply::empty())
        }
        _ => None,
    }
}

fn server(state: &mut State, path: &str, body: &Value) -> Reply {
    let mut value = state.resources.server.as_ref().map_or_else(
        || {
            json!({
                "id": "00000000-0000-4000-8000-000000000000",
                "name": "fixture",
                "public": true,
                "created_at": CREATED_AT,
            })
        },
        |server| json!(server),
    );
    for key in ["name", "public", "icon_format", "icon_blob"] {
        if let Some(field) = body.get(key) {
            value[key] = field.clone();
        }
    }
    let Ok(server) = serde_json::from_value::<Server>(value.clone()) else {
        return problem(400, "invalid-param", path);
    };
    state.resources.server = Some(server);
    Reply::json(200, value)
}

fn create_role(state: &mut State, path: &str, body: &Value) -> Reply {
    let mut users = Vec::new();
    if let Some(initial) = body["users"].as_array() {
        for value in initial {
            let Some(user) = value.as_str() else {
                return problem(400, "invalid-param", path);
            };
            if !state.resources.profiles.contains_key(user) {
                return problem(404, "not-found", path);
            }
            users.push(user.to_owned());
        }
    }
    let mut value = body.clone();
    value["id"] = state.operations.id().into();
    value["created_at"] = CREATED_AT.into();
    let Ok(role) = serde_json::from_value::<Role>(value) else {
        return problem(400, "invalid-param", path);
    };
    for user in users {
        if let Some(profile) = state.resources.profiles.get_mut(&user) {
            profile.roles.get_or_insert_default().push(RoleSummary {
                id: role.id.clone(),
                name: role.name.clone(),
                color: role.color.clone(),
            });
        }
    }
    state.resources.roles.push(role.clone());
    Reply::json(201, json!(role))
}

fn update_role(state: &mut State, path: &str, id: &str, body: &Value) -> Reply {
    let Some(role) = state
        .resources
        .roles
        .iter_mut()
        .find(|role| role.id.as_deref() == Some(id))
    else {
        return problem(404, "not-found", path);
    };
    role.name = body["name"].as_str().map(str::to_owned);
    if let Some(color) = body.get("color") {
        role.color = color.as_str().map(str::to_owned);
    }
    if let Some(permissions) = body.get("permissions") {
        let Ok(permissions) = serde_json::from_value(permissions.clone()) else {
            return problem(400, "invalid-param", path);
        };
        role.permissions = Some(permissions);
    }
    Reply::json(200, json!(role))
}

fn assign_role(state: &mut State, path: &str, user: &str, body: &Value) -> Reply {
    let Some(id) = body["role_id"].as_str() else {
        return problem(400, "invalid-param", path);
    };
    let Some(role) = state
        .resources
        .roles
        .iter()
        .find(|role| role.id.as_deref() == Some(id))
    else {
        return problem(404, "not-found", path);
    };
    let Some(profile) = state.resources.profiles.get_mut(user) else {
        return problem(404, "not-found", path);
    };
    let roles = profile.roles.get_or_insert_default();
    if !roles.iter().any(|role| role.id.as_deref() == Some(id)) {
        roles.push(RoleSummary {
            id: role.id.clone(),
            name: role.name.clone(),
            color: role.color.clone(),
        });
    }
    Reply::json(
        201,
        json!({"user_id":user,"role_id":id,"assigned_at":CREATED_AT}),
    )
}

fn overwrite(state: &mut State, path: &str, id: &str, role: &str, body: &Value) -> Reply {
    let Some(channel) = state.resources.channels.get_mut(id) else {
        return problem(404, "not-found", path);
    };
    let Ok(permissions) = serde_json::from_value(body["permissions"].clone()) else {
        return problem(400, "invalid-param", path);
    };
    let entries = channel.permissions.get_or_insert_default();
    entries.retain(|entry| entry.role_id.as_deref() != Some(role));
    entries.push(ChannelPermissionEntry {
        role_id: Some(role.into()),
        permissions: Some(permissions),
        ..ChannelPermissionEntry::default()
    });
    Reply::json(
        200,
        json!({"channel_id":id,"role_id":role,"permissions":body["permissions"]}),
    )
}
