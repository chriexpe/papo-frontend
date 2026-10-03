//! Windows update discovery and installer handoff.
//!
//! GitHub Releases is only the feed. Inno Setup remains the authority that
//! actually replaces an installed Papo copy.

#![cfg(target_os = "windows")]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use ring::digest::{Context, SHA256};
use serde::Deserialize;

const LATEST_RELEASE: &str =
    "https://api.github.com/repos/chriexpe/papo-frontend/releases/latest";

#[derive(Clone, Debug)]
pub struct Available {
    pub version: String,
    pub notes: String,
    pub release_url: String,
    installer_url: String,
    checksum_url: String,
}

#[derive(Debug)]
pub enum Event {
    Current,
    Available(Available),
    Ready { release: Available, installer: PathBuf },
    Error(String),
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    body: Option<String>,
    html_url: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub struct Updater {
    events: mpsc::Receiver<Event>,
    sender: mpsc::Sender<Event>,
    checking: bool,
    downloading: bool,
}

impl Updater {
    pub fn new() -> Self {
        let (sender, events) = mpsc::channel();
        let mut updater = Self {
            events,
            sender,
            checking: false,
            downloading: false,
        };
        updater.check();
        updater
    }

    pub fn enabled(&self) -> bool {
        true
    }

    pub fn check(&mut self) {
        if self.checking || self.downloading {
            return;
        }
        self.checking = true;
        let sender = self.sender.clone();
        std::thread::Builder::new()
            .name("papo-update-check".into())
            .spawn(move || {
                let event = match check_latest() {
                    Ok(Some(release)) => match verified_cached_installer(&release) {
                        Ok(Some(installer)) => Event::Ready { release, installer },
                        Ok(None) => Event::Available(release),
                        Err(error) => {
                            log::warn!("atualização em cache ignorada: {error}");
                            Event::Available(release)
                        }
                    },
                    Ok(None) => Event::Current,
                    Err(error) => Event::Error(error),
                };
                let _ = sender.send(event);
            })
            .ok();
    }

    pub fn download(&mut self, release: Available) {
        if self.downloading {
            return;
        }
        self.downloading = true;
        let sender = self.sender.clone();
        std::thread::Builder::new()
            .name("papo-update-download".into())
            .spawn(move || {
                let event = match download_release(&release) {
                    Ok(installer) => Event::Ready { release, installer },
                    Err(error) => Event::Error(error),
                };
                let _ = sender.send(event);
            })
            .ok();
    }

    pub fn poll(&mut self) -> Option<Event> {
        let event = self.events.try_recv().ok()?;
        match event {
            Event::Current | Event::Available(_) | Event::Error(_) => self.checking = false,
            Event::Ready { .. } => {
                self.checking = false;
                self.downloading = false;
            }
        }
        if matches!(event, Event::Error(_)) {
            self.downloading = false;
        }
        Some(event)
    }

    pub fn checking(&self) -> bool {
        self.checking
    }

    pub fn downloading(&self) -> bool {
        self.downloading
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("Papo/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| error.to_string())
}

fn check_latest() -> Result<Option<Available>, String> {
    let release: Release = client()?
        .get(LATEST_RELEASE)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("GitHub Release: {error}"))?
        .json()
        .map_err(|error| format!("GitHub Release inválido: {error}"))?;

    let latest_text = release.tag_name.trim_start_matches('v');
    let latest = semver::Version::parse(latest_text)
        .map_err(|error| format!("versão de release inválida {latest_text:?}: {error}"))?;
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("versão local inválida: {error}"))?;
    if latest <= current {
        return Ok(None);
    }

    let expected = format!("Papo-{latest}-Setup.exe");
    let expected_checksum = format!("{expected}.sha256");
    let installer = release
        .assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case(&expected))
        .ok_or_else(|| format!("release v{latest} sem {expected}"))?;
    let checksum = release
        .assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case(&expected_checksum))
        .ok_or_else(|| format!("release v{latest} sem {expected_checksum}"))?;

    Ok(Some(Available {
        version: latest.to_string(),
        notes: release
            .body
            .unwrap_or_else(|| "Sem notas de versão.".to_owned()),
        release_url: release.html_url,
        installer_url: installer.browser_download_url.clone(),
        checksum_url: checksum.browser_download_url.clone(),
    }))
}

fn update_dir() -> PathBuf {
    std::env::temp_dir().join("Papo").join("updates")
}

fn installer_path(release: &Available) -> PathBuf {
    update_dir().join(format!("Papo-{}-Setup.exe", release.version))
}

fn expected_checksum(
    client: &reqwest::blocking::Client,
    release: &Available,
) -> Result<String, String> {
    let checksum = client
        .get(&release.checksum_url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("download do checksum: {error}"))?
        .text()
        .map_err(|error| format!("leitura do checksum: {error}"))?;
    checksum
        .split_whitespace()
        .next()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "checksum vazio".to_owned())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("abrir instalador em cache: {error}"))?;
    let mut digest = Context::new(&SHA256);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("ler instalador em cache: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn verified_cached_installer(release: &Available) -> Result<Option<PathBuf>, String> {
    let path = installer_path(release);
    if !path.is_file() {
        return Ok(None);
    }
    let client = client()?;
    let expected = expected_checksum(&client, release)?;
    let actual = sha256_file(&path)?;
    if actual == expected {
        return Ok(Some(path));
    }
    let _ = std::fs::remove_file(&path);
    Ok(None)
}

fn download_release(release: &Available) -> Result<PathBuf, String> {
    let client = client()?;
    let installer = client
        .get(&release.installer_url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("download do instalador: {error}"))?
        .bytes()
        .map_err(|error| format!("leitura do instalador: {error}"))?;
    let expected = expected_checksum(&client, release)?;

    let mut digest = Context::new(&SHA256);
    digest.update(&installer);
    let actual = digest
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected {
        return Err(format!(
            "checksum do instalador não confere (esperado {expected}, obtido {actual})"
        ));
    }

    let dir = update_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("pasta de atualização: {error}"))?;
    let path = installer_path(release);
    std::fs::write(&path, &installer)
        .map_err(|error| format!("salvar instalador: {error}"))?;
    Ok(path)
}

pub fn launch(installer: &Path) -> Result<(), String> {
    std::process::Command::new(installer)
        .arg("/SP-")
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("abrir instalador: {error}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_order_is_semantic() {
        assert!(semver::Version::parse("0.10.0").unwrap() > semver::Version::parse("0.9.9").unwrap());
    }
}
