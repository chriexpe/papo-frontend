//! The strings of the image editor (`docs/design.md` §14.10) and of what
//! it sets: its title per use, the name field and Random name, the crop
//! controls and hint, Anonymize, the foot note, the tile marks, the
//! refusal of a file that can't be anonymized, the device setting, and
//! the profile's Photo and Banner rows. The catalog entries are
//! `editor.*`, `tray.anonymous`, `tray.renamed`, `tray.edit`,
//! `attach.cannot_anonymize`, `settings.anonymize*`, `settings.photo`,
//! `settings.banner`, `settings.image_*` and `settings.pictures`.

use super::{I18N, Key, catalog};

/// "Edit attachment".
pub(crate) fn editor_attachment() -> &'static str {
    text!("editor.attachment")
}

/// "Profile photo".
pub(crate) fn editor_avatar() -> &'static str {
    text!("editor.avatar")
}

/// "Banner".
pub(crate) fn editor_banner() -> &'static str {
    text!("editor.banner")
}

/// The name field's spoken name.
pub(crate) fn editor_file_name() -> &'static str {
    text!("editor.file_name")
}

/// "Random name": the switch, and what stands in the field while it is on.
pub(crate) fn editor_random_name() -> &'static str {
    text!("editor.random_name")
}

/// What Random name gives.
pub(crate) fn editor_random_name_sub() -> &'static str {
    text!("editor.random_name_sub")
}

/// The header's reroll button.
pub(crate) fn editor_reroll() -> &'static str {
    text!("editor.reroll")
}

/// The stage, spoken.
pub(crate) fn editor_crop_area() -> &'static str {
    text!("editor.crop_area")
}

/// The zoom slider's name.
pub(crate) fn editor_zoom() -> &'static str {
    text!("editor.zoom")
}

/// The crop shapes' group.
pub(crate) fn editor_crop() -> &'static str {
    text!("editor.crop")
}

/// "Original".
pub(crate) fn editor_original() -> &'static str {
    text!("editor.original")
}

/// "Free".
pub(crate) fn editor_free() -> &'static str {
    text!("editor.free")
}

/// "Square".
pub(crate) fn editor_square() -> &'static str {
    text!("editor.square")
}

/// "Rotate".
pub(crate) fn editor_rotate() -> &'static str {
    text!("editor.rotate")
}

/// The hint under the controls.
pub(crate) fn editor_hint() -> &'static str {
    text!("editor.hint")
}

/// The hint under the controls in Free.
pub(crate) fn editor_hint_free() -> &'static str {
    text!("editor.hint_free")
}

/// "Anonymize".
pub(crate) fn editor_anonymize() -> &'static str {
    text!("editor.anonymize")
}

/// What Anonymize keeps.
pub(crate) fn editor_anonymize_sub() -> &'static str {
    text!("editor.anonymize_sub")
}

/// The foot note of an untouched picture: "3151 × 2102 · JPEG · 1.8 MB".
pub(crate) fn editor_note(width: u32, height: u32, format: &str, size: &str) -> String {
    let (w, h) = (pixels(width), pixels(height));
    template!("editor.note", "w" => w, "h" => h, "format" => format, "size" => size)
}

/// The foot note once edited: "3151 × 2102 → 2102 × 2102 · JPEG".
pub(crate) fn editor_note_edited(
    width: u32,
    height: u32,
    cut_width: u32,
    cut_height: u32,
    format: &str,
) -> String {
    let (w, h) = (pixels(width), pixels(height));
    let (cw, ch) = (pixels(cut_width), pixels(cut_height));
    template!(
        "editor.note_edited",
        "w" => w,
        "h" => h,
        "cw" => cw,
        "ch" => ch,
        "format" => format
    )
}

/// The foot note of a file: "PDF · 2.4 MB".
pub(crate) fn editor_note_file(format: &str, size: &str) -> String {
    template!("editor.note_file", "format" => format, "size" => size)
}

/// The foot note of a profile picture: "Saved at 512 × 512".
pub(crate) fn editor_note_fixed(width: u32, height: u32) -> String {
    let (w, h) = (pixels(width), pixels(height));
    template!("editor.note_fixed", "w" => w, "h" => h)
}

/// "Cancel".
pub(crate) fn editor_cancel() -> &'static str {
    text!("editor.cancel")
}

/// "Save".
pub(crate) fn editor_save() -> &'static str {
    text!("editor.save")
}

/// A tile's anonymized mark, spoken.
pub(crate) fn tray_anonymous() -> &'static str {
    text!("tray.anonymous")
}

/// A tile's random-name mark, spoken.
pub(crate) fn tray_renamed() -> &'static str {
    text!("tray.renamed")
}

/// A tile's menu: "Edit".
pub(crate) fn tray_edit() -> &'static str {
    text!("tray.edit")
}

/// "`IMG_2041.HEIC` can't be anonymized."
pub(crate) fn attach_cannot_anonymize(name: &str) -> String {
    template!("attach.cannot_anonymize", "name" => name)
}

/// "Anonymize new attachments".
pub(crate) fn settings_anonymize() -> &'static str {
    text!("settings.anonymize")
}

/// What the setting covers.
pub(crate) fn settings_anonymize_sub() -> &'static str {
    text!("settings.anonymize_sub")
}

/// "Photo": the profile photo's row.
pub(crate) fn settings_photo() -> &'static str {
    text!("settings.photo")
}

/// "Banner": the profile banner's row.
pub(crate) fn settings_banner() -> &'static str {
    text!("settings.banner")
}

/// A picture row's menu: "Change".
pub(crate) fn settings_image_change() -> &'static str {
    text!("settings.image_change")
}

/// The banner row's menu: "Remove".
pub(crate) fn settings_image_remove() -> &'static str {
    text!("settings.image_remove")
}

/// The file chooser's filter for pictures.
pub(crate) fn settings_pictures() -> &'static str {
    text!("settings.pictures")
}

/// A pixel count, without grouping: sizes read as "3151 × 2102".
fn pixels(count: u32) -> String {
    count.to_string()
}
