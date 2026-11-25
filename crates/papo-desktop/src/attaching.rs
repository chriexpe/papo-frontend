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

/// The tile a file makes, from its first bytes and size: a picture Rune
/// decodes (JPEG, PNG, WebP, GIF) small enough to decode, a video the
/// server will call one (`is_video`), else a file.
/// Only for drawing: what the SDK may clean is the SDK's to say.
pub(crate) fn kind_of(head: &[u8], size_bytes: u64) -> TileKind {
    if is_video(head) {
        return TileKind::Video;
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

/// ISO media brands the server takes for audio (M4A and kin) or a picture (AVIF, HEIC).
const NOT_VIDEO_BRANDS: [&[u8; 4]; 14] = [
    b"M4A ", b"M4B ", b"M4P ", b"F4A ", b"avif", b"avis", b"heic", b"heix", b"hevc", b"hevx",
    b"heim", b"heis", b"mif1", b"msf1",
];

/// Whether a file starting with `head` is a video as the server sniffs one, so its unsent row
/// takes the shape its sent row will: an ISO media file (MP4, MOV, 3GP) whose brand is not an
/// audio or picture one, a Matroska or `WebM` file, or an AVI.
fn is_video(head: &[u8]) -> bool {
    if head.get(4..8) == Some(b"ftyp") {
        let Some(size) = head
            .get(..4)
            .and_then(|size| <[u8; 4]>::try_from(size).ok())
        else {
            return false;
        };
        // A box size of 1 means a 64-bit size follows, and the brand after it.
        let brand_at = if u32::from_be_bytes(size) == 1 { 16 } else { 8 };
        let Some(brand) = head.get(brand_at..brand_at + 4) else {
            return false;
        };
        return !NOT_VIDEO_BRANDS.iter().any(|not| brand == not.as_slice());
    }
    let matroska = head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]);
    let avi = head.starts_with(b"RIFF") && head.get(8..12) == Some(b"AVI ");
    matroska || avi
}

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
mod tests {
    //! The rules of §14.1 against the SDK's limits.
    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn file(name: &str, size_bytes: u64) -> Offered {
        Offered::File {
            name: name.to_owned(),
            size_bytes,
        }
    }

    #[test]
    fn files_within_the_limits_are_all_taken() {
        let plan = plan(&[file("a.jpg", MIB), file("b.pdf", 2 * MIB)], 0, 0);
        assert_eq!(plan.taken, [0, 1]);
        assert_eq!(plan.refused, None);
    }

    #[test]
    fn one_file_over_the_limit_is_named_and_the_others_still_go() {
        let plan = plan(
            &[
                file("big.mkv", MAX_ATTACHMENT_BYTES + 1),
                file("a.jpg", MIB),
            ],
            0,
            0,
        );
        assert_eq!(plan.taken, [1]);
        assert_eq!(
            plan.refused,
            Some(Refused::TooLarge {
                name: "big.mkv".to_owned()
            })
        );
    }

    #[test]
    fn several_files_over_the_limit_are_counted() {
        let over = MAX_ATTACHMENT_BYTES + 1;
        let plan = plan(&[file("a", over), file("b", over)], 0, 0);
        assert_eq!(plan.taken, [0_usize; 0]);
        assert_eq!(plan.refused, Some(Refused::ManyTooLarge { count: 2 }));
    }

    #[test]
    fn a_full_tray_takes_up_to_ten_and_says_so() {
        let offered: Vec<Offered> = (0..4).map(|n| file(&n.to_string(), 1)).collect();
        let plan = plan(&offered, MAX_ATTACHMENTS_PER_MESSAGE - 2, 0);
        assert_eq!(plan.taken, [0, 1]);
        assert_eq!(plan.refused, Some(Refused::TooMany));
    }

    #[test]
    fn files_too_large_together_are_refused_once_the_total_would_pass() {
        let half = MAX_MESSAGE_ATTACHMENT_BYTES / 2;
        let plan = plan(&[file("a", half), file("b", half + 1)], 0, 0);
        assert_eq!(plan.taken, [0]);
        assert_eq!(plan.refused, Some(Refused::TooMuchTogether));
    }

    #[test]
    fn folders_and_unreadable_files_are_refused_and_the_rest_taken() {
        let plan = plan(
            &[
                Offered::Folder,
                file("a.jpg", 1),
                Offered::Unreadable {
                    name: "gone.txt".to_owned(),
                },
            ],
            0,
            0,
        );
        assert_eq!(plan.taken, [1]);
        assert_eq!(plan.refused, Some(Refused::Folder));
        let plan = super::plan(
            &[Offered::Unreadable {
                name: "gone.txt".to_owned(),
            }],
            0,
            0,
        );
        assert_eq!(
            plan.refused,
            Some(Refused::Unreadable {
                name: "gone.txt".to_owned()
            })
        );
    }

    #[test]
    fn pictures_are_told_from_files_by_their_first_bytes() {
        assert_eq!(kind_of(b"\xFF\xD8\xFF\xE0", 10), TileKind::Image);
        assert_eq!(kind_of(b"\x89PNG\r\n\x1a\n", 10), TileKind::Image);
        assert_eq!(kind_of(b"RIFF\0\0\0\0WEBP", 10), TileKind::Image);
        assert_eq!(kind_of(b"GIF89a", 10), TileKind::Gif);
        assert_eq!(kind_of(b"%PDF-1.7", 10), TileKind::File);
        assert_eq!(
            kind_of(b"\0\0\0\x20ftypisom\0\0\x02\0", 10),
            TileKind::Video
        );
        assert_eq!(kind_of(b"\0\0\0\x14ftypqt  \0\0\0\0", 10), TileKind::Video);
        assert_eq!(kind_of(b"\0\0\0\x20ftypM4A \0\0\0\0", 10), TileKind::File);
        assert_eq!(
            kind_of(&[0x1A, 0x45, 0xDF, 0xA3, 0x9F], 10),
            TileKind::Video
        );
        assert_eq!(kind_of(b"RIFF\0\0\0\0AVI LIST", 10), TileKind::Video);
        // A video is a video at any size: its first frame is read, not the whole decoded.
        assert_eq!(
            kind_of(b"\0\0\0\x20ftypisom\0\0\x02\0", 90 << 20),
            TileKind::Video
        );
        assert_eq!(kind_of(b"\0\0\0\x18ftypheic", 10), TileKind::File);
    }

    #[test]
    fn a_picture_too_large_to_decode_is_a_file_tile() {
        assert_eq!(
            kind_of(b"\xFF\xD8\xFF\xE0", THUMBNAIL_SOURCE_MAX_BYTES + 1),
            TileKind::File
        );
    }

    #[test]
    fn pasted_pictures_are_numbered_from_two() {
        let held = ["image.png".to_owned(), "image (2).png".to_owned()];
        let taken = |name: &str| held.iter().any(|held| held == name);
        assert_eq!(pasted_name("image.png", |_| false), "image.png");
        assert_eq!(pasted_name("image.png", taken), "image (3).png");
        assert_eq!(pasted_name("imagem.png", taken), "imagem.png");
    }

    #[test]
    fn the_extension_block_shows_up_to_four_letters() {
        assert_eq!(extension_label("relatório.pdf"), "PDF");
        assert_eq!(extension_label("capture.webm.part"), "PART");
        assert_eq!(extension_label("notes.markdown"), "MARK");
        assert_eq!(extension_label("README"), "");
        assert_eq!(extension_label(".bashrc"), "");
    }
}
