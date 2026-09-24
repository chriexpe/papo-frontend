//! Modo demonstração: povoa o estado sem servidor.
//!
//! Serve para ver a interface inteira — anexos, reações, respostas — enquanto
//! o backend não responde. Os arquivos de mídia são gerados de verdade e
//! gravados no cache com o id do anexo falso, então a imagem, o vídeo e o
//! áudio passam pelo mesmo caminho que passariam vindos da rede.

use chrono::{Duration, Local};

use crate::api::models::Attachment;
use crate::state::{
    Channel, ChannelKind, CustomEmoji, Emoji, Member, Message, Presence, Reaction, Screen, Server,
    Store,
};

/// Quem está na call de mentira. O modo demonstração não abre microfone
/// nenhum: a sala existe só para a grade, a pastilha e a folha terem o que
/// mostrar enquanto se trabalha no desenho delas.
///
/// Todo mundo entra de câmera desligada de propósito — assim a call começa
/// no canal, e é o botão da câmera que a faz subir para a folha. As três
/// formas ficam a um clique uma da outra.
pub fn call_members() -> Vec<crate::api::ws::VoiceMember> {
    vec![
        crate::api::ws::VoiceMember {
            user_id: "u-eu".into(),
            muted: true,
            camera_on: false,
            screen_sharing: false,
        },
        crate::api::ws::VoiceMember {
            user_id: "u-ana".into(),
            muted: false,
            camera_on: false,
            screen_sharing: false,
        },
        crate::api::ws::VoiceMember {
            user_id: "u-bruno".into(),
            muted: false,
            camera_on: false,
            screen_sharing: false,
        },
    ]
}

/// Preenche o store com um servidor de mentira.
pub fn seed(store: &mut Store, server_key: &str) {
    store.screen = Screen::Chat;
    store.connection = crate::api::ws::Connection::Online;
    store.me = "u-eu".into();
    store.my_name = "Christian".into();
    store.my_username = "christian".into();
    store.server = Some(Server {
        name: "Papo".into(),
        description: Some("demonstração".into()),
    });

    store.members = vec![
        member("u-eu", "Christian", Presence::Online, Some((88, 166, 255))),
        member("u-ana", "Ana", Presence::Online, Some((255, 138, 101))),
        member("u-bruno", "Bruno", Presence::Away, None),
        member("u-dora", "Dora", Presence::Busy, Some((167, 139, 250))),
        member("u-edu", "Edu", Presence::Offline, None),
    ];

    store.channels = vec![
        channel("c-geral", "geral", "Onde tudo começa — avisos e conversa solta", 1),
        channel("c-dev", "dev", "Código, revisões e o que quebrou hoje", 2),
        channel("c-design", "design", "Telas, protótipos e discussões de cor", 3),
        Channel {
            id: "c-voz".into(),
            name: "sala de voz".into(),
            kind: ChannelKind::Voice,
            topic: None,
            position: 4,
            unread: false,
            mentions: 0,
        },
    ];
    store.channels[1].unread = true;
    store.channels[1].mentions = 2;
    store.channels[2].unread = true;
    store.selected_channel = "c-geral".into();
    store.mark_loading("c-geral");
    store.mark_loading("c-dev");
    store.mark_loading("c-design");

    store.emojis = custom_emojis();
    let media = generate_media(server_key);
    let now = Local::now();
    let mut messages = vec![
        Message {
            id: "m-1".into(),
            channel_id: "c-geral".into(),
            author_id: "u-ana".into(),
            content: "bom dia! subi o clipe do teste de render, deu bem melhor que ontem".into(),
            at: now - Duration::minutes(94),
            edited: false,
            reply_to: None,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: vec![Reaction {
                emoji: Emoji::Unicode("👍".into()),
                count: 3,
                mine: true,
            }],
            pinned: false,
            pending: false,
        },
        Message {
            id: "m-2".into(),
            channel_id: "c-geral".into(),
            author_id: "u-ana".into(),
            content: String::new(),
            at: now - Duration::minutes(93),
            edited: false,
            reply_to: None,
            attachments: media.video.clone().into_iter().collect(),
            previews: Vec::new(),
            reactions: vec![
                Reaction {
                    emoji: Emoji::Unicode("🔥".into()),
                    count: 2,
                    mine: false,
                },
                Reaction {
                    emoji: Emoji::Unicode("😮".into()),
                    count: 1,
                    mine: false,
                },
            ],
            pinned: true,
            pending: false,
        },
        Message {
            id: "m-3".into(),
            channel_id: "c-geral".into(),
            author_id: "u-bruno".into(),
            content: "ficou ótimo. o degradê do fundo ainda serrilha um pouco na borda?".into(),
            at: now - Duration::minutes(88),
            edited: false,
            reply_to: Some("m-2".into()),
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            pending: false,
        },
        Message {
            id: "m-4".into(),
            channel_id: "c-geral".into(),
            author_id: "u-dora".into(),
            content: "peguei a paleta nova aqui, olha como fica no escuro".into(),
            at: now - Duration::minutes(41),
            edited: true,
            reply_to: None,
            attachments: media.image.clone().into_iter().collect(),
            previews: Vec::new(),
            reactions: vec![Reaction {
                emoji: Emoji::Unicode("❤️".into()),
                count: 4,
                mine: false,
            }],
            pinned: false,
            pending: false,
        },
        Message {
            id: "m-5".into(),
            channel_id: "c-geral".into(),
            author_id: "u-dora".into(),
            content: "e o áudio daquele aviso, pra quem não viu".into(),
            at: now - Duration::minutes(40),
            edited: false,
            reply_to: None,
            attachments: media.audio.clone().into_iter().collect(),
            previews: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            pending: false,
        },
        Message {
            id: "m-5b".into(),
            channel_id: "c-geral".into(),
            author_id: "u-eu".into(),
            content: "ficou muito bom 🔥 :papo:".into(),
            at: now - Duration::minutes(30),
            edited: false,
            reply_to: None,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: vec![Reaction {
                emoji: Emoji::Custom("e-papo".into()),
                count: 2,
                mine: false,
            }],
            pinned: false,
            pending: false,
        },
        Message {
            id: "m-5c".into(),
            channel_id: "c-geral".into(),
            author_id: "u-ana".into(),
            content: "🎉".into(),
            at: now - Duration::minutes(29),
            edited: false,
            reply_to: None,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            pending: false,
        },
        Message {
            id: "m-6".into(),
            channel_id: "c-geral".into(),
            author_id: "u-bruno".into(),
            content: "perfeito, @christian fecha isso hoje então 👀".into(),
            at: now - Duration::minutes(12),
            edited: false,
            reply_to: Some("m-4".into()),
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            pending: false,
        },
        Message {
            id: "m-7".into(),
            channel_id: "c-dev".into(),
            author_id: "u-bruno".into(),
            content: "o deploy quebrou no migrate, alguém olha? @christian".into(),
            at: now - Duration::minutes(6),
            edited: false,
            reply_to: None,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            pending: false,
        },
    ];
    messages.sort_by_key(|message| message.at);
    store.messages = messages;
}

/// Dois emojis do servidor, desenhados na hora: um deles pisca, para mostrar
/// que figurinha animada também funciona.
fn custom_emojis() -> Vec<CustomEmoji> {
    use base64::Engine as _;

    let still = circle_png((255, 138, 101), (64, 24, 16));
    let animated = blinking_gif();
    let encode = |bytes: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(bytes);

    let mut emojis = Vec::new();
    if let Some(bytes) = still {
        emojis.push(CustomEmoji {
            id: "e-papo".into(),
            name: "papo".into(),
            blob: Some(encode(bytes)),
        });
    }
    if let Some(bytes) = animated {
        emojis.push(CustomEmoji {
            id: "e-pisca".into(),
            name: "pisca".into(),
            blob: Some(encode(bytes)),
        });
    }
    emojis
}

/// Um círculo cheio com um brilho, em PNG.
fn circle_png(fill: (u8, u8, u8), shadow: (u8, u8, u8)) -> Option<Vec<u8>> {
    let size = 96u32;
    let mut buffer = image::RgbaImage::new(size, size);
    let centre = size as f32 / 2.0;
    for (x, y, pixel) in buffer.enumerate_pixels_mut() {
        let dx = x as f32 - centre;
        let dy = y as f32 - centre;
        let distance = (dx * dx + dy * dy).sqrt();
        let edge = centre - 4.0;
        if distance > edge {
            *pixel = image::Rgba([0, 0, 0, 0]);
            continue;
        }
        let light = 1.0 - (distance / edge) * 0.45;
        *pixel = image::Rgba([
            (fill.0 as f32 * light + shadow.0 as f32 * (1.0 - light)) as u8,
            (fill.1 as f32 * light + shadow.1 as f32 * (1.0 - light)) as u8,
            (fill.2 as f32 * light + shadow.2 as f32 * (1.0 - light)) as u8,
            255,
        ]);
    }
    let mut out = Vec::new();
    buffer
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

/// GIF de dois quadros: figurinha animada de verdade, do tamanho de um emoji.
fn blinking_gif() -> Option<Vec<u8>> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::Delay;

    let size = 96u32;
    let mut frames = Vec::new();
    for step in 0..2 {
        let mut buffer = image::RgbaImage::new(size, size);
        let centre = size as f32 / 2.0;
        let radius = if step == 0 { centre - 6.0 } else { centre - 18.0 };
        for (x, y, pixel) in buffer.enumerate_pixels_mut() {
            let dx = x as f32 - centre;
            let dy = y as f32 - centre;
            let inside = (dx * dx + dy * dy).sqrt() <= radius;
            *pixel = if inside {
                image::Rgba([120, 200, 255, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            };
        }
        frames.push(image::Frame::from_parts(
            buffer,
            0,
            0,
            Delay::from_numer_denom_ms(400, 1),
        ));
    }

    let mut out = Vec::new();
    {
        let mut encoder = GifEncoder::new(std::io::Cursor::new(&mut out));
        encoder.set_repeat(Repeat::Infinite).ok()?;
        encoder.encode_frames(frames).ok()?;
    }
    Some(out)
}

struct DemoMedia {
    image: Option<Attachment>,
    video: Option<Attachment>,
    audio: Option<Attachment>,
}

/// Gera os arquivos e os grava no cache com o id do anexo, que é onde o
/// carregador de mídia procura antes de pedir pela rede.
fn generate_media(server_key: &str) -> DemoMedia {
    DemoMedia {
        image: image_attachment(server_key),
        video: encoded_attachment(
            server_key,
            "a-video",
            "render.ogv",
            "video/ogg",
            "videotestsrc num-buffers=150 pattern=smpte ! \
             video/x-raw,width=960,height=540,framerate=30/1 ! theoraenc ! oggmux",
        ),
        audio: encoded_attachment(
            server_key,
            "a-audio",
            "aviso.ogg",
            "audio/ogg",
            "audiotestsrc num-buffers=360 wave=ticks ! audioconvert ! vorbisenc ! oggmux",
        ),
    }
}

/// Um degradê desenhado na mão, para não depender de arquivo externo.
fn image_attachment(server_key: &str) -> Option<Attachment> {
    let path =
        crate::media::authenticated_cache_path(server_key, "files", "a-imagem", "paleta.png");
    let (width, height) = (960u32, 600u32);
    if !path.exists() {
        let mut buffer = image::RgbaImage::new(width, height);
        for (x, y, pixel) in buffer.enumerate_pixels_mut() {
            let u = x as f32 / width as f32;
            let v = y as f32 / height as f32;
            let r = (24.0 + 180.0 * u * (1.0 - v * 0.6)) as u8;
            let g = (28.0 + 90.0 * v + 40.0 * u) as u8;
            let b = (60.0 + 170.0 * (1.0 - u) * (0.4 + v * 0.6)) as u8;
            *pixel = image::Rgba([r, g, b, 255]);
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        buffer.save(&path).ok()?;
    }
    // A miniatura e a imagem cheia saem do mesmo arquivo: o carregador
    // procura cada uma no seu lugar antes de pedir pela rede.
    for target in [
        crate::media::authenticated_cache_path(server_key, "thumbs", "a-imagem", ""),
        crate::media::authenticated_cache_path(server_key, "files", "a-imagem", ""),
    ] {
        if !target.exists() {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::copy(&path, &target);
        }
    }
    Some(attachment("a-imagem", "paleta.png", "image/png", &path))
}

/// Os anexos gerados do modo demonstração saem de um pipeline do GStreamer,
/// que no Android ainda não existe. Sem eles a demonstração continua de pé:
/// só não tem vídeo nem áudio para abrir.
#[cfg(target_os = "android")]
fn encoded_attachment(
    _server_key: &str,
    _id: &str,
    _name: &str,
    _mime: &str,
    _chain: &str,
) -> Option<Attachment> {
    None
}

/// Roda um pipeline do GStreamer até o fim e devolve o anexo correspondente.
#[cfg(not(target_os = "android"))]
fn encoded_attachment(
    server_key: &str,
    id: &str,
    name: &str,
    mime: &str,
    chain: &str,
) -> Option<Attachment> {
    use gstreamer as gst;
    use gstreamer::prelude::*;

    let path = crate::media::authenticated_cache_path(server_key, "files", id, name);
    if path.exists() {
        return Some(attachment(id, name, mime, &path));
    }
    if !crate::media::player::init() {
        return None;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let description = format!("{chain} ! filesink location=\"{}\"", path.display());
    let pipeline = gst::parse::launch(&description).ok()?;
    if pipeline.set_state(gst::State::Playing).is_err() {
        return None;
    }
    if let Some(bus) = pipeline.bus() {
        for message in bus.iter_timed(gst::ClockTime::from_seconds(60)) {
            match message.view() {
                gst::MessageView::Eos(_) => break,
                gst::MessageView::Error(error) => {
                    log::warn!("demo: {}", error.error());
                    break;
                }
                _ => {}
            }
        }
    }
    let _ = pipeline.set_state(gst::State::Null);
    path.exists().then(|| attachment(id, name, mime, &path))
}

fn attachment(id: &str, name: &str, mime: &str, path: &std::path::Path) -> Attachment {
    Attachment {
        id: id.into(),
        mime_type: Some(mime.into()),
        original_file_name: Some(name.into()),
        size_bytes: std::fs::metadata(path).map(|meta| meta.len() as i64).unwrap_or(0),
        thumbnail_id: (mime.starts_with("image/")).then(|| id.to_owned()),
        created_at: None,
        moderation_status: Some("clean".into()),
    }
}

fn member(id: &str, name: &str, presence: Presence, color: Option<(u8, u8, u8)>) -> Member {
    Member {
        id: id.into(),
        username: name.to_lowercase().replace(' ', "."),
        name: name.into(),
        presence,
        role_color: color.map(|(r, g, b)| [r, g, b]),
        roles: Vec::new(),
    }
}

fn channel(id: &str, name: &str, topic: &str, position: i32) -> Channel {
    Channel {
        id: id.into(),
        name: name.into(),
        kind: ChannelKind::Text,
        topic: Some(topic.into()),
        position,
        unread: false,
        mentions: 0,
    }
}
