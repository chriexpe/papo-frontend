//! The strings of sending attachments (`docs/design.md` §14.3, §14.4):
//! each upload's progress, its cancel, an album's and a file's spoken
//! names, and "Quit while uploading?". The catalog entries are `upload.*`,
//! `media.*` and `quit.*`.

use super::{I18N, Key, catalog};

/// A ring's or bar's spoken name: "Uploading praia.jpg, 34 %".
pub(crate) fn upload_progress(name: &str, percent: u32) -> String {
    let percent = super::integer(i64::from(percent));
    template!("upload.progress", "name" => name, "percent" => percent)
}

/// A file card's size line while it uploads: "12.4 of 100 MB", the unit
/// once, the total's.
pub(crate) fn upload_size_progress(done_bytes: u64, total_bytes: u64) -> String {
    let (done, total) = super::size_pair(done_bytes, total_bytes);
    template!("upload.size_progress", "done" => done, "total" => total)
}

/// ✕ on an uploading item.
pub(crate) fn upload_cancel() -> &'static str {
    text!("upload.cancel")
}

/// An album, spoken: "Album, 3 items".
pub(crate) fn media_album(count: usize) -> String {
    let n = super::integer(i64::try_from(count).unwrap_or(i64::MAX));
    template!("media.album", "n" => n)
}

/// The count over an album's last cell: "+3".
pub(crate) fn media_more(count: usize) -> String {
    let n = super::integer(i64::try_from(count).unwrap_or(i64::MAX));
    template!("media.more", "n" => n)
}

/// A file card, spoken: "File, notes.pdf, 2.4 MB".
pub(crate) fn media_file(name: &str, size: &str) -> String {
    template!("media.file", "name" => name, "size" => size)
}

/// "Quit while uploading?"
pub(crate) fn quit_uploading_title() -> &'static str {
    text!("quit.uploading_title")
}

/// What quitting during an upload loses.
pub(crate) fn quit_uploading_body() -> &'static str {
    text!("quit.uploading_body")
}

/// The question's action.
pub(crate) fn quit_uploading_confirm() -> &'static str {
    text!("quit.uploading_confirm")
}
