//! A drag in the settings panel (`docs/design.md` §8, Saving): a slider or
//! the colour field draws what it would set while it is held, and saves
//! only when it lets go.
//!
//! Owns the drag's states and their one step; the panel holds the current
//! one (`SettingsPanel::held`).

use crate::appearance::Appearance;
use crate::design::BackgroundColour;

/// A drag in the panel (§8, Saving): what it shows is drawn, not saved,
/// until it lets go.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Held {
    /// Nothing held.
    #[default]
    Idle,
    /// A colour on the field pressed, not moved yet: where it sat, which
    /// its drag moves it from however often the field is drawn meanwhile.
    Colour { origin: BackgroundColour },
    /// A slider, the field or a colour dragged: the settings as drawn,
    /// and a colour's `origin` while it is that colour's own drag.
    Drawing {
        draft: Appearance,
        origin: Option<BackgroundColour>,
    },
}

/// What happens to a drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HeldEvent {
    /// A colour's own drag pressed it where it sits.
    ColourPressed(BackgroundColour),
    /// The drag draws these settings.
    Drew(Appearance),
    /// It let go (the caller saves what it ends on) or was cancelled.
    Ended,
}

impl Held {
    /// The drag after `event`.
    pub(crate) fn step(self, event: HeldEvent) -> Held {
        match (self, event) {
            (_, HeldEvent::ColourPressed(origin)) => Held::Colour { origin },
            (Held::Colour { origin }, HeldEvent::Drew(draft)) => Held::Drawing {
                draft,
                origin: Some(origin),
            },
            (Held::Drawing { origin, .. }, HeldEvent::Drew(draft)) => {
                Held::Drawing { draft, origin }
            }
            (Held::Idle, HeldEvent::Drew(draft)) => Held::Drawing {
                draft,
                origin: None,
            },
            (_, HeldEvent::Ended) => Held::Idle,
        }
    }

    /// The settings the drag draws, while it draws any.
    pub(crate) fn draft(&self) -> Option<&Appearance> {
        match self {
            Held::Drawing { draft, .. } => Some(draft),
            Held::Idle | Held::Colour { .. } => None,
        }
    }

    /// Where the colour being dragged sat at the press.
    pub(crate) fn origin(&self) -> Option<BackgroundColour> {
        match *self {
            Held::Colour { origin } => Some(origin),
            Held::Drawing { origin, .. } => origin,
            Held::Idle => None,
        }
    }
}
