//! Owns cookie rotation, connection records and one-use reset links; sockets stay with the gateway.

#[cfg(test)]
mod tests;

use serde::Deserialize;
use serde_json::json;

use super::{Auth, CREATED_AT};
use crate::{
    Config,
    response::{Reply, problem},
};

#[derive(Deserialize)]
struct DropRequest {
    connection_id: String,
}

#[derive(Deserialize)]
struct ResetRequest {
    token: String,
    password: String,
}

impl Auth {
    pub(super) fn refresh(&mut self, path: &str, cookie: Option<&str>) -> Reply {
        let Some((token, user)) = cookie.and_then(|token| {
            self.sessions
                .get(token)
                .map(|user| (token.to_owned(), user.clone()))
        }) else {
            return problem(401, "unauthorized", path).clear_cookie();
        };
        let Some(connection) = self.connections.remove(&token) else {
            return problem(401, "unauthorized", path).clear_cookie();
        };
        self.sessions.remove(&token);
        let replacement = self.id();
        self.sessions.insert(replacement.clone(), user);
        self.connections
            .insert(replacement.clone(), connection.clone());
        Reply::json(200, json!({"connection": connection_record(&connection)}))
            .with_cookie(&replacement, 2_592_000)
    }

    pub(super) fn devices(&self, path: &str, cookie: Option<&str>) -> Reply {
        let Some(user) = self.user_id(cookie) else {
            return problem(401, "unauthorized", path);
        };
        let connections: Vec<_> = self
            .sessions
            .iter()
            .filter(|(_, owner)| owner.as_str() == user)
            .filter_map(|(token, _)| self.connections.get(token))
            .map(|id| connection_record(id))
            .collect();
        Reply::json(200, json!({"connections": connections}))
    }

    pub(super) fn drop_connection(
        &mut self,
        path: &str,
        cookie: Option<&str>,
        body: &[u8],
    ) -> Reply {
        let Some(user) = self.user_id(cookie).map(str::to_owned) else {
            return problem(401, "unauthorized", path);
        };
        let Ok(request) = serde_json::from_slice::<DropRequest>(body) else {
            return problem(400, "invalid-param", path);
        };
        let all = request.connection_id.eq_ignore_ascii_case("ALL");
        if !all && !valid_id(&request.connection_id) {
            return problem(400, "invalid-param", path);
        }
        let tokens: Vec<_> = self
            .sessions
            .iter()
            .filter(|(token, owner)| {
                **owner == user
                    && (all || self.connections.get(*token) == Some(&request.connection_id))
            })
            .map(|(token, _)| token.clone())
            .collect();
        if tokens.is_empty() && !all {
            return problem(404, "not-found", path);
        }
        let clear = cookie.is_some_and(|cookie| tokens.iter().any(|token| token == cookie));
        for token in &tokens {
            self.sessions.remove(token);
            self.connections.remove(token);
        }
        let reply = Reply::json(200, json!({"dropped": tokens.len()}));
        if clear { reply.clear_cookie() } else { reply }
    }

    pub(crate) fn issue_password_reset(&mut self, username: &str) -> Option<String> {
        let user = self.users.get(username)?.id.clone();
        let token = self.id();
        self.resets.insert(token.clone(), user);
        Some(token)
    }

    pub(super) fn password_reset(&mut self, config: &Config, path: &str, body: &[u8]) -> Reply {
        let Ok(request) = serde_json::from_slice::<ResetRequest>(body) else {
            return problem(400, "invalid-param", path);
        };
        if request.token.is_empty() || !valid_password(config, &request.password) {
            return problem(400, "invalid-param", path);
        }
        let Some(user_id) = self.resets.remove(&request.token) else {
            return problem(410, "reset-link-invalid", path);
        };
        let Some(user) = self.users.values_mut().find(|user| user.id == user_id) else {
            return problem(410, "reset-link-invalid", path);
        };
        user.password = request.password;
        let tokens: Vec<_> = self
            .sessions
            .iter()
            .filter(|(_, owner)| **owner == user_id)
            .map(|(token, _)| token.clone())
            .collect();
        for token in tokens {
            self.sessions.remove(&token);
            self.connections.remove(&token);
        }
        Reply::json(200, json!({"response": "Password updated successfully"}))
    }
}

fn connection_record(id: &str) -> serde_json::Value {
    json!({"id": id, "created_at": CREATED_AT, "expires_at": "2026-01-31T00:00:00Z"})
}

fn valid_password(config: &Config, password: &str) -> bool {
    let chars = password.chars().count();
    (config.min_password_chars..=config.max_password_chars).contains(&chars)
        && password.chars().any(char::is_uppercase)
        && password
            .chars()
            .any(|character| !character.is_alphanumeric())
}

fn valid_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}
