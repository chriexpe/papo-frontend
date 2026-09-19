//! Reprodução de áudio e vídeo dentro da janela.
//!
//! O egui não decodifica nada: o GStreamer decodifica, entrega quadros RGBA
//! por um `appsink` e nós os subimos como textura. O áudio sai pelo sink
//! padrão do sistema, com sincronia por conta do `playbin`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::VideoFrameExt;

/// Liga o GStreamer uma vez só; sem ele a mídia vira um cartão de arquivo.
pub fn init() -> bool {
    use std::sync::OnceLock;
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| match gst::init() {
        Ok(()) => true,
        Err(error) => {
            log::warn!("GStreamer indisponível: {error}");
            false
        }
    })
}

struct Frame {
    width: usize,
    height: usize,
    pixels: Vec<egui::Color32>,
}

#[derive(Default)]
struct Shared {
    frame: Mutex<Option<Frame>>,
    seq: AtomicU64,
    eos: AtomicBool,
}

pub struct Player {
    pipeline: gst::Element,
    shared: Arc<Shared>,
    texture: Option<egui::TextureHandle>,
    shown: u64,
    /// Proporção do vídeo, conhecida depois do primeiro quadro.
    pub aspect: f32,
    playing: bool,
    /// Posição em segundos enquanto o usuário arrasta o cursor.
    pub scrubbing: Option<f64>,
    /// Última duração conhecida: a consulta só responde depois do preroll e
    /// volta a falhar em alguns formatos, então guardamos a boa.
    duration: std::cell::Cell<f64>,
    pub muted: bool,
    pub error: Option<String>,
}

impl Player {
    pub fn open(path: &Path, video: bool, repaint: egui::Context) -> Option<Self> {
        if !init() {
            return None;
        }
        let uri = gst::glib::filename_to_uri(path, None).ok()?;
        let pipeline = gst::ElementFactory::make("playbin3")
            .build()
            .or_else(|_| gst::ElementFactory::make("playbin").build())
            .ok()?;
        pipeline.set_property("uri", uri.as_str());

        let shared = Arc::new(Shared::default());
        if video {
            let sink = video_sink(Arc::clone(&shared), repaint)?;
            pipeline.set_property("video-sink", &sink);
        } else if let Ok(fake) = gst::ElementFactory::make("fakesink").build() {
            pipeline.set_property("video-sink", &fake);
        }

        // O playbin pede `autoaudiosink`, que vem do gst-plugins-good e nem
        // sempre está instalado. Sem sink não há preroll: o pipeline morre em
        // silêncio, sem duração e sem poder buscar posição.
        if let Some(sink) = audio_sink() {
            pipeline.set_property("audio-sink", &sink);
        }

        // Pausado já decodifica o primeiro quadro: o cartão aparece com a
        // imagem do vídeo em vez de um retângulo vazio.
        if pipeline.set_state(gst::State::Paused).is_err() {
            log::warn!("não deu para preparar {}", path.display());
            return None;
        }
        // O preroll é assíncrono; esperar um instante por ele faz a duração
        // já existir na primeira vez que a linha do tempo for desenhada.
        let (result, _, _) = pipeline.state(gst::ClockTime::from_mseconds(600));
        if let Err(error) = result {
            log::warn!("preroll de {}: {error}", path.display());
        }

        Some(Self {
            pipeline,
            shared,
            texture: None,
            shown: 0,
            aspect: 16.0 / 9.0,
            playing: false,
            scrubbing: None,
            duration: std::cell::Cell::new(0.0),
            muted: false,
            error: None,
        })
    }

    pub fn play(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Playing);
        self.playing = true;
        self.shared.eos.store(false, Ordering::Relaxed);
    }

    pub fn pause(&mut self) {
        if self.playing {
            let _ = self.pipeline.set_state(gst::State::Paused);
            self.playing = false;
        }
    }

    pub fn toggle(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Lê o barramento do áudio, que não tem quadro para acordá-lo.
    pub fn update(&mut self) {
        self.pump_bus();
    }

    pub fn position(&self) -> f64 {
        if let Some(at) = self.scrubbing {
            return at;
        }
        self.pipeline
            .query_position::<gst::ClockTime>()
            .map(|time| time.seconds_f64())
            .unwrap_or(0.0)
    }

    pub fn duration(&self) -> f64 {
        if let Some(time) = self.pipeline.query_duration::<gst::ClockTime>() {
            let seconds = time.seconds_f64();
            if seconds > 0.0 {
                self.duration.set(seconds);
                return seconds;
            }
        }
        self.duration.get()
    }

    pub fn seek(&mut self, seconds: f64) {
        let target = gst::ClockTime::from_nseconds((seconds.max(0.0) * 1e9) as u64);
        let _ = self.pipeline.seek_simple(
            gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
            target,
        );
        self.shared.eos.store(false, Ordering::Relaxed);
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.pipeline.set_property("mute", muted);
    }

    /// Lê o barramento e devolve a textura do quadro mais recente.
    pub fn frame(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        self.pump_bus();

        if self.playing {
            // Enquanto toca, a janela precisa acompanhar o vídeo.
            ctx.request_repaint();
        }

        let seq = self.shared.seq.load(Ordering::Relaxed);
        if seq != self.shown {
            if let Ok(mut slot) = self.shared.frame.lock() {
                if let Some(frame) = slot.take() {
                    self.shown = seq;
                    if frame.height > 0 {
                        self.aspect = frame.width as f32 / frame.height as f32;
                    }
                    let image = egui::ColorImage {
                        size: [frame.width, frame.height],
                        pixels: frame.pixels,
                        source_size: egui::Vec2::new(frame.width as f32, frame.height as f32),
                    };
                    match &mut self.texture {
                        Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                        None => {
                            self.texture = Some(ctx.load_texture(
                                format!("video-{seq}"),
                                image,
                                egui::TextureOptions::LINEAR,
                            ))
                        }
                    }
                }
            }
        }
        self.texture.as_ref()
    }

    /// Chegou ao fim: volta ao começo e espera, como um player de verdade.
    fn pump_bus(&mut self) {
        if self.shared.eos.swap(false, Ordering::Relaxed) {
            self.playing = false;
            let _ = self.pipeline.set_state(gst::State::Paused);
            self.seek(0.0);
        }
        let Some(bus) = self.pipeline.bus() else { return };
        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::Eos(_) => {
                    self.playing = false;
                    let _ = self.pipeline.set_state(gst::State::Paused);
                    self.seek(0.0);
                }
                gst::MessageView::Error(error) => {
                    self.error = Some(error.error().to_string());
                    self.playing = false;
                }
                _ => {}
            }
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// O primeiro sink de áudio que este sistema consegue criar.
fn audio_sink() -> Option<gst::Element> {
    for name in ["autoaudiosink", "pipewiresink", "pulsesink", "alsasink"] {
        if let Ok(sink) = gst::ElementFactory::make(name).build() {
            log::debug!("saída de áudio: {name}");
            return Some(sink);
        }
    }
    log::warn!("sem saída de áudio: o som não vai tocar");
    None
}

/// `videoconvert ! appsink` empacotado como sink do playbin.
fn video_sink(shared: Arc<Shared>, repaint: egui::Context) -> Option<gst::Element> {
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .build();
    let sink = gst_app::AppSink::builder()
        .caps(&caps)
        .max_buffers(2)
        .drop(true)
        .sync(true)
        .build();

    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let Some(buffer) = sample.buffer() else {
                    return Ok(gst::FlowSuccess::Ok);
                };
                let Some(caps) = sample.caps() else {
                    return Ok(gst::FlowSuccess::Ok);
                };
                let Ok(info) = gst_video::VideoInfo::from_caps(caps) else {
                    return Ok(gst::FlowSuccess::Ok);
                };
                let Ok(frame) = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
                else {
                    return Ok(gst::FlowSuccess::Ok);
                };

                let width = frame.width() as usize;
                let height = frame.height() as usize;
                let stride = frame.plane_stride()[0] as usize;
                let data = frame.plane_data(0).map_err(|_| gst::FlowError::Error)?;

                // O stride raramente bate com a largura: copiamos linha a
                // linha para não torcer a imagem.
                let mut pixels = Vec::with_capacity(width * height);
                for row in 0..height {
                    let start = row * stride;
                    let line = &data[start..start + width * 4];
                    for pixel in line.chunks_exact(4) {
                        pixels.push(egui::Color32::from_rgba_premultiplied(
                            pixel[0], pixel[1], pixel[2], pixel[3],
                        ));
                    }
                }

                if let Ok(mut slot) = shared.frame.lock() {
                    *slot = Some(Frame {
                        width,
                        height,
                        pixels,
                    });
                }
                shared.seq.fetch_add(1, Ordering::Relaxed);
                repaint.request_repaint();
                Ok(gst::FlowSuccess::Ok)
            })
            .eos(|_| {})
            .build(),
    );

    let bin = gst::Bin::new();
    let convert = gst::ElementFactory::make("videoconvert").build().ok()?;
    let element = sink.upcast_ref::<gst::Element>().clone();
    bin.add_many([&convert, &element]).ok()?;
    gst::Element::link_many([&convert, &element]).ok()?;
    let pad = convert.static_pad("sink")?;
    let ghost = gst::GhostPad::with_target(&pad).ok()?;
    bin.add_pad(&ghost).ok()?;
    Some(bin.upcast::<gst::Element>())
}

/// Percorre o áudio inteiro mais rápido que o tempo real e devolve os picos
/// para desenhar a forma de onda.
pub fn waveform(path: &Path) -> Option<Vec<f32>> {
    const BUCKETS: usize = 220;

    if !init() {
        return None;
    }
    let uri = gst::glib::filename_to_uri(path, None).ok()?;
    let description = format!(
        "uridecodebin uri=\"{uri}\" ! audioconvert ! audioresample ! \
         audio/x-raw,format=S16LE,channels=1,rate=8000 ! \
         appsink name=peaks sync=false max-buffers=64 drop=false"
    );
    let pipeline = gst::parse::launch(&description).ok()?;
    let sink = pipeline
        .downcast_ref::<gst::Bin>()?
        .by_name("peaks")?
        .downcast::<gst_app::AppSink>()
        .ok()?;

    if pipeline.set_state(gst::State::Playing).is_err() {
        return None;
    }

    let mut samples: Vec<f32> = Vec::new();
    loop {
        match sink.pull_sample() {
            Ok(sample) => {
                let Some(buffer) = sample.buffer() else { continue };
                let Ok(map) = buffer.map_readable() else { continue };
                for chunk in map.chunks_exact(2) {
                    let value = i16::from_le_bytes([chunk[0], chunk[1]]);
                    samples.push((value as f32 / i16::MAX as f32).abs());
                }
                // Uma hora de áudio não precisa virar um vetor de 30 milhões.
                if samples.len() > 8_000 * 60 * 30 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = pipeline.set_state(gst::State::Null);

    if samples.is_empty() {
        return None;
    }
    let per_bucket = (samples.len() / BUCKETS).max(1);
    let peaks: Vec<f32> = samples
        .chunks(per_bucket)
        .map(|chunk| chunk.iter().copied().fold(0.0_f32, f32::max))
        .collect();
    let ceiling = peaks.iter().copied().fold(0.0_f32, f32::max).max(0.02);
    Some(peaks.into_iter().map(|peak| peak / ceiling).collect())
}

// ---------------------------------------------------------------------------
// Gravação de voz
// ---------------------------------------------------------------------------

/// Gravador de recado de voz: entra pelo microfone do sistema e sai num Ogg
/// Opus, pronto para virar anexo.
pub struct Recorder {
    pipeline: gst::Element,
    path: std::path::PathBuf,
    started: std::time::Instant,
    finished: bool,
}

impl Recorder {
    /// Começa a gravar num arquivo novo dentro da pasta indicada.
    pub fn start(dir: &Path) -> Option<Self> {
        if !init() {
            return None;
        }
        let _ = std::fs::create_dir_all(dir);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = dir.join(format!("recado-{stamp}.ogg"));

        // A fonte varia com o sistema: PipeWire onde existe, ALSA como
        // reserva. Sem microfone, nada disso abre e a gravação nem começa.
        for source in ["pipewiresrc", "alsasrc"] {
            let description = format!(
                "{source} ! audioconvert ! audioresample ! \
                 audio/x-raw,channels=1,rate=48000 ! opusenc ! oggmux ! \
                 filesink location=\"{}\"",
                path.display()
            );
            let Ok(pipeline) = gst::parse::launch(&description) else {
                continue;
            };
            if pipeline.set_state(gst::State::Playing).is_ok() {
                log::info!("gravando de {source} em {}", path.display());
                return Some(Self {
                    pipeline,
                    path,
                    started: std::time::Instant::now(),
                    finished: false,
                });
            }
            let _ = pipeline.set_state(gst::State::Null);
        }
        log::warn!("sem entrada de áudio para gravar");
        None
    }

    pub fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    /// Fecha o arquivo e devolve o caminho. Precisa esperar o fim de verdade,
    /// ou o Ogg fica sem o cabeçalho final e ninguém consegue tocar.
    pub fn finish(mut self) -> Option<std::path::PathBuf> {
        self.pipeline.send_event(gst::event::Eos::new());
        if let Some(bus) = self.pipeline.bus() {
            for message in bus.iter_timed(gst::ClockTime::from_seconds(3)) {
                match message.view() {
                    gst::MessageView::Eos(_) => break,
                    gst::MessageView::Error(error) => {
                        log::warn!("gravação: {}", error.error());
                        break;
                    }
                    _ => {}
                }
            }
        }
        let _ = self.pipeline.set_state(gst::State::Null);
        self.finished = true;
        let path = self.path.clone();
        let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        // Um toque sem querer não vira anexo.
        if size < 1024 {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        Some(path)
    }

    /// Desiste da gravação e apaga o arquivo.
    pub fn cancel(mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        self.finished = true;
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.pipeline.set_state(gst::State::Null);
        }
    }
}
