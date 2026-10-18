//! Local attachment edits. Decoding and writing copies run off the UI thread.

use std::path::PathBuf;
use std::sync::mpsc;

use egui::{RichText, Vec2};
use papo_core::api::client::Upload;
use papo_core::api::models::Kind;

use crate::i18n::Strings;
use crate::platform::files::{self, Chosen, ImagePick};

use super::crop::{CropEditor, CropOutcome, Face};
use super::theme::{Tokens, text};

pub struct AttachmentEditor {
    pub index: usize,
    pub original: PathBuf,
    upload: Upload,
    loading: Option<mpsc::Receiver<Chosen>>,
    saving: Option<mpsc::Receiver<Result<Upload, String>>>,
    crop: Option<CropEditor>,
    error: Option<String>,
}

pub enum Outcome {
    Cancel,
    Save(Upload),
}

pub fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && !name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
        && !matches!(name.trim(), "." | "..")
}

pub fn name_field(ui: &mut egui::Ui, upload: &mut Upload, s: &Strings) {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 32.0).max(40.0);
        #[cfg(not(target_os = "android"))]
        ui.add_sized(
            Vec2::new(width, 30.0),
            egui::TextEdit::singleline(&mut upload.name)
                .font(text::body())
                .vertical_align(egui::Align::Center),
        );
        #[cfg(target_os = "android")]
        {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), egui::Sense::hover());
            crate::platform::native_field::show(
                ui.ctx(),
                "attachment-editor:name",
                &mut upload.name,
                rect,
                "",
                crate::platform::native_field::Mode::Text,
                255,
                false,
                ui.visuals().text_color(),
                ui.visuals().weak_text_color(),
                text::body().size,
            );
        }
        if ui
            .small_button(egui_phosphor::regular::SHUFFLE)
            .on_hover_text(s.attach_random_name)
            .clicked()
        {
            upload.name = crate::platform::anonymize::gibberish_name(&upload.name);
        }
    });
}

impl AttachmentEditor {
    pub fn new(index: usize, upload: Upload, ctx: egui::Context) -> Self {
        let loading = if Kind::guess(&upload.mime, &upload.name) == Kind::Image {
            let (tx, rx) = mpsc::channel();
            let input = upload.clone();
            std::thread::spawn(move || {
                let result = files::after_pick(&input.path, input.name, ImagePick::Attachment);
                let _ = tx.send(result);
                ctx.request_repaint();
            });
            Some(rx)
        } else {
            None
        };
        Self {
            index,
            original: upload.path.clone(),
            upload,
            loading,
            saving: None,
            crop: None,
            error: None,
        }
    }

    pub fn draw(&mut self, ctx: &egui::Context, t: &Tokens, s: &Strings) -> Option<Outcome> {
        if let Some(rx) = &self.loading {
            match rx.try_recv() {
                Ok(Chosen::Crop {
                    purpose,
                    path,
                    name,
                    size,
                    preview,
                    frames,
                }) => {
                    let mut crop = CropEditor::new(ctx, purpose, path, name, size, preview, frames);
                    crop.attachment = Some(self.upload.clone());
                    self.crop = Some(crop);
                    self.loading = None;
                }
                Ok(_) | Err(mpsc::TryRecvError::Disconnected) => {
                    self.loading = None;
                    self.error = Some(s.image_unreadable.to_owned());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.saving {
            match rx.try_recv() {
                Ok(Ok(upload)) => return Some(Outcome::Save(upload)),
                Ok(Err(error)) => {
                    self.error = Some(error);
                    self.saving = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = Some(s.image_unreadable.to_owned());
                    self.saving = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.saving.is_some() || self.loading.is_some() {
            return self.simple(ctx, t, s, true);
        }
        if let Some(editor) = self.crop.as_mut() {
            let face = Face {
                name: String::new(),
                texture: None,
                initials: String::new(),
                tint: t.accent,
            };
            match super::crop::draw(ctx, editor, t, s, &face, false) {
                Some(CropOutcome::Cancel) => return Some(Outcome::Cancel),
                Some(CropOutcome::Save(crop, _)) => {
                    let upload = editor.attachment.as_ref()?.clone();
                    let (tx, rx) = mpsc::channel();
                    let repaint = ctx.clone();
                    self.saving = Some(rx);
                    std::thread::spawn(move || {
                        let _ = tx.send(edited_copy(upload, crop));
                        repaint.request_repaint();
                    });
                }
                None => {}
            }
            if let Some(error) = &self.error {
                egui::Window::new(s.image_unreadable)
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.colored_label(t.danger, error);
                    });
            }
            None
        } else {
            self.simple(ctx, t, s, false)
        }
    }

    fn simple(
        &mut self,
        ctx: &egui::Context,
        t: &Tokens,
        s: &Strings,
        busy: bool,
    ) -> Option<Outcome> {
        let mut outcome = None;
        let screen = ctx.content_rect();
        let rect = egui::Rect::from_center_size(
            screen.center(),
            Vec2::new(420.0_f32.min(screen.width() - 24.0), 210.0),
        );
        egui::Area::new(egui::Id::new("attachment-editor"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.set_min_size(screen.size());
                ui.painter()
                    .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(110));
                ui.interact(
                    screen,
                    ui.id().with("backdrop"),
                    egui::Sense::click_and_drag(),
                );
                super::emoji::popup_frame(ui, t, rect, |ui| {
                    ui.label(RichText::new(s.attach_edit).font(text::headline()));
                    ui.add_enabled_ui(!busy, |ui| {
                        name_field(ui, &mut self.upload, s);
                        ui.checkbox(&mut self.upload.spoiler, s.attach_spoiler);
                    });
                    if busy {
                        ui.spinner();
                    }
                    if let Some(error) = &self.error {
                        ui.colored_label(t.danger, error);
                    }
                    ui.horizontal(|ui| {
                        if ui.button(s.cancel).clicked() {
                            outcome = Some(Outcome::Cancel);
                        }
                        if ui
                            .add_enabled(
                                !busy && valid_name(&self.upload.name),
                                egui::Button::new(s.save),
                            )
                            .clicked()
                        {
                            outcome = Some(Outcome::Save(self.upload.clone()));
                        }
                    });
                });
            });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            return Some(Outcome::Cancel);
        }
        outcome
    }
}

fn edited_copy(upload: Upload, crop: crate::media::prepare::Crop) -> Result<Upload, String> {
    if crop
        == (crate::media::prepare::Crop {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        })
    {
        return Ok(upload);
    }
    let prepared = crate::media::prepare::fit_cropped(
        &upload.path,
        Some(crop),
        u16::MAX as u32,
        u16::MAX as u32,
        usize::MAX,
    )?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&prepared.blob)
        .map_err(|error| error.to_string())?;
    let extension = prepared.format.to_ascii_lowercase();
    let name = PathBuf::from(&upload.name)
        .with_extension(&extension)
        .to_string_lossy()
        .into_owned();
    let dir = crate::media::cache_root().join("edited");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(crate::platform::anonymize::gibberish_name(&name));
    std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
    let mut edited = files::describe(&path);
    edited.name = name;
    edited.spoiler = upload.spoiler;
    edited.anonymize = upload.anonymize;
    Ok(edited)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_cannot_be_paths_or_empty() {
        for name in ["", "  ", ".", "..", "../image.png", "a\\b", "bad\nname"] {
            assert!(!valid_name(name), "{name:?}");
        }
        assert!(valid_name("my photo é.png"));
    }
}
