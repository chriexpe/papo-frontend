//! Image actions shared by attachments and provider GIFs.

use std::sync::mpsc;

use super::shell::{ChatAction, UiState};
use crate::i18n::Strings;

pub enum Action {
    Copy,
    Save,
    CopyLink,
    OpenLink,
}

fn item(ui: &mut egui::Ui, glyph: &str, label: &str) -> bool {
    ui.button((
        egui::RichText::new(glyph).font(super::theme::text::icon(16.0)),
        egui::RichText::new(label).font(super::theme::text::body()),
    ))
    .clicked()
}

pub fn menu(response: &egui::Response, s: &Strings) -> Option<Action> {
    use egui_phosphor::regular as icon;
    let mut action = None;
    response.context_menu(|ui| {
        if item(ui, icon::IMAGE, s.media_copy_image) {
            action = Some(Action::Copy);
            ui.close();
        }
        if item(ui, icon::DOWNLOAD_SIMPLE, s.media_save_image) {
            action = Some(Action::Save);
            ui.close();
        }
        ui.separator();
        if item(ui, icon::LINK, s.media_copy_link) {
            action = Some(Action::CopyLink);
            ui.close();
        }
        if item(ui, icon::ARROW_SQUARE_OUT, s.media_open_link) {
            action = Some(Action::OpenLink);
            ui.close();
        }
    });
    action
}

pub struct RemoteTask {
    url: String,
    name: String,
    copy: bool,
    decoded: Option<mpsc::Receiver<Result<egui::ColorImage, String>>>,
}

pub fn remote(
    ui: &egui::Ui,
    state: &mut UiState,
    s: &Strings,
    response: &egui::Response,
    url: &str,
    name: &str,
) {
    match menu(response, s) {
        Some(action @ (Action::Copy | Action::Save)) => state.media_tasks.push(RemoteTask {
            url: url.to_owned(),
            name: name.to_owned(),
            copy: matches!(action, Action::Copy),
            decoded: None,
        }),
        Some(Action::CopyLink) => ui.ctx().copy_text(url.to_owned()),
        Some(Action::OpenLink) => {
            state.request_external_url(ui.ctx(), url);
        }
        None => {}
    }
}

pub fn pump(ctx: &egui::Context, state: &mut UiState, s: &Strings) {
    for mut task in std::mem::take(&mut state.media_tasks) {
        if let Some(rx) = &task.decoded {
            match rx.try_recv() {
                Ok(Ok(image)) => ctx.copy_image(image),
                Ok(Err(error)) => state.error = Some((error, ctx.input(|input| input.time))),
                Err(mpsc::TryRecvError::Disconnected) => {}
                Err(mpsc::TryRecvError::Empty) => state.media_tasks.push(task),
            }
            continue;
        }
        let failed = matches!(
            state.media.remote_image_full(&task.url),
            Some(crate::media::Texture::Failed)
        );
        if let Some(path) = crate::media::remote_image_cached_path(&task.url) {
            if task.copy {
                let (tx, rx) = mpsc::channel();
                let repaint = ctx.clone();
                std::thread::spawn(move || {
                    let result = image::ImageReader::open(path)
                        .map_err(|error| error.to_string())
                        .and_then(|reader| {
                            reader
                                .with_guessed_format()
                                .map_err(|error| error.to_string())
                        })
                        .and_then(|reader| reader.decode().map_err(|error| error.to_string()))
                        .map(|image| {
                            let image = image.to_rgba8();
                            egui::ColorImage::from_rgba_unmultiplied(
                                [image.width() as usize, image.height() as usize],
                                image.as_raw(),
                            )
                        });
                    let _ = tx.send(result);
                    repaint.request_repaint();
                });
                task.decoded = Some(rx);
                state.media_tasks.push(task);
            } else {
                state.actions.push(ChatAction::SaveCachedImage {
                    path,
                    name: task.name,
                });
            }
        } else if failed {
            state.error = Some((s.image_unreadable.into(), ctx.input(|input| input.time)));
        } else {
            state.media_tasks.push(task);
        }
    }
    if !state.media_tasks.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_click_opens_image_actions_and_save_selects_the_action() {
        let ctx = egui::Context::default();
        super::super::theme::install_fonts(&ctx, None);
        let strings = crate::i18n::Lang::En.strings();
        let run = |events| {
            let mut action = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (_, response) =
                        ui.allocate_exact_size(egui::vec2(150.0, 100.0), egui::Sense::click());
                    action = menu(&response, strings);
                },
            );
            output.textures_delta.clear();
            (output.shapes, action)
        };
        let point = egui::pos2(40.0, 40.0);
        let click = |point, button| {
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: point,
                    button,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        run(vec![egui::Event::PointerMoved(point)]);
        run(click(point, egui::PointerButton::Secondary));
        let (shapes, _) = run(vec![]);
        let label_rect = |label: &str| {
            shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
        };
        for label in [
            strings.media_copy_image,
            strings.media_save_image,
            strings.media_copy_link,
            strings.media_open_link,
        ] {
            assert!(label_rect(label).is_some(), "missing {label}");
        }
        let save = label_rect(strings.media_save_image).unwrap().center();
        let (_, action) = run(click(save, egui::PointerButton::Primary));
        assert!(matches!(action, Some(Action::Save)));
    }
}
