//! The measures of audio in messages (`docs/design.md` §14.5; the D7 canvas's "Audio: waveform
//! player" artboard for what §14.5 leaves unsized). Colours are the palette's.

/// An audio file's capsule: its height (its ends fully round), in points.
pub(crate) const AUDIO_FILE_PT: f32 = 58.0;
/// The play disc on an audio file, and the icon in it, in points.
pub(crate) const AUDIO_PLAY_PT: f32 = 38.0;
pub(crate) const AUDIO_PLAY_ICON_PT: f32 = 18.0;
/// The space after the waveform's time, to the capsule's end, in points.
pub(crate) const AUDIO_END_PAD_PT: f32 = 16.0;
/// The waveform's bars: their width, the gap between them, and their shortest and tallest, in
/// points. Flat bars (peaks unknown) are the shortest.
pub(crate) const AUDIO_BAR_PT: f32 = 2.0;
pub(crate) const AUDIO_BAR_GAP_PT: f32 = 2.0;
pub(crate) const AUDIO_BAR_MIN_PT: f32 = 4.0;
pub(crate) const AUDIO_BAR_MAX_PT: f32 = 28.0;
/// The hairline over the waveform under the pointer, and the gap from the capsule's foot to
/// the time under it, in points.
pub(crate) const AUDIO_HAIRLINE_PT: f32 = 1.0;
pub(crate) const AUDIO_HOVER_TIME_GAP_PT: f32 = 4.0;
/// A step with ←/→ on the waveform, in ms.
pub(crate) const AUDIO_SEEK_STEP_MS: u64 = 5_000;
