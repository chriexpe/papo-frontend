//! What presses on received media do (`docs/design.md` §14.4, §14.6, §14.7): open the viewer, the
//! media menu (Open, Save image or file with its folder, Save as…, Copy image; Copy link is
//! left out, B-024), Retry, and saving: into Save media to, or where the save dialog says, a
//! file card showing its progress and a save from the menu ending with the saved pill. Open on
//! a file saves it into the app's cache first, then hands it to the system; that copy is not
//! the card's save, which it neither shows nor stops.

use std::path::{Path, PathBuf};

use papo_sdk::{Attachment, SaveTarget};
use papo_sdk_ui::AttachmentKind;
use rune::{FileDialogOptions, Tx};

use super::Shell;
use crate::icons;
use crate::media_store::MediaKey;
use crate::menu::{Entry, MenuAction, MenuItem, OpenMenu, Placement};
use crate::message_media::received::{MediaAt, MediaPress, attachment_id, media_element_key};
use crate::saving::{SAVED_PILL_FOR_S, SaveView};
use crate::strings;

/// The media menu's width, in points.
const MEDIA_MENU_WIDTH_PT: f32 = 248.0;

/// What the media menu does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaAction {
    /// The viewer, or for a file the system's app.
    Open,
    /// Save into Save media to.
    Save,
    /// Save where the system's save dialog says.
    SaveAs,
    /// Copy the picture as PNG.
    CopyImage,
}

/// Where a save goes, and what happens once it is done.
enum SaveTo {
    /// Save media to; a file card shows its progress.
    MediaFolder { with_pill: bool },
    /// The file the save dialog named; it ends with the pill.
    Chosen(PathBuf),
}

impl Shell {
    /// Attachment `at`, from the message the shown timeline or the pins panel holds.
    pub(super) fn attachment_at(&self, at: MediaAt) -> Option<Attachment> {
        let message = self.message_at(at)?;
        message.attachments.as_ref()?.get(at.index).cloned()
    }

    /// A press on attachment `at`.
    pub(super) fn press_media(&mut self, at: MediaAt, press: MediaPress, tx: &mut Tx) {
        let Some(attachment) = self.attachment_at(at) else {
            return;
        };
        let Some(id) = attachment_id(&attachment) else {
            return;
        };
        let key = MediaKey {
            account: at.account,
            attachment: id,
        };
        match press {
            MediaPress::Open => self.media_action(at, MediaAction::Open, tx),
            MediaPress::Menu(point) => self.open_media_menu(at, &attachment, point),
            MediaPress::Retry => {
                let jobs = self.media.retry(key);
                self.run_media_jobs(jobs);
            }
            MediaPress::Save => {
                self.save_media(at, &attachment, SaveTo::MediaFolder { with_pill: false });
            }
            MediaPress::CancelSave => {
                self.saves.cancel(key);
            }
            MediaPress::OpenSaved | MediaPress::ShowSaved => {
                let views = self.saves.views();
                if let Some(SaveView::Saved(path)) = views.get(&key) {
                    let asked = if press == MediaPress::OpenSaved {
                        tx.open_path(path)
                    } else {
                        tx.reveal_path(path)
                    };
                    if let Err(error) = asked {
                        log::debug!("a saved file was not opened: {error}");
                    }
                }
            }
        }
    }

    /// What the media menu's item does for attachment `at`.
    pub(crate) fn media_action(&mut self, at: MediaAt, action: MediaAction, tx: &mut Tx) {
        let Some(attachment) = self.attachment_at(at) else {
            return;
        };
        let picture = AttachmentKind::of(&attachment).is_picture();
        match action {
            MediaAction::Open if picture => self.open_viewer(at, tx),
            MediaAction::Open => {
                let saved = attachment_id(&attachment).and_then(|id| {
                    let key = MediaKey {
                        account: at.account,
                        attachment: id,
                    };
                    match self.saves.views().get(&key) {
                        Some(SaveView::Saved(path)) => Some(path.clone()),
                        _ => None,
                    }
                });
                match saved {
                    Some(path) => {
                        if let Err(error) = tx.open_path(&path) {
                            log::debug!("a saved file was not opened: {error}");
                        }
                    }
                    None => self.open_file(at, &attachment),
                }
            }
            MediaAction::Save => {
                self.save_media(at, &attachment, SaveTo::MediaFolder { with_pill: picture });
            }
            MediaAction::SaveAs => {
                let name = attachment.original_file_name.clone().unwrap_or_default();
                let options = FileDialogOptions {
                    title: strings::menu_save_as().to_owned(),
                    directory: self.general.settings().media_folder(),
                    file_name: Some(name),
                    filters: Vec::new(),
                };
                // The attachment is kept, not looked up again: the conversation may change
                // while the dialog is open.
                tx.save_file(
                    options,
                    move |shell: &mut Shell, answer, _tx| match answer {
                        Ok(Some(path)) => shell.save_media(at, &attachment, SaveTo::Chosen(path)),
                        Ok(None) => {}
                        Err(error) => log::warn!("the save dialog did not open: {error}"),
                    },
                );
            }
            MediaAction::CopyImage => self.copy_image(at, &attachment),
        }
    }

    /// The media menu for attachment `at`, beside `point` (the pointer, or the media's corner
    /// for the menu key).
    fn open_media_menu(&mut self, at: MediaAt, attachment: &Attachment, point: Option<[f32; 2]>) {
        let kind = AttachmentKind::of(attachment);
        let picture = kind.is_picture();
        let action = |action| MenuAction::Media { at, action };
        let folder = self
            .general
            .settings()
            .media_folder()
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned());
        let (open_icon, save_label) = if picture {
            (&icons::IMAGE, strings::menu_save_image())
        } else {
            (&icons::EXTERNAL, strings::menu_save_file())
        };
        let mut entries = vec![
            Entry::Item(
                MenuItem::plain(
                    "menu-media-open",
                    strings::menu_media_open(),
                    action(MediaAction::Open),
                    false,
                )
                .with_icon(open_icon)
                .with_shortcut("O"),
            ),
            Entry::Item(
                MenuItem::plain(
                    "menu-media-save",
                    save_label,
                    action(MediaAction::Save),
                    false,
                )
                .with_icon(&icons::DOWNLOAD)
                .with_sub(folder),
            ),
            Entry::Item(
                MenuItem::plain(
                    "menu-media-save-as",
                    strings::menu_save_as(),
                    action(MediaAction::SaveAs),
                    false,
                )
                .with_icon(&icons::FOLDER),
            ),
        ];
        if picture {
            entries.push(Entry::Divider);
            entries.push(Entry::Item(
                MenuItem::plain(
                    "menu-media-copy-image",
                    strings::menu_copy_image(),
                    action(MediaAction::CopyImage),
                    false,
                )
                .with_icon(&icons::COPY),
            ));
        }
        let placement = match point {
            Some(point) => Placement::AtPointer(point),
            None => Placement::BelowEnd(media_element_key(at)),
        };
        self.nav.close_accounts();
        self.nav.menu = Some(OpenMenu::with_entries(
            placement,
            media_element_key(at),
            entries,
            MEDIA_MENU_WIDTH_PT,
        ));
    }

    /// Saves attachment `at` to `to`, following it until it ends.
    fn save_media(&mut self, at: MediaAt, attachment: &Attachment, to: SaveTo) {
        let (Some(sdk), Some(id)) = (self.sdk.clone(), attachment_id(attachment)) else {
            return;
        };
        let name = attachment.original_file_name.clone().unwrap_or_default();
        let key = MediaKey {
            account: at.account,
            attachment: id,
        };
        let (target, with_pill) = match to {
            SaveTo::MediaFolder { with_pill } => {
                let Some(folder) = self.general.settings().media_folder() else {
                    // No Downloads folder and none chosen: the save fails where it shows.
                    let serial = self.saves.started(key, || {}, with_pill);
                    let failed = papo_sdk::SaveState::Failed(papo_sdk::SaveFailure::Destination(
                        std::io::ErrorKind::NotFound,
                    ));
                    self.saves.updated(key, serial, &failed, &name);
                    return;
                };
                (
                    SaveTarget::Folder {
                        folder,
                        name: name.clone(),
                    },
                    with_pill,
                )
            }
            SaveTo::Chosen(path) => (SaveTarget::File(path), true),
        };
        // The save's handle lands in `slot` once the SDK runtime made it; ✕ before then still
        // stops it.
        let slot: std::sync::Arc<std::sync::Mutex<SaveSlot>> = std::sync::Arc::default();
        let cancel_slot = std::sync::Arc::clone(&slot);
        let serial = self.saves.started(
            key,
            move || {
                let mut slot = cancel_slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                slot.cancelled = true;
                if let Some(handle) = &slot.handle {
                    handle.cancel();
                }
            },
            with_pill,
        );
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(at.account) else {
                // The account went or is signed out: the save cannot reach the server.
                updates.send(move |shell| {
                    let failed = papo_sdk::SaveState::Failed(papo_sdk::SaveFailure::Unreachable);
                    shell.saves.updated(key, serial, &failed, &name)
                });
                return;
            };
            let mut handle = session.media().save_attachment(id, target);
            {
                let mut slot = slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if slot.cancelled {
                    handle.cancel();
                }
                slot.handle = Some(handle.clone());
            }
            while let Some(state) = handle.changed().await {
                let name = name.clone();
                updates.send_tx(move |shell, tx| {
                    let changed = shell.saves.updated(key, serial, &state, &name);
                    // Only a save that ends with the pill starts the pill's time.
                    if with_pill && matches!(state, papo_sdk::SaveState::Saved(_)) {
                        tx.wake_after(std::time::Duration::from_secs_f64(SAVED_PILL_FOR_S));
                        shell.saved_pill_until_s = Some(tx.clock() + SAVED_PILL_FOR_S);
                    }
                    changed
                });
            }
        });
    }

    /// Open on a file: it is saved into the app's cache, then the system opens it with its app.
    /// Apart from the card's save; a second Open while one runs does nothing.
    fn open_file(&mut self, at: MediaAt, attachment: &Attachment) {
        let (Some(sdk), Some(id)) = (self.sdk.clone(), attachment_id(attachment)) else {
            return;
        };
        let Some(folder) = crate::profile_dir::opened_files_dir() else {
            log::warn!("a file was not opened: the app has no cache folder");
            return;
        };
        let key = MediaKey {
            account: at.account,
            attachment: id,
        };
        if !self.opening.insert(key) {
            return;
        }
        let name = attachment.original_file_name.clone().unwrap_or_default();
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let saved = match client.session(at.account) {
                // The folder is made here, off the UI thread.
                Ok(session) => match std::fs::create_dir_all(&folder) {
                    Ok(()) => {
                        let target = SaveTarget::Folder { folder, name };
                        session.media().save_attachment(id, target).finished().await
                    }
                    Err(error) => Err(papo_sdk::SaveFailure::Destination(error.kind())),
                },
                Err(_) => Err(papo_sdk::SaveFailure::Unreachable),
            };
            updates.send_tx(move |shell, tx| {
                shell.opening.remove(&key);
                match saved {
                    Ok(path) => {
                        if let Err(error) = tx.open_path(&path) {
                            log::warn!("an opened file was not handed on: {error}");
                        }
                    }
                    // The design has no place for an Open that failed (left for later, see
                    // the D7b note): it is logged.
                    Err(failure) => log::warn!("a file was not opened: {failure:?}"),
                }
                false
            });
        });
    }

    /// Ends the saved pill once its time is over. Returns whether the view changed.
    pub(super) fn poll_saved_pill(&mut self, clock_s: f64) -> bool {
        if self
            .saved_pill_until_s
            .is_some_and(|until_s| clock_s >= until_s)
        {
            self.saved_pill_until_s = None;
            if let Some(serial) = self.saves.pill().map(|pill| pill.serial) {
                return self.saves.pill_gone(serial);
            }
        }
        false
    }

    /// The saved pill's Show in folder.
    pub(crate) fn saved_pill_show(&mut self, tx: &mut Tx) {
        if let Some(pill) = self.saves.pill()
            && let Err(error) = tx.reveal_path(&pill.path)
        {
            log::debug!("a saved file was not shown: {error}");
        }
    }
}

/// A save's handle once made, and whether ✕ came first.
#[derive(Default)]
struct SaveSlot {
    handle: Option<papo_sdk::MediaSave>,
    cancelled: bool,
}
