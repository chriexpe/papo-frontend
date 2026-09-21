use std::collections::HashMap;
use std::sync::Mutex;

/// Segredos persistentes que o núcleo precisa reapresentar automaticamente.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Secret {
    SessionToken,
    ServerPassword,
}

impl Secret {
    pub fn key(self) -> &'static str {
        match self {
            Self::SessionToken => "token",
            Self::ServerPassword => "server",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct StorageError(String);

impl StorageError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<std::io::Error> for StorageError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

pub type StorageResult<T> = Result<T, StorageError>;

/// Armazenamento de credenciais do cliente.
///
/// O núcleo entrega uma chave de servidor estável e o tipo do segredo. Cada
/// frontend decide onde isso vive: arquivos 0600 no desktop, Keystore no
/// Android, Keychain no iOS, ou memória em testes.
pub trait SecretStore: Send + Sync {
    fn load(&self, server: &str, secret: Secret) -> StorageResult<Option<String>>;
    fn store(&self, server: &str, secret: Secret, value: &str) -> StorageResult<()>;
    fn remove(&self, server: &str, secret: Secret) -> StorageResult<()>;

    fn forget_server(&self, server: &str) -> StorageResult<()> {
        self.remove(server, Secret::SessionToken)?;
        self.remove(server, Secret::ServerPassword)
    }
}

/// Backend simples para testes, ferramentas e protótipos sem persistência.
#[derive(Default)]
pub struct MemorySecretStore {
    values: Mutex<HashMap<(String, Secret), String>>,
}

impl SecretStore for MemorySecretStore {
    fn load(&self, server: &str, secret: Secret) -> StorageResult<Option<String>> {
        let values = self
            .values
            .lock()
            .map_err(|_| StorageError::new("armazenamento em memória envenenado"))?;
        Ok(values.get(&(server.to_owned(), secret)).cloned())
    }

    fn store(&self, server: &str, secret: Secret, value: &str) -> StorageResult<()> {
        let mut values = self
            .values
            .lock()
            .map_err(|_| StorageError::new("armazenamento em memória envenenado"))?;
        values.insert((server.to_owned(), secret), value.to_owned());
        Ok(())
    }

    fn remove(&self, server: &str, secret: Secret) -> StorageResult<()> {
        let mut values = self
            .values
            .lock()
            .map_err(|_| StorageError::new("armazenamento em memória envenenado"))?;
        values.remove(&(server.to_owned(), secret));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memoria_separa_servidores_e_tipos_de_segredo() {
        let store = MemorySecretStore::default();
        store.store("um", Secret::SessionToken, "token-1").unwrap();
        store.store("um", Secret::ServerPassword, "senha").unwrap();
        store.store("dois", Secret::SessionToken, "token-2").unwrap();

        assert_eq!(
            store.load("um", Secret::SessionToken).unwrap().as_deref(),
            Some("token-1")
        );
        assert_eq!(
            store.load("um", Secret::ServerPassword).unwrap().as_deref(),
            Some("senha")
        );
        assert_eq!(
            store.load("dois", Secret::SessionToken).unwrap().as_deref(),
            Some("token-2")
        );
    }

    #[test]
    fn esquecer_servidor_apaga_todos_os_segredos_dele() {
        let store = MemorySecretStore::default();
        store.store("um", Secret::SessionToken, "token").unwrap();
        store.store("um", Secret::ServerPassword, "senha").unwrap();
        store.store("dois", Secret::SessionToken, "fica").unwrap();

        store.forget_server("um").unwrap();

        assert_eq!(store.load("um", Secret::SessionToken).unwrap(), None);
        assert_eq!(store.load("um", Secret::ServerPassword).unwrap(), None);
        assert_eq!(
            store.load("dois", Secret::SessionToken).unwrap().as_deref(),
            Some("fica")
        );
    }
}
