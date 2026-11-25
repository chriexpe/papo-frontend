//! The shell's side of the image editor (`docs/design.md` §14.10): opening
//! it on a tray tile (a click, Enter or Space, the menu's Edit) or on a
//! picture picked for the profile, reading the picture and what the SDK says
//! of it off the UI thread, the stage's input, and Save: a tile keeps its
//! name and edit until Send; a profile picture is cut and uploaded by the
//! SDK at once.
//!
//! The editor's state is `image_editor::state`'s, its look
//! `image_editor::view`'s.

use papo_sdk::{AccountId, AttachmentEdit, StagedAttachment};
use rune::{Drag, Image, Key, KeyEvent, Phase, TextEdit, Tx};

use super::Shell;
use super::staged_files::read_staged;
use crate::attaching::THUMBNAIL_SOURCE_MAX_BYTES;
use crate::composer::tray::{TileId, TileKind};
use crate::image_editor::crop::{Corner, Shape, ZOOM_RANGE};
use crate::image_editor::state::{Editor, Facts, Use};
use crate::settings::ProfilePicture;
use crate::timeline::TimelineKey;

/// The side the stage's picture is decoded to fit, in points: the stage's
/// width, twice, so a zoom keeps it sharp.
const STAGE_DECODE_PT: f32 = 1200.0;
/// The zoom slider's arrow step.
const ZOOM_KEY_STEP: f32 = 0.1;

/// What the editor edits.
#[derive(Clone)]
pub(crate) enum Target {
    /// Tile `id` of `key`'s tray.
    Tile { key: TimelineKey, id: TileId },
    /// A picture picked for `account`'s profile photo or banner.
    Profile {
        account: AccountId,
        staged: StagedAttachment,
    },
}

/// An open editor and what it shows.
pub(crate) struct OpenEditor {
    pub(crate) target: Target,
    pub(crate) editor: Editor,
    /// The picture at the stage's size, once read.
    pub(crate) picture: Option<Image>,
    pub(crate) kind: TileKind,
    /// The file's name as it is now.
    pub(crate) name: String,
    /// The file's size, when known.
    pub(crate) size_bytes: Option<u64>,
    /// The edit it was opened with: the crop to restore once the picture's
    /// size is known.
    opened_with: AttachmentEdit,
    /// This opening, so a read asked by an earlier one is dropped.
    opening: u64,
}

impl Shell {
    /// Opens the editor on tile `id` of the shown tray. A tile still
    /// copying doesn't open: its file can't be read yet.
    pub(crate) fn edit_tile(&mut self, id: TileId) {
        let opening = self.next_opening();
        let Some((key, _)) = self.composer.shown() else {
            return;
        };
        let Some(tile) = self.composer.tile(key, id) else {
            return;
        };
        if tile.preparing() {
            return;
        }
        let staged = tile.staged().cloned();
        let mut editor = Editor::attachment(&tile.name, tile.edit, tile.cleanable());
        editor.remember_typed(tile.typed_name.as_deref());
        draw_if_needed(&mut editor);
        self.editor = Some(OpenEditor {
            target: Target::Tile { key, id },
            editor,
            picture: None,
            kind: tile.kind.clone(),
            name: tile.name.clone(),
            size_bytes: tile.size_bytes,
            opened_with: tile.edit,
            opening,
        });
        if let (Some(staged), true) = (staged, matches!(tile.kind, TileKind::Image | TileKind::Gif))
        {
            self.read_for_editor(key.account, staged, opening);
        }
    }

    /// Opens the editor on a picture picked for `account`'s profile
    /// (`use_`, a photo or a banner), staged by the SDK.
    pub(crate) fn edit_profile_picture(
        &mut self,
        account: AccountId,
        use_: Use,
        staged: StagedAttachment,
    ) {
        let opening = self.next_opening();
        self.editor = Some(OpenEditor {
            target: Target::Profile {
                account,
                staged: staged.clone(),
            },
            editor: Editor::profile(use_),
            picture: None,
            kind: TileKind::Image,
            name: String::new(),
            size_bytes: Some(staged.size_bytes()),
            opened_with: AttachmentEdit::default(),
            opening,
        });
        self.read_for_editor(account, staged, opening);
    }

    /// The next opening's id.
    fn next_opening(&mut self) -> u64 {
        self.editor_openings += 1;
        self.editor_openings
    }

    /// Reads the picture at the stage's size and what the SDK says of it,
    /// for `opening`. A failed read lands too, with nothing.
    fn read_for_editor(&mut self, account: AccountId, staged: StagedAttachment, opening: u64) {
        let (Some(sdk), Some(images)) = (self.sdk.clone(), self.images.clone()) else {
            return;
        };
        let scale = self.scale;
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(account) else {
                updates.send(move |shell| shell.editor_read(opening, None, None));
                return;
            };
            let info = session.outgoing().inspect_image(&staged).await.ok();
            let picture = read_staged(&session, &staged, THUMBNAIL_SOURCE_MAX_BYTES)
                .await
                .map(|bytes| images.load_scaled(bytes, STAGE_DECODE_PT, scale));
            let facts = info.map(|info| Facts {
                width: info.width,
                height: info.height,
                format: format_name(info.format),
                size_bytes: staged.size_bytes(),
                animated: info.animated,
            });
            updates.send(move |shell| shell.editor_read(opening, facts, picture));
        });
    }

    /// The picture and its facts arrived for `opening`. A profile picture
    /// that can't be read or cut (an animation) closes the editor and says
    /// so on its row.
    fn editor_read(&mut self, opening: u64, facts: Option<Facts>, picture: Option<Image>) -> bool {
        let Some(open) = self.editor.as_mut().filter(|open| open.opening == opening) else {
            return false;
        };
        if let Target::Profile { account, .. } = open.target {
            let usable = facts.as_ref().is_some_and(|facts| !facts.animated);
            if !usable {
                let use_ = open.editor.use_;
                self.editor = None;
                self.picture_unusable(account, use_);
                return true;
            }
        }
        open.picture = picture;
        if let Some(facts) = facts {
            open.editor.inspected(facts, &open.opened_with);
            draw_if_needed(&mut open.editor);
        }
        true
    }

    fn editing(&mut self) -> Option<&mut Editor> {
        self.editor.as_mut().map(|open| &mut open.editor)
    }

    /// The name field's text, for Rune to edit; the field shows only while
    /// an editor is open.
    pub(crate) fn editor_name_mut(&mut self) -> &mut TextEdit {
        let Shell {
            editor,
            search_spare,
            ..
        } = self;
        editor.as_mut().map_or_else(
            || {
                *search_spare = TextEdit::new("");
                search_spare
            },
            |open| &mut open.editor.name,
        )
    }

    pub(crate) fn editor_name_changed(&mut self) {
        if let Some(editor) = self.editing() {
            editor.name_edited();
        }
    }

    pub(crate) fn editor_toggle_random_name(&mut self) {
        if let Some(editor) = self.editing() {
            editor.toggle_random_name();
            draw_if_needed(editor);
        }
    }

    pub(crate) fn editor_toggle_anonymize(&mut self) {
        if let Some(editor) = self.editing() {
            editor.toggle_anonymize();
            draw_if_needed(editor);
        }
    }

    /// Reroll: another random name in the field, Random name on.
    pub(crate) fn editor_reroll(&mut self) {
        if let Some(editor) = self.editing() {
            draw(editor);
        }
    }

    pub(crate) fn editor_shape(&mut self, shape: Shape) {
        if let Some(crop) = self.editing().and_then(|editor| editor.crop.as_mut()) {
            crop.set_shape(shape);
        }
    }

    pub(crate) fn editor_rotate(&mut self) {
        if let Some(crop) = self.editing().and_then(|editor| editor.crop.as_mut()) {
            crop.rotate();
        }
    }

    /// A key on the stage: arrows, + / −, 0 and R.
    pub(crate) fn editor_stage_key(&mut self, key: &KeyEvent) -> bool {
        self.editing().is_some_and(|editor| editor.stage_key(key))
    }

    /// A press and drag on the stage, or on a Free frame's `corner`.
    pub(crate) fn editor_drag(&mut self, corner: Option<Corner>, drag: &Drag) {
        let Some(editor) = self.editing() else {
            return;
        };
        match drag.phase {
            Phase::Down => editor.press((drag.x, drag.y), corner),
            Phase::Move => editor.drag((drag.x, drag.y), (drag.dx, drag.dy)),
            Phase::Up => editor.release(),
        }
    }

    /// The wheel over the stage zooms.
    pub(crate) fn editor_wheel(&mut self, dy: f32, precise: bool) -> bool {
        let Some(editor) = self.editing() else {
            return false;
        };
        editor.wheel(dy, precise);
        true
    }

    /// The zoom slider set to `share` of its range.
    pub(crate) fn editor_zoom_to(&mut self, share: f32) {
        if let Some(crop) = self.editing().and_then(|editor| editor.crop.as_mut()) {
            crop.set_zoom(ZOOM_RANGE.0 + share * (ZOOM_RANGE.1 - ZOOM_RANGE.0));
        }
    }

    /// The arrows on the zoom slider.
    pub(crate) fn editor_zoom_key(&mut self, key: &KeyEvent) -> bool {
        let Some(crop) = self.editing().and_then(|editor| editor.crop.as_mut()) else {
            return false;
        };
        let step = match key.key {
            Key::Left | Key::Down => -ZOOM_KEY_STEP,
            Key::Right | Key::Up => ZOOM_KEY_STEP,
            _ => return false,
        };
        let range = ZOOM_RANGE.1 - ZOOM_RANGE.0;
        crop.set_zoom(crop.zoom + step * range);
        true
    }

    /// A key anywhere in the dialog: Ctrl+Enter saves.
    pub(crate) fn editor_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        if key.key == Key::Enter && key.mods.ctrl {
            self.editor_save(tx);
            return true;
        }
        false
    }

    /// Whether the open editor can't save yet: a profile picture while
    /// another account call waits or the panel left its account.
    pub(crate) fn editor_held(&self) -> bool {
        match self.editor.as_ref().map(|open| &open.target) {
            Some(Target::Profile { account, .. }) => !self.profile_save_open(*account),
            Some(Target::Tile { .. }) | None => false,
        }
    }

    /// Tile `id` left its tray: an editor on it closes.
    pub(super) fn tile_left(&mut self, id: TileId) {
        let on_it = self.editor.as_ref().is_some_and(
            |open| matches!(open.target, Target::Tile { id: edited, .. } if edited == id),
        );
        if on_it {
            self.editor = None;
        }
    }

    /// Cancel, or Escape: nothing changes.
    pub(crate) fn editor_cancel(&mut self) {
        self.editor = None;
    }

    /// Save: a tile keeps its name and edit; a profile picture goes to the
    /// SDK, its row waiting for it.
    pub(crate) fn editor_save(&mut self, tx: &mut Tx) {
        let Some(open) = &self.editor else {
            return;
        };
        if !open.editor.can_save() {
            return;
        }
        match open.target.clone() {
            Target::Tile { key, id } => {
                let saved = open.editor.saved();
                self.editor = None;
                self.composer.tile_typed_name(key, id, saved.typed);
                let effects = self.composer.tile_edited(key, id, saved.name, saved.edit);
                self.run_composer(effects, Some(tx));
                tx.focus(crate::composer::tray_view::TRAY_KEY, true);
            }
            Target::Profile { account, staged } => {
                let picture = ProfilePicture {
                    account,
                    use_: open.editor.use_,
                    staged,
                    edit: open.editor.saved().edit,
                };
                self.save_profile_picture(picture);
            }
        }
    }
}

/// Draws a random name for `editor`'s field when Random name is on without
/// one.
fn draw_if_needed(editor: &mut Editor) {
    if editor.needs_draw() {
        draw(editor);
    }
}

/// Draws a random name for `editor`'s field from the system's entropy. With
/// none to draw from, the field keeps its name and the SDK draws at Send.
fn draw(editor: &mut Editor) {
    match papo_sdk::random_attachment_name(&editor.file_name()) {
        Ok(name) => editor.drew(&name),
        Err(error) => log::warn!("no random name could be drawn: {error}"),
    }
}

/// A format as the foot note names it: "JPEG", "PNG", "GIF", "WebP".
fn format_name(format: papo_sdk::ImageFormat) -> &'static str {
    match format {
        papo_sdk::ImageFormat::Jpeg => "JPEG",
        papo_sdk::ImageFormat::Png => "PNG",
        papo_sdk::ImageFormat::Gif => "GIF",
        papo_sdk::ImageFormat::Webp => "WebP",
        // A format the SDK learns later: its own label until named here.
        other => other.label(),
    }
}
