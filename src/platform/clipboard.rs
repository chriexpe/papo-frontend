//! Clipboard attachments for desktop composers.
//!
//! egui handles text paste itself. Files and bitmap clipboard payloads are
//! intentionally converted into the same Upload objects as picker/drop input.

use arboard::Clipboard;
use papo_core::api::client::Upload;

pub fn attachments() -> Vec<Upload> {
    let Ok(mut clipboard) = Clipboard::new() else {
        return Vec::new();
    };

    // Native file-list payloads should win over bitmap fallbacks. File managers
    // frequently also publish text/HTML versions of the same copy operation.
    let files = clipboard
        .get()
        .file_list()
        .unwrap_or_default()
        .into_iter()
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
    if let Err(error) = image::save_buffer(
        &path,
        image.bytes.as_ref(),
        image.width as u32,
        image.height as u32,
        image::ColorType::Rgba8,
    ) {
        log::warn!("clipboard image: {error}");
        return Vec::new();
    }

    let mut upload = super::files::describe(&path);
    upload.name = "clipboard.png".to_owned();
    vec![upload]
}
