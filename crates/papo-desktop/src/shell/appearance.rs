//! The shell's side of the Appearance settings: the frame's look made from
//! them and the system's preferences, a change applied (saved first, kept
//! only once saved, the message being read held in its place), and the
//! text-size keys (`docs/design.md` §12: Ctrl+= / Ctrl+- / Ctrl+0).

use rune::Tx;

use super::Shell;
use crate::appearance::{Appearance, ShapeValue, TEXT_SIZE_DEFAULT_PT, text_size_step};
use crate::design::{Glass, Look, LookSpec, Scale};
use crate::settings::RowId;
use crate::timeline::OpenTimeline;
use crate::timeline::anchor::{Anchor, scroll_to};

/// A text-size key's step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextSizeStep {
    Larger,
    Smaller,
    Default,
}

impl Shell {
    /// The frame's look: the Appearance settings, the theme following the
    /// system's light or dark when it is set to.
    pub(crate) fn look(&self, system_dark: bool) -> Look {
        look_of(&self.shown_appearance(), system_dark, self.system_font)
    }

    /// Shows `appearance` and saves it, the message being read kept in its
    /// place. The write runs off the UI thread; if it fails, the settings go
    /// back to what the file holds (`appearance_written`).
    pub(crate) fn set_appearance(&mut self, appearance: Appearance, tx: &mut Tx) {
        let reading = self.reading_place();
        if let Some(next) = self.appearance.change(appearance) {
            self.write_appearance(next, tx);
        }
        keep_place(reading, tx);
    }

    /// Writes `appearance`. The settings row that asked for it, if any, is
    /// taken now: it hears how the write went.
    fn write_appearance(&mut self, appearance: Appearance, tx: &mut Tx) {
        let write = self.appearance.write_of(&appearance);
        let row = self.saving_row;
        self.run_write(
            write,
            move |shell, ok, tx| shell.appearance_written(appearance, row, ok, tx),
            tx,
        );
    }

    /// The write of `appearance`, asked for by `row`, landed, or did not: a
    /// failure takes the settings back, the message being read kept in its
    /// place, and shows under the row.
    pub(crate) fn appearance_written(
        &mut self,
        appearance: Appearance,
        row: Option<RowId>,
        ok: bool,
        tx: &mut Tx,
    ) {
        let reading = (!ok).then(|| self.reading_place()).flatten();
        if !ok {
            log::warn!("an Appearance setting was not saved");
        }
        let next = self.appearance.written(appearance, ok);
        if let (Some(panel), Some(row)) = (self.settings.as_mut(), row) {
            panel.saved(row, appearance, ok);
        }
        keep_place(reading, tx);
        if let Some(next) = next {
            self.write_appearance(next, tx);
        }
    }

    /// The system's interface font was found (`system_font`): "Use the
    /// system font" can be offered, and if it is on, the text is set in it,
    /// the message being read kept in its place.
    pub(crate) fn system_font_found(&mut self, family: String, tx: &mut Tx) -> bool {
        if self.system_font == Some(family.as_str()) {
            return false;
        }
        let reading = self.reading_place();
        // A text style names its family for the program's life; the font is
        // found once a run, so this is one small string.
        self.system_font = Some(Box::leak(family.into_boxed_str()));
        if self.appearance.appearance().system_font {
            keep_place(reading, tx);
        }
        true
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
        // No settings row asks: a failed write goes back quietly.
        self.saving_row = None;
        self.set_appearance(changed, tx);
        true
    }
}

/// Text size, spacing or density changed: the message being read keeps its
/// place on screen, by its anchor (`docs/design.md` §5).
fn keep_place(reading: Option<Anchor>, tx: &mut Tx) {
    if let Some(anchor) = reading {
        scroll_to(anchor, tx);
    }
}

/// `appearance` as a frame's look, with the system's interface font when
/// that setting is on and one was found.
pub(crate) fn look_of(
    appearance: &Appearance,
    system_dark: bool,
    system_font: Option<&'static str>,
) -> Look {
    let theme = appearance.theme_or(system_dark);
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
            glassy: appearance.glass,
            blur_pt: shape.blur_pt,
            shadow_depth: shape.shadow_depth,
        },
        font_family: system_font.filter(|_| appearance.system_font),
    })
}
