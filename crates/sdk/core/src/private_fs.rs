//! Owner-private profile and staged-file permissions; storage threads own file I/O and lifetime.

use crate::StorageError;
use std::{
    fs::{self, File},
    path::Path,
};

pub(crate) fn directory(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(StorageError::InvalidState);
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    windows(path, "directory")?;
    #[cfg(not(any(unix, windows)))]
    fs::create_dir_all(path)?;
    Ok(())
}

pub(crate) fn file(path: &Path) -> Result<File, StorageError> {
    let mut options = File::options();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    windows(path, "file")?;
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(StorageError::InvalidState);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// Staging runs under the profile lease after the legacy ACL migration. On Windows,
/// new descendants inherit only the owner's allow ACE; migrated blobs are already private.
/// Same-user modifications while the SDK owns the profile are outside this boundary.
pub(crate) fn staging_directory(path: &Path) -> Result<(), StorageError> {
    #[cfg(windows)]
    {
        fs::create_dir_all(path)?;
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(StorageError::InvalidState);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    directory(path)
}

/// Uses the migrated owner-only parent ACL without starting a shell per attachment.
pub(crate) fn staged_file(path: &Path) -> Result<File, StorageError> {
    #[cfg(windows)]
    {
        if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(StorageError::InvalidState);
        }
        Ok(File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?)
    }
    #[cfg(not(windows))]
    file(path)
}

/// Existing Windows files may have explicit broad ACEs despite a private parent.
/// Unix traversal through the owner-only root protects all SQLite sidecars too.
pub(crate) fn profile(path: &Path) -> Result<(), StorageError> {
    directory(path)?;
    Ok(())
}

pub(crate) fn audit(path: &Path) -> Result<(), StorageError> {
    #[cfg(windows)]
    windows(path, "profile")?;
    #[cfg(not(windows))]
    if !fs::metadata(path)?.is_dir() {
        return Err(StorageError::InvalidState);
    }
    Ok(())
}

#[cfg(windows)]
fn windows(path: &Path, kind: &str) -> Result<(), StorageError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Windows PowerShell provides the safe .NET ACL boundary without introducing
    // unsafe native calls into this crate. Values cross via environment, never script text.
    let shell = std::env::var_os("SystemRoot").ok_or(StorageError::InvalidState)?;
    let shell =
        std::path::PathBuf::from(shell).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let result = std::process::Command::new(shell)
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            include_str!("private_windows.ps1"),
        ])
        .env("PAPO_PRIVATE_PATH", path)
        .env("PAPO_PRIVATE_KIND", kind)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !result.success() {
        return Err(StorageError::InvalidState);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
