//! What each settings page holds (`docs/design.md` §8, Rows): its groups of
//! rows, each a title, an optional summary and its control, built from the
//! current settings. The pages draw these, and the search reads the same
//! rows, so a row is described once. Each nav page's rows are its own
//! module's (`account`, `appearance`, `conversations`, `general`,
//! `diagnostics`).
//!
//! Owns the description's shape and which module describes a page. Drawing
//! is `controls`'; what a change does is the shell's (`shell/settings.rs`).

use std::collections::BTreeSet;

use crate::appearance::Appearance;
use crate::appearance::presets::Direction;
use crate::conversation_settings::ConversationSettings;
use crate::design::Accent;
use crate::diagnostics::Link;
use crate::general_settings::GeneralSettings;
use crate::strings;
use account::AccountFacts;

use super::{PageId, RowId};

pub(crate) mod account;
mod appearance;
mod conversations;
mod diagnostics;
mod general;

pub(crate) use appearance::{
    accent_name, appearance_summary, density_name, direction_name, slider_value, theme_name,
};
pub(crate) use conversations::{HIGHLIGHT_HUES, conversations_summary};
pub(crate) use general::{general_summary, language_name};

/// A group of rows: one block of tiles, with an optional title.
pub(crate) struct Group {
    pub(crate) title: Option<String>,
    pub(crate) rows: Vec<Row>,
}

/// One setting.
pub(crate) struct Row {
    pub(crate) id: RowId,
    pub(crate) title: String,
    pub(crate) sub: Option<String>,
    pub(crate) kind: Kind,
}

/// A row's control (§8's row kinds that Appearance uses).
pub(crate) enum Kind {
    /// Flips at once.
    Switch(bool),
    /// A menu under the row; its current value shown.
    Choice(String),
    /// Opens a sub-page; its current value shown.
    SubPage(PageId, String),
    /// The accent swatches.
    Accents(Accent),
    /// A slider over a range; the value as shown.
    Slider(Slider),
    /// The background colour field with its tools and presets.
    ColourField,
    /// Shape and type's three starting points.
    Directions(Option<Direction>),
    /// Hue swatches (Message highlights), the chosen hue in OKLCH degrees.
    Hues(f32),
    /// A field that adds a trusted domain, with its Add button.
    DomainField,
    /// A list item with Remove: what it removes.
    Remove(String),
    /// A line of text with no control (an empty list's).
    Note,
    /// Text, edited in place (§8: Cancel and Save, a count, Enter and
    /// Escape): its saved value, what shows without one, its longest, and
    /// whether it can be edited yet (the profile has been read back).
    Text {
        value: Option<String>,
        placeholder: String,
        most: usize,
        editable: bool,
    },
    /// A blocked person, with Unblock: their name.
    Unblock(String),
    /// Change password's form.
    PasswordForm,
    /// Facts with no control: the title and summary say it all.
    Info,
    /// A value shown at the row's end, with no control.
    Value(String),
    /// Runs at once.
    Action,
    /// A profile picture (Photo, Banner): its picture at the row's end and a
    /// menu with Change, and Remove for a banner that is set.
    Picture(crate::image_editor::state::Use),
    /// Loses something, so it asks inside the row first: the question and
    /// its red confirmation; `asking` while it does, `busy` while it runs.
    Destructive {
        question: String,
        confirm: String,
        asking: bool,
        busy: bool,
    },
}

/// A slider's value, range and look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Slider {
    pub(crate) value: f32,
    pub(crate) min: f32,
    pub(crate) max: f32,
    /// What one arrow key moves it by.
    pub(crate) step: f32,
    /// What its track shows.
    pub(crate) track: Track,
}

/// What a slider's track draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Track {
    /// Colour strength (§4): from neutral grey to the colour.
    Strength,
    /// A plain track in the hover colour.
    Plain,
    /// Grain (§4): from clean to coarse, the thumb showing the grain.
    Grain,
}

/// What the rows are built from.
pub(crate) struct Context<'a> {
    pub(crate) appearance: &'a Appearance,
    pub(crate) conversations: &'a ConversationSettings,
    pub(crate) general: &'a GeneralSettings,
    pub(crate) trusted: &'a BTreeSet<String>,
    pub(crate) diagnostics: &'a DiagnosticsFacts,
    /// The panel's account; none when no account is signed in.
    pub(crate) account: Option<&'a AccountFacts>,
    pub(crate) system_dark: bool,
    /// Whether "Use the system font" can work: the desktop names one.
    pub(crate) system_font_known: bool,
    /// The panel's account's own photo and banner, once fetched.
    pub(crate) pictures: Option<&'a crate::shell::Pictures>,
}

/// What Diagnostics shows: the panel's account's server and connection,
/// and the media cache.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiagnosticsFacts {
    /// The server's name; none when no account is signed in.
    pub(crate) server: Option<String>,
    /// Its account's connection, once the SDK said.
    pub(crate) link: Option<Link>,
    pub(crate) cache_bytes: Option<u64>,
    pub(crate) clearing: bool,
    pub(crate) asking: Option<RowId>,
}

/// The groups of `page`.
pub(crate) fn page(page: PageId, cx: &Context<'_>) -> Vec<Group> {
    match page {
        PageId::Profile | PageId::Privacy | PageId::Blocked | PageId::Password => {
            account::of(page, cx)
        }
        PageId::Appearance => appearance::appearance(cx),
        PageId::Background => appearance::background(cx),
        PageId::Shape => appearance::shape(cx),
        PageId::Conversations => conversations::conversations(cx),
        PageId::Highlights => conversations::highlights(cx),
        PageId::General => general::general(cx),
        PageId::Trusted => general::trusted(cx),
        PageId::Diagnostics => diagnostics::diagnostics(cx),
    }
}

/// A page's title.
pub(crate) fn page_title(page: PageId) -> &'static str {
    match page {
        PageId::Profile => strings::settings_profile(),
        PageId::Privacy => strings::settings_privacy(),
        PageId::Blocked => strings::settings_blocked(),
        PageId::Password => strings::settings_password(),
        PageId::Appearance => strings::settings_appearance(),
        PageId::Background => strings::settings_background(),
        PageId::Shape => strings::settings_shape(),
        PageId::Conversations => strings::settings_conversations(),
        PageId::Highlights => strings::settings_highlights(),
        PageId::General => strings::settings_general(),
        PageId::Trusted => strings::settings_trusted(),
        PageId::Diagnostics => strings::settings_diagnostics(),
    }
}

/// What a search can match in `row`: its title, summary and value.
pub(crate) fn searchable(row: &Row) -> Vec<String> {
    let mut words = vec![row.title.clone()];
    words.extend(row.sub.clone());
    match &row.kind {
        Kind::Choice(value) | Kind::SubPage(_, value) | Kind::Value(value) => {
            words.push(value.clone());
        }
        Kind::Text { value, .. } => words.extend(value.clone()),
        Kind::Accents(accent) => words.push(accent_name(*accent).to_owned()),
        Kind::Directions(direction) => {
            words.extend(direction.map(|direction| direction_name(direction).to_owned()));
        }
        Kind::Slider(slider) => words.push(slider_value(row.id, slider.value)),
        Kind::Switch(_)
        | Kind::ColourField
        | Kind::Hues(_)
        | Kind::DomainField
        | Kind::Remove(_)
        | Kind::Note
        | Kind::Unblock(_)
        | Kind::PasswordForm
        | Kind::Info
        | Kind::Action
        | Kind::Picture(_)
        | Kind::Destructive { .. } => {}
    }
    words
}
