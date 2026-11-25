//! Adding files to the tray (`docs/design.md` §14.1): which of the files
//! offered (by the chooser, a paste or a drop) are taken, which are refused
//! and why, what kind of tile each makes, and the name a pasted picture gets.
//!
//! Pure rules over what the shell read from the files: the limits are the
//! SDK's public ones (`MAX_ATTACHMENTS_PER_MESSAGE`, `MAX_ATTACHMENT_BYTES`,
//! `MAX_MESSAGE_ATTACHMENT_BYTES`), and the SDK enforces them again when it
//! stages and sends. Reading the files and staging them is the shell's
//! (`shell/attaching.rs`); the refusal's words are `notice`'s.

use papo_sdk::{MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS_PER_MESSAGE, MAX_MESSAGE_ATTACHMENT_BYTES};

use papo_sdk_ui::AttachmentKind;

use crate::composer::tray::TileKind;

/// The most bytes a picture may have for its tile to be decoded; a larger
/// one is drawn as a file tile, so a tile never holds a huge decode. The
/// SDK's limit, so the sent message draws it as the same kind.
pub(crate) const THUMBNAIL_SOURCE_MAX_BYTES: u64 = papo_sdk_ui::MAX_PICTURE_BYTES;

/// What the shell found about one file offered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Offered {
    /// A file of `size_bytes`.
    File { name: String, size_bytes: u64 },
    /// A folder: never attached.
    Folder,
    /// A file that could not be opened or measured.
    Unreadable { name: String },
}

/// Why files were not added. One notice says the first of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The tray was full: some files were left out.
    TooMany,
    /// One file over the per-file limit.
    TooLarge { name: String },
    /// Several files over the per-file limit.
    ManyTooLarge { count: usize },
    /// The files together would pass the per-message limit.
    TooMuchTogether,
    /// A folder was offered.
    Folder,
    /// A file could not be read.
    Unreadable { name: String },
    /// An image the SDK cannot clean (HEIC, TIFF, RAW) while Anonymize is
    /// on: never sent as it is.
    CannotAnonymize { name: String },
}

/// What one offer comes to: the indices (into the offer) of the files
/// taken, in order, and the first refusal, if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) taken: Vec<usize>,
    pub(crate) refused: Option<Refused>,
}

/// Which of `offered` a tray holding `held` files of `held_bytes` takes.
/// Files that pass are taken even when others are refused (§14.1).
pub(crate) fn plan(offered: &[Offered], held: usize, held_bytes: u64) -> Plan {
    let mut taken = Vec::new();
    let mut count = held;
    let mut bytes = held_bytes;
    let mut too_large: Vec<&str> = Vec::new();
    let mut too_many = false;
    let mut together = false;
    let mut folder = false;
    let mut unreadable: Option<&str> = None;
    for (index, offer) in offered.iter().enumerate() {
        match offer {
            Offered::Folder => folder = true,
            Offered::Unreadable { name } => {
                unreadable.get_or_insert(name);
            }
            Offered::File { name, size_bytes } => {
                if *size_bytes > MAX_ATTACHMENT_BYTES {
                    too_large.push(name);
                } else if count >= MAX_ATTACHMENTS_PER_MESSAGE {
                    too_many = true;
                } else if bytes.saturating_add(*size_bytes) > MAX_MESSAGE_ATTACHMENT_BYTES {
                    together = true;
                } else {
                    taken.push(index);
                    count += 1;
                    bytes += size_bytes;
                }
            }
        }
    }
    let refused = if too_many {
        Some(Refused::TooMany)
    } else if let [name] = too_large.as_slice() {
        Some(Refused::TooLarge {
            name: (*name).to_owned(),
        })
    } else if !too_large.is_empty() {
        Some(Refused::ManyTooLarge {
            count: too_large.len(),
        })
    } else if together {
        Some(Refused::TooMuchTogether)
    } else if folder {
        Some(Refused::Folder)
    } else {
        unreadable.map(|name| Refused::Unreadable {
            name: name.to_owned(),
        })
    };
    Plan { taken, refused }
}

/// How many leading bytes [`kind_of`] reads: enough for an MP4's brand after a 64-bit box size.
pub(crate) const HEAD_BYTES: usize = 24;

/// The tile a file named `name` makes, from its first bytes and size: a video or an audio as
/// its sent message will be one (the label the server will give it, `server_label`, read with
/// its name as the SDK reads a received attachment's), a picture Rune decodes (JPEG, PNG, WebP,
/// GIF) small enough to decode, else a file.
/// Only for drawing: what the SDK may clean is the SDK's to say.
pub(crate) fn kind_of(head: &[u8], size_bytes: u64, name: &str) -> TileKind {
    match AttachmentKind::from_file(server_label(head), Some(name), Some(size_bytes)) {
        AttachmentKind::Video => return TileKind::Video,
        AttachmentKind::Audio => return TileKind::Audio,
        _ => {}
    }
    if size_bytes > THUMBNAIL_SOURCE_MAX_BYTES {
        return TileKind::File;
    }
    let jpeg = head.starts_with(&[0xFF, 0xD8, 0xFF]);
    let png = head.starts_with(b"\x89PNG\r\n\x1a\n");
    let webp = head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP";
    let gif = head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a");
    if gif {
        TileKind::Gif
    } else if jpeg || png || webp {
        TileKind::Image
    } else {
        TileKind::File
    }
}

/// ISO media brands the server takes for audio (M4A and kin).
const AUDIO_BRANDS: [&[u8; 4]; 4] = [b"M4A ", b"M4B ", b"M4P ", b"F4A "];

/// ISO media brands the server takes for a picture (AVIF, HEIC).
const PICTURE_BRANDS: [&[u8; 4]; 10] = [
    b"avif", b"avis", b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"mif1", b"msf1",
];

/// The media type the server will give a file starting with `head`, as far as playing it here
/// goes (its `DetectMimeType`): an ISO media file (MP4, MOV, 3GP) is video unless its brand is
/// an audio or picture one, Matroska and `WebM` are video, an AVI is video; an Ogg is
/// `application/ogg`, an MP3 or a WAV audio; the rest is unlabelled here.
fn server_label(head: &[u8]) -> &'static str {
    if head.get(4..8) == Some(b"ftyp") {
        let Some(size) = head
            .get(..4)
            .and_then(|size| <[u8; 4]>::try_from(size).ok())
        else {
            return UNLABELLED;
        };
        // A box size of 1 means a 64-bit size follows, and the brand after it.
        let brand_at = if u32::from_be_bytes(size) == 1 { 16 } else { 8 };
        let Some(brand) = head.get(brand_at..brand_at + 4) else {
            return UNLABELLED;
        };
        if AUDIO_BRANDS.iter().any(|audio| brand == audio.as_slice()) {
            return "audio/mp4";
        }
        if PICTURE_BRANDS
            .iter()
            .any(|picture| brand == picture.as_slice())
        {
            return UNLABELLED;
        }
        return "video/mp4";
    }
    let riff = |form: &[u8; 4]| head.starts_with(b"RIFF") && head.get(8..12) == Some(form);
    let mpeg_frame = head.len() >= 2 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0;
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        "video/webm"
    } else if riff(b"AVI ") {
        "video/x-msvideo"
    } else if head.starts_with(b"OggS") {
        "application/ogg"
    } else if head.starts_with(b"ID3") || mpeg_frame {
        "audio/mpeg"
    } else if riff(b"WAVE") {
        "audio/wave"
    } else {
        UNLABELLED
    }
}

/// What the server labels a file it doesn't recognise.
const UNLABELLED: &str = "application/octet-stream";

/// The name for a pasted picture: `base` ("image.png") for the first,
/// "image (2).png" and on for the next ones in the same message, skipping
/// names `taken` already uses.
pub(crate) fn pasted_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    let (stem, extension) = base.rsplit_once('.').unwrap_or((base, ""));
    // A tray holds at most ten files, so one of the first eleven numbers
    // is always free.
    let last = u32::try_from(MAX_ATTACHMENTS_PER_MESSAGE + 1).unwrap_or(u32::MAX);
    (2_u32..=last)
        .map(|n| {
            if extension.is_empty() {
                format!("{stem} ({n})")
            } else {
                format!("{stem} ({n}).{extension}")
            }
        })
        .find(|name| !taken(name))
        .unwrap_or_else(|| base.to_owned())
}

/// The extension a file tile shows in its block: up to four characters,
/// upper case ("PDF", "TAR"); empty when the name has none.
pub(crate) fn extension_label(name: &str) -> String {
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, extension)| extension.chars().take(4).collect::<String>().to_uppercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
