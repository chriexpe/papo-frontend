//! Shared SFU/WebRTC transport. No egui types, Android Activity, textures or UI navigation.
//! Frames are immutable RGBA snapshots; the host owns permissions and presentation callbacks.
mod engine;
pub mod ice;
pub mod platform;
pub mod slots;

pub use ice::IceConfig;
use papo_core::api::net::{Command as NetCommand, EventCallback, NetSender};
use papo_core::api::ws::Event;
pub use slots::Kind;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};

/// Implemented by the UI shell. Background transport never needs a rendered frame to progress.
pub trait Host: Send + Sync + 'static {
    fn initialize(&self) -> bool;
    fn wake(&self);
    fn camera_allowed(&self) -> bool {
        true
    }
    fn display_rotation(&self) -> i32 {
        0
    }
    fn present_frame(&self, _publisher: Option<&str>, _frame: &Frame) {}
}

pub fn check(host: Arc<dyn Host>) {
    engine::check(host);
}

/// Um quadro de vídeo pronto para virar textura.
#[derive(Clone)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[u8; 4]>,
}

/// Um lugar de vídeo: de quem é e qual foi o último quadro dele.
#[derive(Default)]
pub struct Tile {
    /// Quem ocupa o lugar, segundo a nossa conta (ver `slots`).
    pub publisher: Option<String>,
    pub kind: Option<Kind>,
    pub frame: Option<Arc<Frame>>,
    pub seq: u64,
}

/// O que a thread da call publica e a janela lê.
#[derive(Default)]
pub struct Shared {
    pub tiles: Mutex<Vec<Tile>>,
    /// A sua própria câmera, para o retrato de canto.
    pub preview: Mutex<Option<Arc<Frame>>>,
    preview_seq: AtomicU64,
    /// A conexão WebRTC fechou de verdade (ICE e DTLS prontos).
    live: AtomicBool,
    /// A câmera está mesmo capturando, e quantos resultados de tentativa já
    /// foram publicados.
    ///
    /// O contador existe porque a resposta vem de outra thread e nem toda
    /// resposta muda o valor: tentar ligar uma câmera que não existe começa
    /// e termina com `camera == false`. Amostrar só o booleano não veria
    /// resposta nenhuma, e o botão ficaria aceso com câmera nenhuma. O
    /// contador publica o resultado da tentativa mesmo quando o valor não
    /// mudou — por isso ele avança a cada publicação, nunca só nas trocas.
    camera: AtomicBool,
    camera_revision: AtomicU64,
    /// Quebrou de um jeito que a call não continua.
    failed: AtomicBool,
    error: Mutex<Option<String>>,
    /// Aviso que não acaba com a call, para ser mostrado uma vez só.
    warning: Mutex<Option<String>>,
}

impl Shared {
    /// Publica o resultado **concluído** de uma tentativa de câmera: abriu,
    /// não abriu, desligou, morreu no meio.
    ///
    /// Avança o contador sempre, inclusive quando o valor é o mesmo de
    /// antes: é justamente a tentativa que fracassa — `false` para `false` —
    /// que a janela precisa enxergar.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    fn set_camera(&self, on: bool) {
        self.camera.store(on, Ordering::Relaxed);
        self.camera_revision.fetch_add(1, Ordering::Release);
    }

    /// O par que a janela lê: a conta primeiro (Acquire contra o Release da
    /// escrita), para o valor que vem depois nunca ser mais velho que ela.
    pub fn camera_state(&self) -> (u64, bool) {
        let revision = self.camera_revision.load(Ordering::Acquire);
        (revision, self.camera.load(Ordering::Relaxed))
    }

    pub fn preview_sequence(&self) -> u64 {
        self.preview_seq.load(Ordering::Acquire)
    }

    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::Relaxed)
    }

    /// O aviso que ainda não foi mostrado. Sai da gaveta ao ser lido.
    pub fn take_warning(&self) -> Option<String> {
        self.warning.lock().ok().and_then(|mut slot| slot.take())
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|slot| slot.clone())
    }

    /// Um aviso que não acaba com a call — o microfone que não abriu, ou a
    /// câmera que não existe: dá para participar assim mesmo. Fica numa
    /// gaveta própria, longe do erro fatal, porque quem o lê o tira de lá:
    /// no erro ele apareceria em todo quadro, ou em nenhum.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    fn warn(&self, message: impl Into<String>) {
        let message = message.into();
        log::warn!("call: {message}");
        if let Ok(mut slot) = self.warning.lock() {
            *slot = Some(message);
        }
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
// Sem a call, no Android, isto fica sem uso — mas continua sendo o
// contrato da thread de voz, que volta inteiro no PR de mídia.
#[cfg_attr(target_os = "android", allow(dead_code))]
pub enum Command {
    /// O servidor aceitou a entrada: pode montar e mandar a oferta.
    Ready,
    Muted(bool),
    Camera(bool),
    /// De quem queremos ver a câmera. Vai o conjunto inteiro, não a
    /// diferença: quem decide o que cabe nos seis lugares é a thread da
    /// call, que é quem sabe quais estão livres.
    Watching(Vec<String>),
    Answer(String),
    Offer(String),
    Candidate {
        candidate: String,
        sdp_mline_index: u32,
    },
    /// O `webrtcbin` avisou que a sessão mudou e precisa de oferta nova.
    #[allow(dead_code)]
    Negotiate,
    /// A SDP do servidor terminou de ser aplicada (`true` quando era
    /// resposta à nossa oferta).
    RemoteApplied(bool),
    Stop,
}

/// Owns the media and signaling workers. Drop requests shutdown without blocking a UI thread.
pub struct Call {
    pub channel_id: String,
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
}
impl Call {
    pub fn start(
        channel_id: String,
        ice: IceConfig,
        host: Arc<dyn Host>,
        net: NetSender,
    ) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let (signals_tx, signals_rx) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        engine::spawn(
            channel_id.clone(),
            ice,
            Arc::clone(&shared),
            rx,
            tx.clone(),
            signals_tx,
            host,
        )?;
        std::thread::Builder::new()
            .name("papo-call-signal".into())
            .spawn(move || {
                while let Ok(signal) = signals_rx.recv() {
                    net.send(NetCommand::VoiceSignal(signal));
                }
            })
            .ok()?;
        Some(Self {
            channel_id,
            commands: tx,
            shared,
        })
    }
    pub fn shared(&self) -> Arc<Shared> {
        Arc::clone(&self.shared)
    }
    pub fn commands(&self) -> mpsc::Sender<Command> {
        self.commands.clone()
    }
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
    pub fn event_callback(&self) -> EventCallback {
        let channel = self.channel_id.clone();
        let commands = self.commands.clone();
        Arc::new(move |event| {
            let command = match event {
                Event::VoiceAnswer { channel_id, sdp } if channel_id == &channel => {
                    Command::Answer(sdp.clone())
                }
                Event::VoiceOffer { channel_id, sdp } if channel_id == &channel => {
                    Command::Offer(sdp.clone())
                }
                Event::VoiceCandidate {
                    channel_id,
                    candidate,
                    sdp_mline_index,
                    ..
                } if channel_id == &channel => Command::Candidate {
                    candidate: candidate.clone(),
                    sdp_mline_index: sdp_mline_index.unwrap_or(0),
                },
                _ => return,
            };
            let _ = commands.send(command);
        })
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        self.send(Command::Stop);
    }
}
