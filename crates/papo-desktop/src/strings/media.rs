//! The strings of media in messages (`docs/design.md` §14.4, §14.6–§14.9): a picture's spoken
//! name and its failure, a file card's save, the media menu, the saved pill, the viewer, and the
//! Media and Files settings. The catalog entries are `media.*`, `file.*`, `menu.*`, `saved.*`,
//! `viewer.*` and `settings.*`.

use super::{I18N, Key, catalog};

/// A picture, spoken: "Image, praia.jpg".
pub(crate) fn media_image(name: &str) -> String {
    template!("media.image", "name" => name)
}

/// "Couldn't load the image".
pub(crate) fn media_load_failed() -> &'static str {
    text!("media.load_failed")
}

/// Retry, under a picture that could not load.
pub(crate) fn media_retry() -> &'static str {
    text!("media.retry")
}

/// A file card's Save.
pub(crate) fn file_save() -> &'static str {
    text!("file.save")
}

/// A file card's size line while it saves: "4.1 of 17.8 MB".
pub(crate) fn file_saving(done_bytes: u64, total_bytes: u64) -> String {
    let (done, total) = super::size_pair(done_bytes, total_bytes);
    template!("file.saving", "done" => done, "total" => total)
}

/// ✕ while a file saves.
pub(crate) fn file_cancel() -> &'static str {
    text!("file.cancel")
}

/// A saved file's size line: "17.8 MB · Saved".
pub(crate) fn file_saved(size: &str) -> String {
    template!("file.saved", "size" => size)
}

/// A saved file's Open.
pub(crate) fn file_open() -> &'static str {
    text!("file.open")
}

/// A saved file's Show in folder.
pub(crate) fn file_show() -> &'static str {
    text!("file.show")
}

/// A failed save's line, as what happened and the pressable "Try again" (the catalog marks it
/// between asterisks).
pub(crate) fn file_failed() -> (String, String) {
    split_action(text!("file.failed"))
}

/// `text` before its `*action*` and the action, each trimmed.
pub(super) fn split_action(text: &str) -> (String, String) {
    let mut parts = text.split('*');
    let said = parts.next().unwrap_or_default().trim().to_owned();
    let action = parts.next().unwrap_or_default().trim().to_owned();
    (said, action)
}

/// The media menu's Open.
pub(crate) fn menu_media_open() -> &'static str {
    text!("menu.media_open")
}

/// The media menu's Save image.
pub(crate) fn menu_save_image() -> &'static str {
    text!("menu.save_image")
}

/// The media menu's Save file.
pub(crate) fn menu_save_file() -> &'static str {
    text!("menu.save_file")
}

/// The media menu's Save as….
pub(crate) fn menu_save_as() -> &'static str {
    text!("menu.save_as")
}

/// The media menu's Copy image.
pub(crate) fn menu_copy_image() -> &'static str {
    text!("menu.copy_image")
}

/// The pill after a save: "Saved praia.jpg".
pub(crate) fn saved_pill(name: &str) -> String {
    template!("saved.pill", "name" => name)
}

/// The saved pill's Show in folder.
pub(crate) fn saved_show() -> &'static str {
    text!("saved.show")
}

/// The viewer, spoken: "praia.jpg, image 2 of 5", or "praia.jpg, image" alone.
pub(crate) fn viewer_label(name: &str, place: usize, count: usize) -> String {
    if count < 2 {
        return template!("viewer.label_one", "name" => name);
    }
    let i = super::integer(i64::try_from(place).unwrap_or(i64::MAX));
    let n = super::integer(i64::try_from(count).unwrap_or(i64::MAX));
    template!("viewer.label", "name" => name, "i" => i, "n" => n)
}

/// An album's place in the viewer's author pill: "2 of 5".
pub(crate) fn viewer_count(place: usize, count: usize) -> String {
    let i = super::integer(i64::try_from(place).unwrap_or(i64::MAX));
    let n = super::integer(i64::try_from(count).unwrap_or(i64::MAX));
    template!("viewer.count", "i" => i, "n" => n)
}

/// The viewer's ‹.
pub(crate) fn viewer_prev() -> &'static str {
    text!("viewer.prev")
}

/// The viewer's ›.
pub(crate) fn viewer_next() -> &'static str {
    text!("viewer.next")
}

/// The viewer's Zoom in.
pub(crate) fn viewer_zoom_in() -> &'static str {
    text!("viewer.zoom_in")
}

/// The viewer's Zoom out.
pub(crate) fn viewer_zoom_out() -> &'static str {
    text!("viewer.zoom_out")
}

/// The viewer's zoom value's name: pressing it fits the window.
pub(crate) fn viewer_zoom_fit() -> &'static str {
    text!("viewer.zoom_fit")
}

/// The viewer's Copy image.
pub(crate) fn viewer_copy() -> &'static str {
    text!("viewer.copy")
}

/// The viewer's Save.
pub(crate) fn viewer_save() -> &'static str {
    text!("viewer.save")
}

/// The viewer's Jump to message.
pub(crate) fn viewer_jump() -> &'static str {
    text!("viewer.jump")
}

/// Full screen, in the shortcuts sheet's Viewer group.
pub(crate) fn viewer_fullscreen() -> &'static str {
    text!("viewer.fullscreen")
}

/// The viewer's ✕.
pub(crate) fn viewer_close() -> &'static str {
    text!("viewer.close")
}

/// The Media group on Settings › Conversations.
pub(crate) fn settings_media() -> &'static str {
    text!("settings.media")
}

/// Image size.
pub(crate) fn settings_image_size() -> &'static str {
    text!("settings.image_size")
}

/// The size presets' names.
pub(crate) fn settings_size_small() -> &'static str {
    text!("settings.size_small")
}

/// Medium.
pub(crate) fn settings_size_medium() -> &'static str {
    text!("settings.size_medium")
}

/// Large.
pub(crate) fn settings_size_large() -> &'static str {
    text!("settings.size_large")
}

/// Custom.
pub(crate) fn settings_size_custom() -> &'static str {
    text!("settings.size_custom")
}

/// A preset's box beside its name in the menu: "448 × 400".
pub(crate) fn settings_size_dims(width_pt: u16, height_pt: u16) -> String {
    let w = super::integer(i64::from(width_pt));
    let h = super::integer(i64::from(height_pt));
    template!("settings.size_dims", "w" => w, "h" => h)
}

/// Max width.
pub(crate) fn settings_max_width() -> &'static str {
    text!("settings.max_width")
}

/// Max height.
pub(crate) fn settings_max_height() -> &'static str {
    text!("settings.max_height")
}

/// Animate GIFs and stickers.
pub(crate) fn settings_animate_gifs() -> &'static str {
    text!("settings.animate_gifs")
}

/// The Files group on Settings › General.
pub(crate) fn settings_group_files() -> &'static str {
    text!("settings.group_files")
}

/// Save media to.
pub(crate) fn settings_save_media_to() -> &'static str {
    text!("settings.save_media_to")
}

/// Change, on Save media to.
pub(crate) fn settings_change() -> &'static str {
    text!("settings.change")
}

/// The folder chooser's title.
pub(crate) fn settings_choose_folder() -> &'static str {
    text!("settings.choose_folder")
}

/// The shortcuts sheet's Viewer group.
pub(crate) fn shortcuts_viewer() -> &'static str {
    text!("shortcuts.viewer")
}

/// The shortcuts sheet's O on a focused message.
pub(crate) fn shortcuts_open_media() -> &'static str {
    text!("shortcuts.open_media")
}

#[cfg(test)]
mod tests {
    //! The action marked in a catalog line.

    use super::split_action;

    #[test]
    fn the_marked_action_is_split_off() {
        assert_eq!(
            split_action("Couldn't save the file. *Try again*"),
            ("Couldn't save the file.".to_owned(), "Try again".to_owned())
        );
    }
}
