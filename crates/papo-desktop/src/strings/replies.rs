//! The strings of replying, editing and deleting (`docs/design.md` §11):
//! the reply and edit bars, the reply line, edits and deletions in flight,
//! and the delete confirmation. The catalog entries are `reply.*`, `edit.*`
//! and `delete.*`.

use super::{I18N, Key, catalog};

/// The reply bar's lead: "Replying to Lia".
pub(crate) fn reply_bar(name: &str) -> String {
    template!("reply.bar", "name" => name)
}

/// The reply bar when the message replied to is your own.
pub(crate) fn reply_bar_self() -> &'static str {
    text!("reply.bar_self")
}

/// The @ switch's spoken name: "Mention Lia".
pub(crate) fn reply_mention(name: &str) -> String {
    template!("reply.mention", "name" => name)
}

/// ✕ on the reply bar.
pub(crate) fn reply_cancel() -> &'static str {
    text!("reply.cancel")
}

/// A snippet for an image.
pub(crate) fn reply_image() -> &'static str {
    text!("reply.image")
}

/// A snippet for a video.
pub(crate) fn reply_video() -> &'static str {
    text!("reply.video")
}

/// A snippet for audio.
pub(crate) fn reply_audio() -> &'static str {
    text!("reply.audio")
}

/// A snippet for a GIF.
pub(crate) fn reply_gif() -> &'static str {
    text!("reply.gif")
}

/// A snippet for another file: "File · notes.pdf".
pub(crate) fn reply_file(name: &str) -> String {
    template!("reply.file", "name" => name)
}

/// A reply line whose original history knows was deleted.
pub(crate) fn reply_deleted() -> &'static str {
    text!("reply.deleted")
}

/// A reply line whose original is not available, and not known deleted.
pub(crate) fn reply_earlier() -> &'static str {
    text!("reply.earlier")
}

/// The spoken name of a reply line that opens its original, with what it
/// says: "Open the original message: Lia is the build green?".
pub(crate) fn reply_open(line: &str) -> String {
    template!("reply.open", "line" => line)
}

/// The edit bar's lead.
pub(crate) fn edit_bar() -> &'static str {
    text!("edit.bar")
}

/// The edit bar's hint.
pub(crate) fn edit_esc() -> &'static str {
    text!("edit.esc")
}

/// ✕ on the edit bar.
pub(crate) fn edit_cancel() -> &'static str {
    text!("edit.cancel")
}

/// Save in Send's place while editing.
pub(crate) fn edit_save() -> &'static str {
    text!("edit.save")
}

/// An edit on its way, spoken.
pub(crate) fn edit_saving() -> &'static str {
    text!("edit.saving")
}

/// An edit the server refused, spoken.
pub(crate) fn edit_failed() -> &'static str {
    text!("edit.failed")
}

/// The failed edit's menu: send it again.
pub(crate) fn edit_retry() -> &'static str {
    text!("edit.retry")
}

/// The failed edit's menu: keep the confirmed text.
pub(crate) fn edit_discard() -> &'static str {
    text!("edit.discard")
}

/// The line under a message the server did not delete.
pub(crate) fn delete_failed() -> &'static str {
    text!("delete.failed")
}

/// That line's action.
pub(crate) fn delete_try_again() -> &'static str {
    text!("delete.try_again")
}

/// The failed deletion line's other action: the deletion is dropped and
/// the message stays.
pub(crate) fn delete_dismiss() -> &'static str {
    text!("delete.dismiss")
}

/// A deletion on its way, spoken.
pub(crate) fn delete_deleting() -> &'static str {
    text!("delete.deleting")
}

/// The confirmation's title for your own message.
pub(crate) fn delete_title_own() -> &'static str {
    text!("delete.title_own")
}

/// The confirmation's title for someone else's: "Delete Lia's message?".
pub(crate) fn delete_title_other(name: &str) -> String {
    template!("delete.title_other", "name" => name)
}

/// The moderator's note.
pub(crate) fn delete_audit() -> &'static str {
    text!("delete.audit")
}

/// Cancel on a confirmation that destroys something.
pub(crate) fn button_cancel() -> &'static str {
    text!("button.cancel")
}

/// Delete on the delete confirmation.
pub(crate) fn button_delete() -> &'static str {
    text!("button.delete")
}
