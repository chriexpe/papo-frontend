//! egui texture adapter for the shared `papo-call` engine.
pub use papo_call::{Command, Frame, IceConfig, Kind, Shared, ice, slots};
use papo_core::api::net::{EventCallback, NetSender};
use papo_core::api::ws::Event;
use std::collections::HashMap;
use std::sync::Arc;
#[cfg(target_os = "android")]
use std::sync::mpsc;

struct EguiHost(egui::Context);
impl papo_call::Host for EguiHost {
    fn initialize(&self) -> bool {
        crate::media::player::init()
    }
    fn wake(&self) {
        self.0.request_repaint();
    }
    fn camera_allowed(&self) -> bool {
        #[cfg(target_os = "android")]
        {
            return crate::platform::permission::ensure(crate::platform::permission::CAMERA)
                == crate::platform::permission::Status::Granted;
        }
        #[cfg(not(target_os = "android"))]
        {
            true
        }
    }
    fn display_rotation(&self) -> i32 {
        #[cfg(target_os = "android")]
        {
            return crate::platform::jvm::call_activity_int("displayRotation").unwrap_or(0);
        }
        #[cfg(not(target_os = "android"))]
        {
            0
        }
    }
    fn present_frame(&self, publisher: Option<&str>, frame: &Frame) {
        #[cfg(target_os = "android")]
        if publisher
            .map(crate::platform::android_call::pip_wants)
            .unwrap_or_else(crate::platform::android_call::pip_wants_local)
        {
            crate::platform::android_call::present_pip_frame(frame);
        }
        #[cfg(not(target_os = "android"))]
        let _ = (publisher, frame);
    }
}
pub fn check() {
    papo_call::check(Arc::new(EguiHost(egui::Context::default())));
}

/// A call em andamento, do lado da janela.
pub struct Call {
    pub channel_id: String,
    transport: papo_call::Call,
    shared: Arc<Shared>,
    textures: HashMap<usize, (egui::TextureHandle, u64)>,
    preview: Option<(egui::TextureHandle, u64)>,
}

impl Call {
    /// Abre a call: monta o pipeline numa thread própria e espera o sinal
    /// verde do servidor (`Command::Ready`) para mandar a oferta.
    pub fn start(
        channel_id: String,
        ice: IceConfig,
        repaint: egui::Context,
        net: NetSender,
    ) -> Option<Self> {
        let transport =
            papo_call::Call::start(channel_id.clone(), ice, Arc::new(EguiHost(repaint)), net)?;
        let shared = transport.shared();
        Some(Self {
            channel_id,
            transport,
            shared,
            textures: HashMap::new(),
            preview: None,
        })
    }

    pub(crate) fn send(&self, command: Command) {
        self.transport.send(command);
    }

    /// Callback instalado no Net enquanto esta call existe. SDP/ICE que
    /// chegam do socket entram na thread do GStreamer sem esperar a UI.
    pub fn event_callback(&self, me: String, names: HashMap<String, String>) -> EventCallback {
        let channel_id = self.channel_id.clone();
        let commands = self.transport.commands();
        #[cfg(not(target_os = "android"))]
        let _ = (&me, &names);
        Arc::new(move |event| match event {
            Event::VoiceAnswer {
                channel_id: event_channel,
                sdp,
            } if event_channel == &channel_id => {
                let _ = commands.send(Command::Answer(sdp.clone()));
            }
            Event::VoiceOffer {
                channel_id: event_channel,
                sdp,
            } if event_channel == &channel_id => {
                let _ = commands.send(Command::Offer(sdp.clone()));
            }
            Event::VoiceCandidate {
                channel_id: event_channel,
                candidate,
                sdp_mline_index,
                ..
            } if event_channel == &channel_id => {
                let _ = commands.send(Command::Candidate {
                    candidate: candidate.clone(),
                    sdp_mline_index: sdp_mline_index.unwrap_or(0),
                });
            }
            #[cfg(target_os = "android")]
            Event::ActiveSpeakers {
                channel_id: event_channel,
                user_ids,
            } if event_channel == &channel_id => {
                // O servidor manda lista vazia entre rajadas de fala. Isso
                // não significa "escolha qualquer câmera": o PiP mantém o
                // último speaker até outro tomar a prioridade.
                if let Some(speaker) = user_ids.first() {
                    crate::platform::android_call::set_pip_target(Some(speaker));
                    let name = names.get(speaker).map(String::as_str).unwrap_or("");
                    crate::platform::android_call::set_pip_speaker(name);
                }
            }
            #[cfg(target_os = "android")]
            Event::VoiceLeft {
                channel_id: event_channel,
                user_id,
            } if event_channel == &channel_id && user_id == &me => {
                crate::platform::android_call::call_ended_from_network();
            }
            #[cfg(target_os = "android")]
            Event::Failure { code, .. }
                if code
                    .as_deref()
                    .is_some_and(|code| crate::state::call::fatal(code, false)) =>
            {
                crate::platform::android_call::call_ended_from_network();
            }
            _ => {}
        })
    }

    #[cfg(target_os = "android")]
    pub(crate) fn command_sender(&self) -> mpsc::Sender<Command> {
        self.transport.commands()
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

    /// Quem deve aparecer na grade. Pedir de novo o mesmo conjunto não
    /// custa nada; o que não couber fica esperando um lugar vagar.
    pub fn watch(&self, publishers: Vec<String>) {
        self.send(Command::Watching(publishers));
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

    pub fn is_live(&self) -> bool {
        self.shared.is_live()
    }

    /// O estado da câmera e a conta de quantas vezes ele mudou. O botão
    /// pergunta a ela, não ao próprio clique: onde não há webcam — no
    /// Flatpak, hoje — ligar não liga nada, e o botão tem de voltar sozinho.
    ///
    /// Quem chama compara a conta, não o valor: é a conta que denuncia a
    /// tentativa que não abriu câmera nenhuma e por isso não mudou nada.
    pub fn camera_state(&self) -> (u64, bool) {
        self.shared.camera_state()
    }

    /// Aviso que não acaba com a call, para aparecer uma vez.
    pub fn take_warning(&self) -> Option<String> {
        self.shared.take_warning()
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
        if let Some((texture, seq)) = cached
            && *seq == tile.seq
        {
            return Some(texture.clone());
        }
        let frame = tile.frame.clone()?;
        let seq = tile.seq;
        let image = egui::ColorImage {
            size: [frame.width, frame.height],
            pixels: frame
                .pixels
                .iter()
                .map(|[r, g, b, a]| egui::Color32::from_rgba_premultiplied(*r, *g, *b, *a))
                .collect(),
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
            tiles
                .iter()
                .any(|tile| tile.publisher.as_deref() == Some(publisher) && tile.kind == Some(kind))
        })
    }

    /// A sua própria imagem, direto da câmera.
    pub fn preview(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        let seq = self.shared.preview_sequence();
        if let Some((texture, stored)) = &self.preview
            && *stored == seq
        {
            return Some(texture.clone());
        }
        let frame = self.shared.preview.lock().ok()?.clone()?;
        let image = egui::ColorImage {
            size: [frame.width, frame.height],
            pixels: frame
                .pixels
                .iter()
                .map(|[r, g, b, a]| egui::Color32::from_rgba_premultiplied(*r, *g, *b, *a))
                .collect(),
            source_size: egui::vec2(frame.width as f32, frame.height as f32),
        };
        match &mut self.preview {
            Some((texture, stored)) => {
                texture.set(image, egui::TextureOptions::LINEAR);
                *stored = seq;
                Some(texture.clone())
            }
            None => {
                let texture = ctx.load_texture("call-preview", image, egui::TextureOptions::LINEAR);
                self.preview = Some((texture.clone(), seq));
                Some(texture)
            }
        }
    }
}
