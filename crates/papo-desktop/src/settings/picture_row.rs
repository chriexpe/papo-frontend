//! A profile picture's row (`docs/design.md` §8): Photo or Banner, the
//! picture at the row's end (a circle for the photo, a 3 : 1 band for the
//! banner, `surface.hover` while none is set or fetched), and ⌄, whose menu
//! holds Change and, for a banner that is set, Remove. While its save runs,
//! the row waits; while another row's runs, it can't be pressed (one
//! account call at a time).
//!
//! Owns the row's look; the menu and the calls are the shell's
//! (`shell/settings.rs`, `shell/profile_pictures.rs`).

use rune::{El, SurfaceFit, col, el, icon, row, text};

use super::RowId;
use super::controls::pressable;
use super::rows::Row;
use crate::design::settings::{ROW_INSET_PT, ROW_MIN_HEIGHT, ROW_PADDING};
use crate::design::{
    Look, PILL_OUTLINE_PT, RADIUS_CONTROL, ROW_ICON_PT, SPACE_LG, TYPE_BODY_MEDIUM,
};
use crate::icons::CHEVRON_DOWN;
use crate::image_editor::state::Use;
use crate::shell::{Pictures, Shell};

/// The photo's side at the row's end, in points.
const PHOTO_PT: f32 = 36.0;
/// The banner's height at the row's end, in points; its width is 3 ×.
const BANNER_HEIGHT_PT: f32 = 36.0;

/// The row for `setting`, a `use_` picture, drawn from `pictures`.
pub(super) fn view(
    setting: &Row,
    use_: Use,
    pictures: Option<&Pictures>,
    waiting: Option<RowId>,
    look: &Look,
) -> El {
    let id = setting.id;
    let picture = pictures.and_then(|pictures| match use_ {
        Use::Banner => pictures.banner.as_ref(),
        Use::Avatar | Use::Attachment => pictures.avatar.as_ref(),
    });
    let (width, height, radius) = match use_ {
        Use::Banner => (
            BANNER_HEIGHT_PT * 3.0,
            BANNER_HEIGHT_PT,
            look.radius(RADIUS_CONTROL),
        ),
        Use::Avatar | Use::Attachment => (PHOTO_PT, PHOTO_PT, PHOTO_PT / 2.0),
    };
    let mut shown = el()
        .size(width, height)
        .radius(radius)
        .clip_rounded(radius)
        .fill(look.palette.surface_hover)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .a11y_hidden();
    if let Some(image) = picture.filter(|image| image.error().is_none()) {
        shown = shown.pixels_fit(image.pixels(), SurfaceFit::Fill);
    }
    let line = row()
        .key(id.key())
        .w_full()
        .min_h(look.space(ROW_MIN_HEIGHT))
        .py(look.space(ROW_PADDING))
        .px(ROW_INSET_PT)
        .gap(look.space(SPACE_LG))
        .center_y()
        .child(
            col().grow(1.0).min_w(0.0).child(
                text(setting.title.clone())
                    .font(look.font(TYPE_BODY_MEDIUM))
                    .color(look.palette.text_primary),
            ),
        )
        .child(shown)
        .role("button")
        .label(setting.title.clone())
        .busy(waiting == Some(id));
    if waiting.is_some() {
        return line;
    }
    let drawn = look.theme();
    pressable(line, look)
        .on_press_tx(move |shell: &mut Shell, tx| shell.settings_choice(id, drawn, tx))
        .child(icon(&CHEVRON_DOWN, ROW_ICON_PT).color(look.palette.text_muted))
}
