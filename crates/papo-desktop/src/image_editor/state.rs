//! The editor's state (`docs/design.md` §14.10): what it edits, the name
//! field and Random name, Anonymize, the cropper and the gesture on its
//! stage, and what Save keeps. One state per open dialog; the shell opens
//! and closes it and carries Save out (`shell/editing.rs`).
//!
//! Pure: no SDK call, no I/O. The picture's facts arrive from the shell
//! once the SDK inspected the staged file.

use papo_sdk::{Anonymize, AttachmentEdit, CropRect};
use rune::{Key, KeyEvent, TextEdit};

use super::crop::{Corner, Crop, Rect};

/// The stage's height, in points; its width is the dialog's.
pub(crate) const STAGE_HEIGHT_PT: f32 = 320.0;
/// The dialog's width, in points.
pub(crate) const DIALOG_WIDTH_PT: f32 = 600.0;
/// A wheel notch or + / − zooms by this much.
const ZOOM_STEP: f32 = 1.1;
/// How far a precise wheel (a trackpad) moves for one zoom step, in points.
const WHEEL_STEP_PT: f32 = 40.0;
/// An arrow moves the picture this far, in points; with ⇧, 1.
const ARROW_STEP_PT: (f32, f32) = (10.0, 1.0);
/// Characters a file name may not hold.
const FORBIDDEN: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// What the editor edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Use {
    /// A tray tile: crop shapes, name, Random name, Anonymize.
    Attachment,
    /// The profile photo: a circle, 1 : 1.
    Avatar,
    /// The profile banner: 3 : 1.
    Banner,
}

impl Use {
    /// The fixed shape a use crops to; none for an attachment's choice.
    pub(crate) fn fixed_shape(self) -> Option<f32> {
        match self {
            Use::Attachment => None,
            Use::Avatar => Some(1.0),
            Use::Banner => Some(3.0),
        }
    }
}

/// What the SDK said of the picture.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Facts {
    /// As displayed (orientation applied), in pixels.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// "JPEG", "PNG", "GIF", "WebP".
    pub(crate) format: &'static str,
    pub(crate) size_bytes: u64,
    /// More than one frame: no stage (it can't be cropped or turned).
    pub(crate) animated: bool,
}

/// A gesture on the stage.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Gesture {
    #[default]
    Idle,
    /// The picture is dragged; the thirds show.
    Panning { last: (f32, f32) },
    /// A Free frame's corner is dragged from where it began.
    Resizing { corner: Corner, start: Rect },
}

/// What Save keeps for an attachment.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Saved {
    /// The name it is sent under, extension kept.
    pub(crate) name: String,
    pub(crate) edit: AttachmentEdit,
    /// The name it had before a random one, while Random name is on.
    pub(crate) typed: Option<String>,
}

/// One open editor.
pub(crate) struct Editor {
    pub(crate) use_: Use,
    /// The name without its extension (attachments).
    pub(crate) name: TextEdit,
    /// The extension, with its dot, fixed and muted (".jpg").
    pub(crate) extension: String,
    /// Random name: the field shows the random name that will be sent,
    /// and reroll draws another.
    pub(crate) random_name: bool,
    /// The field holds a random name drawn for it (`random_attachment_name`),
    /// so the SDK keeps it. Random name on without one asks for a draw.
    drawn: bool,
    /// The name as the person had it before Random name, stem and
    /// extension: it comes back when Random name goes off.
    typed: Option<(String, String)>,
    /// The Anonymize switch, when the file is one the SDK cleans; none when
    /// it is not offered (documents, videos, HEIC).
    pub(crate) anonymize: Option<Anonymize>,
    /// The tile's own Anonymize, kept as it was where the switch is not
    /// offered (a device default on a document is the SDK's to settle).
    kept_anonymize: Anonymize,
    /// The turns and crop it was opened with: what Save keeps while the
    /// stage has no picture (not read yet, or unreadable).
    opened_cut: (u8, Option<CropRect>),
    /// The cropper, once the picture's size is known; none for a file or
    /// an animation.
    pub(crate) crop: Option<Crop>,
    pub(crate) facts: Option<Facts>,
    pub(crate) gesture: Gesture,
}

impl Editor {
    /// An editor for an attachment named `name`, with its saved `edit`.
    /// `cleanable`: whether the SDK cleans this kind (an image or GIF).
    pub(crate) fn attachment(name: &str, edit: AttachmentEdit, cleanable: bool) -> Editor {
        let (stem, extension) = split_name(name);
        // Anonymize holds Random name on only where it cleans the file.
        let anonymized = cleanable && edit.anonymize.is_on();
        let random_name = edit.random_name || anonymized;
        Editor {
            use_: Use::Attachment,
            name: TextEdit::new(&stem),
            extension,
            random_name,
            // A name the SDK would pick at Send (a device default, an
            // earlier draft) is drawn now instead, so it shows.
            drawn: random_name && edit.random_name_chosen,
            typed: None,
            anonymize: cleanable.then_some(edit.anonymize),
            kept_anonymize: edit.anonymize,
            opened_cut: (edit.quarter_turns, edit.crop),
            crop: None,
            facts: None,
            gesture: Gesture::Idle,
        }
    }

    /// An editor for the profile photo or banner.
    pub(crate) fn profile(use_: Use) -> Editor {
        Editor {
            use_,
            name: TextEdit::new(""),
            extension: String::new(),
            random_name: false,
            drawn: false,
            typed: None,
            anonymize: None,
            kept_anonymize: Anonymize::Off,
            opened_cut: (0, None),
            crop: None,
            facts: None,
            gesture: Gesture::Idle,
        }
    }

    /// The SDK inspected the picture: the stage shows (unless it is an
    /// animation), at the saved edit's turns and crop.
    pub(crate) fn inspected(&mut self, facts: Facts, edit: &AttachmentEdit) {
        // The editor's own read shows the SDK cleans this file: Anonymize is
        // offered even if the tile's inspection hadn't landed at opening.
        if self.use_ == Use::Attachment && self.anonymize.is_none() {
            self.anonymize = Some(self.kept_anonymize);
            if self.kept_anonymize.is_on() {
                self.random_name = true;
            }
        }
        if !facts.animated {
            self.crop = Some(Crop::restored(
                (facts.width, facts.height),
                (DIALOG_WIDTH_PT, STAGE_HEIGHT_PT),
                self.use_.fixed_shape(),
                edit.quarter_turns,
                edit.crop,
            ));
        }
        self.facts = Some(facts);
    }

    /// Whether Anonymize is on.
    pub(crate) fn anonymized(&self) -> bool {
        self.anonymize.is_some_and(Anonymize::is_on)
    }

    /// Anonymize flipped: on, it holds Random name on.
    pub(crate) fn toggle_anonymize(&mut self) {
        let Some(anonymize) = self.anonymize else {
            return;
        };
        let on = !anonymize.is_on();
        self.anonymize = Some(if on {
            Anonymize::Required
        } else {
            Anonymize::Off
        });
        if on {
            self.random_name = true;
        }
    }

    /// Random name flipped; Anonymize holds it on. Off, the name the
    /// person had comes back.
    pub(crate) fn toggle_random_name(&mut self) {
        if self.anonymized() {
            return;
        }
        self.random_name = !self.random_name;
        if !self.random_name {
            if let Some((stem, extension)) = self.typed.take() {
                self.name.replace_text(stem);
                self.extension = extension;
            }
            self.drawn = false;
        }
    }

    /// The tile's name before a random one replaced it (`typed`): Random
    /// name off brings it back.
    pub(crate) fn remember_typed(&mut self, typed: Option<&str>) {
        if let Some(typed) = typed.filter(|_| self.random_name) {
            self.typed = Some(split_name(typed));
        }
    }

    /// Whether Random name is on with no random name drawn for the field
    /// yet: the shell draws one (`drew`).
    pub(crate) fn needs_draw(&self) -> bool {
        self.random_name && !self.drawn
    }

    /// The name for reroll and Random name to draw from: the file's.
    pub(crate) fn file_name(&self) -> String {
        format!("{}{}", self.name.text, self.extension)
    }

    /// A random name was drawn (`random_attachment_name`, extension
    /// included): the field shows it, and Random name is on.
    pub(crate) fn drew(&mut self, name: &str) {
        if !self.drawn && self.typed.is_none() {
            self.typed = Some((self.name.text.clone(), self.extension.clone()));
        }
        let (stem, extension) = match name.split_once('.') {
            Some((stem, extension)) => (stem, format!(".{extension}")),
            None => (name, String::new()),
        };
        self.name.replace_text(stem);
        self.extension = extension;
        self.random_name = true;
        self.drawn = true;
    }

    /// The name field changed: the characters a file name may not hold go.
    pub(crate) fn name_edited(&mut self) {
        if self.name.text.contains(FORBIDDEN) {
            let clean: String = self
                .name
                .text
                .chars()
                .filter(|c| !FORBIDDEN.contains(c))
                .collect();
            self.name.replace_text(&clean);
        }
    }

    /// A key on the stage. Returns whether it was taken.
    pub(crate) fn stage_key(&mut self, key: &KeyEvent) -> bool {
        // Shortcuts with Ctrl, Alt or the logo key are the app's.
        if key.mods.ctrl || key.mods.alt || key.mods.logo {
            return false;
        }
        let Some(crop) = &mut self.crop else {
            return false;
        };
        let step = if key.mods.shift {
            ARROW_STEP_PT.1
        } else {
            ARROW_STEP_PT.0
        };
        match &key.key {
            Key::Left => crop.pan(-step, 0.0),
            Key::Right => crop.pan(step, 0.0),
            Key::Up => crop.pan(0.0, -step),
            Key::Down => crop.pan(0.0, step),
            Key::Text(text) => match text.as_str() {
                "+" | "=" => crop.set_zoom(crop.zoom * ZOOM_STEP),
                "-" => crop.set_zoom(crop.zoom / ZOOM_STEP),
                "0" => crop.reset(),
                "r" | "R" => crop.rotate(),
                _ => return false,
            },
            _ => return false,
        }
        true
    }

    /// A press on the stage at `at` (stage points), on `corner` when it hit
    /// a Free frame's handle.
    pub(crate) fn press(&mut self, at: (f32, f32), corner: Option<Corner>) {
        let Some(crop) = &self.crop else {
            return;
        };
        self.gesture = match corner {
            Some(corner) => Gesture::Resizing {
                corner,
                start: crop.frame(),
            },
            None => Gesture::Panning { last: at },
        };
    }

    /// The pointer moved to `at`, `moved` from where the press began.
    pub(crate) fn drag(&mut self, at: (f32, f32), moved: (f32, f32)) {
        let Some(crop) = &mut self.crop else {
            return;
        };
        match self.gesture {
            Gesture::Idle => {}
            Gesture::Panning { last } => {
                crop.pan(at.0 - last.0, at.1 - last.1);
                self.gesture = Gesture::Panning { last: at };
            }
            Gesture::Resizing { corner, start } => crop.resize(corner, start, moved.0, moved.1),
        }
    }

    /// The press ended.
    pub(crate) fn release(&mut self) {
        self.gesture = Gesture::Idle;
    }

    /// The wheel over the stage: up zooms in.
    /// `dy` points of wheel over the stage (positive zooms in): a notch is
    /// one step; a `precise` stream (a trackpad) zooms by how far it moved.
    pub(crate) fn wheel(&mut self, dy: f32, precise: bool) {
        let Some(crop) = &mut self.crop else {
            return;
        };
        if dy == 0.0 || !dy.is_finite() {
            return;
        }
        let steps = if precise {
            dy / WHEEL_STEP_PT
        } else {
            dy.signum()
        };
        crop.set_zoom(crop.zoom * ZOOM_STEP.powf(steps));
    }

    /// What Save keeps for an attachment: the name (stem and extension),
    /// Random name, Anonymize, the turns and the crop.
    pub(crate) fn saved(&self) -> Saved {
        let stem = self.name.text.trim();
        let name = format!("{stem}{}", self.extension);
        let (quarter_turns, crop) = self
            .crop
            .as_ref()
            .map_or(self.opened_cut, |crop| (crop.turns, crop.rect()));
        let typed = self
            .typed
            .as_ref()
            .filter(|_| self.random_name)
            .map(|(stem, extension)| format!("{stem}{extension}"));
        Saved {
            name,
            typed,
            edit: AttachmentEdit {
                anonymize: self.anonymize.unwrap_or(self.kept_anonymize),
                random_name: self.random_name,
                random_name_chosen: self.random_name && self.drawn,
                quarter_turns,
                crop,
            },
        }
    }

    /// Whether Save can go: a name is left, and the stage (when there is
    /// one) has its picture's size.
    pub(crate) fn can_save(&self) -> bool {
        let named =
            self.use_ != Use::Attachment || self.random_name || !self.name.text.trim().is_empty();
        let framed = self.use_ == Use::Attachment || self.crop.is_some();
        named && framed
    }
}

/// A file name as the field shows it: the stem, and the extension with its
/// dot (fixed and muted).
fn split_name(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_owned(), format!(".{extension}")),
        _ => (name.to_owned(), String::new()),
    }
}

#[cfg(test)]
mod tests;
