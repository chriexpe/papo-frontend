//! Targeted account appearance changes over the backend's full-config PUT contract.
use super::{Api, ApiError, ApiResult};
use crate::api::models::{UserConfig, UserSettings};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayFontSize {
    Small,
    Normal,
    Large,
}
impl DisplayFontSize {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "small" => Some(Self::Small),
            "medium" => Some(Self::Normal),
            "huge" => Some(Self::Large),
            _ => None,
        }
    }
    pub fn wire(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Normal => "medium",
            Self::Large => "huge",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageDensity {
    Compact,
    Normal,
    Comfortable,
}
impl MessageDensity {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "compact" => Some(Self::Compact),
            "normal" => Some(Self::Normal),
            "comfortable" => Some(Self::Comfortable),
            _ => None,
        }
    }
    pub fn wire(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Normal => "normal",
            Self::Comfortable => "comfortable",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum DisplayChange {
    FontSize(DisplayFontSize),
    MessageDensity(MessageDensity),
    /// Import the former device switch only when the account still uses normal density.
    ImportLegacyCompact,
}

fn incompatible() -> ApiError {
    ApiError::Decode("invalid_user_settings_config".into())
}

/// Fill only omitted defaults; retain every supplied field, including future protocol fields.
fn merged_config(mut config: Value, change: DisplayChange) -> ApiResult<(Value, bool)> {
    let defaults = serde_json::to_value(UserConfig::default()).map_err(|_| incompatible())?;
    let object = config.as_object_mut().ok_or_else(incompatible)?;
    for (key, value) in defaults.as_object().ok_or_else(incompatible)? {
        object.entry(key.clone()).or_insert_with(|| value.clone());
    }
    for section in ["notifications", "display"] {
        let fields = object
            .get_mut(section)
            .and_then(Value::as_object_mut)
            .ok_or_else(incompatible)?;
        for (key, value) in defaults[section].as_object().ok_or_else(incompatible)? {
            fields.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    let display = object
        .get_mut("display")
        .and_then(Value::as_object_mut)
        .ok_or_else(incompatible)?;
    let (key, value) = match change {
        DisplayChange::FontSize(value) => ("fontSize", value.wire()),
        DisplayChange::MessageDensity(value) => ("messageDensity", value.wire()),
        DisplayChange::ImportLegacyCompact => {
            if display.get("messageDensity").and_then(Value::as_str) != Some("normal") {
                return Ok((config, false));
            }
            ("messageDensity", "compact")
        }
    };
    display.insert(key.into(), Value::String(value.into()));
    Ok((config, true))
}

impl Api {
    /// A fresh read prevents an unrelated change made by another client from being overwritten
    /// by our stale typed projection. The backend has no conditional/partial settings update.
    pub async fn account_display_settings(
        &self,
        owner: &str,
        change: Option<DisplayChange>,
    ) -> ApiResult<UserSettings> {
        let who: Value = self.get("/auth/whoami").await?;
        if owner.is_empty() || who.get("id").and_then(Value::as_str) != Some(owner) {
            return Err(ApiError::Unauthorized);
        }
        let mut settings = who.get("settings").cloned().ok_or_else(incompatible)?;
        settings
            .as_object_mut()
            .ok_or_else(incompatible)?
            .insert("user_id".into(), json!(owner));
        if let Some(change) = change {
            let config = settings.get("config").cloned().ok_or_else(incompatible)?;
            let (config, write) = merged_config(config, change)?;
            if write {
                let acknowledged: UserSettings = self
                    .put("/users/settings", &json!({"config":config}))
                    .await?;
                if acknowledged.user_id != owner {
                    return Err(ApiError::Unauthorized);
                }
                return Ok(acknowledged);
            }
        }
        serde_json::from_value(settings).map_err(|_| incompatible())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wire_updates_read_fresh_preserve_fields_and_never_write_for_another_owner() {
        use super::super::Session;
        use std::sync::Arc;
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let mut first = serde_json::to_value(UserConfig::default()).unwrap();
        first["theme"] = json!("dark");
        first["notifications"]["sound"] = json!(false);
        first["future"] = json!({"keep":[1,2]});
        first["display"]["future"] = json!("untouched");
        let mut font_ack = first.clone();
        font_ack["display"]["fontSize"] = json!("huge");
        let mut remote = first.clone();
        remote["display"]["fontSize"] = json!("small");
        let mut density_ack = remote.clone();
        density_ack["display"]["messageDensity"] = json!("comfortable");
        let who = |owner: &str, config: Value| json!({"id":owner,"settings":{"version":1,"config":config}});
        let ack = |config: Value| json!({"user_id":"owner","version":1,"config":config});
        let script = vec![
            ("GET /auth/whoami ", 200, who("owner", first)),
            ("PUT /users/settings ", 200, ack(font_ack.clone())),
            ("GET /auth/whoami ", 200, who("owner", remote.clone())),
            ("PUT /users/settings ", 200, ack(density_ack.clone())),
            ("GET /auth/whoami ", 200, who("owner", remote)),
            (
                "PUT /users/settings ",
                403,
                json!({"status":403,"detail":"rejected"}),
            ),
            ("GET /auth/whoami ", 200, who("other", font_ack.clone())),
            ("GET /auth/whoami ", 200, who("owner", density_ack.clone())),
        ];
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut bodies = Vec::new();
            for (expected, status, response) in script {
                let (stream, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                assert!(line.starts_with(expected), "{line}");
                let mut length = 0;
                let mut authenticated = false;
                loop {
                    line.clear();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    let header = line.to_ascii_lowercase();
                    if let Some(value) = header.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    if header.starts_with("cookie:") {
                        authenticated = line.contains("appearance-fixture");
                    }
                }
                assert!(authenticated);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.unwrap();
                if expected.starts_with("PUT") {
                    bodies.push(serde_json::from_slice::<Value>(&body).unwrap());
                }
                let response = response.to_string();
                reader.get_mut().write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
            }
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(300), listener.accept())
                    .await
                    .is_err()
            );
            bodies
        });
        let session = Arc::new(Session::default());
        session.set_token(Some("appearance-fixture".into()));
        let api = Api::new(&url, session).unwrap();
        let font = api
            .account_display_settings(
                "owner",
                Some(DisplayChange::FontSize(DisplayFontSize::Large)),
            )
            .await
            .unwrap();
        assert_eq!(font.config.display.font_size, "huge");
        let density = api
            .account_display_settings(
                "owner",
                Some(DisplayChange::MessageDensity(MessageDensity::Comfortable)),
            )
            .await
            .unwrap();
        assert_eq!(density.config.display.font_size, "small");
        assert_eq!(density.config.display.message_density, "comfortable");
        assert!(matches!(
            api.account_display_settings(
                "owner",
                Some(DisplayChange::MessageDensity(MessageDensity::Compact))
            )
            .await,
            Err(ApiError::Forbidden(_))
        ));
        assert!(matches!(
            api.account_display_settings(
                "owner",
                Some(DisplayChange::FontSize(DisplayFontSize::Large))
            )
            .await,
            Err(ApiError::Unauthorized)
        ));
        assert_eq!(
            api.account_display_settings("owner", Some(DisplayChange::ImportLegacyCompact))
                .await
                .unwrap()
                .config
                .display
                .message_density,
            "comfortable"
        );
        let bodies = server.await.unwrap();
        assert_eq!(bodies[0], json!({"config":font_ack}));
        assert_eq!(bodies[1], json!({"config":density_ack}));
    }

    #[test]
    fn edits_preserve_unknown_fields_and_unrelated_values() {
        let mut original = serde_json::to_value(UserConfig::default()).unwrap();
        original["theme"] = json!("dark");
        original["future"] = json!({"enabled":true});
        original["notifications"]["sound"] = json!(false);
        original["notifications"]["future"] = json!([1, 2]);
        original["display"]["messageDensity"] = json!("future-density");
        original["display"]["showAvatars"] = json!(false);
        original["display"]["future"] = json!("keep");
        let (changed, write) = merged_config(
            original.clone(),
            DisplayChange::FontSize(DisplayFontSize::Large),
        )
        .unwrap();
        assert!(write);
        original["display"]["fontSize"] = json!("huge");
        assert_eq!(changed, original);
    }

    #[test]
    fn legacy_import_is_conditional_and_never_mixes_accounts_defaults() {
        for density in ["compact", "normal", "comfortable", "future-density"] {
            let mut original = serde_json::to_value(UserConfig::default()).unwrap();
            original["display"]["messageDensity"] = json!(density);
            let (changed, write) =
                merged_config(original.clone(), DisplayChange::ImportLegacyCompact).unwrap();
            assert_eq!(write, density == "normal");
            if write {
                original["display"]["messageDensity"] = json!("compact");
            }
            assert_eq!(changed, original);
        }
        assert!(
            merged_config(json!({"display":null}), DisplayChange::ImportLegacyCompact).is_err()
        );
        assert!(merged_config(json!(null), DisplayChange::ImportLegacyCompact).is_err());
    }
}
