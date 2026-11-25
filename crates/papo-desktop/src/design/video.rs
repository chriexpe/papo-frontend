//! The measures of video in messages and in the viewer (`docs/design.md` §14.5, §14.8; the D7
//! canvas's Video artboard for what §14.5 leaves unsized). Colours are the palette's, or white
//! and black at the alphas here: the marks sit on the picture, whatever the theme.

/// The control bar over a video's bottom: its height and its padding (top, sides), in points.
pub(crate) const VIDEO_BAR_PT: f32 = 44.0;
pub(crate) const VIDEO_BAR_PAD_PT: (f32, f32) = (8.0, 6.0);
/// Between the bar's controls, in points.
pub(crate) const VIDEO_BAR_GAP_PT: f32 = 4.0;
/// The black the bar's scrim reaches at its bottom (from none at its top).
pub(crate) const VIDEO_SCRIM_ALPHA: f32 = 0.55;
/// The bar's buttons: their side, their corner and the icon in them, in points; the white
/// behind one hovered.
pub(crate) const VIDEO_BUTTON_PT: f32 = 28.0;
pub(crate) const VIDEO_BUTTON_RADIUS_PT: f32 = 8.0;
pub(crate) const VIDEO_ICON_PT: f32 = 16.0;
pub(crate) const VIDEO_BUTTON_HOVER_ALPHA: f32 = 0.16;
/// The seek bar's and the volume slider's track: its thickness, the white of what is left,
/// and the seek bar's narrowest, in points.
pub(crate) const VIDEO_TRACK_PT: f32 = 4.0;
pub(crate) const VIDEO_TRACK_ALPHA: f32 = 0.3;
pub(crate) const VIDEO_SEEK_MIN_PT: f32 = 40.0;
/// The volume slider grown up out of the speaker on hover (Christian, 2026-11-25: upward, so
/// the speaker never moves): its track's length, the padding around it on its glass, and the
/// gap above the speaker.
pub(crate) const VIDEO_VOLUME_PT: f32 = 64.0;
pub(crate) const VIDEO_VOLUME_PAD_PT: f32 = 10.0;
pub(crate) const VIDEO_VOLUME_GAP_PT: f32 = 4.0;
/// How long the bar and the volume slider take to fade or grow in (none under reduced motion).
pub(crate) const VIDEO_FADE_S: f32 = 0.12;
/// The sound-off button at rest: its side and its inset from the corner, on dark glass (the
/// canvas's near-black at 55 %, blurred 8 pt).
pub(crate) const VIDEO_MUTE_PT: f32 = 28.0;
pub(crate) const VIDEO_MARK_INSET_PT: f32 = 8.0;
pub(crate) const VIDEO_GLASS_RGB: [u8; 3] = [16, 18, 24];
pub(crate) const VIDEO_GLASS_ALPHA: f32 = 0.55;
pub(crate) const VIDEO_GLASS_BLUR_PT: f32 = 8.0;
/// The play button of a video that waits: the disc, the black behind it and the icon, in
/// points. An album's video cell shows the same play mark.
pub(crate) const VIDEO_PLAY_PT: f32 = 48.0;
pub(crate) const VIDEO_PLAY_ALPHA: f32 = 0.5;
pub(crate) const VIDEO_PLAY_ICON_PT: f32 = 22.0;
/// The viewer's control bar: its height, and how long a still pointer leaves it while a video
/// plays, in seconds.
pub(crate) const VIEWER_VIDEO_BAR_PT: f32 = 48.0;
pub(crate) const VIEWER_BAR_IDLE_S: f64 = 2.0;
/// The seek a step takes: ←/→ on the seek bar (5 s), J and L in the viewer (10 s), in ms.
pub(crate) const VIDEO_SEEK_STEP_MS: u64 = 5_000;
pub(crate) const VIEWER_SEEK_STEP_MS: u64 = 10_000;
/// A volume step with the arrows on the volume slider.
pub(crate) const VIDEO_VOLUME_STEP: f32 = 0.1;
