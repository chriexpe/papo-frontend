//! The measures of the image editor (`docs/design.md` §14.10): its stage,
//! the frame over it and the preview that stands for the stage when a file
//! can't be cropped. The dialog's own measures are the popups'.

use rune::Color;

/// The stage's fill, near black in every theme (it is about the picture):
/// `#0B0C0E`, in Rune's linear light.
pub(crate) const EDITOR_STAGE: Color = Color {
    r: 0.003_346_536,
    g: 0.003_676_507,
    b: 0.004_391_442,
    a: 1.0,
};
/// The frame's white edge, in points.
pub(crate) const EDITOR_FRAME_EDGE_PT: f32 = 2.0;
/// How dark the stage is outside the frame.
pub(crate) const EDITOR_OUTSIDE_ALPHA: f32 = 0.55;
/// The rule-of-thirds lines while the picture moves.
pub(crate) const EDITOR_THIRDS_ALPHA: f32 = 0.35;
/// A Free frame's corner handle, in points.
pub(crate) const EDITOR_HANDLE_PT: f32 = 14.0;
/// A GIF's or video's preview, where an image has its stage, in points.
pub(crate) const EDITOR_PREVIEW_HEIGHT_PT: f32 = 240.0;
/// A tile's marks (anonymized, random name), and their icons, in points:
/// smaller than ✕, so two sit clear of it on a 72 pt tile.
pub(crate) const TILE_MARK_PT: (f32, f32) = (18.0, 11.0);
