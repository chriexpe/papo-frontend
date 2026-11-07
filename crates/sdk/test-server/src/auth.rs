//! Owns deterministic registration, cookie sessions and discovery; leaves sockets and script timing outside.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Config,
    response::{Reply, problem},
};

const CREATED_AT: &str = "2026-01-01T00:00:00Z";

struct User {
    id: String,
    username: String,
    password: String,
}

pub(crate) struct Auth {
    users: BTreeMap<String, User>,
    sessions: BTreeMap<String, String>,
    temporary: Vec<String>,
    next_id: u64,
}

#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
}

impl Auth {
    pub(crate) fn new() -> Self {
        Self {
            users: BTreeMap::new(),
            sessions: BTreeMap::new(),
            temporary: Vec::new(),
            next_id: 1,
        }
    }

    fn id(&mut self) -> String {
        let value = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        format!("00000000-0000-4000-8000-{value:012x}")
    }

    pub(crate) fn user_id(&self, cookie: Option<&str>) -> Option<&str> {
        self.sessions.get(cookie?).map(String::as_str)
    }

    pub(crate) fn handle(
        &mut self,
        config: &Config,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: &[u8],
    ) -> Reply {
        match (method, path) {
            ("GET", "/server") => self.discovery(config),
            ("POST", "/auth/register" | "/auth/login") => {
                self.credentials(config, path, cookie, body)
            }
            ("POST", "/auth/login_server") => self.login_server(config, path, body),
            ("GET", "/auth/whoami") => self.whoami(path, cookie),
            ("POST", "/auth/logout") => {
                if let Some(cookie) = cookie {
                    self.sessions.remove(cookie);
                }
                Reply::empty().clear_cookie()
            }
            _ => problem(404, "not-found", path),
        }
    }

    fn discovery(&self, config: &Config) -> Reply {
        if !config.server_exists {
            return problem(404, "not-found", "/server");
        }
        Reply::json(
            200,
            json!({
                "id": "00000000-0000-4000-8000-000000000000", "name": config.server_name,
                "icon_blob": null, "icon_format": "", "owner_id": null, "public": config.public,
                "owner_username": null, "created_at": CREATED_AT, "role_count": 0,
                "member_count": self.users.len(), "channel_count": 0
            }),
        )
    }

    fn credentials(
        &mut self,
        config: &Config,
        path: &str,
        cookie: Option<&str>,
        body: &[u8],
    ) -> Reply {
        let Ok(request) = serde_json::from_slice::<Credentials>(body) else {
            return problem(400, "invalid-param", path);
        };
        let username_chars = request.username.chars().count();
        let password_chars = request.password.chars().count();
        let register = path == "/auth/register";
        if username_chars == 0
            || username_chars > config.max_username_chars
            || password_chars == 0
            || password_chars > config.max_password_chars
            || (register && username_chars < 3)
        {
            return problem(400, "invalid-param", path);
        }
        if config.server_exists
            && !config.public
            && !cookie.is_some_and(|value| self.temporary.iter().any(|token| token == value))
        {
            return problem(401, "server-access-required", path);
        }
        if register {
            if password_chars < config.min_password_chars
                || !request.password.chars().any(char::is_uppercase)
                || !request.password.chars().any(|c| !c.is_alphanumeric())
            {
                return problem(400, "invalid-param", path);
            }
            if self.users.contains_key(&request.username) {
                return problem(409, "username-taken", path);
            }
            let id = self.id();
            let reply = Reply::json(
                201,
                json!({"id": id, "username": request.username, "created_at": CREATED_AT}),
            );
            self.users.insert(
                request.username.clone(),
                User {
                    id,
                    username: request.username,
                    password: request.password,
                },
            );
            reply
        } else {
            let Some(user) = self
                .users
                .get(&request.username)
                .filter(|user| user.password == request.password)
            else {
                return problem(401, "invalid-credentials", path);
            };
            let id = user.id.clone();
            let reply = Reply::json(
                200,
                json!({"user": {"id": id, "username": user.username}, "connection_violation": false}),
            );
            let token = self.id();
            self.sessions.insert(token.clone(), id);
            reply.with_cookie(&token, 2_592_000)
        }
    }

    fn login_server(&mut self, config: &Config, path: &str, body: &[u8]) -> Reply {
        let Ok(value) = serde_json::from_slice::<Value>(body) else {
            return problem(400, "invalid-param", path);
        };
        let Some(password) = value.get("server_password").and_then(Value::as_str) else {
            return problem(400, "invalid-param", path);
        };
        if password.is_empty() || password.chars().count() > config.max_password_chars {
            return problem(400, "invalid-param", path);
        }
        if !config.server_exists {
            return problem(404, "not-found", path);
        }
        if !config.public && config.server_password.as_deref() != Some(password) {
            return problem(401, "invalid-credentials", path);
        }
        let token = self.id();
        let reply = Reply::empty().with_cookie(&token, 1_800);
        self.temporary.push(token);
        reply
    }

    fn whoami(&self, path: &str, cookie: Option<&str>) -> Reply {
        let Some(user) = self
            .user_id(cookie)
            .and_then(|id| self.users.values().find(|user| user.id == id))
        else {
            return problem(401, "unauthorized", path);
        };
        Reply::json(
            200,
            json!({
                "id": user.id, "username": user.username, "nickname": null, "avatar_blob": null,
                "avatar_format": "", "status": null, "status_message": null, "typing": null,
                "status_updated_at": null, "created_at": CREATED_AT, "roles": [],
                "settings": {"version": 1, "config": {
                "theme": "system",
                "notifications": {"enabled": true, "messagePreview": true, "sound": true, "mentions": true},
                "display": {"fontSize": "medium", "messageDensity": "normal", "showTimestamps": true, "showAvatars": true}
            }}, "connection_violation": false
            }),
        )
    }
}

pub(crate) fn cookie_value(headers: &hyper::HeaderMap) -> Option<&str> {
    headers
        .get_all(hyper::header::COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == "Auth" && !value.is_empty()).then_some(value)
        })
}

#[cfg(test)]
mod tests {
    //! Owns clock-free handler lifecycle tests; leaves cookie transport to integration tests.

    use super::*;

    fn credentials() -> Vec<u8> {
        let password: String = std::iter::once('A')
            .chain((0_u8..16).map(|index| char::from(b'a' + index)))
            .chain(std::iter::once('!'))
            .collect();
        serde_json::to_vec(&json!({"username": "tester", "password": password})).unwrap()
    }

    #[test]
    fn registration_does_not_create_a_session() {
        let mut auth = Auth::new();
        let reply = auth.handle(
            &Config::default(),
            "POST",
            "/auth/register",
            None,
            &credentials(),
        );
        assert_eq!(
            (reply.status, reply.cookie.is_none(), auth.sessions.len()),
            (201, true, 0)
        );
    }

    #[test]
    fn duplicate_registration_is_a_conflict() {
        let mut auth = Auth::new();
        auth.handle(
            &Config::default(),
            "POST",
            "/auth/register",
            None,
            &credentials(),
        );
        assert_eq!(
            auth.handle(
                &Config::default(),
                "POST",
                "/auth/register",
                None,
                &credentials()
            )
            .status,
            409
        );
    }

    #[test]
    fn private_server_requires_temporary_not_user_authorization() {
        let mut auth = Auth::new();
        let config = Config {
            public: false,
            ..Config::default()
        };
        assert_eq!(
            auth.handle(&config, "POST", "/auth/register", None, &credentials())
                .status,
            401
        );
    }

    #[test]
    fn unknown_user_has_invalid_credentials_problem() {
        let mut auth = Auth::new();
        let reply = auth.handle(
            &Config::default(),
            "POST",
            "/auth/login",
            None,
            &credentials(),
        );
        assert_eq!(reply.body.unwrap()["type"], "/errors/invalid-credentials");
    }

    #[test]
    fn malformed_json_is_rejected_before_private_server_gate() {
        let mut auth = Auth::new();
        let config = Config {
            public: false,
            ..Config::default()
        };
        assert_eq!(
            auth.handle(&config, "POST", "/auth/login", None, b"{")
                .status,
            400
        );
    }

    #[test]
    fn discovery_is_public_and_tracks_registered_member_count() {
        let mut auth = Auth::new();
        auth.handle(
            &Config::default(),
            "POST",
            "/auth/register",
            None,
            &credentials(),
        );
        let reply = auth.handle(&Config::default(), "GET", "/server", None, b"");
        assert_eq!(reply.body.unwrap()["member_count"], 1);
    }

    #[test]
    fn absent_server_discovery_returns_not_found() {
        let config = Config {
            server_exists: false,
            ..Config::default()
        };
        assert_eq!(
            Auth::new()
                .handle(&config, "GET", "/server", None, b"")
                .status,
            404
        );
    }
}
