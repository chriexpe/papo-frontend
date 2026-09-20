//! Voz e vídeo: a call de um canal de voz.
//!
//! O backend é um SFU: cada pessoa fala com o servidor, nunca com as outras.
//! O transporte é WebRTC, feito pelo `webrtcbin` do GStreamer — ICE, DTLS e
//! SRTP vêm prontos —, e a sinalização (oferta, resposta e candidatos) vai
//! pelo mesmo WebSocket que já carrega mensagem e presença.
//!
//! Quem oferece é sempre o cliente. A sequência de uma entrada é:
//!
//! 1. `GET /voice/ice-servers` (STUN e, se houver, TURN com credencial só sua);
//! 2. `voice_join` pelo socket → o servidor responde `voice_joined` com quem
//!    já está na sala;
//! 3. montamos a oferta — microfone saindo, mais os lugares de recepção — e
//!    mandamos `voice_offer`; a resposta vem em `voice_answer`;
//! 4. os candidatos de ICE vão e voltam em `voice_ice_candidate` até a
//!    conexão fechar.
//!
//! **Nada do GStreamer é chamado pela thread que desenha**, a mesma regra do
//! player: a call vive numa thread própria e a janela só troca comandos e lê
//! o que está publicado.

pub mod ice;
mod engine;
pub mod slots;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub use ice::IceConfig;
pub use slots::Kind;

/// Um quadro de vídeo pronto para virar textura.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<egui::Color32>,
}

/// Um lugar de vídeo: de quem é e qual foi o último quadro dele.
#[derive(Default)]
pub struct Tile {
    /// Quem ocupa o lugar, segundo a nossa conta (ver `slots`).
    pub publisher: Option<String>,
    pub kind: Option<Kind>,
    pub frame: Option<Frame>,
    pub seq: u64,
}

/// O que a thread da call publica e a janela lê.
#[derive(Default)]
pub struct Shared {
    pub tiles: Mutex<Vec<Tile>>,
    /// A sua própria câmera, para o retrato de canto.
    pub preview: Mutex<Option<Frame>>,
    preview_seq: AtomicU64,
    /// A conexão WebRTC fechou de verdade (ICE e DTLS prontos).
    live: AtomicBool,
    /// Quebrou de um jeito que a call não continua.
    failed: AtomicBool,
    error: Mutex<Option<String>>,
}

impl Shared {
    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::Relaxed)
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|slot| slot.clone())
    }

    fn fail(&self, message: impl Into<String>) {
        let message = message.into();
        log::warn!("call: {message}");
        if let Ok(mut slot) = self.error.lock() {
            *slot = Some(message);
        }
        self.failed.store(true, Ordering::Relaxed);
    }
}

/// Ordens da janela para a thread da call.
pub(crate) enum Command {
    /// O servidor aceitou a entrada: pode montar e mandar a oferta.
    Ready,
    Muted(bool),
    Camera(bool),
    /// Começa (ou para) de receber a câmera ou a tela de alguém.
    Watch {
        publisher: String,
        kind: Kind,
        on: bool,
    },
    /// Alguém parou de publicar aquele tipo de vídeo — o servidor larga o
    /// lugar sozinho, e a nossa conta precisa largar junto.
    Published {
        publisher: String,
        kind: Kind,
        on: bool,
    },
    /// Alguém saiu da call.
    Gone(String),
    Answer(String),
    Offer(String),
    Candidate {
        candidate: String,
        sdp_mline_index: u32,
    },
    /// O `webrtcbin` avisou que a sessão mudou e precisa de oferta nova.
    Negotiate,
    Stop,
}

/// A call em andamento, do lado da janela.
pub struct Call {
    pub channel_id: String,
    commands: mpsc::Sender<Command>,
    signals: mpsc::Receiver<String>,
    shared: Arc<Shared>,
    /// Texturas por lugar de vídeo, recriadas só quando o quadro muda.
    textures: HashMap<usize, (egui::TextureHandle, u64)>,
    preview: Option<(egui::TextureHandle, u64)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Call {
    /// Abre a call: monta o pipeline numa thread própria e espera o sinal
    /// verde do servidor (`Command::Ready`) para mandar a oferta.
    pub fn start(channel_id: String, ice: IceConfig, repaint: egui::Context) -> Option<Self> {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (signals_tx, signals_rx) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let thread = engine::spawn(
            channel_id.clone(),
            ice,
            Arc::clone(&shared),
            commands_rx,
            commands_tx.clone(),
            signals_tx,
            repaint,
        )?;
        Some(Self {
            channel_id,
            commands: commands_tx,
            signals: signals_rx,
            shared,
            textures: HashMap::new(),
            preview: None,
            thread: Some(thread),
        })
    }

    pub(crate) fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// O JSON que a call quer mandar pelo socket. A janela drena isto todo
    /// quadro e repassa para a rede.
    pub fn take_signals(&self) -> Vec<String> {
        self.signals.try_iter().collect()
    }

    pub fn ready(&self) {
        self.send(Command::Ready);
    }

    pub fn set_muted(&self, muted: bool) {
        self.send(Command::Muted(muted));
    }

    pub fn set_camera(&self, on: bool) {
        self.send(Command::Camera(on));
    }

    pub fn watch(&self, publisher: &str, kind: Kind, on: bool) {
        self.send(Command::Watch {
            publisher: publisher.to_owned(),
            kind,
            on,
        });
    }

    pub fn published(&self, publisher: &str, kind: Kind, on: bool) {
        self.send(Command::Published {
            publisher: publisher.to_owned(),
            kind,
            on,
        });
    }

    /// A resposta SDP do servidor à nossa oferta.
    pub fn answer(&self, sdp: String) {
        self.send(Command::Answer(sdp));
    }

    /// Uma oferta que partiu do servidor.
    pub fn offer(&self, sdp: String) {
        self.send(Command::Offer(sdp));
    }

    pub fn candidate(&self, candidate: String, sdp_mline_index: u32) {
        self.send(Command::Candidate {
            candidate,
            sdp_mline_index,
        });
    }

    pub fn gone(&self, publisher: &str) {
        self.send(Command::Gone(publisher.to_owned()));
    }

    pub fn is_live(&self) -> bool {
        self.shared.is_live()
    }

    pub fn failed(&self) -> bool {
        self.shared.failed()
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error()
    }

    /// A textura do vídeo de alguém, se estivermos recebendo. Sobe o quadro
    /// novo só quando ele muda de verdade.
    pub fn video(
        &mut self,
        ctx: &egui::Context,
        publisher: &str,
        kind: Kind,
    ) -> Option<egui::TextureHandle> {
        let mut tiles = self.shared.tiles.lock().ok()?;
        let index = tiles.iter().position(|tile| {
            tile.publisher.as_deref() == Some(publisher) && tile.kind == Some(kind)
        })?;
        let tile = &mut tiles[index];
        let cached = self.textures.get(&index);
        if let Some((texture, seq)) = cached {
            if *seq == tile.seq {
                return Some(texture.clone());
            }
        }
        let frame = tile.frame.take()?;
        let seq = tile.seq;
        let image = egui::ColorImage {
            size: [frame.width, frame.height],
            pixels: frame.pixels,
            source_size: egui::vec2(frame.width as f32, frame.height as f32),
        };
        let texture = match self.textures.get_mut(&index) {
            Some((texture, stored)) => {
                texture.set(image, egui::TextureOptions::LINEAR);
                *stored = seq;
                texture.clone()
            }
            None => {
                let texture =
                    ctx.load_texture(format!("call-{index}"), image, egui::TextureOptions::LINEAR);
                self.textures.insert(index, (texture.clone(), seq));
                texture
            }
        };
        Some(texture)
    }

    /// Estamos recebendo (ou esperando) o vídeo desta pessoa? Falso quando
    /// os seis lugares já estão ocupados por outras câmeras.
    pub fn has_slot(&self, publisher: &str, kind: Kind) -> bool {
        self.shared.tiles.lock().is_ok_and(|tiles| {
            tiles.iter().any(|tile| {
                tile.publisher.as_deref() == Some(publisher) && tile.kind == Some(kind)
            })
        })
    }

    /// A sua própria imagem, direto da câmera.
    pub fn preview(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        let seq = self.shared.preview_seq.load(Ordering::Relaxed);
        if let Some((texture, stored)) = &self.preview {
            if *stored == seq {
                return Some(texture.clone());
            }
        }
        let frame = self.shared.preview.lock().ok()?.take()?;
        let image = egui::ColorImage {
            size: [frame.width, frame.height],
            pixels: frame.pixels,
            source_size: egui::vec2(frame.width as f32, frame.height as f32),
        };
        match &mut self.preview {
            Some((texture, stored)) => {
                texture.set(image, egui::TextureOptions::LINEAR);
                *stored = seq;
                Some(texture.clone())
            }
            None => {
                let texture =
                    ctx.load_texture("call-preview", image, egui::TextureOptions::LINEAR);
                self.preview = Some((texture.clone(), seq));
                Some(texture)
            }
        }
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        // Sem esperar pela thread: desmontar um pipeline com microfone e
        // câmera leva dezenas de milissegundos, e nenhum deles pode sair do
        // orçamento do quadro. A thread fecha tudo sozinha ao ver o Stop.
        let _ = self.commands.send(Command::Stop);
        self.thread.take();
    }
}
