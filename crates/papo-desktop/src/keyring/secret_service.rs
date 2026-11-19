//! The keyring backend on the freedesktop Secret Service, through `oo7`.
//!
//! Owns the connection and the mapping from a [`SecretKey`] to an entry's
//! attributes. On a host, `oo7` talks to the Secret Service (GNOME Keyring,
//! KWallet, KeePassXC); inside Flatpak it keeps an encrypted file in the
//! sandbox, keyed by the Secret portal. The same code serves both. Queueing
//! and fencing belong to `keyring`.

use std::collections::HashMap;

use papo_sdk::secret::{SecretKey, SecretStoreError, SecretValue};

use super::Backend;
use crate::window::APP_ID;

/// The schema attribute every Papo entry carries.
const SCHEMA: &str = "io.github.chriexpe.Papo.Credentials";
/// The name a keyring manager (Seahorse, KDE Wallet Manager) shows for an entry.
/// It names no server or account: the attributes identify the entry.
const LABEL: &str = "Papo sign-in";

/// The Secret Service, connected on first use and again after it was lost.
#[derive(Default)]
pub(crate) struct SecretService {
    keyring: Option<oo7::Keyring>,
}

impl SecretService {
    /// The connected keyring, unlocked (the OS may prompt for that).
    async fn keyring(&mut self) -> Result<&oo7::Keyring, SecretStoreError> {
        if self.keyring.is_none() {
            let keyring = oo7::Keyring::new().await.map_err(category)?;
            self.keyring = Some(keyring);
        }
        let keyring = self.keyring.as_ref().ok_or(SecretStoreError::Unavailable)?;
        keyring.unlock().await.map_err(category)?;
        Ok(keyring)
    }

    /// Logs a failure's category, and forgets the connection when the service
    /// went away, so the next call connects again.
    fn settle<T>(&mut self, result: Result<T, SecretStoreError>) -> Result<T, SecretStoreError> {
        if let Err(error) = &result {
            log::warn!("keyring call failed: {error}");
            if *error == SecretStoreError::Unavailable {
                self.keyring = None;
            }
        }
        result
    }
}

impl Backend for SecretService {
    async fn read(&mut self, key: SecretKey) -> Result<Option<Vec<u8>>, SecretStoreError> {
        let result = async {
            let keyring = self.keyring().await?;
            let items = keyring
                .search_items(&attributes(key))
                .await
                .map_err(category)?;
            let Some(item) = items.first() else {
                return Ok(None);
            };
            item.unlock().await.map_err(category)?;
            let secret = item.secret().await.map_err(category)?;
            Ok(Some(secret.as_bytes().to_vec()))
        }
        .await;
        self.settle(result)
    }

    async fn write(&mut self, key: SecretKey, value: SecretValue) -> Result<(), SecretStoreError> {
        let result = async {
            let keyring = self.keyring().await?;
            let secret = oo7::Secret::blob(value.expose());
            keyring
                .create_item(LABEL, &attributes(key), secret, true)
                .await
                .map_err(category)
        }
        .await;
        self.settle(result)
    }

    async fn delete(&mut self, key: SecretKey) -> Result<(), SecretStoreError> {
        let result = async {
            let keyring = self.keyring().await?;
            keyring.delete(&attributes(key)).await.map_err(category)
        }
        .await;
        self.settle(result)
    }
}

/// The attributes that name `key`'s entry: every part of the key, so no two
/// accounts or credential generations share an entry.
pub(super) fn attributes(key: SecretKey) -> HashMap<&'static str, String> {
    let account = key.account();
    HashMap::from([
        (oo7::XDG_SCHEMA_ATTRIBUTE, SCHEMA.to_owned()),
        ("application", APP_ID.to_owned()),
        ("server", account.server().to_string()),
        ("user", account.user().to_string()),
        ("generation", key.generation().get().to_string()),
    ])
}

/// The SDK's category for an `oo7` failure. The OS's own text never leaves
/// this function.
fn category(error: oo7::Error) -> SecretStoreError {
    match error {
        oo7::Error::DBus(
            oo7::dbus::Error::Dismissed
            | oo7::dbus::Error::Service(oo7::dbus::ServiceError::IsLocked(_)),
        ) => SecretStoreError::AccessDenied,
        // The service restarted or the collection went away: the next call
        // connects again.
        oo7::Error::DBus(
            oo7::dbus::Error::ZBus(_)
            | oo7::dbus::Error::NotFound(_)
            | oo7::dbus::Error::Deleted
            | oo7::dbus::Error::Service(
                oo7::dbus::ServiceError::ZBus(_)
                | oo7::dbus::ServiceError::NoSession(_)
                | oo7::dbus::ServiceError::NoSuchObject(_),
            ),
        ) => SecretStoreError::Unavailable,
        oo7::Error::File(file) => match *file {
            oo7::file::Error::Portal(_) | oo7::file::Error::NoDataDir => {
                SecretStoreError::Unavailable
            }
            _ => SecretStoreError::Failed,
        },
        oo7::Error::DBus(_) => SecretStoreError::Failed,
    }
}
