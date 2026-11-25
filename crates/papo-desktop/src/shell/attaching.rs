//! The shell's side of adding files (`docs/design.md` §14.1): the paperclip
//! and Ctrl+O through the `FileChooser` portal, a paste of files or picture
//! data, a drop anywhere in the window. Each file is measured, checked
//! against the SDK's limits (`attaching::plan`) and copied into the SDK's
//! staging with `begin_staging`, so Send works while a copy still runs.
//!
//! Owns the platform work: the portal, the clipboard's forms, the drop and
//! opening the files. Following each copy is `copies`', the tray and its
//! saves the composer's, the rules `attaching`'s, the words `notice`'s.
//! Opening a file and starting its copy run on the SDK's runtime, never on
//! the UI thread.

use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use rune::{ClipboardContent, FileDialogOptions, Tx};

use super::Shell;
use super::copies::{Started, started_copies};
use crate::attaching::{self, HEAD_BYTES, Offered, Refused};
use crate::composer::attached::Closed;
use crate::composer::tray::TileKind;
use crate::notice::Refusal;
use crate::strings;
use crate::timeline::TimelineKey;

/// A file read for the tray: its name, size and kind, and what its copy
/// reads (the first bytes, already read, then the rest).
struct Opened {
    name: String,
    size_bytes: u64,
    kind: TileKind,
    reader: Box<dyn Read + Send>,
}

impl Shell {
    /// Where files offered now go: the shown draft, unless no field shows
    /// (a direct conversation's state pill stands in its place) or a
    /// message is being edited.
    pub(super) fn attach_target(&self) -> Result<TimelineKey, Closed> {
        if self.composer_hidden() {
            return Err(Closed::NoDraft);
        }
        self.composer.attach_target()
    }

    /// The paperclip, or Ctrl+O: the system's file chooser, several files
    /// at once. While a message is edited nothing opens and the notice
    /// says why.
    pub(crate) fn attach_pressed(&mut self, tx: &mut Tx) {
        match self.attach_target() {
            Ok(key) => {
                let options = FileDialogOptions {
                    title: strings::attach_chooser_title().to_owned(),
                    ..FileDialogOptions::default()
                };
                tx.open_files(
                    options,
                    move |shell: &mut Shell, answer, _tx| match answer {
                        Ok(Some(paths)) => shell.offer_paths(key, paths),
                        Ok(None) => {}
                        Err(error) => log::warn!("the file chooser did not open: {error}"),
                    },
                );
            }
            Err(Closed::Editing) => self.refuse(Refusal::FinishEdit, tx),
            Err(Closed::NoDraft) => {}
        }
    }

    /// Whether a file drag over the window is taken: a field shows, or a
    /// message is being edited (the card then says the edit must end).
    pub(crate) fn takes_drags(&self) -> bool {
        matches!(self.attach_target(), Ok(_) | Err(Closed::Editing))
    }

    /// Files dropped anywhere in the window (§14.1): they go to the shown
    /// conversation's tray. During an edit the drop takes nothing.
    pub(crate) fn files_dropped(&mut self, paths: &[PathBuf]) {
        self.drop_hover = false;
        if let Ok(key) = self.attach_target() {
            self.offer_paths(key, paths.to_vec());
        }
    }

    /// A file drag came over the window or left it: the window dims and
    /// shows the drop card while it is over.
    pub(crate) fn drop_hovered(&mut self, over: bool) {
        self.drop_hover = over;
    }

    /// What the drop card says while files are dragged over the window;
    /// none when no drag is over it or no conversation shows.
    pub(super) fn drop_target(&self) -> Option<crate::drop_card::DropTarget> {
        if !self.drop_hover {
            return None;
        }
        match self.attach_target() {
            Ok(_) => Some(crate::drop_card::DropTarget::Conversation(
                self.title().name(),
            )),
            Err(Closed::Editing) => Some(crate::drop_card::DropTarget::Editing),
            Err(Closed::NoDraft) => None,
        }
    }

    /// A paste into the field: files copied in a file manager and a
    /// picture alone become tiles; anything with text is left to the field
    /// (pasted text always stays text, even beside a picture).
    pub(crate) fn composer_paste(&mut self, content: &ClipboardContent, tx: &mut Tx) -> bool {
        let has_files = !content.files().is_empty();
        let picture = content.image().filter(|_| content.text().trim().is_empty());
        if !has_files && picture.is_none() {
            return false;
        }
        let key = match self.attach_target() {
            Ok(key) => key,
            Err(Closed::Editing) => {
                self.refuse(Refusal::FinishEdit, tx);
                return true;
            }
            Err(Closed::NoDraft) => return true,
        };
        if has_files {
            self.offer_paths(key, content.files().to_vec());
        } else if let Some(picture) = picture {
            self.offer_picture(key, picture.png().to_vec(), tx);
        }
        true
    }

    /// Reads `paths`, checks them against the tray of `key` and starts
    /// copying the ones it takes.
    fn offer_paths(&mut self, key: TimelineKey, paths: Vec<PathBuf>) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let (held, held_bytes) = self.composer.tray_load(key);
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let read = tokio::task::spawn_blocking(move || open_all(&paths)).await;
            let Ok(read) = read else {
                return;
            };
            let (offered, mut opened): (Vec<Offered>, Vec<Option<Opened>>) =
                read.into_iter().unzip();
            let plan = attaching::plan(&offered, held, held_bytes);
            let session = client.session(key.account);
            let mut started = Vec::new();
            let mut refused = plan.refused;
            for index in plan.taken {
                let Some(file) = opened.get_mut(index).and_then(Option::take) else {
                    continue;
                };
                let name = file.name.clone();
                let staging = session
                    .as_ref()
                    .map(|session| session.outgoing().begin_staging(file.reader));
                match staging {
                    Ok(Ok(pending)) => started.push(Started {
                        name: file.name,
                        size_bytes: file.size_bytes,
                        kind: file.kind,
                        pending,
                        picture: None,
                    }),
                    Ok(Err(error)) => {
                        log::warn!("a file was not staged: {error}");
                        refused.get_or_insert(Refused::Unreadable { name });
                    }
                    Err(error) => log::warn!("no session to stage into: {error}"),
                }
            }
            started_copies(&updates, key, started, refused);
        });
    }

    /// Starts copying a pasted picture into the tray of `key`. Its name
    /// ("image.png", "image (2).png") is given when its tile joins, so two
    /// quick pastes never share one.
    fn offer_picture(&mut self, key: TimelineKey, png: Vec<u8>, tx: &mut Tx) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let size_bytes = u64::try_from(png.len()).unwrap_or(u64::MAX);
        let (held, held_bytes) = self.composer.tray_load(key);
        let offered = [Offered::File {
            name: strings::attach_pasted_name().to_owned(),
            size_bytes,
        }];
        let plan = attaching::plan(&offered, held, held_bytes);
        if plan.taken.is_empty() {
            if let Some(refused) = plan.refused {
                self.refuse(Refusal::Attach(refused), tx);
            }
            return;
        }
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let staged = client
                .session(key.account)
                .map_err(papo_sdk::OutgoingError::from)
                .and_then(|session| session.outgoing().begin_staging(Cursor::new(png.clone())));
            match staged {
                Ok(pending) => {
                    let kind = attaching::kind_of(&png, size_bytes);
                    let started = Started {
                        // Named when the tile joins (`copies_started`).
                        name: String::new(),
                        size_bytes,
                        kind,
                        pending,
                        picture: Some(png),
                    };
                    started_copies(&updates, key, vec![started], None);
                }
                Err(error) => log::warn!("a pasted picture was not staged: {error}"),
            }
        });
    }
}

/// Measures and opens each of `paths`: a folder, an unreadable file, or a
/// file with its first bytes read for its kind.
fn open_all(paths: &[PathBuf]) -> Vec<(Offered, Option<Opened>)> {
    paths.iter().map(|path| open_one(path)).collect()
}

fn open_one(path: &Path) -> (Offered, Option<Opened>) {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let unreadable = || (Offered::Unreadable { name: name.clone() }, None);
    let Ok(metadata) = std::fs::metadata(path) else {
        return unreadable();
    };
    if metadata.is_dir() {
        return (Offered::Folder, None);
    }
    // A pipe or a device would block the copy, or never end.
    if !metadata.is_file() || name.is_empty() {
        return unreadable();
    }
    let Ok(mut file) = File::open(path) else {
        return unreadable();
    };
    let size_bytes = metadata.len();
    let mut head = Vec::with_capacity(HEAD_BYTES);
    if (&mut file)
        .take(HEAD_BYTES as u64)
        .read_to_end(&mut head)
        .is_err()
    {
        return unreadable();
    }
    let kind = attaching::kind_of(&head, size_bytes);
    let opened = Opened {
        name: name.clone(),
        size_bytes,
        kind,
        reader: Box::new(Cursor::new(head).chain(file)),
    };
    (Offered::File { name, size_bytes }, Some(opened))
}
