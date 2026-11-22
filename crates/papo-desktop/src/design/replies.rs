//! The measures of replying, editing and deleting (`docs/design.md` §11):
//! the bar over the text pill, its @ switch, the reply line over a message
//! and the deletion line under one.

/// The reply or edit bar's height, in points.
pub(crate) const REPLY_BAR_HEIGHT_PT: f32 = 32.0;
/// An icon in the reply or edit bar, in points.
pub(crate) const REPLY_BAR_ICON_PT: f32 = 16.0;
/// The @ switch's height and its inner side padding, in points.
pub(crate) const MENTION_SWITCH_PT: (f32, f32) = (20.0, 6.0);
/// The reply line's avatar, in points.
pub(crate) const REPLY_AVATAR_PT: f32 = 15.0;
/// The elbow from the avatar's centre up to the reply line: its stroke and
/// corner radius, in points.
pub(crate) const REPLY_ELBOW_PT: (f32, f32) = (2.0, 6.0);
/// The `reply-line` icon that stands for the elbow in Extra compact, in
/// points.
pub(crate) const REPLY_LINE_ICON_PT: f32 = 14.0;
/// The dashed square beside a deleted original, in points.
pub(crate) const REPLY_DELETED_PT: f32 = 15.0;
/// How long a jump's target holds `highlight.selected`, in seconds.
pub(crate) const JUMP_HIGHLIGHT_S: f64 = 1.5;
/// The tallest a message shown in a confirmation grows before it scrolls,
/// in points.
pub(crate) const CONFIRM_MESSAGE_MAX_PT: f32 = 240.0;
/// A deleting message's opacity until the server confirms.
pub(crate) const DELETING_OPACITY: f32 = 0.5;
