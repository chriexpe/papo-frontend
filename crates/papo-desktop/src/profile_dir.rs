//! Where the SDK keeps this user's profile on disk, and where files opened through the system
//! wait (a file card's Open).
//!
//! Owns only the choice of directories. The SDK creates it with private
//! permissions, lays out its files and locks it against a second owner.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::window::APP_ID;

/// The profile directory: `$XDG_DATA_HOME/io.github.chriexpe.Papo`, or
/// `~/.local/share/io.github.chriexpe.Papo` when `XDG_DATA_HOME` is unset.
/// Flatpak points `XDG_DATA_HOME` inside the app's own sandbox directory.
/// `None` when neither variable names an absolute directory.
pub(crate) fn profile_dir() -> Option<PathBuf> {
    profile_dir_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

/// Where a file opened through the system is saved first, so its app can read it:
/// `$XDG_CACHE_HOME/io.github.chriexpe.Papo/opened`, or `~/.cache/…` when that is unset.
pub(crate) fn opened_files_dir() -> Option<PathBuf> {
    let absolute = |value: OsString| Some(PathBuf::from(value)).filter(|path| path.is_absolute());
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .and_then(absolute)
        .or_else(|| {
            std::env::var_os("HOME")
                .and_then(absolute)
                .map(|home| home.join(".cache"))
        })?;
    Some(cache.join(APP_ID).join("opened"))
}

/// [`profile_dir`] for the given environment values. The XDG specification
/// says a relative `XDG_DATA_HOME` is invalid and is ignored.
fn profile_dir_from(data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let absolute = |value: OsString| Some(PathBuf::from(value)).filter(|path| path.is_absolute());
    let data_home = data_home.and_then(absolute).or_else(|| {
        home.and_then(absolute)
            .map(|home| home.join(".local/share"))
    })?;
    Some(data_home.join(APP_ID))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::profile_dir_from;

    #[test]
    fn the_profile_lives_under_xdg_data_home() {
        let dir = profile_dir_from(Some("/data".into()), Some("/home/u".into()));
        assert_eq!(
            dir.as_deref(),
            Some(Path::new("/data/io.github.chriexpe.Papo"))
        );
    }

    #[test]
    fn without_xdg_data_home_the_profile_lives_in_local_share() {
        let dir = profile_dir_from(None, Some("/home/u".into()));
        assert_eq!(
            dir.as_deref(),
            Some(Path::new("/home/u/.local/share/io.github.chriexpe.Papo"))
        );
    }

    #[test]
    fn a_relative_xdg_data_home_is_ignored() {
        let dir = profile_dir_from(Some("data".into()), Some("/home/u".into()));
        assert_eq!(
            dir.as_deref(),
            Some(Path::new("/home/u/.local/share/io.github.chriexpe.Papo"))
        );
    }

    #[test]
    fn with_no_usable_directory_there_is_no_profile() {
        assert_eq!(profile_dir_from(None, Some("rel".into())), None);
    }
}
