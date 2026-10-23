//! Import the existing official Android app's private persistence. Secrets are
//! written directly into Android Keystore-backed storage, never returned to UI.
use super::*;
use crate::bindings::{CoreError, Credentials};

#[derive(Default, Deserialize)]
enum Theme {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Deserialize)]
struct Server {
    url: String,
}
#[derive(Deserialize)]
#[serde(default)]
struct Settings {
    servers: Vec<Server>,
    server_url: String,
    active: usize,
    theme: Theme,
    translucency: bool,
    notifications: bool,
    open_at_newest: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            servers: vec![],
            server_url: String::new(),
            active: 0,
            theme: Theme::System,
            translucency: true,
            notifications: true,
            open_at_newest: true,
        }
    }
}
#[derive(Debug, uniffi::Record)]
pub struct LegacyImport {
    pub servers: Vec<String>,
    pub active: String,
    pub theme: String,
    pub glass: bool,
    pub notifications: bool,
    pub open_newest: bool,
}
#[uniffi::export]
pub fn import_legacy_android(
    root: String,
    credentials: Arc<dyn Credentials>,
) -> Result<Option<LegacyImport>, CoreError> {
    let data = PathBuf::from(root).join("data");
    let path = data.join("app.ron");
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|_| "Could not read previous Android settings".to_owned())?;
    let outer: HashMap<String, String> =
        ron::from_str(&raw).map_err(|_| "Could not decode previous Android settings".to_owned())?;
    let Some(raw) = outer.get("app") else {
        return Ok(None);
    };
    let settings: Settings =
        ron::from_str(raw).map_err(|_| "Could not import previous Android settings".to_owned())?;
    let mut urls = settings
        .servers
        .iter()
        .filter_map(|s| normalize_url(&s.url).ok())
        .collect::<Vec<_>>();
    // The old single-server field can contain an untouched factory default.
    // Import it only if this installation really has a session for it.
    if urls.is_empty()
        && let Ok(url) = normalize_url(&settings.server_url)
        && data
            .join("sessions")
            .join(format!("{}.token", papo_core::server_key(&url)))
            .is_file()
    {
        urls.push(url)
    }
    let active = urls.get(settings.active).cloned().unwrap_or_default();
    let mut seen = HashSet::new();
    urls.retain(|u| seen.insert(u.clone()));
    for url in &urls {
        let key = papo_core::server_key(url);
        for secret in ["token", "server"] {
            if credentials.load(key.clone(), secret.into())?.is_some() {
                continue;
            }
            let path = data.join("sessions").join(format!("{key}.{secret}"));
            let value = match std::fs::read_to_string(path) {
                Ok(value) => value,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    return Err("Could not import a previous Android session"
                        .to_owned()
                        .into());
                }
            };
            if !value.trim().is_empty() {
                credentials.store(key.clone(), secret.into(), value.trim().into())?
            }
        }
    }
    Ok(Some(LegacyImport {
        servers: urls,
        active,
        theme: match settings.theme {
            Theme::Light => "Light",
            Theme::Dark => "Dark",
            Theme::System => "System",
        }
        .into(),
        glass: settings.translucency,
        notifications: settings.notifications,
        open_newest: settings.open_at_newest,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Secrets(Mutex<HashMap<(String, String), String>>);
    impl Credentials for Secrets {
        fn load(&self, server: String, secret: String) -> Result<Option<String>, CoreError> {
            Ok(self.0.lock().unwrap().get(&(server, secret)).cloned())
        }
        fn store(&self, server: String, secret: String, value: String) -> Result<(), CoreError> {
            self.0.lock().unwrap().insert((server, secret), value);
            Ok(())
        }
        fn remove(&self, server: String, secret: String) -> Result<(), CoreError> {
            self.0.lock().unwrap().remove(&(server, secret));
            Ok(())
        }
    }
    #[test]
    fn official_upgrade_preserves_servers_preferences_and_sessions_without_overwriting_rotated_credentials()
     {
        let root = std::env::temp_dir().join(format!("papo-upgrade-{}", now_millis()));
        let data = root.join("data");
        std::fs::create_dir_all(data.join("sessions")).unwrap();
        let url = "https://papo.example";
        let key = papo_core::server_key(url);
        let raw = r##"(servers:[(url:"https://papo.example",label:"Papo")],active:0,theme:Dark,translucency:false,notifications:false,open_at_newest:false,lang:PtBr)"##;
        let outer = HashMap::from([("app", raw)]);
        std::fs::write(data.join("app.ron"), ron::to_string(&outer).unwrap()).unwrap();
        std::fs::write(
            data.join("sessions").join(format!("{key}.token")),
            "old-session",
        )
        .unwrap();
        std::fs::write(
            data.join("sessions").join(format!("{key}.server")),
            "server-password",
        )
        .unwrap();
        let secrets = Arc::new(Secrets::default());
        let imported = import_legacy_android(root.display().to_string(), secrets.clone())
            .unwrap()
            .unwrap();
        assert_eq!(imported.servers, vec![url]);
        assert_eq!(imported.active, url);
        assert_eq!(imported.theme, "Dark");
        assert!(!imported.glass);
        assert!(!imported.notifications);
        assert!(!imported.open_newest);
        assert_eq!(
            secrets
                .load(key.clone(), "token".into())
                .unwrap()
                .as_deref(),
            Some("old-session")
        );
        secrets
            .store(key.clone(), "token".into(), "new-session".into())
            .unwrap();
        import_legacy_android(root.display().to_string(), secrets.clone()).unwrap();
        assert_eq!(
            secrets.load(key, "token".into()).unwrap().as_deref(),
            Some("new-session")
        );
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn empty_previous_preferences_do_not_import_a_default_server() {
        let root = std::env::temp_dir().join(format!("papo-upgrade-empty-{}", now_millis()));
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let outer = HashMap::from([("app", r#"(server_url:"https://default.example")"#)]);
        std::fs::write(data.join("app.ron"), ron::to_string(&outer).unwrap()).unwrap();
        assert!(
            import_legacy_android(root.display().to_string(), Arc::new(Secrets::default()))
                .unwrap()
                .unwrap()
                .servers
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

/// Preserve unrelated legacy settings verbatim, including enum and nested values.
struct RawSettings(std::collections::BTreeMap<String, Box<ron::value::RawValue>>);
impl<'de> serde::Deserialize<'de> for RawSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> serde::de::Visitor<'de> for Fields {
            type Value = RawSettings;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("legacy settings fields")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut fields = std::collections::BTreeMap::new();
                while let Some((key, value)) = map.next_entry()? {
                    fields.insert(key, value);
                }
                Ok(RawSettings(fields))
            }
        }
        deserializer.deserialize_struct("Settings", &[], Fields)
    }
}

pub(super) fn forget_android_data(root: &std::path::Path, url: &str) -> Result<(), String> {
    use ron::value::RawValue;
    let key = papo_core::server_key(url);
    let settings_path = root.join("data/app.ron");
    if settings_path.exists() {
        let bytes = std::fs::read_to_string(&settings_path).map_err(|e| e.to_string())?;
        let mut outer: HashMap<String, String> =
            ron::from_str(&bytes).map_err(|e| e.to_string())?;
        if let Some(raw) = outer.get("app") {
            let mut fields: RawSettings = ron::from_str(raw).map_err(|e| e.to_string())?;
            if let Some(servers) = fields.0.get("servers") {
                let servers: Vec<Box<RawValue>> = servers.into_rust().map_err(|e| e.to_string())?;
                let old_active = fields
                    .0
                    .get("active")
                    .and_then(|v| v.into_rust::<usize>().ok())
                    .unwrap_or(0);
                let active_url = servers
                    .get(old_active)
                    .and_then(|v| v.into_rust::<Server>().ok())
                    .map(|s| s.url);
                let keep: Vec<_> = servers
                    .into_iter()
                    .filter(|s| {
                        s.into_rust::<Server>()
                            .ok()
                            .and_then(|s| normalize_url(&s.url).ok())
                            .as_deref()
                            != Some(url)
                    })
                    .collect();
                let active = active_url
                    .and_then(|active| {
                        keep.iter().position(|s| {
                            s.into_rust::<Server>()
                                .ok()
                                .is_some_and(|s| s.url == active)
                        })
                    })
                    .unwrap_or(0);
                let first = keep
                    .get(active)
                    .and_then(|s| s.into_rust::<Server>().ok())
                    .map(|s| s.url)
                    .unwrap_or_default();
                fields.0.insert(
                    "servers".into(),
                    RawValue::from_rust(&keep).map_err(|e| e.to_string())?,
                );
                fields.0.insert(
                    "active".into(),
                    RawValue::from_rust(&active).map_err(|e| e.to_string())?,
                );
                if fields
                    .0
                    .get("server_url")
                    .and_then(|v| v.into_rust::<String>().ok())
                    .and_then(|s| normalize_url(&s).ok())
                    .as_deref()
                    == Some(url)
                {
                    fields.0.insert(
                        "server_url".into(),
                        RawValue::from_rust(&first).map_err(|e| e.to_string())?,
                    );
                }
            }
            if fields
                .0
                .get("server_url")
                .and_then(|v| v.into_rust::<String>().ok())
                .and_then(|s| normalize_url(&s).ok())
                .as_deref()
                == Some(url)
            {
                fields.0.insert(
                    "server_url".into(),
                    RawValue::from_rust(&String::new()).map_err(|e| e.to_string())?,
                );
            }
            for value in fields.0.values_mut() {
                if let Ok(mut map) = value.into_rust::<HashMap<String, Box<RawValue>>>() {
                    let before = map.len();
                    map.retain(|k, _| k != &key && k != url && !k.starts_with(&format!("{url}/")));
                    if map.len() != before {
                        *value = RawValue::from_rust(&map).map_err(|e| e.to_string())?;
                    }
                }
            }
            let updated = format!(
                "({})",
                fields
                    .0
                    .iter()
                    .map(|(k, v)| format!("{k}:{v}"))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            // Validate before replacing the existing file; a failed cleanup is retryable.
            ron::from_str::<Settings>(&updated).map_err(|e| e.to_string())?;
            outer.insert("app".into(), updated);
            let temporary = settings_path.with_extension("forget.tmp");
            std::fs::write(
                &temporary,
                ron::to_string(&outer).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            std::fs::rename(temporary, settings_path).map_err(|e| e.to_string())?;
        }
    }
    for path in [
        root.join(format!("data/sessions/{key}.token")),
        root.join(format!("data/sessions/{key}.server")),
    ] {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    let old_media = root.join("cache/servers").join(&key);
    if old_media.exists() {
        std::fs::remove_dir_all(old_media).map_err(|e| e.to_string())?;
    }
    let public_media = root.join("cache/public");
    if public_media.exists() {
        std::fs::remove_dir_all(public_media).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod forget_tests {
    use super::*;
    #[test]
    fn forget_legacy_data_preserves_other_servers_and_unknown_settings() {
        let root = std::env::temp_dir().join(format!(
            "papo-legacy-forget-{}",
            papo_core::cache::new_local_id()
        ));
        let data = root.join("data");
        std::fs::create_dir_all(data.join("sessions")).unwrap();
        let a = papo_core::server_key("https://a.example");
        let b = papo_core::server_key("https://b.example");
        let settings = format!(
            r#"(servers:[(url:"https://a.example",label:"A"),(url:"https://b.example",label:"B")],server_url:"https://a.example",active:0,theme:Dark,server_marks:{{"{a}":true,"{b}":false}},future_setting:(mode:Something,nested:[1,2,3]))"#
        );
        std::fs::write(
            data.join("app.ron"),
            ron::to_string(&HashMap::from([
                ("app", settings),
                ("other", "unchanged".into()),
            ]))
            .unwrap(),
        )
        .unwrap();
        for key in [&a, &b] {
            std::fs::write(data.join(format!("sessions/{key}.token")), "secret").unwrap();
        }
        forget_android_data(&root, "https://a.example").unwrap();
        let outer: HashMap<String, String> =
            ron::from_str(&std::fs::read_to_string(data.join("app.ron")).unwrap()).unwrap();
        let remaining: Settings = ron::from_str(&outer["app"]).unwrap();
        assert_eq!(remaining.servers.len(), 1);
        assert_eq!(remaining.servers[0].url, "https://b.example");
        assert_eq!(remaining.server_url, "https://b.example");
        assert!(outer["app"].contains("future_setting:(mode:Something,nested:[1,2,3])"));
        assert!(outer["app"].contains("theme:Dark"));
        assert!(!outer["app"].contains(&a));
        assert!(outer["app"].contains(&b));
        assert_eq!(outer["other"], "unchanged");
        assert!(!data.join(format!("sessions/{a}.token")).exists());
        assert!(data.join(format!("sessions/{b}.token")).exists());
        forget_android_data(&root, "https://b.example").unwrap();
        let outer: HashMap<String, String> =
            ron::from_str(&std::fs::read_to_string(data.join("app.ron")).unwrap()).unwrap();
        let remaining: Settings = ron::from_str(&outer["app"]).unwrap();
        assert!(remaining.servers.is_empty());
        assert!(remaining.server_url.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
