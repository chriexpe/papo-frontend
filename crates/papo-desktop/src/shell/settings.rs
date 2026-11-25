//! The shell's side of the settings panel (`docs/design.md` §8): opening
//! it from the account pill's ⚙ or Ctrl+, and closing it, Escape's order
//! (a choice's menu closes itself first, then the search clears, then the
//! panel closes), moving between pages, the choice menus, and each row's
//! change applied to the Appearance settings: drawn while a slider or a
//! colour is dragged, saved when it lets go, and otherwise saved at once.
//! A save that fails keeps the earlier value and shows the row's danger
//! line, with Try again.
//!
//! Owns these transitions; the panel's look is `settings`', the settings'
//! storage `appearance`'s.

use rune::{Key, KeyEvent, TextEdit, Tx};

use super::Shell;
use crate::appearance::Appearance;
use crate::design::{Background, Density, Theme};
use crate::general_settings::Language;
use crate::image_editor::state::Use;
use crate::menu::{MenuAction, MenuItem, OpenMenu, Placement};
use crate::settings::colour_field::ColourChange;
use crate::settings::rows::{density_name, language_name, theme_name};
use crate::settings::{
    Change, HeldEvent, PageId, Presence, RowId, SEARCH_KEY, SETTINGS_BUTTON_KEY, SettingsPanel,
    SizedMedia, Target,
};

/// What a choice's menu sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingChoice {
    Theme(Theme),
    Density(Density),
    Language(Language),
    Presence(Presence),
    /// Image size's or Video size's value.
    MediaSize(SizedMedia, papo_sdk_ui::MediaSizeSetting),
    /// A picture row's Change.
    ChangePicture(Use),
    /// The banner row's Remove.
    RemoveBanner,
}

impl Shell {
    /// The account pill's ⚙ or Ctrl+,: opens the panel on Appearance, or
    /// closes it.
    pub(crate) fn toggle_settings(&mut self, tx: &mut Tx) {
        if self.settings.is_some() {
            self.close_settings();
            tx.focus(SETTINGS_BUTTON_KEY, true);
            return;
        }
        self.settings = Some(SettingsPanel::new());
        tx.focus(SEARCH_KEY, true);
    }

    /// Closes the panel; a drag still running is dropped, not saved.
    pub(crate) fn close_settings(&mut self) {
        self.settings = None;
    }

    /// Escape with nothing left to clear, or a press outside: the panel
    /// closes and the account pill's ⚙ gets the keyboard back.
    pub(crate) fn dismiss_settings(&mut self, tx: &mut Tx) {
        self.close_settings();
        tx.focus(SETTINGS_BUTTON_KEY, true);
    }

    #[cfg(test)]
    pub(crate) fn settings(&self) -> Option<&SettingsPanel> {
        self.settings.as_ref()
    }

    /// Escape inside the panel: the search clears first, then the panel
    /// closes. A choice's menu takes its own Escape before this.
    pub(crate) fn settings_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        if key.key != Key::Escape {
            return false;
        }
        let Some(panel) = self.settings.as_mut() else {
            return false;
        };
        // An edit closes before the search does.
        if let Some((row, _)) = panel.editing.take() {
            tx.focus(row.key(), true);
            return true;
        }
        if panel.query().is_empty() {
            // The panel's own dismissal closes it.
            return false;
        }
        panel.search = TextEdit::new("");
        tx.focus(SEARCH_KEY, true);
        true
    }

    /// The search field's text, for its editor.
    pub(crate) fn settings_search_mut(&mut self) -> &mut TextEdit {
        let Shell {
            settings,
            search_spare,
            ..
        } = self;
        settings.as_mut().map_or_else(
            || {
                *search_spare = TextEdit::new("");
                search_spare
            },
            |panel| &mut panel.search,
        )
    }

    /// The search's ✕.
    pub(crate) fn settings_clear_search(&mut self, tx: &mut Tx) {
        if let Some(panel) = self.settings.as_mut() {
            panel.search = TextEdit::new("");
            tx.focus(SEARCH_KEY, true);
        }
    }

    /// A nav row, a sub-page's row or ←: shows `page`.
    pub(crate) fn settings_page(&mut self, page: PageId, _tx: &mut Tx) {
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        panel.go(page);
        match page {
            PageId::Diagnostics => self.measure_media_cache(),
            PageId::Profile => self.read_own_profile(),
            _ => {}
        }
    }

    /// A choice row: its menu under it, the current value ticked; `drawn`
    /// is the theme the window shows (the system's, until one is picked).
    pub(crate) fn settings_choice(&mut self, row: RowId, drawn: Theme, tx: &mut Tx) {
        let appearance = self.shown_appearance();
        let items: Vec<MenuItem> = match row {
            RowId::Theme => Theme::ALL
                .into_iter()
                .map(|theme| {
                    let chosen = theme == drawn;
                    MenuItem::plain(
                        &format!("settings-theme-{theme:?}"),
                        theme_name(theme),
                        MenuAction::Setting(SettingChoice::Theme(theme)),
                        false,
                    )
                    .with_radio(chosen)
                })
                .collect(),
            RowId::Density => Density::ALL
                .into_iter()
                .map(|density| {
                    MenuItem::plain(
                        &format!("settings-density-{density:?}"),
                        density_name(density),
                        MenuAction::Setting(SettingChoice::Density(density)),
                        false,
                    )
                    .with_radio(appearance.messages.density == density)
                })
                .collect(),
            RowId::Presence => {
                let current = crate::settings::rows::account::chosen_presence(
                    self.account_facts().and_then(|facts| facts.presence),
                );
                [Presence::Available, Presence::Away, Presence::Busy]
                    .into_iter()
                    .map(|presence| {
                        MenuItem::plain(
                            &format!("settings-presence-{presence:?}"),
                            crate::settings::rows::account::presence_name(presence),
                            MenuAction::Setting(SettingChoice::Presence(presence)),
                            false,
                        )
                        .with_radio(presence == current)
                    })
                    .collect()
            }
            RowId::Language => {
                let current = self.general.settings().language;
                std::iter::once(Language::System)
                    .chain(crate::strings::languages().map(|(id, _)| Language::Locale(id)))
                    .map(|language| {
                        let key = match language {
                            Language::System => "settings-language-system".to_owned(),
                            Language::Locale(id) => format!("settings-language-{id}"),
                        };
                        MenuItem::plain(
                            &key,
                            language_name(language),
                            MenuAction::Setting(SettingChoice::Language(language)),
                            false,
                        )
                        .with_radio(language == current)
                    })
                    .collect()
            }
            RowId::Photo | RowId::Banner => self.picture_menu(row),
            RowId::ImageSize => {
                size_items(SizedMedia::Images, self.conversations.settings().image_size)
            }
            RowId::VideoSize => {
                size_items(SizedMedia::Videos, self.conversations.settings().video_size)
            }
            _ => return,
        };
        self.nav.menu = Some(OpenMenu::new(
            Placement::BelowEnd(row.key()),
            row.key(),
            items,
        ));
        tx.focus(crate::menu::MENU_KEY, true);
    }

    /// A choice's menu item was picked.
    pub(crate) fn settings_chosen(&mut self, choice: SettingChoice, tx: &mut Tx) {
        let change = match choice {
            SettingChoice::ChangePicture(use_) => return self.pick_profile_picture(use_, tx),
            SettingChoice::RemoveBanner => return self.remove_banner(),
            SettingChoice::Theme(theme) => Change::Theme(theme),
            SettingChoice::Density(density) => Change::Density(density),
            SettingChoice::Language(language) => Change::Language(language),
            SettingChoice::Presence(presence) => Change::Presence(presence),
            SettingChoice::MediaSize(media, size) => Change::MediaSize(media, size),
        };
        self.settings_change(change, tx);
    }

    /// A row's change: drawn while a drag runs, else saved.
    pub(crate) fn settings_change(&mut self, change: Change, tx: &mut Tx) {
        if let Change::Presence(presence) = change {
            self.save_presence(presence);
            return;
        }
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        let other = super::settings_conversations::applied(self.conversations.settings(), &change)
            .map(Target::Conversations)
            .or_else(|| {
                super::settings_general::applied(self.general.settings(), &change)
                    .map(Target::General)
            });
        if let Some(target) = other {
            panel.held(HeldEvent::Ended);
            let current = self.conversations.settings();
            let resizes = matches!(&target, Target::Conversations(next)
                if next.image_size != current.image_size || next.video_size != current.video_size);
            self.settings_save(change.row(), target, tx);
            if resizes {
                // Every box takes the new size: pictures are decoded for it.
                self.want_media();
            }
            // Autoplay videos applies at once.
            self.reconcile_videos();
            return;
        }
        let current = panel
            .draft()
            .copied()
            .unwrap_or(*self.appearance.appearance());
        let colour = panel.colour;
        let (next, selected) = applied(&current, &change, colour.selected, colour.harmony);
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        panel.colour.selected = selected;
        if matches!(change, Change::Colours(ColourChange::Harmony)) {
            panel.colour.harmony += 1;
        }
        if dragging(&change) {
            panel.held(HeldEvent::Drew(next));
            return;
        }
        panel.held(HeldEvent::Ended);
        self.settings_save(change.row(), Target::Appearance(next), tx);
    }

    /// Saves `target` for `row`'s change; a failure shows under the row
    /// when the write comes back.
    pub(super) fn settings_save(&mut self, row: RowId, target: Target, tx: &mut Tx) {
        match target {
            Target::Appearance(appearance) => {
                self.saving_row = Some(row);
                self.set_appearance(appearance, tx);
            }
            Target::Conversations(settings) => self.set_conversations(settings, Some(row), tx),
            Target::General(settings) => self.set_general(settings, Some(row), tx),
            Target::Trusted(hosts) => {
                self.latest_rows.trusted = Some(row);
                if let Some(hosts) = self.content.trusted.replace(hosts) {
                    self.write_trusted(hosts, Some(row), tx);
                }
            }
            Target::Profile(change) => self.save_profile(row, change),
            Target::Presence(presence) => self.save_presence(presence),
            Target::Password => self.settings_password_save(tx),
            Target::ClearCache => self.clear_media_cache(tx),
            Target::ProfilePicture(picture) => self.save_profile_picture(picture),
            Target::RemoveBanner => self.remove_banner(),
            Target::PickPicture(use_) => self.pick_profile_picture(use_, tx),
        }
    }

    /// Try again under a row whose save failed: the same settings, saved
    /// again (not the change applied again, which would act on what is
    /// saved now: a colour added again, a harmony stepped again).
    pub(crate) fn settings_retry(&mut self, tx: &mut Tx) {
        let Some(panel) = self.settings.as_ref() else {
            return;
        };
        if let (Some(row), Some(target)) = (panel.failed_row(), panel.failed_target()) {
            self.settings_save(row, target, tx);
        }
    }

    /// A drag ended without letting go (Escape, the window losing focus,
    /// the pointer taken away): what it drew goes, and nothing is saved.
    pub(crate) fn settings_drag_cancelled(&mut self) {
        if let Some(panel) = self.settings.as_mut() {
            panel.held(HeldEvent::Ended);
        }
    }

    /// The settings drawn: a drag's draft while one runs, else the saved
    /// ones.
    pub(crate) fn shown_appearance(&self) -> Appearance {
        self.settings
            .as_ref()
            .and_then(SettingsPanel::draft)
            .copied()
            .unwrap_or(*self.appearance.appearance())
    }
}

/// Whether `change` is a drag still running: drawn, not saved.
fn dragging(change: &Change) -> bool {
    matches!(
        change,
        Change::Slider { done: false, .. }
            | Change::Colours(ColourChange::Move { done: false, .. })
    )
}

/// `current` with `change` applied, and the colour the field's arrows act
/// on after it.
fn applied(
    current: &Appearance,
    change: &Change,
    selected: usize,
    harmony: usize,
) -> (Appearance, usize) {
    let mut next = *current;
    let mut selected = selected;
    match change {
        Change::Theme(theme) => next.theme = Some(*theme),
        Change::Density(density) => next.messages.density = *density,
        Change::Switch(RowId::Grouped, on) => next.messages.grouped = *on,
        Change::Switch(RowId::Stacked, on) => next.messages.stacked = *on,
        Change::Switch(RowId::SystemFont, on) => next.system_font = *on,
        Change::Accent(accent) => next.accent = *accent,
        Change::Direction(direction) => next.shape = direction.shape(),
        Change::Slider {
            row: RowId::Strength,
            value,
            ..
        } => {
            let colours = current.background.colours().to_vec();
            next.background = Background::new(&colours, *value);
        }
        Change::Slider {
            row: RowId::Glass,
            value,
            ..
        } => next.glass = value.clamp(0.0, 1.0),
        Change::Slider {
            row: RowId::Grain,
            value,
            ..
        } => next.grain = value.clamp(0.0, 1.0),
        Change::Slider {
            row: RowId::Slider(value_of),
            value,
            ..
        } => next.shape = current.shape.with(*value_of, *value),
        // The other rows' changes are their stores' (Conversations',
        // General's).
        Change::Switch(..)
        | Change::Slider { .. }
        | Change::Hue(..)
        | Change::Language(_)
        | Change::MediaSize(..)
        | Change::Presence(_) => {}
        Change::Colours(colour) => {
            let (background, now) = super::settings_colours::colours_applied(
                &current.background,
                *colour,
                selected,
                harmony,
            );
            next.background = background;
            selected = now;
        }
    }
    (next, selected)
}

/// A size row's menu (Image size's or Video size's): each preset with its box, then Custom
/// (starting at the current box).
fn size_items(media: SizedMedia, current: papo_sdk_ui::MediaSizeSetting) -> Vec<MenuItem> {
    use papo_sdk_ui::MediaSizeSetting;
    let (width, height) = current.max_size_pt();
    [
        MediaSizeSetting::Small,
        MediaSizeSetting::Medium,
        MediaSizeSetting::Large,
        MediaSizeSetting::custom(width, height),
    ]
    .into_iter()
    .map(|size| {
        let chosen = std::mem::discriminant(&size) == std::mem::discriminant(&current);
        // Custom keeps its own sizes when it is already chosen.
        let picked = if chosen { current } else { size };
        let dims = (!matches!(size, MediaSizeSetting::Custom { .. })).then(|| {
            let (w, h) = size.max_size_pt();
            crate::strings::settings_size_dims(w, h)
        });
        MenuItem::plain(
            &format!(
                "settings-{}-size-{}",
                match media {
                    SizedMedia::Images => "image",
                    SizedMedia::Videos => "video",
                },
                crate::settings::rows::size_name(size)
            ),
            crate::settings::rows::size_name(size),
            MenuAction::Setting(SettingChoice::MediaSize(media, picked)),
            false,
        )
        .with_sub(dims)
        .with_radio(chosen)
    })
    .collect()
}
