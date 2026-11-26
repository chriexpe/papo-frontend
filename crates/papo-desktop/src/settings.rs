//! The settings panel (`docs/design.md` §8): grown from the account pill's
//! ⚙ (or Ctrl+,), a nav of pages with one-line summaries, search across
//! every row, sub-pages with ←, and rows that save as they change. The
//! account's pages are Profile and Privacy and security; the App pages are
//! Appearance, Conversations, General (each with its sub-pages) and
//! Diagnostics. Notifications comes with D9.
//!
//! Owns the panel's own state: the page shown, the search, a slider or the
//! colour field's value while it is dragged, the row whose save failed,
//! the colour field's selection, the domain being typed, the media cache's
//! measured size, the destructive row that is asking, the text row being
//! edited, the account call in flight and the password form. The settings
//! themselves are their stores' (`appearance`, `conversation_settings`,
//! `general_settings`, `trusted_domains`); what a row's change does is the
//! shell's (`shell/settings.rs`).

use std::collections::BTreeSet;

use rune::TextEdit;

use crate::appearance::presets::Direction;
use crate::appearance::{Appearance, ShapeValue};
use crate::conversation_settings::ConversationSettings;
use crate::design::{Accent, BackgroundColour, Density, Theme};
use crate::general_settings::{GeneralSettings, Language};
use papo_sdk::Uuid;
use papo_sdk::operations::profile::ProfileChange;

mod account_forms;
mod actions;
mod chip;
pub(crate) mod colour_field;
mod colour_tools;
mod controls;
pub(crate) use controls::switch;
pub(crate) mod domain_field;
mod held;
mod nav;
pub(crate) mod password_form;
mod picture_row;
pub(crate) mod rows;
mod search;
mod sizes;
pub(crate) use sizes::{SizeChoice, SizeSetting, SizedMedia};
mod slider;
mod swatches;
pub(crate) mod text_row;
pub(crate) mod view;

pub(crate) use account_forms::{PasswordForm, PasswordProblem, Presence};
pub(crate) use held::{Held, HeldEvent};
pub(crate) use view::view;

/// The open panel's element key.
pub(crate) const SETTINGS_KEY: &str = "settings";
/// The search field's element key.
pub(crate) const SEARCH_KEY: &str = "settings-search";
/// The account pill's ⚙, which opens the panel and gets the keyboard back
/// when it closes.
pub(crate) const SETTINGS_BUTTON_KEY: &str = "account-settings";

/// A page of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PageId {
    /// The account's profile: the nav's subject tile.
    Profile,
    Privacy,
    /// Privacy and security › Blocked people.
    Blocked,
    /// Privacy and security › Change password.
    Password,
    Appearance,
    /// Appearance › Background colour.
    Background,
    /// Appearance › Shape and type.
    Shape,
    Conversations,
    /// Conversations › Message highlights.
    Highlights,
    General,
    /// General › Trusted domains.
    Trusted,
    /// General › Embedded players.
    Players,
    Diagnostics,
}

impl PageId {
    /// Every page, nav pages first and in the nav's order: what the search
    /// looks through.
    pub(crate) const ALL: [PageId; 13] = [
        PageId::Profile,
        PageId::Privacy,
        PageId::Appearance,
        PageId::Conversations,
        PageId::General,
        PageId::Diagnostics,
        PageId::Blocked,
        PageId::Password,
        PageId::Background,
        PageId::Shape,
        PageId::Highlights,
        PageId::Trusted,
        PageId::Players,
    ];

    /// The account's pages, in the nav's order (Profile is its tile).
    pub(crate) const ACCOUNT: [PageId; 2] = [PageId::Profile, PageId::Privacy];

    /// The App pages in the nav, in order.
    pub(crate) const NAV: [PageId; 4] = [
        PageId::Appearance,
        PageId::Conversations,
        PageId::General,
        PageId::Diagnostics,
    ];

    /// The page a sub-page's ← goes back to.
    pub(crate) fn parent(self) -> Option<PageId> {
        match self {
            PageId::Profile
            | PageId::Privacy
            | PageId::Appearance
            | PageId::Conversations
            | PageId::General
            | PageId::Diagnostics => None,
            PageId::Background | PageId::Shape => Some(PageId::Appearance),
            PageId::Highlights => Some(PageId::Conversations),
            PageId::Trusted | PageId::Players => Some(PageId::General),
            PageId::Blocked | PageId::Password => Some(PageId::Privacy),
        }
    }

    /// The nav page it belongs to.
    pub(crate) fn nav(self) -> PageId {
        self.parent().unwrap_or(self)
    }

    /// Whether it is one of the account's pages, which need a signed-in
    /// account.
    pub(crate) fn of_account(self) -> bool {
        PageId::ACCOUNT.contains(&self.nav())
    }
}

/// One row, by what it sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RowId {
    /// The profile photo.
    Photo,
    /// The profile banner.
    Banner,
    /// "Anonymize new attachments".
    AnonymizeNew,
    Nickname,
    Status,
    About,
    Presence,
    TypingPhrase,
    Blocked,
    /// A blocked person, by their user id.
    BlockedPerson(Uuid),
    Password,
    /// Change password's form.
    PasswordForm,
    SignOut,
    Theme,
    Background,
    Accent,
    Density,
    Grouped,
    Stacked,
    Shape,
    SystemFont,
    Glass,
    EnterSends,
    ShowTimes,
    ShowAvatars,
    OpenAtNewest,
    Hourglass,
    ChannelDescription,
    Highlights,
    MentionHue,
    ReplyHue,
    /// Media › Image size.
    ImageSize,
    /// Media › a Custom Image size's Max width.
    MediaMaxWidth,
    /// Media › a Custom Image size's Max height.
    MediaMaxHeight,
    /// Media › Video size.
    VideoSize,
    /// Media › a Custom Video size's Max width.
    VideoMaxWidth,
    /// Media › a Custom Video size's Max height.
    VideoMaxHeight,
    /// Media › Embed size.
    EmbedSize,
    /// Media › a Custom Embed size's Max width.
    EmbedMaxWidth,
    /// Media › a Custom Embed size's Max height.
    EmbedMaxHeight,
    /// Show link previews.
    LinkPreviews,
    /// Media › Animate GIFs and stickers.
    AnimateGifs,
    /// Media › Autoplay videos.
    AutoplayVideos,
    /// Media › Keep playing when hidden.
    KeepPlaying,
    /// Files › Save media to.
    SaveMediaTo,
    Language,
    ConfirmLinks,
    Trusted,
    /// Trusted domains' field that adds one.
    TrustedAdd,
    /// A trusted domain, by its host's id (`trusted_domains::host_id`).
    TrustedHost(u64),
    /// General › Embedded players.
    Players,
    /// Play videos and music from other sites.
    PlayersOn,
    /// A site on Embedded players' list.
    PlayersSite(papo_sdk_ui::PlayerProvider),
    /// The notes under the sites.
    PlayersNote,
    Connected,
    Reconnect,
    Cache,
    ClearCache,
    Version,
    Field,
    Strength,
    Grain,
    Directions,
    Slider(ShapeValue),
}

impl RowId {
    /// Its element key, unique in the panel.
    pub(crate) fn key(self) -> String {
        format!("settings-row-{self:?}")
    }
}

/// What a row asks for; the shell applies it (`shell/settings.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Change {
    Theme(Theme),
    Density(Density),
    Switch(RowId, bool),
    Accent(Accent),
    Direction(Direction),
    /// A slider at `value`; `done` when the drag ends or a key moved it,
    /// which saves it.
    Slider {
        row: RowId,
        value: f32,
        done: bool,
    },
    /// The colour field's colours, strength aside.
    Colours(colour_field::ColourChange),
    /// A hue swatch picked, in OKLCH degrees.
    Hue(RowId, f32),
    /// Image size's, Video size's or Embed size's menu.
    MediaSize(SizeSetting),
    /// Language's menu.
    Language(Language),
    /// Presence's menu.
    Presence(Presence),
    /// Keep playing when hidden's menu.
    KeepPlaying(crate::keep_playing::KeepPlaying),
}

impl Change {
    /// The row it belongs to, for its danger line.
    pub(crate) fn row(&self) -> RowId {
        match self {
            Change::Theme(_) => RowId::Theme,
            Change::Density(_) => RowId::Density,
            Change::Switch(row, _) | Change::Slider { row, .. } | Change::Hue(row, _) => *row,
            Change::Accent(_) => RowId::Accent,
            Change::Direction(_) => RowId::Directions,
            Change::Language(_) => RowId::Language,
            Change::MediaSize(size) => size.media().row(),
            Change::Presence(_) => RowId::Presence,
            Change::KeepPlaying(_) => RowId::KeepPlaying,
            Change::Colours(_) => RowId::Field,
        }
    }
}

/// The colour field's own state: which colour the arrows move, which page
/// of presets shows, and the harmony arrangement the next press applies.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ColourState {
    pub(crate) selected: usize,
    pub(crate) preset_page: usize,
    pub(crate) harmony: usize,
}

/// What a row saves: the settings its store holds after the change.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    Appearance(Appearance),
    Conversations(ConversationSettings),
    General(GeneralSettings),
    Trusted(BTreeSet<String>),
    /// Embedded players' switch and allowed sites.
    Players(crate::embedded_players::PlayerSettings),
    /// A profile field (`papo_sdk`'s `change_profile`).
    Profile(ProfileChange),
    Presence(Presence),
    /// Change password: the form still holds what it sends.
    Password,
    /// Clear media cache, run again.
    ClearCache,
    /// A profile photo or banner, as the editor saved it.
    ProfilePicture(ProfilePicture),
    /// The banner removed.
    RemoveBanner,
    /// A pick that couldn't be used: Try again opens the chooser again.
    PickPicture(crate::image_editor::state::Use),
}

/// A profile photo or banner as the editor saved it: the staged picture
/// and the cut the SDK makes of it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProfilePicture {
    pub(crate) account: papo_sdk::AccountId,
    pub(crate) use_: crate::image_editor::state::Use,
    pub(crate) staged: papo_sdk::StagedAttachment,
    pub(crate) edit: papo_sdk::AttachmentEdit,
}

impl Target {
    /// Whether it holds its store's whole value (Appearance, Conversations,
    /// General, the trusted domains) rather than one field or one call.
    fn is_snapshot(&self) -> bool {
        matches!(
            self,
            Target::Appearance(_)
                | Target::Conversations(_)
                | Target::General(_)
                | Target::Trusted(_)
                | Target::Players(_)
        )
    }
}

/// A save that failed: the row that shows its danger line, and the
/// settings it was saving, which Try again saves as they were.
#[derive(Clone, Debug, PartialEq)]
struct Failed {
    row: RowId,
    target: Target,
}

/// The open panel.
pub(crate) struct SettingsPanel {
    page: PageId,
    pub(crate) search: TextEdit,
    /// The save that failed, if the last one under its row did.
    failed: Option<Failed>,
    /// A drag, drawn but not saved until it ends (§8: one write per
    /// change).
    held: Held,
    pub(crate) colour: ColourState,
    /// Trusted domains' field, and whether what it holds was refused.
    pub(crate) domain: TextEdit,
    pub(crate) domain_refused: bool,
    /// The media cache's size in bytes, once measured.
    pub(crate) cache_bytes: Option<u64>,
    /// The cache is being cleared.
    pub(crate) clearing: bool,
    /// The destructive row asking its question, if one is.
    pub(crate) asking: Option<RowId>,
    /// The text row being edited, and its field.
    pub(crate) editing: Option<(RowId, TextEdit)>,
    /// The row whose account call is in flight: its control waits.
    pub(crate) waiting: Option<RowId>,
    pub(crate) password: PasswordForm,
    /// The password was changed while the panel is open.
    pub(crate) password_changed: bool,
}

impl SettingsPanel {
    pub(crate) fn new() -> SettingsPanel {
        SettingsPanel {
            page: PageId::Appearance,
            search: TextEdit::new(""),
            failed: None,
            held: Held::Idle,
            colour: ColourState::default(),
            domain: TextEdit::new(""),
            domain_refused: false,
            cache_bytes: None,
            clearing: false,
            asking: None,
            editing: None,
            waiting: None,
            password: PasswordForm::default(),
            password_changed: false,
        }
    }

    pub(crate) fn page(&self) -> PageId {
        self.page
    }

    /// Shows `page`, the search cleared so the page shows.
    pub(crate) fn go(&mut self, page: PageId) {
        self.page = page;
        self.failed = None;
        self.asking = None;
        self.editing = None;
        self.password = PasswordForm::default();
        self.search = TextEdit::new("");
    }

    /// The search as typed, trimmed; empty when there is none.
    pub(crate) fn query(&self) -> &str {
        self.search.text.trim()
    }

    /// The row whose save failed, which shows the danger line.
    pub(crate) fn failed_row(&self) -> Option<RowId> {
        self.failed.as_ref().map(|failed| failed.row)
    }

    /// The settings whose save failed, for Try again.
    pub(crate) fn failed_target(&self) -> Option<Target> {
        self.failed.as_ref().map(|failed| failed.target.clone())
    }

    /// The save of `target` under `row` worked, or did not.
    pub(crate) fn saved(&mut self, row: RowId, target: Target, ok: bool) {
        if ok {
            // A later save to the same store supersedes the failed one: Try
            // again would put back a snapshot older than what is saved now.
            let same_store = self.failed.as_ref().is_some_and(|failed| {
                failed.target.is_snapshot()
                    && std::mem::discriminant(&failed.target) == std::mem::discriminant(&target)
            });
            if self.failed_row() == Some(row) || same_store {
                self.failed = None;
            }
        } else {
            self.failed = Some(Failed { row, target });
        }
    }

    /// The settings a drag draws, while one draws any.
    pub(crate) fn draft(&self) -> Option<&Appearance> {
        self.held.draft()
    }

    /// Where the colour being dragged sat at the press.
    pub(crate) fn drag_origin(&self) -> Option<BackgroundColour> {
        self.held.origin()
    }

    /// The drag moves on `event`.
    pub(crate) fn held(&mut self, event: HeldEvent) {
        self.held = self.held.step(event);
    }
}

#[cfg(test)]
mod tests;
