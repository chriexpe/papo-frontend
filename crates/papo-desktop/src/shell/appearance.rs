//! The shell's side of the Appearance settings: the frame's look made from
//! them and the system's preferences, a change applied (saved first, kept
//! only once saved, the message being read held in its place), and the
//! text-size keys (`docs/design.md` §12: Ctrl+= / Ctrl+- / Ctrl+0).

use rune::Tx;

use super::Shell;
use crate::appearance::{Appearance, NotSaved, ShapeValue, TEXT_SIZE_DEFAULT_PT, text_size_step};
use crate::design::{Glass, Look, LookSpec, Scale};
use crate::timeline::OpenTimeline;
use crate::timeline::anchor::{Anchor, scroll_to};

/// A text-size key's step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextSizeStep {
    Larger,
    Smaller,
    Default,
}

/// What the system asks of the look, read once per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SystemLook {
    pub(crate) dark: bool,
    /// Reduce transparency: every theme draws solid pills.
    pub(crate) reduced_transparency: bool,
}

impl Shell {
    /// The frame's look: the Appearance settings, as `system` allows.
    pub(crate) fn look(&self, system: SystemLook) -> Look {
        look_of(&self.shown_appearance(), system, self.system_font)
    }

    /// Saves `appearance` and shows it, the message being read kept in
    /// its place; a failed write keeps the earlier settings.
    pub(crate) fn set_appearance(
        &mut self,
        appearance: Appearance,
        tx: &mut Tx,
    ) -> Result<(), NotSaved> {
        let reading = self.reading_place();
        self.appearance.set(appearance)?;
        keep_place(reading, tx);
        Ok(())
    }

    /// The shown timeline's anchor before a change that reflows its rows.
    fn reading_place(&self) -> Option<Anchor> {
        self.tabs.shown().map(OpenTimeline::current_anchor)
    }

    /// The text size a whole point larger or smaller, or the default (§12:
    /// Ctrl+=, Ctrl+-, Ctrl+0). True when it changed.
    pub(crate) fn step_text_size(&mut self, step: TextSizeStep, tx: &mut Tx) -> bool {
        let appearance: Appearance = *self.appearance.appearance();
        let current = appearance.shape.text_pt;
        let next = match step {
            TextSizeStep::Larger => text_size_step(current, true),
            TextSizeStep::Smaller => text_size_step(current, false),
            TextSizeStep::Default => TEXT_SIZE_DEFAULT_PT,
        };
        if (next - current).abs() < f32::EPSILON {
            return false;
        }
        let mut changed = appearance;
        changed.shape = appearance.shape.with(ShapeValue::TextSize, next);
        // Nothing to show a failed write under: the size stays as it was.
        self.set_appearance(changed, tx).is_ok()
    }
}

/// Text size, spacing or density changed: the message being read keeps its
/// place on screen, by its anchor (`docs/design.md` §5).
fn keep_place(reading: Option<Anchor>, tx: &mut Tx) {
    if let Some(anchor) = reading {
        scroll_to(anchor, tx);
    }
}

/// `appearance` as a frame's look under `system`, with the system's
/// interface font when that setting is on and one was found.
pub(crate) fn look_of(
    appearance: &Appearance,
    system: SystemLook,
    system_font: Option<&'static str>,
) -> Look {
    let theme = appearance.theme_or(system.dark);
    let shape = appearance.shape;
    Look::with(&LookSpec {
        theme,
        accent: appearance.accent,
        background: appearance.background,
        scale: Scale {
            text_pt: shape.text_pt,
            spacing: shape.spacing,
            roundness: shape.roundness,
        },
        glass: Glass {
            on: appearance.glass(theme) && !system.reduced_transparency,
            blur_pt: shape.blur_pt,
            shadow_depth: shape.shadow_depth,
        },
        font_family: system_font.filter(|_| appearance.system_font),
    })
}
