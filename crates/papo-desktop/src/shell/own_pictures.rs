//! Your own profile photo and banner (`docs/design.md` §8, §14.10): a
//! picture picked through the file chooser is staged, framed in the image
//! editor, then cut, scaled and uploaded by the SDK
//! (`update_avatar_from_image`, `update_banner_from_image`). Only the banner
//! can be removed; the backend has no removal for a photo.
//!
//! Every outcome shows on the row: it waits while the SDK works, and a pick
//! that can't be used (unreadable, animated, a format the SDK can't cut) or
//! a save that failed leaves the row's danger line with Try again. Reading
//! the pictures back is `profile_pictures`'.

use std::path::PathBuf;

use papo_sdk::operations::profile::UpdateBannerRequest;
use papo_sdk::{AccountId, StagedAttachment};
use rune::{FileDialogOptions, Tx};

use super::SettingChoice;
use super::Shell;
use crate::image_editor::state::Use;
use crate::menu::{MenuAction, MenuItem};
use crate::settings::{ProfilePicture, RowId, Target};
use crate::strings;

/// What a profile picture may be picked as: stills the SDK can cut.
const PICKABLE: [&str; 4] = ["jpg", "jpeg", "png", "webp"];

/// The row a profile picture's use shows on.
pub(super) fn row_of(use_: Use) -> RowId {
    match use_ {
        Use::Banner => RowId::Banner,
        Use::Avatar | Use::Attachment => RowId::Photo,
    }
}

impl Shell {
    /// A picture row's menu: Change, and Remove for a banner that is set.
    pub(super) fn picture_menu(&self, row: RowId) -> Vec<MenuItem> {
        let use_ = if row == RowId::Banner {
            Use::Banner
        } else {
            Use::Avatar
        };
        let mut items = vec![MenuItem::plain(
            &format!("settings-picture-change-{row:?}"),
            strings::settings_image_change(),
            MenuAction::Setting(SettingChoice::ChangePicture(use_)),
            false,
        )];
        let banner_set = self
            .account_facts()
            .is_some_and(|facts| facts.own.has_banner);
        if row == RowId::Banner && banner_set {
            items.push(MenuItem::plain(
                "settings-picture-remove",
                strings::settings_image_remove(),
                MenuAction::Setting(SettingChoice::RemoveBanner),
                false,
            ));
        }
        items
    }

    /// Photo's or Banner's Change: the file chooser, then the editor on the
    /// picture picked.
    pub(crate) fn pick_profile_picture(&mut self, use_: Use, tx: &mut Tx) {
        let Some(account) = self.subject_account() else {
            return;
        };
        let title = match use_ {
            Use::Banner => strings::editor_banner(),
            Use::Avatar | Use::Attachment => strings::editor_avatar(),
        };
        let options = FileDialogOptions {
            title: title.to_owned(),
            ..FileDialogOptions::default()
        }
        .filter(strings::settings_pictures(), &PICKABLE);
        tx.open_file(
            options,
            move |shell: &mut Shell, answer, _tx| match answer {
                Ok(Some(path)) => shell.stage_profile_picture(account, use_, path),
                Ok(None) => {}
                Err(error) => log::warn!("the file chooser did not open: {error}"),
            },
        );
    }

    /// Stages the picture at `path` off the UI thread, then opens the
    /// editor on it while the panel still shows `account`.
    fn stage_profile_picture(&mut self, account: AccountId, use_: Use, path: PathBuf) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let staged = match client.session(account) {
                Ok(session) => stage(&session, path).await,
                Err(error) => {
                    log::warn!("a picture for the profile was not staged: {error}");
                    None
                }
            };
            updates.send(move |shell| {
                match staged {
                    Some(staged) => shell.picked(account, use_, staged),
                    None => shell.picture_unusable(account, use_),
                }
                true
            });
        });
    }

    /// The picture was staged: the editor opens on it, unless the panel
    /// closed or moved to another account meanwhile.
    fn picked(&mut self, account: AccountId, use_: Use, staged: StagedAttachment) {
        if self.subject_account() != Some(account) {
            return;
        }
        self.edit_profile_picture(account, use_, staged);
    }

    /// A pick can't be used: the row says so, and Try again opens the
    /// chooser again.
    pub(super) fn picture_unusable(&mut self, account: AccountId, use_: Use) {
        if self.subject_account() != Some(account) {
            return;
        }
        if let Some(panel) = self.settings.as_mut() {
            panel.saved(row_of(use_), Target::PickPicture(use_), false);
        }
    }

    /// Whether a profile picture can be saved for `account` now: the panel
    /// shows it and no other account call waits.
    pub(super) fn profile_save_open(&self, account: AccountId) -> bool {
        self.subject_account() == Some(account)
            && self
                .settings
                .as_ref()
                .is_some_and(|panel| panel.waiting.is_none())
    }

    /// Save in the editor on a profile picture: the dialog closes and the
    /// row waits for the SDK, then shows the new picture or its failure.
    /// Not while another call waits (Save is held then).
    pub(super) fn save_profile_picture(&mut self, picture: ProfilePicture) {
        if !self.profile_save_open(picture.account) {
            return;
        }
        self.editor = None;
        let read = self.own_picture_read(picture.account);
        let work = picture.clone();
        self.account_call(
            row_of(picture.use_),
            Target::ProfilePicture(picture),
            move |client| async move {
                let session = client.session(work.account)?;
                let profile = session.operations();
                let (turns, crop) = (work.edit.quarter_turns, work.edit.crop);
                match work.use_ {
                    Use::Banner => {
                        profile
                            .update_banner_from_image(&work.staged, turns, crop)
                            .await?;
                    }
                    Use::Avatar | Use::Attachment => {
                        profile
                            .update_avatar_from_image(&work.staged, turns, crop)
                            .await?;
                    }
                }
                if let Some(read) = read {
                    read.run(&session).await;
                }
                Ok(())
            },
        );
    }

    /// Banner's Remove: at once (§8: undone the same way).
    pub(crate) fn remove_banner(&mut self) {
        let Some(account) = self.subject_account() else {
            return;
        };
        let read = self.own_picture_read(account);
        self.account_call(
            RowId::Banner,
            Target::RemoveBanner,
            move |client| async move {
                let session = client.session(account)?;
                let request = UpdateBannerRequest {
                    banner: String::new(),
                    banner_format: String::new(),
                };
                session.operations().update_banner(request).await?;
                if let Some(read) = read {
                    read.run(&session).await;
                }
                Ok(())
            },
        );
    }
}

/// Opens `path` on a blocking worker and stages it; none when it can't be
/// read or staged.
async fn stage(session: &papo_sdk::Session, path: PathBuf) -> Option<StagedAttachment> {
    let opened = tokio::task::spawn_blocking(move || std::fs::File::open(path)).await;
    let Ok(Ok(file)) = opened else {
        log::warn!("a picture for the profile could not be read");
        return None;
    };
    match session.outgoing().stage_attachment(file).await {
        Ok(staged) => Some(staged),
        Err(error) => {
            log::warn!("a picture for the profile was not staged: {error}");
            None
        }
    }
}
