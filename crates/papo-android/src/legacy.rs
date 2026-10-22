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
