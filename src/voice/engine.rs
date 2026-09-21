//! A thread da call: monta o pipeline do `webrtcbin`, responde à sinalização
//! e publica os quadros que chegam.
//!
//! Desenho do pipeline, do jeito que o SFU espera:
//!
//! ```text
//!   microfone ─ opusenc ─ rtpopuspay ─┐
//!                                     ├─ webrtcbin ─┬─ 6 lugares de vídeo → appsink → janela
//!   câmera ─ appsrc ─ vp8enc ─ rtpvp8pay ┘          └─ 8 lugares de áudio → alto-falante
//! ```
//!
//! Os lugares de recepção são ofertados **vazios** logo na entrada: o
//! servidor não renegocia quando alguém liga a câmera, ele só passa a mandar
//! pacotes num lugar que já existe. Por isso a oferta inicial já leva seis
//! `recvonly` de vídeo e oito de áudio.
//!
//! A câmera entra por um `appsrc` em vez de ligar a `v4l2src` direto no
//! pipeline: assim desligar a câmera fecha o dispositivo de verdade (a luz
//! apaga) sem mexer numa linha da sessão WebRTC. Desmontar a linha seria
//! pior — cada volta criaria uma `m=` nova, e o servidor recusa ofertas com
//! mais linhas do que os lugares que ele abriu.

use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};

use gstreamer as gst;
use gstreamer::glib;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_rtp as gst_rtp;
use gstreamer_rtp::prelude::RTPHeaderExtensionExt;

use gstreamer_sdp as gst_sdp;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::VideoFrameExt;
use gstreamer_webrtc as gst_webrtc;

use super::ice::IceConfig;
use super::slots::{Kind, Slots, AUDIO_SLOTS, VIDEO_SLOTS};
use super::{Command, Frame, Shared, Tile};

/// Tamanho da imagem que sai da câmera. 360p a 30 quadros cabe folgado no
/// que um SFU de sala pequena aguenta e é o que a grade mostra.
const CAMERA_WIDTH: i32 = 640;
const CAMERA_HEIGHT: i32 = 360;

/// Teto de banda da câmera, em bits por segundo. O controle de congestão do
/// servidor limita o que ele reenvia; o que sai daqui quem limita somos nós.
const CAMERA_BITRATE: i32 = 600_000;

/// Espera entre voltas do laço quando não há comando nenhum — é também de
/// quanto em quanto tempo o barramento do GStreamer é lido.
const TICK: std::time::Duration = std::time::Duration::from_millis(50);

pub(super) fn spawn(
    channel_id: String,
    ice: IceConfig,
    shared: Arc<Shared>,
    commands: mpsc::Receiver<Command>,
    inbox: mpsc::Sender<Command>,
    signals: mpsc::Sender<String>,
    repaint: egui::Context,
) -> Option<std::thread::JoinHandle<()>> {
    if !crate::media::player::init() {
        shared.fail("GStreamer indisponível");
        return None;
    }
    std::thread::Builder::new()
        .name("papo-call".into())
        .spawn(move || {
            let mut engine =
                match Engine::build(channel_id, &ice, shared.clone(), inbox, signals, repaint) {
                    Some(engine) => engine,
                    None => return,
                };
            engine.run(commands);
        })
        .ok()
}

struct Engine {
    channel_id: String,
    pipeline: gst::Pipeline,
    webrtc: gst::Element,
    shared: Arc<Shared>,
    signals: mpsc::Sender<String>,
    /// Quem ocupa cada lugar de vídeo, na mesma conta que o servidor faz.
    slots: Slots,
    mic: Option<gst::Element>,
    camera: Option<Camera>,
    /// A linha de envio de vídeo, criada na primeira vez que a câmera liga e
    /// reaproveitada depois (ligada e desligada pela direção).
    camera_line: Option<gst_webrtc::WebRTCRTPTransceiver>,
    camera_src: Option<gst_app::AppSrc>,
    camera_on: bool,
    /// De quem a janela quer ver a câmera. Pode ser mais gente do que cabe:
    /// o que sobra espera um lugar vagar.
    wanted: Vec<String>,
    repaint: egui::Context,
    /// A volta para a própria thread: as promessas do GStreamer respondem
    /// por aqui, em vez de mexerem no estado de outra thread.
    inbox: mpsc::Sender<Command>,
    /// Já podemos ofertar (o servidor aceitou a entrada).
    ready: bool,
    /// Uma oferta está no ar e ainda não voltou.
    negotiating: bool,
    /// Alguma mudança pediu renegociação enquanto a anterior não terminou.
    pending: bool,
    stopping: bool,
}

/// A captura da câmera, num pipeline à parte: ligar e desligar não encosta
/// na sessão WebRTC.
struct Camera {
    pipeline: gst::Pipeline,
}

impl Drop for Camera {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl Engine {
    fn build(
        channel_id: String,
        ice: &IceConfig,
        shared: Arc<Shared>,
        inbox: mpsc::Sender<Command>,
        signals: mpsc::Sender<String>,
        repaint: egui::Context,
    ) -> Option<Self> {
        let pipeline = gst::Pipeline::new();
        let webrtc = match gst::ElementFactory::make("webrtcbin")
            .name("call")
            // Uma sessão de ICE e um DTLS só para as dezesseis linhas. Sem
            // isso seria uma negociação por linha: o servidor recusa, e
            // nenhum SFU trabalha assim.
            .property_from_str("bundle-policy", "max-bundle")
            .property("latency", 120u32)
            .build()
        {
            Ok(element) => element,
            Err(error) => {
                shared.fail(format!("sem webrtcbin: {error}"));
                return None;
            }
        };
        if let Some(stun) = &ice.stun {
            webrtc.set_property("stun-server", stun);
        }
        for turn in &ice.turn {
            let added: bool = webrtc.emit_by_name("add-turn-server", &[&turn.as_str()]);
            if !added {
                log::warn!("call: o servidor TURN não foi aceito");
            }
        }
        if pipeline.add(&webrtc).is_err() {
            shared.fail("pipeline não aceitou o webrtcbin");
            return None;
        }

        let video_lines_slot: Arc<Mutex<Vec<gst_webrtc::WebRTCRTPTransceiver>>> =
            Arc::new(Mutex::new(Vec::new()));

        connect_signals(
            &webrtc,
            &pipeline,
            &shared,
            &inbox,
            &signals,
            &channel_id,
            Arc::clone(&video_lines_slot),
            repaint.clone(),
        );

        // O transporte sobe primeiro, sozinho. Assim uma falha aqui é uma
        // falha dele — o ICE que não existe no sandbox, por exemplo — e não
        // se confunde com a do microfone, que sobe depois e pode faltar sem
        // acabar com a call.
        if let Err(error) = pipeline.set_state(gst::State::Playing) {
            let detail = drain_error(&pipeline).unwrap_or_else(|| error.to_string());
            shared.fail(format!("a call não entrou no ar: {detail}"));
            return None;
        }

        // O microfone é a linha 0 da oferta: a ordem em que os transceptores
        // nascem é a ordem das linhas lá, e ele nasce antes dos lugares de
        // recepção mesmo com o pipeline já andando.
        let mic = microphone(&pipeline, &webrtc, &shared);

        // Depois os lugares de recepção. A ordem em que entram é a ordem em
        // que o servidor os preenche, e é dela que sai a conta de quem ocupa
        // qual lugar.
        let mut video_lines = Vec::with_capacity(VIDEO_SLOTS);
        for _ in 0..VIDEO_SLOTS {
            match add_receiver(&webrtc, video_caps()) {
                Some(line) => video_lines.push(line),
                None => {
                    shared.fail("não deu para abrir os lugares de vídeo");
                    return None;
                }
            }
        }
        for _ in 0..AUDIO_SLOTS {
            if add_receiver(&webrtc, audio_caps()).is_none() {
                shared.fail("não deu para abrir os lugares de áudio");
                return None;
            }
        }
        // Os lugares só existem agora, e o `pad-added` precisa deles para
        // saber de quem é o vídeo que chegou. Ele não pode disparar antes da
        // negociação, que ainda nem começou.
        if let Ok(mut slot) = video_lines_slot.lock() {
            *slot = video_lines;
        }

        if let Ok(mut tiles) = shared.tiles.lock() {
            *tiles = (0..VIDEO_SLOTS).map(|_| Tile::default()).collect();
        }

        Some(Self {
            channel_id,
            pipeline,
            webrtc,
            shared,
            signals,
            slots: Slots::new(VIDEO_SLOTS),
            mic,
            camera: None,
            camera_line: None,
            camera_src: None,
            camera_on: false,
            wanted: Vec::new(),
            repaint,
            inbox,
            ready: false,
            negotiating: false,
            pending: false,
            stopping: false,
        })
    }

    fn run(&mut self, commands: mpsc::Receiver<Command>) {
        let bus = self.pipeline.bus();
        loop {
            match commands.recv_timeout(TICK) {
                Ok(command) => self.handle(command),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if self.stopping {
                break;
            }
            if let Some(bus) = &bus {
                while let Some(message) = bus.pop() {
                    if let gst::MessageView::Error(error) = message.view() {
                        // Um erro de elemento derruba a call: sem áudio nem
                        // vídeo não há o que continuar mostrando.
                        self.shared.fail(format!(
                            "{} ({})",
                            error.error(),
                            error.debug().unwrap_or_default()
                        ));
                    }
                }
            }
            // A câmera é um pipeline à parte e pode morrer sozinha depois de
            // ter começado — o cabo saiu, outro programa tomou o
            // dispositivo. Sem ler o barramento dela, a call ficaria com a
            // câmera "ligada" e sem quadro nenhum, sem dizer por quê.
            self.watch_camera();
        }
        self.camera = None;
        let _ = self.pipeline.set_state(gst::State::Null);
    }

    /// Lê o barramento da câmera. Erro dela apaga a câmera e vira aviso —
    /// a call segue.
    fn watch_camera(&mut self) {
        let Some(camera) = &self.camera else { return };
        let Some(bus) = camera.pipeline.bus() else {
            return;
        };
        let mut broke = None;
        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::Error(error) => broke = Some(error.error().to_string()),
                // Dispositivo que some às vezes termina o fluxo em vez de
                // dar erro; para nós é a mesma notícia.
                gst::MessageView::Eos(_) => {
                    broke = Some("a captura terminou".to_owned());
                }
                _ => {}
            }
        }
        if let Some(reason) = broke {
            self.shared.warn(format!("a câmera parou: {reason}"));
            self.set_camera(false);
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Ready => {
                self.ready = true;
                self.negotiate();
            }
            Command::Muted(muted) => {
                if let Some(mic) = &self.mic {
                    mic.set_property("mute", muted);
                }
                self.signal(serde_json::json!({
                    "type": "voice_mute",
                    "channel_id": self.channel_id,
                    "muted": muted,
                }));
            }
            Command::Camera(on) => self.set_camera(on),
            Command::Watching(publishers) => {
                self.wanted = publishers;
                self.reconcile();
            }
            Command::Answer(sdp) => self.set_remote(&sdp, gst_webrtc::WebRTCSDPType::Answer),
            Command::Offer(sdp) => self.set_remote(&sdp, gst_webrtc::WebRTCSDPType::Offer),
            Command::RemoteApplied(answer) => self.remote_applied(answer),
            Command::Candidate {
                candidate,
                sdp_mline_index,
            } => {
                self.webrtc
                    .emit_by_name::<()>("add-ice-candidate", &[&sdp_mline_index, &candidate]);
            }
            Command::Negotiate => self.negotiate(),
            Command::Stop => self.stopping = true,
        }
    }

    /// Manda a oferta. Só uma de cada vez: o que chegar no meio espera a
    /// resposta da anterior, senão as duas se cruzam e a sessão desmonta.
    fn negotiate(&mut self) {
        if !self.ready || self.stopping {
            return;
        }
        if self.negotiating {
            self.pending = true;
            return;
        }
        self.negotiating = true;

        let webrtc = self.webrtc.clone();
        let signals = self.signals.clone();
        let channel_id = self.channel_id.clone();
        let shared = Arc::clone(&self.shared);
        let promise = gst::Promise::with_change_func(move |reply| {
            let offer = match reply {
                Ok(Some(reply)) => reply.get::<gst_webrtc::WebRTCSessionDescription>("offer").ok(),
                _ => None,
            };
            let Some(offer) = offer else {
                shared.fail("a oferta da call não saiu");
                return;
            };
            webrtc.emit_by_name::<()>("set-local-description", &[&offer, &None::<gst::Promise>]);
            let Ok(text) = offer.sdp().as_text() else {
                shared.fail("oferta ilegível");
                return;
            };
            let _ = signals.send(
                serde_json::json!({
                    "type": "voice_offer",
                    "channel_id": channel_id,
                    "sdp": text,
                })
                .to_string(),
            );
        });
        self.webrtc
            .emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
    }

    /// Aplica a SDP do servidor. O que vem depois — liberar a próxima
    /// oferta, ou responder à dele — só pode acontecer quando ela estiver
    /// mesmo aplicada, e quem avisa disso é a promessa: o `webrtcbin` aceita
    /// a SDP numa thread dele, e seguir na hora era correr com a transição
    /// anterior ainda em andamento.
    fn set_remote(&mut self, sdp: &str, kind: gst_webrtc::WebRTCSDPType) {
        let Ok(message) = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()) else {
            self.shared.fail("a resposta do servidor veio ilegível");
            return;
        };
        let description = gst_webrtc::WebRTCSessionDescription::new(kind, message);
        let inbox = self.inbox.clone();
        let shared = Arc::clone(&self.shared);
        let answer = kind == gst_webrtc::WebRTCSDPType::Answer;
        let promise = gst::Promise::with_change_func(move |reply| {
            // A promessa também chega interrompida ou vencida. Seguir para a
            // próxima oferta nesses casos seria negociar em cima de uma SDP
            // que nunca entrou.
            match reply {
                Ok(_) => {
                    let _ = inbox.send(Command::RemoteApplied(answer));
                }
                Err(error) => shared.fail(format!("a SDP do servidor não entrou: {error:?}")),
            }
        });
        self.webrtc
            .emit_by_name::<()>("set-remote-description", &[&description, &promise]);
    }

    /// A SDP do servidor entrou. `answer` diz se ela era resposta à nossa
    /// oferta (aí a negociação fechou) ou oferta dele (aí falta responder).
    fn remote_applied(&mut self, answer: bool) {
        if answer {
            self.negotiating = false;
            if self.pending {
                self.pending = false;
                self.negotiate();
            }
        } else {
            self.answer_remote();
        }
    }

    /// Responde a uma oferta que partiu do servidor. Hoje ele não faz isso —
    /// quem oferece é sempre o cliente —, mas o contrato prevê, e uma sessão
    /// que ignora a oferta do outro lado fica muda sem dizer por quê.
    fn answer_remote(&mut self) {
        let webrtc = self.webrtc.clone();
        let signals = self.signals.clone();
        let channel_id = self.channel_id.clone();
        let promise = gst::Promise::with_change_func(move |reply| {
            let answer = match reply {
                Ok(Some(reply)) => reply
                    .get::<gst_webrtc::WebRTCSessionDescription>("answer")
                    .ok(),
                _ => None,
            };
            let Some(answer) = answer else { return };
            webrtc.emit_by_name::<()>("set-local-description", &[&answer, &None::<gst::Promise>]);
            let Ok(text) = answer.sdp().as_text() else {
                return;
            };
            let _ = signals.send(
                serde_json::json!({
                    "type": "voice_answer",
                    "channel_id": channel_id,
                    "sdp": text,
                })
                .to_string(),
            );
        });
        self.webrtc
            .emit_by_name::<()>("create-answer", &[&None::<gst::Structure>, &promise]);
    }

    /// Acerta os lugares com o que a janela quer ver: larga quem saiu da
    /// lista, senta quem entrou, e deixa o excedente esperando.
    ///
    /// O servidor só manda vídeo com pedido explícito — é assim que uma sala
    /// de vinte não estoura a banda de quem só quer ouvir. Rodar isto de
    /// novo a cada lugar que vaga é o que faz a sétima câmera aparecer
    /// quando uma das seis primeiras desliga; era o furo de manter a conta
    /// do lado de fora, onde não se sabe o que coube.
    fn reconcile(&mut self) {
        let kind = Kind::Camera;
        let wanted = std::mem::take(&mut self.wanted);
        let (leaving, entering) = self.slots.reconcile(&wanted, kind);
        self.wanted = wanted;

        for publisher in leaving {
            self.signal(serde_json::json!({
                "type": "track_unsubscribe",
                "channel_id": self.channel_id,
                "publisher_id": publisher,
                "kind": kind.wire(),
            }));
        }
        for publisher in entering {
            self.signal(serde_json::json!({
                "type": "track_subscribe",
                "channel_id": self.channel_id,
                "publisher_id": publisher,
                "kind": kind.wire(),
            }));
        }
        self.publish_slots();
    }

    /// Reescreve nos lugares quem é o dono de cada um. O quadro que já
    /// estava lá vai embora junto: ele era da pessoa anterior.
    fn publish_slots(&self) {
        let Ok(mut tiles) = self.shared.tiles.lock() else {
            return;
        };
        for (index, tile) in tiles.iter_mut().enumerate() {
            let occupant = self.slots.occupant(index);
            let publisher = occupant.map(|slot| slot.publisher.clone());
            if tile.publisher != publisher {
                tile.frame = None;
                tile.seq = tile.seq.wrapping_add(1);
            }
            tile.publisher = publisher;
            tile.kind = occupant.map(|slot| slot.kind);
        }
    }

    fn set_camera(&mut self, on: bool) {
        if on == self.camera_on {
            return;
        }
        self.camera_on = on;

        if !on {
            // Solta o dispositivo primeiro: a luz da câmera tem de apagar
            // junto com o clique, não no fim da renegociação.
            self.camera = None;
            self.shared.set_camera(false);
            if let Ok(mut preview) = self.shared.preview.lock() {
                *preview = None;
            }
            if let Some(line) = &self.camera_line {
                line.set_property_from_str("direction", "inactive");
            }
            self.signal(serde_json::json!({
                "type": "voice_camera",
                "channel_id": self.channel_id,
                "on": false,
            }));
            self.negotiate();
            return;
        }

        if self.camera_line.is_none() {
            match self.open_camera_line() {
                Some((line, src)) => {
                    self.camera_line = Some(line);
                    self.camera_src = Some(src);
                }
                None => {
                    self.camera_on = false;
                    self.shared.set_camera(false);
                    return;
                }
            }
        }
        let Some(src) = self.camera_src.clone() else {
            self.camera_on = false;
            self.shared.set_camera(false);
            return;
        };
        match capture(src, Arc::clone(&self.shared), self.repaint.clone()) {
            Some(camera) => {
                self.camera = Some(camera);
                self.shared.set_camera(true);
            }
            None => {
                // Sem webcam — ou, no Flatpak, sem acesso a ela — a call
                // continua: era a câmera que não abriu, não a conversa.
                self.shared.warn("não achei uma câmera para abrir");
                self.camera_on = false;
                self.shared.set_camera(false);
                return;
            }
        }
        if let Some(line) = &self.camera_line {
            line.set_property_from_str("direction", "sendonly");
        }
        // A ordem importa: o servidor lê o estado da câmera para saber que a
        // linha nova é câmera. Se a oferta chegasse antes, ele a recusaria.
        self.signal(serde_json::json!({
            "type": "voice_camera",
            "channel_id": self.channel_id,
            "on": true,
        }));
        self.negotiate();
    }

    /// Cria a linha de envio de vídeo (uma vez por call) e devolve o
    /// `appsrc` por onde os quadros entram.
    fn open_camera_line(&mut self) -> Option<(gst_webrtc::WebRTCRTPTransceiver, gst_app::AppSrc)> {
        let src = gst_app::AppSrc::builder()
            .caps(
                &gst::Caps::builder("video/x-raw")
                    .field("format", "I420")
                    .field("width", CAMERA_WIDTH)
                    .field("height", CAMERA_HEIGHT)
                    .field("framerate", gst::Fraction::new(30, 1))
                    .build(),
            )
            .format(gst::Format::Time)
            .is_live(true)
            .do_timestamp(true)
            .build();

        let queue = make("queue")?;
        let encoder = make("vp8enc")?;
        // `deadline=1` é o modo tempo real do vp8: entrega o quadro no prazo
        // em vez de gastar o tempo que quiser procurando a melhor compressão.
        encoder.set_property("deadline", 1i64);
        encoder.set_property("target-bitrate", CAMERA_BITRATE);
        encoder.set_property("keyframe-max-dist", 60i32);
        encoder.set_property_from_str("end-usage", "cbr");
        encoder.set_property_from_str("error-resilient", "default");
        let payloader = make("rtpvp8pay")?;
        payloader.set_property("pt", 96u32);
        payloader.set_property_from_str("picture-id-mode", "15-bit");
        let filter = make("capsfilter")?;
        filter.set_property("caps", video_caps());

        let elements = [src.upcast_ref::<gst::Element>().clone(), queue, encoder, payloader, filter];
        self.pipeline.add_many(&elements).ok()?;
        gst::Element::link_many(&elements).ok()?;

        let pad = self.webrtc.request_pad_simple("sink_%u")?;
        let last = elements.last()?;
        last.static_pad("src")?.link(&pad).ok()?;
        for element in &elements {
            let _ = element.sync_state_with_parent();
        }
        let line = pad.property::<gst_webrtc::WebRTCRTPTransceiver>("transceiver");
        Some((line, src))
    }

    fn signal(&self, value: serde_json::Value) {
        let _ = self.signals.send(value.to_string());
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.camera = None;
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn make(name: &str) -> Option<gst::Element> {
    match gst::ElementFactory::make(name).build() {
        Ok(element) => Some(element),
        Err(error) => {
            log::warn!("call: falta o elemento {name} ({error})");
            None
        }
    }
}

fn video_caps() -> gst::Caps {
    gst::Caps::builder("application/x-rtp")
        .field("media", "video")
        .field("encoding-name", "VP8")
        .field("clock-rate", 90_000i32)
        .field("payload", 96i32)
        .build()
}

fn audio_caps() -> gst::Caps {
    gst::Caps::builder("application/x-rtp")
        .field("media", "audio")
        .field("encoding-name", "OPUS")
        .field("clock-rate", 48_000i32)
        .field("payload", 111i32)
        .field("encoding-params", "2")
        .build()
}

/// Abre um lugar de recepção — uma linha `recvonly` que nasce vazia e passa
/// a receber quando o servidor põe alguém nela.
fn add_receiver(
    webrtc: &gst::Element,
    caps: gst::Caps,
) -> Option<gst_webrtc::WebRTCRTPTransceiver> {
    let direction = gst_webrtc::WebRTCRTPTransceiverDirection::Recvonly;
    let line = webrtc.emit_by_name::<Option<gst_webrtc::WebRTCRTPTransceiver>>(
        "add-transceiver",
        &[&direction, &caps],
    );
    if line.is_none() {
        log::warn!("call: o webrtcbin recusou um lugar de recepção");
    }
    line
}

/// Confere o que a call precisa e diz o que falta, sem rede e sem janela.
///
/// Existe por causa do Flatpak: lá dentro o que falta não é sempre um
/// plugin — pode ser o soquete do PipeWire, ou a câmera que o sandbox não
/// entrega —, e de fora as duas falhas têm a mesma cara. Aqui cada peça é
/// testada de verdade, uma por uma.
pub fn check() {
    if !crate::media::player::init() {
        println!("GStreamer não iniciou: a call não tem como existir");
        return;
    }
    println!("GStreamer {}\n", gst::version_string());

    println!("peças:");
    for (name, purpose) in [
        ("webrtcbin", "o transporte"),
        ("nicesrc", "ICE — é o que falta quando a call não entra no ar"),
        ("dtlssrtpenc", "DTLS"),
        ("srtpenc", "SRTP"),
        ("rtpbin", "sessão RTP"),
        ("opusenc", "sua voz saindo"),
        ("opusdec", "a voz dos outros chegando"),
        ("rtpopuspay", "áudio em RTP"),
        ("rtpopusdepay", "áudio de volta do RTP"),
        ("rtphdrextclientaudiolevel", "quem está falando"),
        ("vp8enc", "sua câmera saindo"),
        ("vp8dec", "a câmera dos outros chegando"),
        ("rtpvp8pay", "vídeo em RTP"),
        ("rtpvp8depay", "vídeo de volta do RTP"),
        ("v4l2src", "a câmera"),
        ("appsrc", "a ponte da câmera"),
        ("appsink", "os quadros para a janela"),
    ] {
        let found = gst::ElementFactory::find(name).is_some();
        println!("  [{}] {name:<26} {purpose}", if found { "ok" } else { "--" });
    }

    println!("\nmicrofone (o primeiro que abrir é o usado):");
    for factory in ["pipewiresrc", "pulsesrc", "alsasrc"] {
        report(factory, probe(factory, "audioconvert", "fakesink"));
    }

    println!("\nalto-falante:");
    for factory in ["pipewiresink", "pulsesink", "alsasink", "autoaudiosink"] {
        report(factory, probe("audiotestsrc", "audioconvert", factory));
    }

    println!("\ncâmera:");
    report("v4l2src", probe("v4l2src", "videoconvert", "fakesink"));

    println!("\ntransporte:");
    let transport = match gst::ElementFactory::make("webrtcbin")
        .property_from_str("bundle-policy", "max-bundle")
        .build()
    {
        Ok(webrtc) => {
            let pipeline = gst::Pipeline::new();
            let outcome = match pipeline.add(&webrtc) {
                Ok(()) => match pipeline.set_state(gst::State::Playing) {
                    Ok(_) => match pipeline.state(gst::ClockTime::from_mseconds(1500)).0 {
                        Ok(_) => Ok(()),
                        Err(_) => Err(drain_error(&pipeline)
                            .unwrap_or_else(|| "não chegou a tocar".to_owned())),
                    },
                    Err(error) => {
                        Err(drain_error(&pipeline).unwrap_or_else(|| error.to_string()))
                    }
                },
                Err(error) => Err(error.to_string()),
            };
            let _ = pipeline.set_state(gst::State::Null);
            outcome
        }
        Err(error) => Err(error.to_string()),
    };
    report("webrtcbin", transport);
}

fn report(what: &str, outcome: Result<(), String>) {
    match outcome {
        Ok(()) => println!("  [ok] {what}"),
        Err(reason) => println!("  [--] {what}: {reason}"),
    }
}

/// Sobe `fonte ! conversor ! destino` de verdade e diz se ficou de pé.
///
/// O conversor no meio não é enfeite: sem nada que diga "isto é áudio", a
/// `pipewiresrc` não sabe a que se ligar e falha com `target not found` —
/// uma fonte boa reprovada por um teste mal feito. Aqui a cadeia é a mesma
/// que a call monta.
fn probe(source: &str, bridge: &str, sink: &str) -> Result<(), String> {
    let pipeline = gst::Pipeline::new();
    let Some(source) = make(source) else {
        return Err("o elemento não existe".to_owned());
    };
    let Some(bridge) = make(bridge) else {
        return Err("falta o conversor do teste".to_owned());
    };
    let Some(sink) = make(sink) else {
        return Err("o destino não existe".to_owned());
    };
    if source.has_property("do-timestamp") {
        source.set_property("do-timestamp", true);
    }
    let elements = [source, bridge, sink];
    let outcome = if pipeline.add_many(&elements).is_err()
        || gst::Element::link_many(&elements).is_err()
    {
        Err("não deu para montar".to_owned())
    } else {
        match pipeline.set_state(gst::State::Playing) {
            Ok(_) => match pipeline.state(gst::ClockTime::from_mseconds(1500)).0 {
                // Subir não basta: a fonte pode morrer logo depois, na
                // thread de fluxo, e é no barramento que isso aparece.
                Ok(_) => match drain_error(&pipeline) {
                    Some(reason) => Err(reason),
                    None => Ok(()),
                },
                Err(_) => Err(drain_error(&pipeline).unwrap_or_else(|| "não abriu".to_owned())),
            },
            Err(error) => Err(drain_error(&pipeline).unwrap_or_else(|| error.to_string())),
        }
    };
    let _ = pipeline.set_state(gst::State::Null);
    outcome
}

/// O que o barramento tem a dizer sobre uma falha.
///
/// Sem isto, "não entrou no ar" é tudo que se sabe — e o que costuma faltar
/// é um plugin, que tem nome. Dentro de um sandbox então, onde o que falta
/// pode ser o acesso ao dispositivo e não o plugin, a diferença é tudo.
fn drain_error(pipeline: &gst::Pipeline) -> Option<String> {
    let bus = pipeline.bus()?;
    let message = bus.timed_pop_filtered(
        gst::ClockTime::from_mseconds(250),
        &[gst::MessageType::Error],
    )?;
    let source = message
        .src()
        .map(|object| object.path_string().to_string())
        .unwrap_or_default();
    match message.view() {
        gst::MessageView::Error(error) => Some(format!("{} [{source}]", error.error())),
        _ => None,
    }
}

/// Microfone → Opus → a primeira linha da oferta. O silenciador fica antes
/// do codificador: mudo é o volume em zero, não o pipeline desmontado, para
/// falar de novo ser instantâneo.
///
/// A fonte é escolhida tentando de verdade. Não basta o elemento existir: no
/// Flatpak a `pipewiresrc` existe e falha ao conectar (o soquete do PipeWire
/// não está no sandbox), e é só ao subir que isso aparece. Cada tentativa
/// que falha é desfeita por inteiro — elementos fora do pipeline, pad
/// devolvido — antes da seguinte.
fn microphone(
    pipeline: &gst::Pipeline,
    webrtc: &gst::Element,
    shared: &Arc<Shared>,
) -> Option<gst::Element> {
    for factory in ["pipewiresrc", "pulsesrc", "alsasrc"] {
        let Some((elements, volume)) = mic_chain(factory) else {
            continue;
        };
        if pipeline.add_many(&elements).is_err() {
            continue;
        }
        if gst::Element::link_many(&elements).is_err() {
            let _ = pipeline.remove_many(&elements);
            continue;
        }
        let Some(pad) = webrtc.request_pad_simple("sink_%u") else {
            let _ = pipeline.remove_many(&elements);
            continue;
        };
        let linked = elements
            .last()
            .and_then(|last| last.static_pad("src"))
            .is_some_and(|src| src.link(&pad).is_ok());
        if linked {
            // Só enviar: uma linha `sendrecv` convidaria o servidor a
            // devolver áudio de outra pessoa por ela, e a conta dos lugares
            // deixaria de bater.
            if let Some(line) =
                pad.property::<Option<gst_webrtc::WebRTCRTPTransceiver>>("transceiver")
            {
                line.set_property_from_str("direction", "sendonly");
            }
        }

        // O pipeline já está andando: quem entra depois sobe sozinho. É aqui
        // que a fonte abre o dispositivo, e é aqui que ela falha.
        let mut opened = linked
            && elements
                .iter()
                .all(|element| element.sync_state_with_parent().is_ok());
        if opened {
            opened = elements
                .first()
                .is_some_and(|source| source.state(gst::ClockTime::from_mseconds(1500)).0.is_ok());
        }
        // E o erro que só aparece depois, na thread de fluxo: a
        // `pipewiresrc` dentro do Flatpak sobe e morre em seguida
        // ("target not found"). Sem olhar o barramento, a fonte passaria por
        // boa e a call ficaria muda — ou, pior, nem subiria.
        let mut reason = None;
        if opened {
            reason = drain_error(pipeline);
            opened = reason.is_none();
        }
        if opened {
            log::info!("call: microfone por {factory}");
            return Some(volume);
        }

        log::warn!(
            "call: {factory} não abriu ({})",
            reason
                .or_else(|| drain_error(pipeline))
                .unwrap_or_else(|| "sem detalhe".to_owned())
        );
        for element in &elements {
            let _ = element.set_state(gst::State::Null);
        }
        let _ = pipeline.remove_many(&elements);
        webrtc.release_request_pad(&pad);
    }
    // Dá para participar só ouvindo: o aviso aparece, a call continua.
    shared.warn("sem microfone: você entra só ouvindo");
    None
}

/// A cadeia do microfone, ainda solta. Devolve os elementos em ordem de
/// ligação e o silenciador, que é quem o mudo desliga.
fn mic_chain(factory: &str) -> Option<(Vec<gst::Element>, gst::Element)> {
    let source = make(factory)?;
    if source.has_property("do-timestamp") {
        source.set_property("do-timestamp", true);
    }
    let queue = make("queue")?;
    queue.set_property("max-size-time", 2_000_000_000u64);
    let convert = make("audioconvert")?;
    let resample = make("audioresample")?;
    let filter = make("capsfilter")?;
    // Dois canais porque é o que o Opus do navegador oferece e o que o
    // servidor registrou; um microfone mono é duplicado pelo convert.
    filter.set_property(
        "caps",
        gst::Caps::builder("audio/x-raw")
            .field("rate", 48_000i32)
            .field("channels", 2i32)
            .build(),
    );
    let volume = make("volume")?;
    // Entra-se na call em silêncio, como o servidor assume (`muted=true`).
    volume.set_property("mute", true);

    // O medidor de nível, depois do silenciador. Ele não é enfeite: a
    // extensão de cabeçalho que diz ao servidor quem está falando não mede
    // nada — ela só copia a medida que este elemento pendura no buffer
    // (`GstAudioLevelMeta`). Sem ele a extensão ia vazia em todo pacote, e
    // ninguém nunca aparecia falando. Depois do silenciador de propósito:
    // mudo mede silêncio, que é o que o servidor precisa acreditar.
    let level = make("level")?;
    level.set_property("audio-level-meta", true);
    level.set_property("post-messages", false);
    // Uma medida por quadro de Opus; medir menos deixaria o destaque lento.
    level.set_property("interval", 20_000_000u64);

    let encoder = make("opusenc")?;
    encoder.set_property("bitrate", 48_000i32);
    encoder.set_property("inband-fec", true);
    encoder.set_property_from_str("audio-type", "voice");
    let payloader = make("rtpopuspay")?;
    payloader.set_property("pt", 111u32);
    // Nível de voz no cabeçalho de cada pacote (RFC 6464): é assim que o
    // servidor sabe quem está falando sem decodificar nada.
    if let Some(extension) = audio_level_extension() {
        payloader.emit_by_name::<()>("add-extension", &[&extension]);
    }
    let cap = make("capsfilter")?;
    cap.set_property("caps", audio_caps());

    Some((
        vec![
            source,
            queue,
            convert,
            resample,
            filter,
            volume.clone(),
            level,
            encoder,
            payloader,
            cap,
        ],
        volume,
    ))
}

fn audio_level_extension() -> Option<gst_rtp::RTPHeaderExtension> {
    let extension = gst_rtp::RTPHeaderExtension::create_from_uri(
        "urn:ietf:params:rtp-hdrext:ssrc-audio-level",
    )?;
    // O identificador é nosso: o servidor lê o que a oferta declarar. Um
    // número alto fica longe da faixa que o webrtcbin usa para as extensões
    // dele (mid e controle de congestão).
    extension.set_id(9);
    Some(extension)
}

/// Liga os sinais do `webrtcbin`: candidatos de ICE saindo, tracks chegando
/// e o estado da conexão.
#[allow(clippy::too_many_arguments)]
fn connect_signals(
    webrtc: &gst::Element,
    pipeline: &gst::Pipeline,
    shared: &Arc<Shared>,
    inbox: &mpsc::Sender<Command>,
    signals: &mpsc::Sender<String>,
    channel_id: &str,
    video_lines: Arc<Mutex<Vec<gst_webrtc::WebRTCRTPTransceiver>>>,
    repaint: egui::Context,
) {
    let out = signals.clone();
    let id = channel_id.to_owned();
    webrtc.connect_closure(
        "on-ice-candidate",
        false,
        glib::closure!(move |_webrtc: &gst::Element, mline: u32, candidate: String| {
            // O servidor recusa candidato de loopback (e tem razão: ninguém
            // fora desta máquina chega em 127.0.0.1). Mandar assim mesmo só
            // renderia um erro por candidato.
            if is_loopback(&candidate) {
                return;
            }
            let _ = out.send(
                serde_json::json!({
                    "type": "voice_ice_candidate",
                    "channel_id": id,
                    "candidate": candidate,
                    "sdp_mline_index": mline,
                })
                .to_string(),
            );
        }),
    );

    let waiting = inbox.clone();
    webrtc.connect_closure(
        "on-negotiation-needed",
        false,
        glib::closure!(move |_webrtc: &gst::Element| {
            let _ = waiting.send(Command::Negotiate);
        }),
    );

    let state = Arc::clone(shared);
    webrtc.connect_notify(Some("connection-state"), move |webrtc, _| {
        let value = webrtc.property::<gst_webrtc::WebRTCPeerConnectionState>("connection-state");
        match value {
            gst_webrtc::WebRTCPeerConnectionState::Connected => {
                state.live.store(true, Ordering::Relaxed);
            }
            gst_webrtc::WebRTCPeerConnectionState::Failed => {
                state.live.store(false, Ordering::Relaxed);
                state.fail("a conexão da call caiu");
            }
            gst_webrtc::WebRTCPeerConnectionState::Closed => {
                state.live.store(false, Ordering::Relaxed);
            }
            _ => {}
        }
    });

    let bin = pipeline.clone();
    let tiles = Arc::clone(shared);
    webrtc.connect_pad_added(move |_webrtc, pad| {
        if pad.direction() != gst::PadDirection::Src {
            return;
        }
        let media = pad
            .current_caps()
            .and_then(|caps| caps.structure(0).and_then(|s| s.get::<String>("media").ok()))
            .unwrap_or_default();
        let line = pad.property::<Option<gst_webrtc::WebRTCRTPTransceiver>>("transceiver");
        match media.as_str() {
            "audio" => play_audio(&bin, pad),
            "video" => {
                let known = video_lines.lock().ok();
                let Some(index) = line.zip(known).and_then(|(line, known)| {
                    known.iter().position(|slot| *slot == line)
                }) else {
                    log::warn!("call: chegou vídeo num lugar que não pedimos");
                    return;
                };
                show_video(&bin, pad, index, Arc::clone(&tiles), repaint.clone());
            }
            _ => {}
        }
    });
}

/// Um candidato de loopback: `candidate:<fundação> <componente> <protocolo>
/// <prioridade> <endereço> …` — o endereço é o quinto campo.
fn is_loopback(candidate: &str) -> bool {
    let value = candidate.trim().trim_start_matches("candidate:");
    let Some(address) = value.split_whitespace().nth(4) else {
        return false;
    };
    address == "::1" || address.starts_with("127.")
}

/// Áudio que chega: um alto-falante por fluxo. Deixar o servidor de som
/// misturar é mais simples (e mais robusto) do que um `audiomixer` que
/// precisa esperar todas as entradas.
fn play_audio(pipeline: &gst::Pipeline, pad: &gst::Pad) {
    let Some(depay) = make("rtpopusdepay") else {
        return;
    };
    let Some(decoder) = make("opusdec") else { return };
    let Some(convert) = make("audioconvert") else {
        return;
    };
    let Some(resample) = make("audioresample") else {
        return;
    };
    let Some(queue) = make("queue") else { return };
    let Some(sink) = audio_sink() else { return };
    let elements = [depay.clone(), decoder, convert, resample, queue, sink];
    if pipeline.add_many(&elements).is_err() {
        return;
    }
    if gst::Element::link_many(&elements).is_err() {
        log::warn!("call: não deu para ligar o áudio que chegou");
        return;
    }
    for element in &elements {
        let _ = element.sync_state_with_parent();
    }
    if let Some(sink_pad) = depay.static_pad("sink")
        && pad.link(&sink_pad).is_err()
    {
        log::warn!("call: o áudio que chegou não encaixou");
    }
}

fn audio_sink() -> Option<gst::Element> {
    for factory in ["pipewiresink", "pulsesink", "alsasink", "autoaudiosink"] {
        let Some(sink) = make(factory) else { continue };
        // Um sink que entra com o pipeline já andando não pode segurar a
        // troca de estado esperando o próprio preroll.
        if sink.has_property("async") {
            sink.set_property("async", false);
        }
        return Some(sink);
    }
    None
}

/// Vídeo que chega: decodifica e publica o quadro no lugar de onde ele veio.
fn show_video(
    pipeline: &gst::Pipeline,
    pad: &gst::Pad,
    index: usize,
    shared: Arc<Shared>,
    repaint: egui::Context,
) {
    let Some(depay) = make("rtpvp8depay") else {
        return;
    };
    let Some(decoder) = make("vp8dec") else { return };
    let Some(convert) = make("videoconvert") else {
        return;
    };
    let sink = gst_app::AppSink::builder()
        .caps(
            &gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .build(),
        )
        .max_buffers(2)
        .drop(true)
        .sync(false)
        .build();
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                if let Some(frame) = to_frame(&sample) {
                    if let Ok(mut tiles) = shared.tiles.lock()
                        && let Some(tile) = tiles.get_mut(index)
                    {
                        tile.frame = Some(frame);
                        tile.seq = tile.seq.wrapping_add(1);
                    }
                    repaint.request_repaint();
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    let elements = [
        depay.clone(),
        decoder,
        convert,
        sink.upcast_ref::<gst::Element>().clone(),
    ];
    if pipeline.add_many(&elements).is_err() {
        return;
    }
    if gst::Element::link_many(&elements).is_err() {
        log::warn!("call: não deu para ligar o vídeo que chegou");
        return;
    }
    for element in &elements {
        let _ = element.sync_state_with_parent();
    }
    if let Some(sink_pad) = depay.static_pad("sink")
        && pad.link(&sink_pad).is_err()
    {
        log::warn!("call: o vídeo que chegou não encaixou");
    }
}

/// A captura da câmera: um pipeline à parte que entrega o mesmo quadro duas
/// vezes — cru para o codificador, RGBA para o retrato na tela.
fn capture(
    target: gst_app::AppSrc,
    shared: Arc<Shared>,
    repaint: egui::Context,
) -> Option<Camera> {
    let pipeline = gst::Pipeline::new();
    let source = make("v4l2src")?;
    let tee = make("tee")?;
    let convert = make("videoconvert")?;
    let scale = make("videoscale")?;
    let rate = make("videorate")?;
    let filter = make("capsfilter")?;
    filter.set_property(
        "caps",
        gst::Caps::builder("video/x-raw")
            .field("format", "I420")
            .field("width", CAMERA_WIDTH)
            .field("height", CAMERA_HEIGHT)
            .field("framerate", gst::Fraction::new(30, 1))
            .build(),
    );

    let feed = gst_app::AppSink::builder().max_buffers(2).drop(true).sync(false).build();
    let preview_queue = make("queue")?;
    let preview_convert = make("videoconvert")?;
    let preview_scale = make("videoscale")?;
    let preview = gst_app::AppSink::builder()
        .caps(
            &gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .field("width", 320i32)
                .build(),
        )
        .max_buffers(1)
        .drop(true)
        .sync(false)
        .build();

    let main = [
        source,
        convert,
        scale,
        rate,
        filter,
        tee.clone(),
    ];
    pipeline.add_many(&main).ok()?;
    gst::Element::link_many(&main).ok()?;

    let encode_branch = [
        make("queue")?,
        feed.upcast_ref::<gst::Element>().clone(),
    ];
    pipeline.add_many(&encode_branch).ok()?;
    gst::Element::link_many(&encode_branch).ok()?;
    tee.link(&encode_branch[0]).ok()?;

    let preview_branch = [
        preview_queue,
        preview_convert,
        preview_scale,
        preview.upcast_ref::<gst::Element>().clone(),
    ];
    pipeline.add_many(&preview_branch).ok()?;
    gst::Element::link_many(&preview_branch).ok()?;
    tee.link(&preview_branch[0]).ok()?;

    // O quadro cru vai para o `appsrc` da sessão WebRTC. O carimbo de tempo
    // é refeito lá (`do-timestamp`), então ligar a câmera de novo não deixa
    // um buraco de horas no meio da linha do tempo.
    feed.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let Some(buffer) = sample.buffer_owned() else {
                    return Ok(gst::FlowSuccess::Ok);
                };
                let mut buffer = buffer;
                if let Some(reference) = buffer.get_mut() {
                    reference.set_pts(None);
                    reference.set_dts(None);
                }
                target.push_buffer(buffer).map_err(|_| gst::FlowError::Error)?;
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    preview.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                if let Some(frame) = to_frame(&sample) {
                    if let Ok(mut slot) = shared.preview.lock() {
                        *slot = Some(frame);
                    }
                    shared.preview_seq.fetch_add(1, Ordering::Relaxed);
                    // Sem isto o seu próprio retrato congelava numa janela
                    // parada: ninguém pedia o quadro seguinte.
                    repaint.request_repaint();
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    if pipeline.set_state(gst::State::Playing).is_err() {
        let _ = pipeline.set_state(gst::State::Null);
        return None;
    }
    Some(Camera { pipeline })
}

/// Copia o quadro RGBA para a memória da janela, linha a linha: o passo de
/// linha do GStreamer quase nunca bate com a largura.
fn to_frame(sample: &gst::Sample) -> Option<Frame> {
    let buffer = sample.buffer()?;
    let caps = sample.caps()?;
    let info = gst_video::VideoInfo::from_caps(caps).ok()?;
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info).ok()?;
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    let stride = frame.plane_stride()[0] as usize;
    let data = frame.plane_data(0).ok()?;
    let mut pixels = Vec::with_capacity(width * height);
    for row in 0..height {
        let start = row * stride;
        let line = data.get(start..start + width * 4)?;
        for [r, g, b, a] in line.as_chunks::<4>().0 {
            pixels.push(egui::Color32::from_rgba_premultiplied(*r, *g, *b, *a));
        }
    }
    Some(Frame {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidato_de_loopback_nao_sai() {
        assert!(is_loopback(
            "candidate:1 1 UDP 2015363327 127.0.0.1 51234 typ host"
        ));
        assert!(is_loopback("candidate:2 1 UDP 2015363327 ::1 51234 typ host"));
        assert!(!is_loopback(
            "candidate:3 1 UDP 2015363327 192.168.0.114 51234 typ host"
        ));
    }
}
