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
    assert_eq!(kind_of(b"\xFF\xD8\xFF\xE0", 10, "a.jpg"), TileKind::Image);
    assert_eq!(kind_of(b"\x89PNG\r\n\x1a\n", 10, "a.png"), TileKind::Image);
    assert_eq!(kind_of(b"RIFF\0\0\0\0WEBP", 10, "a.webp"), TileKind::Image);
    assert_eq!(kind_of(b"GIF89a", 10, "a.gif"), TileKind::Gif);
    assert_eq!(kind_of(b"%PDF-1.7", 10, "a.pdf"), TileKind::File);
    assert_eq!(kind_of(b"\0\0\0\x18ftypheic", 10, "a.heic"), TileKind::File);
}

#[test]
fn videos_are_what_the_server_will_call_video() {
    let mp4 = b"\0\0\0\x20ftypisom\0\0\x02\0";
    assert_eq!(kind_of(mp4, 10, "clip.mp4"), TileKind::Video);
    assert_eq!(
        kind_of(b"\0\0\0\x14ftypqt  \0\0\0\0", 10, "a.mov"),
        TileKind::Video
    );
    assert_eq!(
        kind_of(&[0x1A, 0x45, 0xDF, 0xA3, 0x9F], 10, "a.webm"),
        TileKind::Video
    );
    assert_eq!(
        kind_of(b"RIFF\0\0\0\0AVI LIST", 10, "a.avi"),
        TileKind::Video
    );
    // A video is a video at any size: its first frame is read, not the whole decoded.
    assert_eq!(kind_of(mp4, 90 << 20, "clip.mp4"), TileKind::Video);
}

#[test]
fn audio_is_told_by_its_bytes_and_its_name_as_a_sent_message_will_be() {
    assert_eq!(
        kind_of(b"\0\0\0\x20ftypM4A \0\0\0\0", 10, "a.m4a"),
        TileKind::Audio
    );
    // Android's voice note: an MP4 the server calls video, named as audio.
    let voice = b"\0\0\0\x18ftypmp42\0\0\0\0";
    assert_eq!(
        kind_of(voice, 10, "Voice note 2026-11-25 10.00.00.m4a"),
        TileKind::Audio
    );
    assert_eq!(kind_of(voice, 10, "clip.mp4"), TileKind::Video);
    assert_eq!(kind_of(b"OggS\0\x02", 10, "track.ogg"), TileKind::Audio);
    assert_eq!(kind_of(b"ID3\x04\0", 10, "song.mp3"), TileKind::Audio);
    assert_eq!(
        kind_of(b"\xFF\xFB\x90\x64", 10, "song.mp3"),
        TileKind::Audio
    );
    assert_eq!(
        kind_of(b"RIFF\0\0\0\0WAVEfmt ", 10, "a.wav"),
        TileKind::Audio
    );
    // Unknown to the server, an audio name makes it audio: FLAC.
    assert_eq!(kind_of(b"fLaC\0\0\0\x22", 10, "a.flac"), TileKind::Audio);
    assert_eq!(kind_of(b"export const", 10, "main.ts"), TileKind::File);
}

#[test]
fn a_picture_too_large_to_decode_is_a_file_tile() {
    assert_eq!(
        kind_of(b"\xFF\xD8\xFF\xE0", THUMBNAIL_SOURCE_MAX_BYTES + 1, "a.jpg"),
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
