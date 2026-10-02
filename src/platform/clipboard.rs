//! Clipboard attachments for desktop composers.
//!
//! egui handles text paste itself. Files and bitmap clipboard payloads are
//! intentionally converted into the same Upload objects as picker/drop input.

use clipboard_rs::{common::RustImage, Clipboard, ClipboardContext};
use papo_core::api::client::Upload;
use std::path::{Path, PathBuf};

pub fn attachments() -> Vec<Upload> {
    let Ok(clipboard) = ClipboardContext::new() else {
        return Vec::new();
    };

    let files = clipboard
        .get_files()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| clipboard_path(&entry))
        .filter(|path| path.is_file())
        .map(|path| super::files::describe(&path))
        .collect::<Vec<_>>();
    if !files.is_empty() {
        return files;
    }

    let Ok(image) = clipboard.get_image() else {
        return Vec::new();
    };
    let root = crate::media::cache_root().join("clipboard");
    if let Err(error) = std::fs::create_dir_all(&root) {
        log::warn!("clipboard cache: {error}");
        return Vec::new();
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let path = root.join(format!("clipboard-{nanos}.png"));
    if let Err(error) = image.save_to_path(path.to_string_lossy().as_ref()) {
        log::warn!("clipboard image: {error}");
        return Vec::new();
    }

    let mut upload = super::files::describe(&path);
    upload.name = "clipboard.png".to_owned();
    vec![upload]
}

fn clipboard_path(value: &str) -> Option<PathBuf> {
    if let Ok(url) = url::Url::parse(value)
        && url.scheme() == "file"
    {
        return url.to_file_path().ok();
    }
    let path = Path::new(value);
    path.is_absolute().then(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::clipboard_path;
    use std::path::PathBuf;

    #[test]
    fn accepts_file_uris_and_absolute_paths() {
        assert_eq!(
            clipboard_path("file:///tmp/papo.png"),
            Some(PathBuf::from("/tmp/papo.png"))
        );
        #[cfg(unix)]
        assert_eq!(
            clipboard_path("/tmp/papo.png"),
            Some(PathBuf::from("/tmp/papo.png"))
        );
    }

    #[test]
    fn rejects_non_file_urls() {
        assert_eq!(clipboard_path("https://example.com/image.png"), None);
    }
}
