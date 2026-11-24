//! Liquid glass on the floating surfaces (`docs/design.md` §5, Glass): the
//! window's compositor, which draws each glass surface as Rune's lens
//! (`rune_fx::Fx::lens`), bending what is behind it at its rim, frosting it
//! and tinting it, and otherwise draws the frame as Rune does.
//!
//! A glass surface is an `El::effect` slot marked [`GLASS_MARK`] that asks
//! for a copy of what is behind it (`effect_backdrop`); its four floats say
//! how glassy it is, its frost, and its tint colour and amount
//! ([`GlassData`]). The lens is a still image of the backdrop: nothing here
//! moves on its own, so the window rests as it does without the compositor.
//!
//! Owns the compositor and the mapping from the Glass setting to the lens;
//! which surfaces are glass is `design::Look`'s.

use rune::compose::{Compositor, Frame};
use rune::render::{EffectPhase, EffectSite, KIND_EFFECT};
use rune_fx::{Fx, FxFrame, FxLayer, FxOptions, FxRenderer, FxSlot, LensParams, Rgb, Shape};

use crate::design::{GLASS_MARK, GlassData};
use crate::shell::Shell;

/// Blur, in points, of a fully frosted lens (rune-fx's `LENS_FROST_SIGMA`).
const FULL_FROST_PT: f32 = 18.0;
/// The veil of the surface's colour over the bent backdrop at full glass:
/// never thinner, so text on a pill keeps a surface to read on.
const VEIL_LEAST: f32 = 0.35;
/// At full glass, the rim's width and how far it bends, in points: a pill's
/// share of its height, and no wider on a large surface, whose corners
/// would otherwise drag what is behind them across it.
const BEVEL_MOST_PT: f32 = 14.0;
const DEPTH_MOST_PT: f32 = 18.0;
/// The rim's width and bend as shares of a small surface's shorter side.
const BEVEL_SHARE: f32 = 0.35;
const DEPTH_SHARE: f32 = 0.45;

/// The lens for one glass slot (§5, Glass), after Apple's Liquid Glass: a
/// body that shows what is behind it, bent inwards at the rim. Every term
/// starts from the solid surface at 0 and moves with `glassy`: the veil
/// of the surface's colour thins, the rim widens and bends further,
/// and the frost eases to clear. The lens's own light (the rim's lit line
/// and sheen) stays off: on an interface it reads as a raised button
/// (Christian, 2026-11-23).
fn lens(data: [f32; 4], shape: Shape) -> FxLayer {
    let (glassy, blur_pt, word) = GlassData::unpack(data);
    let tint: Rgb = rune_fx::rgb(word);
    let g = glassy.clamp(0.0, 1.0);
    // How clear the body is: the frost and veil together hide what is
    // behind until they are nearly gone, so they follow the square root
    // and the slider's middle looks half way.
    let clear = g.sqrt();
    let side = shape.rect.w.min(shape.rect.h);
    Fx::lens(LensParams {
        bevel: (side * BEVEL_SHARE).min(BEVEL_MOST_PT) * g,
        depth: (side * DEPTH_SHARE).min(DEPTH_MOST_PT) * g,
        dispersion: 0.08 * g,
        magnification: 1.0 + 0.04 * g,
        presence: 1.0,
        frost: (blur_pt / FULL_FROST_PT).clamp(0.0, 1.0) * (1.0 - clear),
        shadow: 0.0,
        rim: 0.0,
        tint,
        tint_amount: 0.0,
        veil: 1.0 - (1.0 - VEIL_LEAST) * clear,
        ..LensParams::default()
    })
    .on(shape)
}

/// The window's compositor: Rune's frame, with a lens at each glass slot.
#[derive(Default)]
pub(crate) struct GlassCompositor {
    fx: Option<FxRenderer>,
}

impl Compositor<Shell> for GlassCompositor {
    fn render(&mut self, frame: &mut Frame<'_, Shell>) {
        let (device, queue, size) = (frame.device, frame.queue, frame.size);
        let marks = &frame.rt.marks;
        let mut at: Vec<u32> = Vec::new();
        let mut layers: Vec<FxLayer> = Vec::new();
        let mut slots: Vec<FxSlot> = Vec::new();
        for (index, instance) in frame.rt.instances.iter().enumerate() {
            if instance.params[1] != KIND_EFFECT
                || marks.get(index).copied().unwrap_or(0) != GLASS_MARK
            {
                continue;
            }
            let scale = instance.params[3];
            let [left, top, width, height] = instance.rect.map(|v| v / scale);
            let radius = instance
                .radii
                .iter()
                .fold(0.0_f32, |most, r| most.max(r.abs()))
                / scale;
            #[expect(clippy::cast_possible_truncation, reason = "an instance index")]
            at.push(index as u32);
            layers.push(lens(
                instance.uv,
                Shape::rounded(left, top, width, height, radius),
            ));
            slots.push(
                FxSlot::default()
                    .scale(scale)
                    .clip(instance.clip, instance.fade),
            );
        }
        let fx = self.fx.get_or_insert_with(|| {
            FxRenderer::new(device, frame.format, FxOptions { dither: true })
        });
        fx.set_srgb_blend(frame.srgb_blend());
        fx.add_format(device, wgpu::TextureFormat::Rgba16Float);
        let fx_frame = FxFrame {
            viewport: [size.0, size.1],
            scale: 1.0,
            layers: &layers,
            backdrop: None,
            sources: &[],
        };
        fx.prepare_with_slots(device, queue, &fx_frame, &slots);
        let clear = frame.rt.clear_color();
        frame.ui.prepare(
            device,
            queue,
            &mut frame.rt.text.atlas,
            &frame.rt.instances,
            size,
        );
        frame.ui.draw_with_backdrop(
            frame.encoder,
            frame.target,
            clear,
            &mut |phase, site: &EffectSite<'_>| {
                let Some(layer) = at.iter().position(|index| *index == site.index) else {
                    return;
                };
                match phase {
                    EffectPhase::Backdrop(encoder) => {
                        if let Some(backdrop) = site.backdrop {
                            fx.blur_backdrop(device, queue, encoder, backdrop, layer);
                        }
                    }
                    EffectPhase::Draw(pass) => {
                        fx.draw_layer_in(pass, site.format, site.origin, layer);
                    }
                }
            },
        );
    }
}
