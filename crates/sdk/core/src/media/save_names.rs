//! The name a saved attachment gets: the uploaded name made safe to write (it comes from the
//! server, so it may hold separators or names a file system refuses), and the first free
//! "name (1)" beside files already there. Writing the bytes lives in `save`.

use std::{
    fs::OpenOptions,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

/// The name used when nothing of the uploaded one survives.
const FALLBACK_NAME: &str = "attachment";

/// The longest name written, in bytes: what common file systems take.
const MAX_NAME_BYTES: usize = 255;

/// The most numbered names tried before giving up on a crowded folder.
const MAX_NUMBERED: u32 = 9_999;

/// `uploaded` made safe to create in a folder: its last path component only, without control
/// characters or characters Windows refuses, without leading dots (no hidden files) or trailing
/// dots and spaces, at most [`MAX_NAME_BYTES`] bytes with its extension kept, and never empty.
pub(super) fn safe_name(uploaded: &str) -> String {
    let last = uploaded.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && !is_direction_mark(*c))
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        return FALLBACK_NAME.to_owned();
    }
    if is_device_name(trimmed) {
        return shortened(&format!("_{trimmed}"));
    }
    shortened(trimmed)
}

/// Marks that change the direction text is shown in: a name holding one can show an extension
/// it does not have ("photo\u{202E}gpj.exe").
fn is_direction_mark(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// A name Windows keeps for a device, whatever its extension ("CON", "com1.txt").
fn is_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            // `get`, not `split_at`: a name's third byte may fall inside a character.
            let (Some(prefix), Some(digit)) = (stem.get(..3), stem.get(3..)) else {
                return false;
            };
            matches!(prefix, "COM" | "LPT")
                && digit.len() == 1
                && digit.chars().all(|c| c.is_ascii_digit() && c != '0')
        }
    }
}

/// `name` cut to [`MAX_NAME_BYTES`], keeping its extension when it has a short one.
fn shortened(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_owned();
    }
    let (stem, extension) = split_extension(name);
    let room = MAX_NAME_BYTES.saturating_sub(extension.len());
    let mut cut = room.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{extension}", &stem[..cut])
}

/// `name`'s stem and its extension with the dot (".tar.gz" counted whole), or no extension.
fn split_extension(name: &str) -> (&str, &str) {
    let Some(dot) = name.rfind('.').filter(|dot| *dot > 0) else {
        return (name, "");
    };
    let (stem, extension) = name.split_at(dot);
    if extension.len() > 16 {
        return (name, "");
    }
    // "archive.tar.gz" numbers as "archive (1).tar.gz", as browsers do.
    match stem.strip_suffix(".tar") {
        Some(inner) if !inner.is_empty() => (inner, &name[inner.len()..]),
        _ => (stem, extension),
    }
}

/// The name for attempt `number`: `name` itself first, then "stem (1).ext" and on.
fn numbered(name: &str, number: u32) -> String {
    if number == 0 {
        return name.to_owned();
    }
    let (stem, extension) = split_extension(name);
    shortened_numbered(stem, &format!(" ({number})"), extension)
}

/// `stem` + `mark` + `extension`, the stem cut so the whole fits [`MAX_NAME_BYTES`].
fn shortened_numbered(stem: &str, mark: &str, extension: &str) -> String {
    let room = MAX_NAME_BYTES.saturating_sub(mark.len() + extension.len());
    let mut cut = room.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{mark}{extension}", &stem[..cut])
}

/// Creates an empty file at the first free name for `name` in `folder` and returns its path:
/// creating it is what claims it, so two saves never take the same name and an existing file is
/// never replaced.
/// # Errors
/// Returns the folder's own error, or `AlreadyExists` when every numbered name is taken.
pub(super) fn claim_free(folder: &Path, name: &str) -> io::Result<PathBuf> {
    for number in 0..=MAX_NUMBERED {
        let path = folder.join(numbered(name, number));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(ErrorKind::AlreadyExists.into())
}

#[cfg(test)]
mod tests {
    //! Names made safe and numbered, against a temporary folder.

    use super::*;

    #[test]
    fn an_uploaded_name_keeps_only_its_last_component() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\Users\\a\\photo.jpg"), "photo.jpg");
        assert_eq!(safe_name("dir/"), FALLBACK_NAME);
        assert_eq!(safe_name(".."), FALLBACK_NAME);
        assert_eq!(safe_name(""), FALLBACK_NAME);
    }

    #[test]
    fn refused_characters_and_hidden_names_are_cleaned() {
        assert_eq!(safe_name("a\0b\nc.txt"), "abc.txt");
        assert_eq!(safe_name("what?<now>.pdf"), "what__now_.pdf");
        assert_eq!(safe_name(".bashrc"), "bashrc");
        assert_eq!(safe_name("notes.  "), "notes");
        assert_eq!(safe_name("relatório final.pdf"), "relatório final.pdf");
        assert_eq!(safe_name("photo\u{202E}gpj.exe"), "photogpj.exe");
    }

    #[test]
    fn device_names_are_not_taken() {
        assert_eq!(safe_name("CON"), "_CON");
        assert_eq!(safe_name("com1.txt"), "_com1.txt");
        assert_eq!(safe_name("console.txt"), "console.txt");
        assert_eq!(safe_name("COM10.txt"), "COM10.txt");
        assert_eq!(safe_name("éé.txt"), "éé.txt");
    }

    #[test]
    fn a_long_name_keeps_its_extension_within_the_limit() {
        let long = format!("{}.pdf", "é".repeat(200));
        let safe = safe_name(&long);
        assert!(safe.len() <= MAX_NAME_BYTES);
        assert_eq!(safe.rsplit('.').next(), Some("pdf"));
    }

    #[test]
    fn numbered_names_go_before_the_extension() {
        assert_eq!(numbered("praia.jpg", 0), "praia.jpg");
        assert_eq!(numbered("praia.jpg", 1), "praia (1).jpg");
        assert_eq!(numbered("archive.tar.gz", 2), "archive (2).tar.gz");
        assert_eq!(numbered("README", 1), "README (1)");
    }

    #[test]
    fn a_free_name_is_claimed_beside_existing_files() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let first = claim_free(folder.path(), "praia.jpg").expect("claimed");
        assert_eq!(first, folder.path().join("praia.jpg"));
        std::fs::write(&first, b"kept").expect("written");
        let second = claim_free(folder.path(), "praia.jpg").expect("claimed");
        assert_eq!(second, folder.path().join("praia (1).jpg"));
        // The existing file is untouched.
        assert_eq!(std::fs::read(&first).expect("read"), b"kept");
    }

    #[test]
    fn a_missing_folder_is_its_own_error() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let missing = folder.path().join("gone");
        let error = claim_free(&missing, "a.txt").expect_err("no folder");
        assert_eq!(error.kind(), ErrorKind::NotFound);
    }
}
