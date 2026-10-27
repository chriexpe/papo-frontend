//! Targeted account appearance changes over the backend's full-config PUT contract.
use super::{Api, ApiError, ApiResult};
use crate::api::models::{UserConfig, UserSettings};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DisplayFontSize {
    Small,
    #[default]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MessageDensity {
    Compact,
    #[default]
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

/// Only fields changed by this client. Merge into a fresh raw config, never a cached projection.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AccountSettingsPatch {
    pub theme: Option<String>,
    pub notifications_enabled: Option<bool>,
    pub font_size: Option<DisplayFontSize>,
    pub message_density: Option<MessageDensity>,
}

impl AccountSettingsPatch {
    pub fn apply_to(&self, config: &mut UserConfig) {
        if let Some(value) = &self.theme {
            config.theme = value.clone();
        }
        if let Some(value) = self.notifications_enabled {
            config.notifications.enabled = value;
        }
        if let Some(value) = self.font_size {
            config.display.font_size = value.wire().into();
        }
        if let Some(value) = self.message_density {
            config.display.message_density = value.wire().into();
        }
    }

    pub fn merge(&mut self, newer: Self) {
        if newer.theme.is_some() {
            self.theme = newer.theme;
        }
        if newer.notifications_enabled.is_some() {
            self.notifications_enabled = newer.notifications_enabled;
        }
        if newer.font_size.is_some() {
            self.font_size = newer.font_size;
        }
        if newer.message_density.is_some() {
            self.message_density = newer.message_density;
        }
    }

    pub fn matches(&self, config: &UserConfig) -> bool {
        let mut desired = config.clone();
        self.apply_to(&mut desired);
        desired == *config
    }
}

fn incompatible() -> ApiError {
    ApiError::Decode("invalid_user_settings_config".into())
}

/// Fill only omitted defaults; retain every supplied field, including future protocol fields.
fn with_defaults(mut config: Value) -> ApiResult<Value> {
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
    Ok(config)
}

fn merged_config(config: Value, change: DisplayChange) -> ApiResult<(Value, bool)> {
    let mut config = with_defaults(config)?;
    let object = config.as_object_mut().ok_or_else(incompatible)?;
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

fn merged_patch(config: Value, patch: &AccountSettingsPatch) -> ApiResult<Value> {
    let mut config = with_defaults(config)?;
    if let Some(theme) = &patch.theme {
        if !matches!(theme.as_str(), "system" | "light" | "dark") {
            return Err(incompatible());
        }
        config["theme"] = json!(theme);
    }
    if let Some(enabled) = patch.notifications_enabled {
        config["notifications"]["enabled"] = json!(enabled);
    }
    if let Some(size) = patch.font_size {
        config["display"]["fontSize"] = json!(size.wire());
    }
    if let Some(density) = patch.message_density {
        config["display"]["messageDensity"] = json!(density.wire());
    }
    Ok(config)
}

impl Api {
    pub async fn patch_account_settings(
        &self,
        owner: &str,
        patch: &AccountSettingsPatch,
    ) -> ApiResult<UserSettings> {
        let who: Value = self.get("/auth/whoami").await?;
        if owner.is_empty() || who.get("id").and_then(Value::as_str) != Some(owner) {
            return Err(ApiError::Unauthorized);
        }
        let config = who
            .get("settings")
            .and_then(|s| s.get("config"))
            .cloned()
            .ok_or_else(incompatible)?;
        let config = merged_patch(config, patch)?;
        let acknowledged: UserSettings = self
            .put("/users/settings", &json!({"config":config}))
            .await?;
        if acknowledged.user_id != owner {
            return Err(ApiError::Unauthorized);
        }
        Ok(acknowledged)
    }

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
    async fn parity_wire_updates_read_fresh_preserve_fields_and_never_write_for_another_owner() {
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
        let mut patch_ack = density_ack.clone();
        patch_ack["theme"] = json!("light");
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
            ("GET /auth/whoami ", 200, who("owner", density_ack.clone())),
            ("PUT /users/settings ", 200, ack(patch_ack.clone())),
            ("GET /auth/whoami ", 200, who("other", density_ack.clone())),
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
        api.patch_account_settings(
            "owner",
            &AccountSettingsPatch {
                theme: Some("light".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            api.patch_account_settings("owner", &AccountSettingsPatch::default())
                .await,
            Err(ApiError::Unauthorized)
        ));
        let bodies = server.await.unwrap();
        assert_eq!(bodies[3], json!({"config":patch_ack}));
        assert_eq!(bodies[0], json!({"config":font_ack}));
        assert_eq!(bodies[1], json!({"config":density_ack}));
    }

    #[test]
    fn parity_portable_patches_preserve_fresh_appearance_and_unknown_fields() {
        let mut original = serde_json::to_value(UserConfig::default()).unwrap();
        original["future"] = json!({"client": "newer"});
        original["display"]["fontSize"] = json!("huge");
        original["display"]["messageDensity"] = json!("comfortable");
        original["display"]["future"] = json!([1, 2]);
        original["notifications"]["sound"] = json!(false);
        original["notifications"]["future"] = json!("retain");
        let patch = AccountSettingsPatch {
            theme: Some("light".into()),
            notifications_enabled: Some(false),
            ..Default::default()
        };
        let merged = merged_patch(original.clone(), &patch).unwrap();
        original["theme"] = json!("light");
        original["notifications"]["enabled"] = json!(false);
        assert_eq!(merged, original);
        let invalid = AccountSettingsPatch {
            theme: Some("translated-label".into()),
            ..Default::default()
        };
        assert!(merged_patch(original, &invalid).is_err());
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
