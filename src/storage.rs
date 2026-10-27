use std::path::{Path, PathBuf};

use papo_core::storage::{Secret, SecretStore, StorageResult};

/// Persistência usada pelo frontend desktop.
///
/// Mantém exatamente o layout legado para não invalidar sessões existentes:
/// `<data>/papo/sessions/<server-key>.token` e `.server`.
pub struct FileSecretStore {
    root: Option<PathBuf>,
}

impl FileSecretStore {
    pub fn new() -> Self {
        Self {
            root: crate::platform::dirs::data_dir().map(|dir| dir.join("sessions")),
        }
    }

    /// Persist intent before stopping transport or touching local data. Retained markers also
    /// suppress credentials and stale eframe server lists after a crash during cleanup.
    pub fn mark_forgotten(&self, server: &str) -> StorageResult<()> {
        self.ensure_root()?;
        if let Some(path) = self.marker(server) {
            let file = std::fs::File::create(path)?;
            file.sync_all()?;
            #[cfg(unix)]
            if let Some(root) = &self.root { std::fs::File::open(root)?.sync_all()?; }
        }
        Ok(())
    }

    fn marker(&self, server: &str) -> Option<PathBuf> {
        self.root.as_ref().map(|root| root.join(format!("{server}.forget")))
    }

    pub fn is_forgotten(&self, server: &str) -> bool {
        self.marker(server).is_some_and(|path| path.try_exists().unwrap_or(true))
    }

    pub fn forgotten_servers(&self) -> StorageResult<Vec<String>> {
        let Some(root) = &self.root else { return Ok(vec![]); };
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(error.into()),
        };
        let mut keys = Vec::new();
        for entry in entries {
            let entry = entry?;
            if let Some(key) = entry.file_name().to_str().and_then(|s| s.strip_suffix(".forget"))
                && !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                keys.push(key.to_owned());
            }
        }
        Ok(keys)
    }

    /// Explicit re-add after cleanup, before creating a replacement authenticated runtime.
    pub fn allow_server(&self, server: &str) -> StorageResult<()> {
        let Some(path) = self.marker(server) else { return Ok(()); };
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn path(&self, server: &str, secret: Secret) -> Option<PathBuf> {
        let root = self.root.as_ref()?;
        Some(root.join(format!("{server}.{}", secret.key())))
    }

    fn ensure_root(&self) -> StorageResult<()> {
        if let Some(root) = &self.root {
            std::fs::create_dir_all(root)?;
        }
        Ok(())
    }
}

impl Default for FileSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for FileSecretStore {
    fn load(&self, server: &str, secret: Secret) -> StorageResult<Option<String>> {
        if self.is_forgotten(server) { return Ok(None); }
        let Some(path) = self.path(server, secret) else {
            return Ok(None);
        };
        match std::fs::read_to_string(path) {
            Ok(value) => {
                let value = value.trim();
                Ok((!value.is_empty()).then(|| value.to_owned()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn store(&self, server: &str, secret: Secret, value: &str) -> StorageResult<()> {
        if self.is_forgotten(server) { return Err(papo_core::storage::StorageError::new("server_cleanup_pending")); }
        self.ensure_root()?;
        let Some(path) = self.path(server, secret) else {
            return Ok(());
        };
        std::fs::write(&path, value)?;
        restrict(&path)?;
        Ok(())
    }

    fn remove(&self, server: &str, secret: Secret) -> StorageResult<()> {
        let Some(path) = self.path(server, secret) else {
            return Ok(());
        };
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// Token e senha de servidor são segredos; no Unix só o dono pode ler.
fn restrict(path: &Path) -> StorageResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensoes_continuam_compativeis_com_o_layout_antigo() {
        let store = FileSecretStore {
            root: Some(PathBuf::from("/tmp/papo-test")),
        };
        assert!(store
            .path("servidor", Secret::SessionToken)
            .unwrap()
            .ends_with("servidor.token"));
        assert!(store
            .path("servidor", Secret::ServerPassword)
            .unwrap()
            .ends_with("servidor.server"));
    }
}

#[cfg(test)]
mod parity_tests {
    use super::*;
    #[test]
    fn parity_forget_journal_blocks_late_credentials_and_preserves_other_servers() {
        let root = std::env::temp_dir().join(format!("papo-forget-{}", papo_core::cache::new_local_id()));
        let store = FileSecretStore { root: Some(root.clone()) };
        store.store("one", Secret::SessionToken, "first").unwrap();
        store.store("two", Secret::SessionToken, "other").unwrap();
        store.mark_forgotten("one").unwrap();
        assert_eq!(store.forgotten_servers().unwrap(), vec!["one"]);
        assert!(store.load("one", Secret::SessionToken).unwrap().is_none());
        assert!(store.store("one", Secret::SessionToken, "late").is_err());
        store.forget_server("one").unwrap();
        store.allow_server("one").unwrap();
        assert!(store.load("one", Secret::SessionToken).unwrap().is_none());
        assert_eq!(store.load("two", Secret::SessionToken).unwrap().as_deref(), Some("other"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
