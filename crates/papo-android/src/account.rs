//! Awaitable account operations. Unlike queued chat commands, success here means
//! the canonical core API has acknowledged the mutation. No foreign HTTP stack.
use super::*;
use crate::bindings::{Backend, CoreError, NativeClient};
use papo_core::{
    api::{
        client::{Api, Session},
        models,
    },
    state::Store,
};

#[derive(Clone, Debug, uniffi::Record)]
pub struct AccountProfile {
    pub nickname: String,
    pub username: String,
    pub status_message: String,
    pub description: String,
    pub typing: String,
    pub presence: String,
}
impl From<&models::UserProfile> for AccountProfile {
    fn from(p: &models::UserProfile) -> Self {
        Self {
            nickname: p.display_name().into(),
            username: p.username.clone(),
            status_message: p.status_message.clone().unwrap_or_default(),
            description: p.description.clone().unwrap_or_default(),
            typing: p.typing.clone().unwrap_or_default(),
            presence: p.status.clone().unwrap_or_else(|| "online".into()),
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct SessionView {
    pub id: String,
    pub created_at: String,
    pub expires_at: String,
}
#[derive(Debug, uniffi::Enum)]
pub enum AccountChange {
    Profile {
        nickname: String,
        status_message: String,
        description: String,
        typing: String,
    },
    Presence {
        value: String,
    },
    Avatar {
        blob: String,
        format: String,
    },
    Banner {
        blob: String,
        format: String,
    },
    Password {
        value: String,
    },
    RevokeSession {
        id: String,
    },
}
impl AccountChange {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Profile { nickname, .. } if nickname.trim().is_empty() => {
                Err("Enter a display name".into())
            }
            Self::Presence { value } if !matches!(value.as_str(), "online" | "away" | "busy") => {
                Err("Choose a supported presence".into())
            }
            Self::Password { value } if value.chars().count() < 8 => {
                Err("Use at least 8 characters".into())
            }
            Self::RevokeSession { id } if id.is_empty() || id == "ALL" => {
                Err("Choose a session to end".into())
            }
            Self::Avatar { blob, format } | Self::Banner { blob, format }
                if blob.len() > 12 * 1024 * 1024
                    || (!blob.is_empty()
                        && !matches!(format.as_str(), "PNG" | "JPEG" | "GIF" | "WEBP")) =>
            {
                Err("Choose a supported image up to 8 MB".into())
            }
            _ => Ok(()),
        }
    }
}
struct AccountApi {
    api: Api,
    owner: String,
    token: Option<String>,
}
impl NativeClient {
    fn account_api(&self, expected_owner: &str) -> Result<AccountApi, CoreError> {
        let backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        let Backend::Live(c) = backend.as_ref().ok_or("Client is closed".to_owned())? else {
            return Err("Demo has no authenticated account".to_owned().into());
        };
        if c.runtime.store.me != expected_owner || expected_owner.is_empty() {
            return Err("Sign in first".to_owned().into());
        }
        let token = c.runtime.net.session.token();
        if token.is_none() {
            return Err("Sign in first".to_owned().into());
        }
        // Freeze credentials so an in-flight request can never borrow the next
        // account's login. Reconcile only if this owner and token still match.
        let session = Arc::new(Session::default());
        session.set_token(token.clone());
        let api = Api::new(&c.runtime.url, session).map_err(|e| e.to_string())?;
        Ok(AccountApi {
            api,
            owner: c.runtime.store.me.clone(),
            token,
        })
    }
    fn finish_account(
        &self,
        request: &AccountApi,
        profile: Option<models::UserProfile>,
    ) -> Result<(), CoreError> {
        let mut backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        let Backend::Live(c) = backend.as_mut().ok_or("Client is closed".to_owned())? else {
            return Err("Account changed".to_owned().into());
        };
        if c.runtime.store.me != request.owner || c.runtime.net.session.token() != request.token {
            return Err("Account changed while the request was running"
                .to_owned()
                .into());
        }
        c.runtime.net.session.set_token(request.api.session.token());
        if let Some(profile) = profile {
            c.runtime.store.apply(Update::Profiles(vec![profile]));
        }
        Ok(())
    }
}
fn run<T>(
    future: impl std::future::Future<Output = Result<T, papo_core::api::client::ApiError>>,
) -> Result<T, CoreError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(future)
        .map_err(|e| e.to_string().into())
}

#[uniffi::export]
impl NativeClient {
    pub fn account_profile(&self, owner: String) -> Result<AccountProfile, CoreError> {
        {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            if let Some(Backend::Demo(c)) = backend.as_ref() {
                if c.store.me != owner {
                    return Err("Account changed".to_owned().into());
                }
                return Ok(AccountProfile::from(&demo_profile(&c.store)));
            }
        }
        let request = self.account_api(&owner)?;
        let profile = run(request.api.profile(&request.owner))?;
        let view = AccountProfile::from(&profile);
        self.finish_account(&request, Some(profile))?;
        Ok(view)
    }
    pub fn account_change(&self, owner: String, change: AccountChange) -> Result<(), CoreError> {
        change.validate()?;
        {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            if let Some(Backend::Demo(c)) = backend.as_mut() {
                if c.store.me != owner {
                    return Err("Account changed".to_owned().into());
                }
                return demo_change(&mut c.store, change).map_err(Into::into);
            }
        }
        let request = self.account_api(&owner)?;
        run(apply_change(&request.api, &request.owner, change))?;
        // A subsequent profile refresh is a separate read. Its failure must not
        // turn an acknowledged password/profile mutation into a failed save.
        self.finish_account(&request, None)
    }
    pub fn account_sessions(&self, owner: String) -> Result<Vec<SessionView>, CoreError> {
        {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            if let Some(Backend::Demo(c)) = backend.as_ref() {
                if c.store.me != owner {
                    return Err("Account changed".to_owned().into());
                }
                return Ok(vec![]);
            }
        }
        let request = self.account_api(&owner)?;
        let sessions = run(request.api.connected_devices())?;
        self.finish_account(&request, None)?;
        Ok(sessions
            .into_iter()
            .map(|s| SessionView {
                id: s.id,
                created_at: s.created_at.map(|v| v.to_rfc3339()).unwrap_or_default(),
                expires_at: s.expires_at.map(|v| v.to_rfc3339()).unwrap_or_default(),
            })
            .collect())
    }
}
async fn apply_change(
    api: &Api,
    owner: &str,
    change: AccountChange,
) -> Result<(), papo_core::api::client::ApiError> {
    match change {
        AccountChange::Profile {
            nickname,
            status_message,
            description,
            typing,
        } => {
            api.update_profile(
                owner,
                &models::UpdateUserRequest {
                    nickname,
                    status: status_message,
                    description,
                    typing: Some(typing),
                },
            )
            .await?;
        }
        AccountChange::Presence { value } => {
            api.set_status(owner, (value != "online").then_some(value.as_str()))
                .await?;
        }
        AccountChange::Avatar { blob, format } => {
            api.set_avatar(owner, &blob, &format).await?;
        }
        AccountChange::Banner { blob, format } => {
            api.set_banner(owner, &blob, &format).await?;
        }
        AccountChange::Password { value } => {
            api.change_password(owner, &value).await?;
        }
        AccountChange::RevokeSession { id } => {
            api.drop_connection(&id).await?;
        }
    }
    Ok(())
}
fn demo_profile(s: &Store) -> models::UserProfile {
    let member = s.member(&s.me).expect("demo owner exists");
    let details = s.profiles.get(&s.me);
    models::UserProfile {
        id: s.me.clone(),
        username: member.username.clone(),
        nickname: Some(member.name.clone()),
        avatar_blob: s.avatars.get(&s.me).cloned(),
        avatar_format: None,
        banner_media: details.and_then(|p| p.banner.clone()),
        description: details.and_then(|p| p.description.clone()),
        status: Some(
            match member.presence {
                papo_core::state::Presence::Away => "away",
                papo_core::state::Presence::Busy => "busy",
                _ => "online",
            }
            .into(),
        ),
        status_message: member.status_message.clone(),
        typing: member.typing_label.clone(),
        created_at: details.and_then(|p| p.created_at).map(|at| at.to_rfc3339()),
        roles: details.map(|p| p.roles.clone()).unwrap_or_default(),
    }
}
fn demo_change(s: &mut Store, change: AccountChange) -> Result<(), String> {
    let mut p = demo_profile(s);
    match change {
        AccountChange::Profile {
            nickname,
            status_message,
            description,
            typing,
        } => {
            p.nickname = Some(nickname);
            p.status_message = Some(status_message);
            p.description = Some(description);
            p.typing = Some(typing);
        }
        AccountChange::Presence { value } => {
            if let Some(member) = s.members.iter_mut().find(|m| m.id == s.me) {
                member.presence = match value.as_str() {
                    "away" => papo_core::state::Presence::Away,
                    "busy" => papo_core::state::Presence::Busy,
                    _ => papo_core::state::Presence::Online,
                };
            }
            p.status = Some(value);
        }
        AccountChange::Avatar { blob, format } => {
            p.avatar_blob = Some(blob);
            p.avatar_format = Some(format);
        }
        AccountChange::Banner { blob, .. } if blob.is_empty() => p.banner_media = None,
        AccountChange::Banner { .. } => return Err("Banner uploads need a connected server".into()),
        AccountChange::Password { .. } | AccountChange::RevokeSession { .. } => {
            return Err("Demo has no authenticated account".into());
        }
    }
    s.apply(Update::Profiles(vec![p]));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_edits_preserve_unedited_fields_and_reduce_into_chat_identity() {
        let client = NativeClient::demo();
        let before = client.account_profile("me".into()).unwrap();
        client
            .account_change(
                "me".into(),
                AccountChange::Profile {
                    nickname: "Updated".into(),
                    status_message: "Hello".into(),
                    description: before.description.clone(),
                    typing: "building".into(),
                },
            )
            .unwrap();
        let after = client.account_profile("me".into()).unwrap();
        assert_eq!(after.nickname, "Updated");
        assert_eq!(after.description, before.description);
        assert_eq!(after.typing, "building");
        assert_eq!(after.status_message, "Hello");
        assert_eq!(client.poll().unwrap().my_name, "Updated");
        client
            .account_change(
                "me".into(),
                AccountChange::Presence {
                    value: "busy".into(),
                },
            )
            .unwrap();
        assert_eq!(
            client.account_profile("me".into()).unwrap().presence,
            "busy"
        );
    }
    #[test]
    fn rejects_unsupported_presence_short_password_and_bulk_revoke() {
        for change in [
            AccountChange::Presence {
                value: "invisible".into(),
            },
            AccountChange::Password {
                value: "short".into(),
            },
            AccountChange::RevokeSession { id: "ALL".into() },
        ] {
            assert!(change.validate().is_err());
        }
    }
    #[test]
    fn stale_owner_cannot_read_or_mutate_another_account() {
        let client = NativeClient::demo();
        assert!(client.account_profile("other".into()).is_err());
        assert!(
            client
                .account_change(
                    "other".into(),
                    AccountChange::Presence {
                        value: "busy".into()
                    }
                )
                .is_err()
        );
        assert_eq!(
            client.account_profile("me".into()).unwrap().presence,
            "online"
        );
    }

    #[test]
    fn live_account_operations_use_core_routes_and_acknowledge_http_failures() {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let script = [
                ("PUT", "/users/owner", 200, r#"{}"#),
                ("PUT", "/users/owner/status", 200, r#"{}"#),
                ("PUT", "/users/owner/avatar", 200, r#"{}"#),
                ("PUT", "/users/owner/banner", 200, r#"{}"#),
                ("POST", "/users/owner/reset", 200, r#"{}"#),
                ("PUT", "/users/owner/password", 200, r#"{}"#),
                (
                    "GET",
                    "/auth/connected_devices",
                    200,
                    r#"{"connections":[{"id":"opaque-session","created_at":"2026-10-21T12:00:00Z","expires_at":null}]}"#,
                ),
                ("POST", "/auth/drop_connection", 200, r#"{}"#),
                (
                    "PUT",
                    "/users/owner",
                    403,
                    r#"{"status":403,"detail":"profile rejected"}"#,
                ),
            ];
            let mut bodies = vec![];
            for (method, path, status, response) in script {
                let until = std::time::Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(std::time::Instant::now() < until, "missing {method} {path}");
                    std::thread::sleep(Duration::from_millis(2));
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with(&format!("{method} {path} ")), "{line}");
                let mut length = 0;
                let mut authenticated = false;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((key, value)) = line.split_once(':') {
                        if key.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                        if key.eq_ignore_ascii_case("cookie") {
                            authenticated = value.trim() == "Auth=fixture";
                        }
                    }
                }
                assert!(authenticated, "core API must use this account's cookie");
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                bodies.push(serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null));
                write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
            }
            bodies
        });
        let session = Arc::new(Session::default());
        session.set_token(Some("fixture".into()));
        let api = Api::new(&url, session).unwrap();
        let profile = || AccountChange::Profile {
            nickname: "A name".into(),
            status_message: "A status".into(),
            description: "Keep my about".into(),
            typing: "building…".into(),
        };
        for change in [
            profile(),
            AccountChange::Presence {
                value: "online".into(),
            },
            AccountChange::Avatar {
                blob: "encoded".into(),
                format: "PNG".into(),
            },
            AccountChange::Banner {
                blob: "".into(),
                format: "".into(),
            },
            AccountChange::Password {
                value: "new-password".into(),
            },
        ] {
            run(apply_change(&api, "owner", change)).unwrap();
        }
        let sessions = run(api.connected_devices()).unwrap();
        assert_eq!(sessions[0].id, "opaque-session");
        run(apply_change(
            &api,
            "owner",
            AccountChange::RevokeSession {
                id: sessions[0].id.clone(),
            },
        ))
        .unwrap();
        assert!(run(apply_change(&api, "owner", profile())).is_err());
        let bodies = worker.join().unwrap();
        assert_eq!(
            bodies[0],
            json!({"nickname":"A name","status":"A status","description":"Keep my about","typing":"building…"})
        );
        assert_eq!(bodies[1], json!({"status":null}));
        assert_eq!(bodies[2], json!({"avatar":"encoded","avatar_format":"PNG"}));
        assert_eq!(bodies[3], json!({"banner":"","banner_format":""}));
        assert_eq!(bodies[5], json!({"password":"new-password"}));
        assert_eq!(bodies[7], json!({"connection_id":"opaque-session"}));
    }
}
