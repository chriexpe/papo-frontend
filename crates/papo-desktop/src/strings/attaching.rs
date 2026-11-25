//! The strings of adding files and the tray (`docs/design.md` §14.1,
//! §14.2): the paperclip, the chooser, the drop card, the tiles' spoken
//! names and hints, and the refusals the notice says. The catalog entries
//! are `attach.*`, `drop.*` and `tray.*`.

use papo_sdk::{MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS_PER_MESSAGE, MAX_MESSAGE_ATTACHMENT_BYTES};

use super::{I18N, Key, catalog, size};
use crate::attaching::Refused;

/// The paperclip's label.
pub(crate) fn attach_button() -> &'static str {
    text!("attach.button")
}

/// Attaching's key, shown in the paperclip's hint.
pub(crate) fn shortcut_attach() -> &'static str {
    text!("shortcut.attach")
}

/// The file chooser's title.
pub(crate) fn attach_chooser_title() -> &'static str {
    text!("attach.chooser_title")
}

/// The name a pasted picture gets: "image.png".
pub(crate) fn attach_pasted_name() -> &'static str {
    text!("attach.pasted_name")
}

/// The drop card's title.
pub(crate) fn drop_title() -> &'static str {
    text!("drop.title")
}

/// The drop card's line: where the files go and the SDK's limits,
/// "#general · up to 10 files, 104.9 MB each".
pub(crate) fn drop_target_limits(channel: &str) -> String {
    let n = super::integer(i64::try_from(MAX_ATTACHMENTS_PER_MESSAGE).unwrap_or(i64::MAX));
    let size = size(MAX_ATTACHMENT_BYTES);
    template!("drop.target_limits", "channel" => channel, "n" => n, "size" => size)
}

/// The drop card while a message is edited, and the notice for files
/// offered then.
pub(crate) fn drop_editing() -> &'static str {
    text!("drop.editing")
}

/// The tray's spoken name: "Attachments, 3".
pub(crate) fn tray_label(count: usize) -> String {
    let n = super::integer(i64::try_from(count).unwrap_or(i64::MAX));
    template!("tray.label", "n" => n)
}

/// A tile's ✕: "Remove praia.jpg".
pub(crate) fn tray_remove(name: &str) -> String {
    template!("tray.remove", "name" => name)
}

/// A tile still copying, spoken: "Preparing praia.jpg".
pub(crate) fn tray_preparing(name: &str) -> String {
    template!("tray.preparing", "name" => name)
}

/// A tile's hint: "relatório.pdf · 2.4 MB".
pub(crate) fn tray_tile_tip(name: &str, size: &str) -> String {
    template!("tray.tile_tip", "name" => name, "size" => size)
}

/// A GIF's badge.
pub(crate) fn media_gif() -> &'static str {
    text!("media.gif")
}

/// The tile menu's Remove.
pub(crate) fn tray_remove_menu() -> &'static str {
    text!("tray.remove_menu")
}

/// A picture tile, spoken: "Image, praia.jpg, 2.4 MB".
pub(crate) fn tray_tile_image(name: &str, size: &str) -> String {
    template!("tray.tile_image", "name" => name, "size" => size)
}

/// A GIF tile, spoken.
pub(crate) fn tray_tile_gif(name: &str, size: &str) -> String {
    template!("tray.tile_gif", "name" => name, "size" => size)
}

/// A video tile, spoken: "Video, onda.mp4, 4.1 MB".
pub(crate) fn tray_tile_video(name: &str, size: &str) -> String {
    template!("tray.tile_video", "name" => name, "size" => size)
}

/// A file tile, spoken.
pub(crate) fn tray_tile_file(name: &str, size: &str) -> String {
    template!("tray.tile_file", "name" => name, "size" => size)
}

/// What the notice says about files the tray did not take.
pub(crate) fn attach_refused(refused: &Refused) -> String {
    match refused {
        Refused::TooMany => {
            let n = super::integer(i64::try_from(MAX_ATTACHMENTS_PER_MESSAGE).unwrap_or(i64::MAX));
            template!("attach.too_many", "n" => n)
        }
        Refused::TooLarge { name } => {
            template!("attach.too_large_one", "name" => name, "size" => size(MAX_ATTACHMENT_BYTES))
        }
        Refused::ManyTooLarge { count } => {
            let n = super::integer(i64::try_from(*count).unwrap_or(i64::MAX));
            template!("attach.too_large_many", "n" => n, "size" => size(MAX_ATTACHMENT_BYTES))
        }
        Refused::TooMuchTogether => {
            template!("attach.too_much_together", "size" => size(MAX_MESSAGE_ATTACHMENT_BYTES))
        }
        Refused::Folder => text!("attach.folder").to_owned(),
        Refused::Unreadable { name } => template!("attach.unreadable", "name" => name),
        Refused::CannotAnonymize { name } => super::attach_cannot_anonymize(name),
    }
}
