//! The Appearance settings (`docs/design.md` §4 to §6 and §8): the theme,
//! the background colour and grain, the accent, message spacing and
//! grouping, Shape and type's five values, the system font and the Glass
//! switch. Every one is this device's, one value for every account: the
//! account's `theme`, `fontSize` and `messageDensity` cannot hold them
//! (B-016 to B-018), so none follows the account until the backend can,
//! and while nothing syncs message spacing is not kept per account
//! either (Christian, 2026-11-23).
//!
//! Owns the values, their defaults and their place in this device's
//! settings file (`rune::prefs`). A change is saved before it is kept, so
//! what the settings show is what is saved (§8, Saving). The tokens drawn
//! from them are `design`'s (`Look`).

use std::path::PathBuf;

use rune::prefs::Prefs;

use crate::design::{Accent, Background, MessageLayout, TYPE_BODY_PT, Theme};
use crate::window::APP_ID;

pub(crate) mod presets;
mod stored;

/// Shape and type's five values (§5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shape {
    /// `type.body`, in points.
    pub(crate) text_pt: f32,
    /// What every radius is multiplied by.
    pub(crate) roundness: f32,
    /// What every space is multiplied by.
    pub(crate) spacing: f32,
    /// How far glass blurs what is behind it, in points.
    pub(crate) blur_pt: f32,
    /// What every shadow is multiplied by.
    pub(crate) shadow_depth: f32,
}

/// One of Shape and type's values, for its slider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ShapeValue {
    TextSize,
    Roundness,
    Spacing,
    Blur,
    ShadowDepth,
}

impl ShapeValue {
    pub(crate) const ALL: [ShapeValue; 5] = [
        ShapeValue::TextSize,
        ShapeValue::Roundness,
        ShapeValue::Spacing,
        ShapeValue::Blur,
        ShapeValue::ShadowDepth,
    ];

    /// The range the slider covers (§5).
    pub(crate) fn range(self) -> (f32, f32) {
        match self {
            ShapeValue::TextSize => (TEXT_SIZE_MIN_PT, TEXT_SIZE_MAX_PT),
            ShapeValue::Roundness => (0.0, 1.6),
            ShapeValue::Spacing => (0.8, 1.25),
            ShapeValue::Blur => (0.0, 32.0),
            ShapeValue::ShadowDepth => (0.0, 1.5),
        }
    }
}

/// The Glass setting at first: frosted, half way to liquid.
pub(crate) const DEFAULT_GLASS: f32 = 0.5;

/// The smallest and largest text size, in points (§5).
pub(crate) const TEXT_SIZE_MIN_PT: f32 = 12.0;
pub(crate) const TEXT_SIZE_MAX_PT: f32 = 16.0;
/// How far Ctrl+= and Ctrl+- move the text size, in points.
pub(crate) const TEXT_SIZE_STEP_PT: f32 = 1.0;

impl Shape {
    pub(crate) fn get(&self, value: ShapeValue) -> f32 {
        match value {
            ShapeValue::TextSize => self.text_pt,
            ShapeValue::Roundness => self.roundness,
            ShapeValue::Spacing => self.spacing,
            ShapeValue::Blur => self.blur_pt,
            ShapeValue::ShadowDepth => self.shadow_depth,
        }
    }

    /// The same shape with `value` at `to`, held to its range.
    pub(crate) fn with(mut self, value: ShapeValue, to: f32) -> Shape {
        let (min, max) = value.range();
        let to = to.clamp(min, max);
        match value {
            ShapeValue::TextSize => self.text_pt = to,
            ShapeValue::Roundness => self.roundness = to,
            ShapeValue::Spacing => self.spacing = to,
            ShapeValue::Blur => self.blur_pt = to,
            ShapeValue::ShadowDepth => self.shadow_depth = to,
        }
        self
    }
}

impl Shape {
    /// Whether every value equals `other`'s, to the slider's resolution.
    pub(crate) fn same_as(&self, other: &Shape) -> bool {
        ShapeValue::ALL
            .into_iter()
            .all(|value| (self.get(value) - other.get(value)).abs() < 0.005)
    }
}

impl Default for Shape {
    /// The Balanced direction, Papo's defaults (§5).
    fn default() -> Shape {
        Shape {
            text_pt: TYPE_BODY_PT,
            roundness: 1.0,
            spacing: 1.0,
            blur_pt: 16.0,
            shadow_depth: 1.0,
        }
    }
}

/// Everything Appearance holds for the whole device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Appearance {
    /// The chosen theme; none until the person picks one, which follows
    /// the system (Graphite or Light).
    pub(crate) theme: Option<Theme>,
    pub(crate) background: Background,
    /// A fine noise over the surfaces (0–1).
    pub(crate) grain: f32,
    pub(crate) accent: Accent,
    /// Message spacing, Group and Stack (§6).
    pub(crate) messages: MessageLayout,
    pub(crate) shape: Shape,
    /// The system's interface font instead of Inter.
    pub(crate) system_font: bool,
    /// How glassy the floating surfaces are, 0–1: 0 solid, then frosted,
    /// towards 1 clear liquid glass (`docs/design.md` §5; Christian,
    /// 2026-11-23). One value for every theme: the conversation scrolls
    /// under the pills, so OLED's glass shows it too.
    pub(crate) glass: f32,
}

impl Default for Appearance {
    fn default() -> Appearance {
        Appearance {
            theme: None,
            background: Background::default(),
            grain: 0.0,
            accent: Accent::default(),
            messages: MessageLayout::default(),
            shape: Shape::default(),
            system_font: false,
            glass: DEFAULT_GLASS,
        }
    }
}

impl Appearance {
    /// The theme drawn: the chosen one, else the system's.
    pub(crate) fn theme_or(&self, system_dark: bool) -> Theme {
        self.theme
            .unwrap_or_else(|| Theme::following_system(system_dark))
    }
}

/// A write to the settings file that did not land; the setting keeps its
/// earlier value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotSaved;

/// The Appearance settings and where they are saved. A change shows at
/// once and is written off the UI thread (`Write`), one write at a time;
/// a failed write takes the settings back to what the file holds.
pub(crate) struct AppearanceStore {
    /// What draws: the saved settings, or a change on its way to the file.
    appearance: Appearance,
    /// What the file holds, as far as this store knows.
    saved: Appearance,
    /// The settings file; none for a store kept nowhere.
    path: Option<PathBuf>,
    saving: Saving,
}

/// The writes to the settings file: none running, or one running with the
/// latest change since waiting for it (§8: a change is saved, in order).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Saving {
    #[default]
    Idle,
    Writing {
        next: Option<Appearance>,
    },
}

/// What happens to the writes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SavingEvent {
    /// A change to write.
    Changed(Appearance),
    /// The running write landed, or did not.
    Written { ok: bool },
}

impl Saving {
    /// The writes after `event`, and what to write now.
    fn step(self, event: SavingEvent) -> (Saving, Option<Appearance>) {
        match (self, event) {
            (Saving::Idle, SavingEvent::Changed(appearance)) => {
                (Saving::Writing { next: None }, Some(appearance))
            }
            // The latest change waits; one before it that never ran is
            // replaced, as the file only needs the last.
            (Saving::Writing { .. }, SavingEvent::Changed(appearance)) => (
                Saving::Writing {
                    next: Some(appearance),
                },
                None,
            ),
            (Saving::Writing { next: Some(next) }, SavingEvent::Written { ok: true }) => {
                (Saving::Writing { next: None }, Some(next))
            }
            // Done, or failed: a failure drops the change waiting behind it,
            // whose write would fail too.
            (Saving::Writing { .. } | Saving::Idle, SavingEvent::Written { .. }) => {
                (Saving::Idle, None)
            }
        }
    }
}

/// One write of the settings, which may run on any thread.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Write {
    appearance: Appearance,
    path: Option<PathBuf>,
}

impl Write {
    /// The settings it writes.
    pub(crate) fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// Writes them, blocking: the file is read afresh first, so what
    /// another window saved since stays. A store kept nowhere has nothing
    /// to write.
    pub(crate) fn run(&self) -> Result<(), NotSaved> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut prefs = Prefs::at(path.clone());
        stored::write(&mut prefs, &self.appearance)
    }
}

impl Default for AppearanceStore {
    /// The defaults, kept nowhere: before the settings file is read, and in
    /// tests.
    fn default() -> AppearanceStore {
        AppearanceStore::read(&Prefs::memory())
    }
}

impl AppearanceStore {
    /// The settings in this user's settings file; a missing or unreadable
    /// value takes its default.
    pub(crate) fn load() -> AppearanceStore {
        AppearanceStore::read(&Prefs::open(APP_ID))
    }

    /// The settings in `prefs`, for tests that need a file of their own
    /// (one that cannot be written, say).
    #[cfg(test)]
    pub(crate) fn from_prefs(prefs: &Prefs) -> AppearanceStore {
        AppearanceStore::read(prefs)
    }

    fn read(prefs: &Prefs) -> AppearanceStore {
        let appearance = stored::read(prefs);
        AppearanceStore {
            appearance,
            saved: appearance,
            path: prefs.path().map(std::path::Path::to_path_buf),
            saving: Saving::Idle,
        }
    }

    /// The settings drawn.
    pub(crate) fn appearance(&self) -> &Appearance {
        &self.appearance
    }

    /// Shows `appearance` at once; the write it needs, unless one is
    /// running (it then follows that one) or nothing changed.
    pub(crate) fn change(&mut self, appearance: Appearance) -> Option<Write> {
        if appearance == self.appearance {
            return None;
        }
        self.appearance = appearance;
        self.step(SavingEvent::Changed(appearance))
    }

    /// The write of `appearance` landed (`ok`) or did not, which takes the
    /// settings back to what the file holds; the write to run next, if any.
    pub(crate) fn written(&mut self, appearance: Appearance, ok: bool) -> Option<Write> {
        if ok {
            self.saved = appearance;
        } else {
            self.appearance = self.saved;
        }
        self.step(SavingEvent::Written { ok })
    }

    fn step(&mut self, event: SavingEvent) -> Option<Write> {
        let (saving, write) = self.saving.step(event);
        self.saving = saving;
        write.map(|appearance| Write {
            appearance,
            path: self.path.clone(),
        })
    }
}

/// The text size one step from `text_pt` (`up` larger), held to its range,
/// on whole points: Ctrl+= and Ctrl+- (§12).
pub(crate) fn text_size_step(text_pt: f32, up: bool) -> f32 {
    let next = if up {
        (text_pt + TEXT_SIZE_STEP_PT).floor()
    } else {
        (text_pt - TEXT_SIZE_STEP_PT).ceil()
    };
    next.clamp(TEXT_SIZE_MIN_PT, TEXT_SIZE_MAX_PT)
}

/// Ctrl+0's text size: the default.
pub(crate) const TEXT_SIZE_DEFAULT_PT: f32 = TYPE_BODY_PT;

#[cfg(test)]
mod tests;
