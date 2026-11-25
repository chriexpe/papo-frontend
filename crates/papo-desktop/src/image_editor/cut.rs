//! A picture drawn as an edit cuts it (`docs/design.md` §14.10): a tray
//! tile shows the crop and turns saved for it, so what will be sent is
//! what the tile shows, though the staged file stays untouched until Send.
//!
//! Pure geometry over Rune's `rotate`, which turns an element about its
//! centre.

use papo_sdk::AttachmentEdit;
use rune::{El, Image, SurfaceFit, el};

/// `picture` (of `size_px`, as displayed) cut by `edit` to cover a box of
/// `side` points square: the kept part centred and filling it.
pub(crate) fn cut_picture(
    picture: &Image,
    size_px: (u32, u32),
    edit: &AttachmentEdit,
    side: f32,
) -> El {
    #[expect(clippy::cast_precision_loss, reason = "picture sides")]
    let (width, height) = (size_px.0 as f32, size_px.1 as f32);
    let turned = edit.quarter_turns % 2 == 1;
    let (turned_w, turned_h) = if turned {
        (height, width)
    } else {
        (width, height)
    };
    #[expect(clippy::cast_precision_loss, reason = "pixel coordinates")]
    let (x, y, w, h) = edit.crop.map_or((0.0, 0.0, turned_w, turned_h), |crop| {
        (
            crop.x as f32,
            crop.y as f32,
            crop.width as f32,
            crop.height as f32,
        )
    });
    let scale = (side / w.max(1.0)).max(side / h.max(1.0));
    // Where the turned picture's centre sits so the kept part's centre is
    // the box's.
    let centre_x = side / 2.0 - (x + w / 2.0) * scale + turned_w * scale / 2.0;
    let centre_y = side / 2.0 - (y + h / 2.0) * scale + turned_h * scale / 2.0;
    let (box_w, box_h) = (width * scale, height * scale);
    el().size(side, side).clip_rounded(0.0).child(
        el().absolute(centre_x - box_w / 2.0, centre_y - box_h / 2.0)
            .size(box_w, box_h)
            .rotate(f32::from(edit.quarter_turns % 4) * std::f32::consts::FRAC_PI_2)
            .rotate_motion(rune::Spring::NONE)
            .pixels_fit(picture.pixels(), SurfaceFit::Fill),
    )
}
