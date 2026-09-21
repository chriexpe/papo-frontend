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
            root: directories::ProjectDirs::from("", "", "papo")
                .map(|dirs| dirs.data_dir().join("sessions")),
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
