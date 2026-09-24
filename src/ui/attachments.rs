//! Anexos dentro da mensagem: imagem, vídeo, áudio e arquivo.
//!
//! Nada aqui decide sozinho: o que precisa de rede ou de janela sai como
//! [`MediaAction`] para a camada de cima resolver.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Vec2};
use egui_phosphor::regular as icon;

use std::path::Path;

use crate::api::models::{Attachment, Kind};
use crate::i18n::Strings;
use crate::media::{FileState, MediaStore};

use super::theme::{radius, space, text, Tokens};

const IMAGE_MAX_W: f32 = 420.0;
const IMAGE_MAX_H: f32 = 300.0;
const VIDEO_MAX_W: f32 = 480.0;
const CONTROLS_H: f32 = 32.0;
const AUDIO_H: f32 = 54.0;
const FILE_H: f32 = 56.0;

/// Folga de pré-busca em volta da área visível. Fora dela o cartão não pede
/// nada: nem disco, nem rede. A margem cobre perto de uma tela, então rolagem
/// rápida ainda encontra o conteúdo a caminho em vez de esperar.
const VIEWPORT_MARGIN: f32 = 600.0;

/// O próximo cartão está perto o bastante da área visível para valer um
/// pedido? É o que deixa a lista de mensagens preguiçosa por viewport sem
/// virtualizar cada linha.
fn near_viewport(ui: &egui::Ui) -> bool {
    let clip = ui.clip_rect();
    let top = ui.cursor().top();
    top <= clip.max.y + VIEWPORT_MARGIN && top >= clip.min.y - VIEWPORT_MARGIN
}

#[derive(Debug, Clone)]
pub enum MediaAction {
    /// Abre a imagem ou o vídeo em tela cheia.
    Open { message_id: String, index: usize },
    Download { id: String, name: String },
    Reveal(String),
}

/// Desenha todos os anexos de uma mensagem.
pub fn draw(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    message_id: &str,
    attachments: &[Attachment],
    width: f32,
    seek_zones: &mut Vec<Rect>,
) -> Option<MediaAction> {
    let mut action = None;
    for (index, attachment) in attachments.iter().enumerate() {
        ui.add_space(space::XS);
        let outcome = match attachment.kind() {
            Kind::Image => image(ui, t, s, media, message_id, index, attachment, width),
            Kind::Video => video(
                ui,
                t,
                s,
                media,
                message_id,
                index,
                attachment,
                width,
                seek_zones,
            ),
            Kind::Audio => audio(ui, t, s, media, attachment, width, seek_zones),
            Kind::Other => file_card(ui, t, s, attachment, width),
        };
        action = action.or(outcome);
    }
    action
}

/// Tamanho que respeita a proporção dentro de um limite.
fn fit(size: Vec2, max: Vec2) -> Vec2 {
    let scale = (max.x / size.x).min(max.y / size.y).min(1.0);
    Vec2::new((size.x * scale).max(1.0), (size.y * scale).max(1.0))
}

fn image(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    message_id: &str,
    index: usize,
    attachment: &Attachment,
    width: f32,
) -> Option<MediaAction> {
    let limit = Vec2::new(width.min(IMAGE_MAX_W), IMAGE_MAX_H);
    let hidden = attachment.sensitive() && media.sensitive_hidden(&attachment.id);

    // Mede com o que já está em memória e só pede a miniatura se o cartão
    // estiver perto da tela e o servidor anunciar uma. Sem `thumbnail_id` não
    // há miniatura, e a imagem inteira nunca é baixada só para desenhar.
    let visible = near_viewport(ui);
    let cached = media
        .loaded_thumb(&attachment.id)
        .and_then(|texture| texture.frame(ui.ctx()))
        .cloned();
    let size = match &cached {
        Some(texture) => fit(texture.size_vec2(), limit),
        None => Vec2::new(limit.x.min(260.0), 150.0),
    };
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let texture = if cached.is_some() || !visible {
        cached
    } else {
        media
            .thumb(attachment)
            .and_then(|texture| texture.frame(ui.ctx()))
            .cloned()
    };
    let corner = CornerRadius::same(radius::CARD);

    match &texture {
        Some(texture) => {
            let tint = if hidden {
                Color32::from_gray(120)
            } else {
                Color32::WHITE
            };
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                tint,
            );
        }
        None => {
            ui.painter().rect_filled(rect, corner, t.fill_soft);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon::IMAGE,
                text::icon(22.0),
                t.label_tertiary,
            );
        }
    }
    ui.painter().rect_stroke(
        rect,
        corner,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    if hidden {
        // A moderação marcou como sensível: nada aparece antes de um clique.
        ui.painter()
            .rect_filled(rect, corner, Color32::from_black_alpha(190));
        ui.painter().text(
            rect.center() - Vec2::new(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            icon::EYE_SLASH,
            text::icon(20.0),
            t.label_secondary,
        );
        ui.painter().text(
            rect.center() + Vec2::new(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            s.sensitive_reveal,
            text::footnote(),
            t.label_secondary,
        );
        if response.clicked() {
            return Some(MediaAction::Reveal(attachment.id.clone()));
        }
        return None;
    }

    if response.hovered() {
        hover_badge(ui, t, rect, icon::ARROWS_OUT, s.open);
    }
    if response.clicked() {
        return Some(MediaAction::Open {
            message_id: message_id.to_owned(),
            index,
        });
    }
    if response.secondary_clicked() {
        return Some(MediaAction::Download {
            id: attachment.id.clone(),
            name: attachment.name().to_owned(),
        });
    }
    None
}

fn video(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    message_id: &str,
    index: usize,
    attachment: &Attachment,
    width: f32,
    seek_zones: &mut Vec<Rect>,
) -> Option<MediaAction> {
    let card_width = width.min(VIDEO_MAX_W);
    // Fora da área visível nem toca o disco: o cartão fica no quadro vazio
    // até chegar perto da tela.
    if near_viewport(ui) {
        media.probe_file(&attachment.id, attachment.name());
    }
    let Some(path) = media.file_ready(&attachment.id) else {
        return video_placeholder(ui, t, s, media, attachment, card_width);
    };

    let ctx = ui.ctx().clone();
    // Antes do primeiro play não existe pipeline, e é esse o ponto: rolar a
    // conversa passava por aqui e montava um decodificador por vídeo.
    //
    // Cada consulta ao `media` vira `(id, tamanho)` na hora: são três
    // empréstimos seguidos do mesmo lugar, e nenhum pode sobreviver ao
    // próximo.
    let live = media
        .existing_player(&attachment.id)
        .and_then(|player| player.frame(&ctx))
        .map(|texture| (texture.id(), texture.size_vec2()));
    let (player_aspect, playing, position, duration) =
        match media.existing_player(&attachment.id) {
            Some(player) => (
                Some(player.aspect().clamp(0.4, 3.0)),
                player.is_playing(),
                player.position(),
                player.duration(),
            ),
            None => (None, false, 0.0, 0.0),
        };
    let poster = media
        .poster(&attachment.id, &path)
        .and_then(|texture| texture.frame(&ctx))
        .map(|texture| (texture.id(), texture.size_vec2()));

    // A proporção sai do que existir de mais concreto: o quadro que está
    // tocando, depois a capa, depois o que o player disse. O 16:9 é só o
    // chute de enquanto não há nenhum dos três — e era ele que achatava
    // vídeo em pé, porque a capa chegava depois do cartão já medido.
    let shown = live.or(poster);
    let aspect = shown
        .map(|(_, size)| size.x / size.y.max(1.0))
        .or(player_aspect)
        .unwrap_or(16.0 / 9.0)
        .clamp(0.4, 3.0);
    let frame_size = Vec2::new(card_width, (card_width / aspect).min(320.0));
    let total = Vec2::new(card_width, frame_size.y + CONTROLS_H);
    let (rect, response) = ui.allocate_exact_size(total, Sense::click());
    let frame_rect = Rect::from_min_size(rect.min, frame_size);
    let corner = CornerRadius::same(radius::CARD);

    ui.painter().rect_filled(rect, corner, Color32::BLACK);

    // Tocando, é o quadro do player; parado, a capa tirada do arquivo.
    if let Some((texture, natural)) = shown {
        let size = fit(natural, frame_size);
        let centered = Rect::from_center_size(frame_rect.center(), size);
        ui.painter().image(
            texture,
            centered,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    // Botão grande no meio enquanto está parado.
    if !playing {
        let button = Rect::from_center_size(frame_rect.center(), Vec2::splat(52.0));
        ui.painter()
            .circle_filled(button.center(), 26.0, Color32::from_black_alpha(140));
        ui.painter().text(
            button.center(),
            egui::Align2::CENTER_CENTER,
            icon::PLAY,
            text::icon(22.0),
            Color32::WHITE,
        );
    }

    let mut action = None;
    if response.clicked() {
        media.toggle_player(&attachment.id, &path, true, &ctx);
        media.solo(&attachment.id);
    }

    let controls = Rect::from_min_size(
        egui::pos2(rect.min.x, frame_rect.max.y),
        Vec2::new(card_width, CONTROLS_H),
    );
    if let Some(command) = transport(
        ui,
        t,
        media,
        &attachment.id,
        &path,
        controls,
        position,
        duration,
        true,
        seek_zones,
    ) {
        match command {
            Transport::Fullscreen => {
                action = Some(MediaAction::Open {
                    message_id: message_id.to_owned(),
                    index,
                })
            }
            Transport::Download => {
                action = Some(MediaAction::Download {
                    id: attachment.id.clone(),
                    name: attachment.name().to_owned(),
                })
            }
        }
    }

    ui.painter().rect_stroke(
        rect,
        corner,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    action
}

#[allow(clippy::ptr_arg)]
fn audio(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    attachment: &Attachment,
    width: f32,
    _seek_zones: &mut Vec<Rect>,
) -> Option<MediaAction> {
    let card_width = width.min(VIDEO_MAX_W);
    // O cartão não baixa o áudio só por estar à vista, e nem sonda o disco se
    // estiver fora da tela. Sem arquivo, a onda é uma linha reta; o play é
    // quem pede o download.
    if near_viewport(ui) {
        media.probe_file(&attachment.id, attachment.name());
    }
    let path = media.file_ready(&attachment.id);
    let state = media.file_state(&attachment.id);
    let loading = matches!(state, Some(FileState::Loading));
    let download_failed = matches!(state, Some(FileState::Failed));

    let ctx = ui.ctx().clone();
    // A forma de onda sai do arquivo, não do player: só existe depois que o
    // arquivo existe. Antes disso, vazia.
    let peaks: Vec<f32> = match &path {
        Some(path) => media
            .waveform(&attachment.id, path)
            .map(<[f32]>::to_vec)
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let (playing, position, duration, failed) = match media.existing_player(&attachment.id) {
        Some(player) => {
            player.update();
            (
                player.is_playing(),
                player.position(),
                player.duration(),
                player.error(),
            )
        }
        None => (false, 0.0, 0.0, None),
    };

    #[cfg(target_os = "android")]
    let audio_sense = Sense::click();
    #[cfg(not(target_os = "android"))]
    let audio_sense = Sense::click_and_drag();
    let (rect, card) = ui.allocate_exact_size(Vec2::new(card_width, AUDIO_H), audio_sense);
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::CARD),
        t.fill_soft,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    // Play
    let button = Rect::from_center_size(
        egui::pos2(rect.min.x + space::XXL, rect.center().y),
        Vec2::splat(28.0),
    );
    let pointer = card.interact_pointer_pos();
    let on_button = pointer.map(|pos| button.expand(6.0).contains(pos)).unwrap_or(false);
    let button_response = card.clone();
    let button_hovered = ui
        .ctx()
        .pointer_latest_pos()
        .map(|pos| button.expand(6.0).contains(pos))
        .unwrap_or(false);
    ui.painter().circle_filled(
        button.center(),
        14.0,
        if button_hovered {
            t.accent
        } else {
            t.accent.gamma_multiply(0.85)
        },
    );
    ui.painter().text(
        button.center(),
        egui::Align2::CENTER_CENTER,
        if loading {
            icon::SPINNER
        } else if download_failed {
            icon::WARNING
        } else if playing {
            icon::PAUSE
        } else {
            icon::PLAY
        },
        text::icon(13.0),
        t.accent_label,
    );
    if button_response.clicked() && on_button && !loading {
        media.toggle_play(&attachment.id, attachment.name(), false, &ctx);
    }

    // Forma de onda, que também serve de barra de progresso.
    let time_width = 64.0;
    let wave = Rect::from_min_max(
        egui::pos2(button.max.x + space::LG, rect.min.y + space::MD),
        egui::pos2(rect.max.x - time_width - space::LG, rect.max.y - space::MD),
    );
    let progress = if duration > 0.0 {
        (position / duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    waveform(ui, t, wave, &peaks, progress);
    // Cabeça de leitura: mostra onde o clique vai cair.
    if duration > 0.0 {
        let head = egui::pos2(wave.min.x + wave.width() * progress, wave.center().y);
        ui.painter().circle_filled(head, 4.0, t.accent);
    }

    // Clicar ou arrastar em cima da onda pula para o ponto.
    let wave_hit = wave.expand2(Vec2::new(0.0, 10.0));
    let over_wave = ui
        .ctx()
        .pointer_latest_pos()
        .map(|pos| wave_hit.contains(pos))
        .unwrap_or(false);
    if over_wave {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    #[cfg(target_os = "android")]
    {
        _seek_zones.push(wave_hit);
        // No Android a onda NÃO possui Sense::drag: isso deixaria o filho
        // roubar a rolagem vertical do ScrollArea. O toque cru decide o eixo
        // e trava a decisão até o dedo subir.
        let response = ui.interact(
            wave_hit,
            ui.id().with(("audio-seek", &attachment.id)),
            Sense::click(),
        );
        if let Some(ratio) = android_seek_ratio(ui, &response, wave)
            && let Some(path) = path.as_ref()
            && let Some(player) = media.start_player(&attachment.id, path, false, &ctx)
        {
            let duration = player.duration();
            if duration > 0.0 {
                player.seek(duration * ratio);
            } else {
                player.play();
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    if let Some(pos) = pointer.filter(|pos| {
        (card.dragged() || card.clicked()) && wave_hit.contains(*pos)
    }) {
        let ratio = ((pos.x - wave.min.x) / wave.width()).clamp(0.0, 1.0) as f64;
        if let Some(path) = path.as_ref()
            && let Some(player) = media.start_player(&attachment.id, path, false, &ctx)
        {
            let duration = player.duration();
            if duration > 0.0 {
                player.seek(duration * ratio);
            } else {
                // Sem duração ainda: toca para o pipeline prerollar e a
                // próxima tentativa já acerta o ponto.
                player.play();
            }
        }
    }

    let time = if loading {
        s.downloading.to_owned()
    } else if download_failed {
        s.media_failed.to_owned()
    } else {
        format!("{} / {}", clock(position), clock(duration))
    };
    ui.painter().text(
        egui::pos2(rect.max.x - space::LG, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        time,
        text::footnote(),
        t.label_tertiary,
    );
    if let Some(error) = failed {
        log::warn!("áudio {}: {error}", attachment.id);
    }

    if playing || loading {
        ui.ctx().request_repaint();
    }
    None
}

/// Cartão de arquivo comum, também usado enquanto a mídia não chegou.
fn file_card(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    attachment: &Attachment,
    width: f32,
) -> Option<MediaAction> {
    let _ = s;
    let card_width = width.min(VIDEO_MAX_W);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(card_width, FILE_H), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::CARD),
        t.fill_soft,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    let glyph = match attachment.kind() {
        Kind::Image => icon::IMAGE,
        Kind::Video => icon::FILM_STRIP,
        Kind::Audio => icon::MUSIC_NOTES,
        Kind::Other => icon::FILE,
    };
    ui.painter().text(
        egui::pos2(rect.min.x + space::XXL, rect.center().y),
        egui::Align2::CENTER_CENTER,
        glyph,
        text::icon(19.0),
        t.label_secondary,
    );
    ui.painter().text(
        egui::pos2(rect.min.x + space::XXL + space::XXL, rect.center().y - 8.0),
        egui::Align2::LEFT_CENTER,
        elide(attachment.name(), 42),
        text::body(),
        t.label,
    );
    ui.painter().text(
        egui::pos2(rect.min.x + space::XXL + space::XXL, rect.center().y + 9.0),
        egui::Align2::LEFT_CENTER,
        size_label(attachment.size_bytes),
        text::footnote(),
        t.label_tertiary,
    );

    let button = Rect::from_center_size(
        egui::pos2(rect.max.x - space::XXL, rect.center().y),
        Vec2::splat(28.0),
    );
    let response = ui.interact(
        button,
        ui.id().with(("download", &attachment.id)),
        Sense::click(),
    );
    if response.hovered() {
        ui.painter().rect_filled(
            button,
            CornerRadius::same(radius::CONTROL),
            t.fill_medium,
        );
    }
    ui.painter().text(
        button.center(),
        egui::Align2::CENTER_CENTER,
        icon::DOWNLOAD_SIMPLE,
        text::icon(15.0),
        t.label_secondary,
    );
    response.clicked().then(|| MediaAction::Download {
        id: attachment.id.clone(),
        name: attachment.name().to_owned(),
    })
}

/// Cartão do vídeo que ainda não foi baixado. Nunca busca o arquivo: mostra a
/// miniatura do servidor quando ela vem de graça, ou um quadro vazio, e deixa
/// o play ser o pedido explícito.
fn video_placeholder(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    attachment: &Attachment,
    card_width: f32,
) -> Option<MediaAction> {
    let visible = near_viewport(ui);
    let frame_size = Vec2::new(card_width, (card_width / (16.0 / 9.0)).min(320.0));
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(card_width, frame_size.y + CONTROLS_H),
        Sense::click(),
    );
    let frame_rect = Rect::from_min_size(rect.min, frame_size);
    let corner = CornerRadius::same(radius::CARD);
    ui.painter().rect_filled(rect, corner, Color32::BLACK);

    // Miniatura do servidor, se houver: é barata e não baixa o vídeo inteiro.
    // Fora da tela, mostra só o que já está em memória.
    let source = if visible {
        media.thumb(attachment)
    } else {
        media.loaded_thumb(&attachment.id)
    };
    let thumb = source
        .and_then(|texture| texture.frame(ui.ctx()))
        .map(|texture| (texture.id(), texture.size_vec2()));
    match thumb {
        Some((texture, natural)) => {
            let size = fit(natural, frame_size);
            let centered = Rect::from_center_size(frame_rect.center(), size);
            ui.painter().image(
                texture,
                centered,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            ui.painter().text(
                frame_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon::FILM_STRIP,
                text::icon(30.0),
                Color32::from_white_alpha(70),
            );
        }
    }

    let state = media.file_state(&attachment.id);
    let loading = matches!(state, Some(FileState::Loading));
    let download_failed = matches!(state, Some(FileState::Failed));
    if loading {
        ui.painter().text(
            frame_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon::SPINNER,
            text::icon(24.0),
            Color32::WHITE,
        );
    } else if download_failed {
        ui.painter().text(
            frame_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon::WARNING,
            text::icon(24.0),
            Color32::WHITE,
        );
    } else {
        let button = Rect::from_center_size(frame_rect.center(), Vec2::splat(52.0));
        ui.painter()
            .circle_filled(button.center(), 26.0, Color32::from_black_alpha(140));
        ui.painter().text(
            button.center(),
            egui::Align2::CENTER_CENTER,
            icon::PLAY,
            text::icon(22.0),
            Color32::WHITE,
        );
    }

    // O clique é o pedido explícito: baixa e toca quando o arquivo chegar.
    if response.clicked() && !loading {
        media.toggle_play(&attachment.id, attachment.name(), true, ui.ctx());
    }

    let status = if loading {
        s.downloading
    } else if download_failed {
        s.media_failed
    } else {
        ""
    };
    let baseline = frame_rect.max.y + CONTROLS_H / 2.0;
    ui.painter().text(
        egui::pos2(rect.min.x + space::MD, baseline),
        egui::Align2::LEFT_CENTER,
        elide(attachment.name(), 34),
        text::footnote(),
        Color32::from_white_alpha(200),
    );
    ui.painter().text(
        egui::pos2(rect.max.x - space::MD, baseline),
        egui::Align2::RIGHT_CENTER,
        status,
        text::footnote(),
        Color32::from_white_alpha(160),
    );
    ui.painter().rect_stroke(
        rect,
        corner,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    if loading {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }
    None
}

enum Transport {
    Fullscreen,
    Download,
}

/// Barra de controles do vídeo: play, linha do tempo, som e tela cheia.
#[allow(clippy::too_many_arguments, clippy::ptr_arg)]
fn transport(
    ui: &mut egui::Ui,
    t: &Tokens,
    media: &mut MediaStore,
    id: &str,
    path: &Path,
    rect: Rect,
    position: f64,
    duration: f64,
    fullscreen: bool,
    _seek_zones: &mut Vec<Rect>,
) -> Option<Transport> {
    let mut outcome = None;
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(190));

    let mid = rect.center().y;
    let mut x = rect.min.x + space::MD;

    let playing = media
        .existing_player(id)
        .map(|player| player.is_playing())
        .unwrap_or(false);
    let play = Rect::from_center_size(egui::pos2(x + 10.0, mid), Vec2::splat(24.0));
    if control(ui, play, if playing { icon::PAUSE } else { icon::PLAY }, id, "play") {
        let ctx = ui.ctx().clone();
        media.toggle_player(id, path, true, &ctx);
        media.solo(id);
    }
    x = play.max.x + space::SM;

    let time = format!("{} / {}", clock(position), clock(duration));
    let time_width = 78.0;
    let right = rect.max.x - space::MD;
    let mute = Rect::from_center_size(egui::pos2(right - 10.0 - 28.0 * 2.0, mid), Vec2::splat(24.0));
    let download = Rect::from_center_size(egui::pos2(right - 10.0 - 28.0, mid), Vec2::splat(24.0));
    let expand = Rect::from_center_size(egui::pos2(right - 10.0, mid), Vec2::splat(24.0));

    let muted = media
        .existing_player(id)
        .map(|player| player.muted)
        .unwrap_or(false);
    if control(
        ui,
        mute,
        if muted {
            icon::SPEAKER_SIMPLE_X
        } else {
            icon::SPEAKER_SIMPLE_HIGH
        },
        id,
        "mute",
    ) && let Some(player) = media.existing_player(id)
    {
        let muted = player.muted;
        player.set_muted(!muted);
    }
    if control(ui, download, icon::DOWNLOAD_SIMPLE, id, "dl") {
        outcome = Some(Transport::Download);
    }
    if fullscreen && control(ui, expand, icon::ARROWS_OUT, id, "full") {
        outcome = Some(Transport::Fullscreen);
    }

    // Linha do tempo
    let line = Rect::from_min_max(
        egui::pos2(x + space::SM, mid - 3.0),
        egui::pos2(mute.min.x - space::SM - time_width, mid + 3.0),
    );
    if line.width() > 20.0 {
        let progress = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        ui.painter().rect_filled(
            line,
            CornerRadius::same(3),
            Color32::from_white_alpha(45),
        );
        let played = Rect::from_min_size(line.min, Vec2::new(line.width() * progress, line.height()));
        ui.painter()
            .rect_filled(played, CornerRadius::same(3), t.accent);
        ui.painter()
            .circle_filled(egui::pos2(played.max.x, mid), 5.0, Color32::WHITE);

        let seek_rect = line.expand2(Vec2::new(0.0, 8.0));

        #[cfg(target_os = "android")]
        {
            _seek_zones.push(seek_rect);
            let response = ui.interact(
                seek_rect,
                ui.id().with(("seek", id)),
                Sense::click(),
            );
            if let Some(ratio) = android_seek_ratio(ui, &response, line)
                && let Some(player) = media.existing_player(id)
            {
                let duration = player.duration();
                player.seek(duration * ratio);
            }
        }

        #[cfg(not(target_os = "android"))]
        {
            let response = ui.interact(
                seek_rect,
                ui.id().with(("seek", id)),
                Sense::click_and_drag(),
            );
            if let Some(pointer) = response
                .interact_pointer_pos()
                .filter(|_| response.dragged() || response.clicked())
            {
                let ratio = ((pointer.x - line.min.x) / line.width()).clamp(0.0, 1.0) as f64;
                if let Some(player) = media.existing_player(id) {
                    let duration = player.duration();
                    player.seek(duration * ratio);
                }
            }
        }

        ui.painter().text(
            egui::pos2(line.max.x + space::SM, mid),
            egui::Align2::LEFT_CENTER,
            time,
            text::footnote(),
            Color32::from_white_alpha(200),
        );
    }

    outcome
}

#[cfg(target_os = "android")]
fn android_seek_ratio(ui: &egui::Ui, response: &egui::Response, line: Rect) -> Option<f64> {
    // 0 = ainda indeciso; 1 = horizontal/scrub; -1 = vertical/scroll.
    let gesture_id = response.id.with("android-axis-lock");

    if response.clicked() {
        ui.ctx().data_mut(|data| data.remove::<i8>(gesture_id));
        let pointer = response
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_interact_pos())?;
        return Some(((pointer.x - line.min.x) / line.width()).clamp(0.0, 1.0) as f64);
    }

    let (down, origin, pointer) = ui.input(|input| {
        (
            input.pointer.primary_down(),
            input.pointer.press_origin(),
            input.pointer.latest_pos(),
        )
    });

    if !down {
        ui.ctx().data_mut(|data| data.remove::<i8>(gesture_id));
        return None;
    }

    let (Some(origin), Some(pointer)) = (origin, pointer) else {
        return None;
    };
    if !response.rect.contains(origin) {
        return None;
    }

    let delta = pointer - origin;
    let mut axis = ui
        .ctx()
        .data(|data| data.get_temp::<i8>(gesture_id))
        .unwrap_or(0);

    // Espera sair do touch slop antes de escolher. Depois de escolhido, o
    // eixo não muda no meio do gesto mesmo se o dedo derivar um pouco.
    if axis == 0 && delta.length() >= 6.0 {
        const BIAS: f32 = 1.15;
        if delta.x.abs() > delta.y.abs() * BIAS {
            axis = 1;
        } else if delta.y.abs() > delta.x.abs() * BIAS {
            axis = -1;
        }
        if axis != 0 {
            ui.ctx().data_mut(|data| data.insert_temp(gesture_id, axis));
        }
    }

    (axis == 1)
        .then(|| ((pointer.x - line.min.x) / line.width()).clamp(0.0, 1.0) as f64)
}

fn control(ui: &mut egui::Ui, rect: Rect, glyph: &str, id: &str, tag: &str) -> bool {
    let response = ui.interact(rect, ui.id().with((tag, id)), Sense::click());
    let color = if response.hovered() {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(190)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        text::icon(14.0),
        color,
    );
    response.clicked()
}

/// Barras do áudio; a parte já tocada usa a cor de destaque.
pub fn waveform(ui: &egui::Ui, t: &Tokens, rect: Rect, peaks: &[f32], progress: f32) {
    if peaks.is_empty() {
        let line = Rect::from_min_max(
            egui::pos2(rect.min.x, rect.center().y - 2.0),
            egui::pos2(rect.max.x, rect.center().y + 2.0),
        );
        ui.painter()
            .rect_filled(line, CornerRadius::same(2), t.fill_medium);
        let played = Rect::from_min_size(line.min, Vec2::new(line.width() * progress, line.height()));
        ui.painter()
            .rect_filled(played, CornerRadius::same(2), t.accent);
        return;
    }

    let bar = 2.0;
    let gap = 1.0;
    let count = ((rect.width() / (bar + gap)).floor() as usize).max(1);
    let step = (peaks.len() as f32 / count as f32).max(1.0);
    let mid = rect.center().y;
    let painter = ui.painter();
    for index in 0..count {
        let peak = peaks[((index as f32 * step) as usize).min(peaks.len() - 1)];
        let height = (peak * rect.height()).max(2.0);
        let x = rect.min.x + index as f32 * (bar + gap);
        let color = if (index as f32 / count as f32) <= progress {
            t.accent
        } else {
            t.fill_medium
        };
        painter.rect_filled(
            Rect::from_min_max(
                egui::pos2(x, mid - height / 2.0),
                egui::pos2(x + bar, mid + height / 2.0),
            ),
            CornerRadius::same(1),
            color,
        );
    }
}

fn hover_badge(ui: &egui::Ui, t: &Tokens, rect: Rect, glyph: &str, tooltip: &str) {
    let _ = tooltip;
    let badge = Rect::from_center_size(
        egui::pos2(rect.max.x - 20.0, rect.min.y + 20.0),
        Vec2::splat(26.0),
    );
    ui.painter().rect_filled(
        badge,
        CornerRadius::same(radius::CONTROL),
        Color32::from_black_alpha(150),
    );
    ui.painter().text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        text::icon(14.0),
        Color32::WHITE,
    );
    let _ = t;
}

pub fn clock(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "0:00".into();
    }
    let total = seconds as u64;
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

pub fn size_label(bytes: i64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes.max(0) as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", value as i64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn elide(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}
