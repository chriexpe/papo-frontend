//! Where Save puts media unless Settings › General › Save media to names a folder
//! (`docs/design.md` §8, §14.6): the system's Downloads folder, as the XDG user directories
//! name it (`user-dirs.dirs`), else `~/Downloads`. Owns only finding it; the setting is
//! `general_settings`', the save the SDK's.

use std::path::{Path, PathBuf};

/// The system's Downloads folder; none when neither the user directories nor `HOME` say.
pub(crate) fn downloads_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let named = std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|text| download_dir(&text, &home));
    Some(named.unwrap_or_else(|| home.join("Downloads")))
}

/// `XDG_DOWNLOAD_DIR` in a `user-dirs.dirs` file's `text`: `"$HOME/…"` or an absolute path.
fn download_dir(text: &str, home: &Path) -> Option<PathBuf> {
    let value = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| line.strip_prefix("XDG_DOWNLOAD_DIR="))?
        .trim()
        .trim_matches('"');
    if let Some(rest) = value.strip_prefix("$HOME") {
        let rest = rest.trim_start_matches('/');
        // "$HOME/" alone means the home folder itself, which the spec says not to use.
        return (!rest.is_empty()).then(|| home.join(rest));
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    //! The Downloads folder as user-dirs files name it.

    use super::download_dir;
    use std::path::{Path, PathBuf};

    #[test]
    fn a_home_relative_or_absolute_folder_is_read() {
        let home = Path::new("/home/ana");
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Transferências\"\n";
        assert_eq!(
            download_dir(text, home),
            Some(PathBuf::from("/home/ana/Transferências"))
        );
        assert_eq!(
            download_dir("XDG_DOWNLOAD_DIR=\"/data/dl\"", home),
            Some(PathBuf::from("/data/dl"))
        );
    }

    #[test]
    fn the_home_folder_itself_or_a_relative_path_is_not_taken() {
        let home = Path::new("/home/ana");
        assert_eq!(download_dir("XDG_DOWNLOAD_DIR=\"$HOME/\"", home), None);
        assert_eq!(download_dir("XDG_DOWNLOAD_DIR=\"dl\"", home), None);
        assert_eq!(download_dir("", home), None);
    }
}
