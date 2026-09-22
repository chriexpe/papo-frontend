//! Reprodução de áudio e vídeo dentro da janela.
//!
//! O egui não decodifica nada: o GStreamer decodifica, entrega quadros RGBA
//! por um `appsink` e nós os subimos como textura. O áudio sai pelo sink
//! padrão do sistema, com sincronia por conta do `playbin`.
//!
//! **Nada do GStreamer é chamado pela thread que desenha.** Uma busca com
//! flush numa mídia que ainda não tocou pode não voltar nunca, e o preço
//! disso é a janela inteira congelada. Cada player tem a sua thread: a
//! interface só põe comandos numa fila e lê o que está publicado.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::VideoFrameExt;

/// De quanto em quanto tempo a thread do player republica posição e duração.
const TICK: std::time::Duration = std::time::Duration::from_millis(40);

/// Liga o GStreamer uma vez só; sem ele a mídia vira um cartão de arquivo.
pub fn init() -> bool {
    use std::sync::OnceLock;
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| match gst::init() {
        Ok(()) => {
            #[cfg(target_os = "android")]
            demote_broken_decoders();
            true
        }
        Err(error) => {
            log::warn!("GStreamer indisponível: {error}");
            false
        }
    })
}

/// Tira a preferência dos decodificadores do aparelho que não entregam.
///
/// O OMX é o caminho antigo do Android — o Codec2 o substituiu na versão 10
/// — e no aparelho de teste o decodificador de vídeo dele recusa o H.264 na
/// primeira tentativa, com "Failed to query component interface for required
/// system resources". O problema é que ele vem com prioridade **acima** de
/// todos os que funcionam: o decodebin o escolhe, ele falha, e o vídeo não
/// abre. Era isso o 0:00/0:00 eterno.
///
/// Baixando a prioridade dele, a escolha volta para quem funciona: o Codec2
/// onde houver, e o `openh264` como piso. Só o vídeo é mexido — o áudio do
/// aparelho decodifica bem, inclusive o Opus dos recados.
#[cfg(target_os = "android")]
fn demote_broken_decoders() {
    let registry = gst::Registry::get();
    let mut demoted = 0;
    for feature in registry.features(gst::ElementFactory::static_type()).iter() {
        let name = feature.name();
        // Vídeo: o caminho OMX, que o Codec2 substituiu na versão 10 do
        // Android. Áudio: o decodificador de Opus do aparelho, que abre,
        // não reclama e não entrega quadro nenhum — o som dos recados de
        // voz nunca chegava ao sink. Nos dois casos o substituto em
        // software existe e funciona (`c2androidavcdecoder`, `opusdec`), e
        // o que faltava era só a preferência, que vinha um degrau acima.
        let broken = name.starts_with("amcviddec-omx")
            || (name.starts_with("amcauddec-") && name.contains("opus"));
        if broken && feature.rank() > gst::Rank::MARGINAL {
            feature.set_rank(gst::Rank::NONE);
            demoted += 1;
        }
    }
    if demoted > 0 {
        log::info!("{demoted} decodificador(es) do aparelho despriorizado(s)");
    }
}

struct Frame {
    width: usize,
    height: usize,
    pixels: Vec<egui::Color32>,
}

/// O que a thread do player publica e a janela lê.
#[derive(Default)]
struct Shared {
    frame: Mutex<Option<Frame>>,
    seq: AtomicU64,
    position_ns: AtomicU64,
    duration_ns: AtomicU64,
    playing: AtomicBool,
    /// Proporção do vídeo em bits de `f32`; zero enquanto não há quadro.
    aspect: AtomicU32,
    error: Mutex<Option<String>>,
}

impl Shared {
    fn seconds(value: &AtomicU64) -> f64 {
        value.load(Ordering::Relaxed) as f64 / 1e9
    }
}

/// Ordens que a janela manda para a thread do player.
enum Command {
    Play,
    Pause,
    Seek(f64),
    Muted(bool),
}

pub struct Player {
    commands: mpsc::Sender<Command>,
    shared: Arc<Shared>,
    texture: Option<egui::TextureHandle>,
    shown: u64,
    /// Posição em segundos enquanto o usuário arrasta o cursor.
    pub scrubbing: Option<f64>,
    pub muted: bool,
}

impl Player {
    /// Abre a mídia numa thread própria. Volta na hora: quem espera pelo
    /// preroll é a thread, não a janela.
    pub fn open(path: &Path, video: bool, repaint: egui::Context) -> Option<Self> {
        if !init() {
            return None;
        }
        let uri = gst::glib::filename_to_uri(path, None).ok()?;
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::channel();

        let worker_shared = Arc::clone(&shared);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        std::thread::Builder::new()
            .name("papo-player".into())
            .spawn(move || {
                run(uri.to_string(), video, worker_shared, repaint, rx, name);
            })
            .ok()?;

        Some(Self {
            commands: tx,
            shared,
            texture: None,
            shown: 0,
            scrubbing: None,
            muted: false,
        })
    }

    fn send(&self, command: Command) {
        // A thread morreu (mídia quebrada, por exemplo): não há o que fazer,
        // e a janela segue desenhando.
        let _ = self.commands.send(command);
    }

    pub fn play(&mut self) {
        self.shared.playing.store(true, Ordering::Relaxed);
        self.send(Command::Play);
    }

    pub fn pause(&mut self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        self.send(Command::Pause);
    }

    pub fn toggle(&mut self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn position(&self) -> f64 {
        self.scrubbing
            .unwrap_or_else(|| Shared::seconds(&self.shared.position_ns))
    }

    pub fn duration(&self) -> f64 {
        Shared::seconds(&self.shared.duration_ns)
    }

    pub fn seek(&mut self, seconds: f64) {
        let seconds = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        // A posição anda na hora, para o cursor não voltar enquanto a busca
        // acontece lá atrás; e o botão já vira pausa, porque buscar toca.
        self.shared
            .position_ns
            .store((seconds * 1e9) as u64, Ordering::Relaxed);
        self.shared.playing.store(true, Ordering::Relaxed);
        self.send(Command::Seek(seconds));
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.send(Command::Muted(muted));
    }

    /// Proporção do vídeo; 16:9 enquanto o primeiro quadro não chega.
    pub fn aspect(&self) -> f32 {
        let bits = self.shared.aspect.load(Ordering::Relaxed);
        let aspect = f32::from_bits(bits);
        if aspect.is_finite() && aspect > 0.05 {
            aspect
        } else {
            16.0 / 9.0
        }
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().ok().and_then(|slot| slot.clone())
    }

    /// Mantida por compatibilidade: o barramento agora é lido pela thread.
    pub fn update(&mut self) {}

    /// Textura do quadro mais recente.
    pub fn frame(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        let seq = self.shared.seq.load(Ordering::Relaxed);
        if seq != self.shown
            && let Ok(mut slot) = self.shared.frame.lock()
            && let Some(frame) = slot.take()
        {
            self.shown = seq;
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
        self.texture.as_ref()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Soltar o canal encerra a thread, que desmonta o pipeline. Esperar
        // por ela aqui seria trocar um congelamento por outro.
        let (dead, _) = mpsc::channel();
        self.commands = dead;
    }
}

/// A thread do player: monta o pipeline, obedece à fila e publica o estado.
fn run(
    uri: String,
    video: bool,
    shared: Arc<Shared>,
    repaint: egui::Context,
    commands: mpsc::Receiver<Command>,
    name: String,
) {
    let pipeline = match gst::ElementFactory::make("playbin3")
        .build()
        .or_else(|_| gst::ElementFactory::make("playbin").build())
    {
        Ok(pipeline) => pipeline,
        Err(error) => {
            report(&shared, format!("sem playbin: {error}"));
            return;
        }
    };
    pipeline.set_property("uri", uri.as_str());

    if video {
        match video_sink(Arc::clone(&shared), repaint.clone()) {
            Some(sink) => pipeline.set_property("video-sink", &sink),
            None => report(&shared, "sem sink de vídeo".into()),
        }
    } else if let Ok(fake) = gst::ElementFactory::make("fakesink").build() {
        pipeline.set_property("video-sink", &fake);
    }
    // O playbin pede `autoaudiosink`, que vem do gst-plugins-good e nem
    // sempre está instalado. Sem sink não há preroll: o pipeline morre em
    // silêncio, sem duração e sem poder buscar posição.
    if let Some(sink) = audio_sink() {
        pipeline.set_property("audio-sink", &sink);
    }

    if pipeline.set_state(gst::State::Paused).is_err() {
        report(&shared, format!("não deu para preparar {name}"));
        let _ = pipeline.set_state(gst::State::Null);
        return;
    }
    // Esperar o preroll aqui é de graça: a janela não depende desta thread.
    let (result, _, _) = pipeline.state(gst::ClockTime::from_seconds(5));
    if let Err(error) = result {
        log::warn!("preroll de {name}: {error}");
    }
    publish(&pipeline, &shared);
    repaint.request_repaint();

    loop {
        match commands.recv_timeout(TICK) {
            Ok(Command::Play) => match pipeline.set_state(gst::State::Playing) {
                // A troca de estado pode voltar como assíncrona, e enquanto
                // ela não termina o relógio não anda: o pipeline diz PLAYING
                // e a posição fica parada. Esperar aqui é de graça.
                Ok(_) => {
                    let (result, current, pending) =
                        pipeline.state(gst::ClockTime::from_mseconds(700));
                    if current != gst::State::Playing {
                        // Não chegou a tocar dentro do prazo. Quase sempre é
                        // o sink que não consegue prerolar, e o pipeline fica
                        // em PAUSED de vez: relógio parado, som nenhum, e
                        // nada no barramento para denunciar.
                        log::warn!(
                            "{name} não chegou a PLAYING: {result:?}, está em                              {current:?}, indo para {pending:?}"
                        );
                    }
                }
                Err(error) => report(&shared, format!("não tocou: {error}")),
            },
            Ok(Command::Pause) => {
                let _ = pipeline.set_state(gst::State::Paused);
                let _ = pipeline.state(gst::ClockTime::from_mseconds(300));
            }
            Ok(Command::Seek(seconds)) => {
                let target = gst::ClockTime::from_nseconds((seconds * 1e9) as u64);
                // Aqui a busca pode bloquear à vontade: quem espera é esta
                // thread, não a janela.
                if pipeline
                    .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, target)
                    .is_err()
                {
                    log::warn!("busca recusada em {name}");
                }
                // A busca com flush desfaz o preroll: é preciso esperar o
                // pipeline se assentar antes de mandá-lo tocar, senão ele
                // fica parado para sempre no ponto novo.
                let _ = pipeline.state(gst::ClockTime::from_mseconds(700));
                // Quem arrasta o cursor quer ouvir dali em diante.
                if pipeline.set_state(gst::State::Playing).is_ok() {
                    let _ = pipeline.state(gst::ClockTime::from_mseconds(700));
                    shared.playing.store(true, Ordering::Relaxed);
                }
                repaint.request_repaint();
            }
            Ok(Command::Muted(muted)) => pipeline.set_property("mute", muted),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // A janela soltou o player.
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if let Some(bus) = pipeline.bus() {
            while let Some(message) = bus.pop() {
                match message.view() {
                    gst::MessageView::Eos(_) => {
                        shared.playing.store(false, Ordering::Relaxed);
                        let _ = pipeline.set_state(gst::State::Paused);
                        let _ = pipeline
                            .seek_simple(gst::SeekFlags::FLUSH, gst::ClockTime::ZERO);
                        repaint.request_repaint();
                    }
                    gst::MessageView::Error(error) => {
                        shared.playing.store(false, Ordering::Relaxed);
                        report(&shared, error.error().to_string());
                        repaint.request_repaint();
                    }
                    // O sink avisa que a latência mudou e espera que alguém
                    // recalcule. Quem ignora fica com o relógio parado: o
                    // pipeline diz PLAYING e a posição não anda.
                    gst::MessageView::Latency(_) => {
                        if let Some(bin) = pipeline.downcast_ref::<gst::Bin>()
                            && let Err(error) = bin.recalculate_latency()
                        {
                            log::warn!("latência de {name}: {error}");
                        }
                    }
                    _ => {}
                }
            }
        }

        publish(&pipeline, &shared);
        if shared.playing.load(Ordering::Relaxed) {
            repaint.request_repaint();
        }
    }

    let _ = pipeline.set_state(gst::State::Null);
}

/// Publica posição e duração para a janela ler sem perguntar ao GStreamer.
fn publish(pipeline: &gst::Element, shared: &Shared) {
    // Perguntar o estado (sem esperar) é o que fecha uma troca assíncrona
    // pendente. Sem isso o pipeline anuncia PLAYING, o relógio não anda e a
    // posição fica congelada — foi exatamente o que aconteceu aqui.
    let _ = pipeline.state(gst::ClockTime::ZERO);
    if let Some(position) = pipeline.query_position::<gst::ClockTime>() {
        shared.position_ns.store(position.nseconds(), Ordering::Relaxed);
    }
    if let Some(duration) = pipeline.query_duration::<gst::ClockTime>()
        && duration.nseconds() > 0
    {
        shared.duration_ns.store(duration.nseconds(), Ordering::Relaxed);
    }
}

fn report(shared: &Shared, message: String) {
    log::warn!("player: {message}");
    if let Ok(mut slot) = shared.error.lock() {
        *slot = Some(message);
    }
}

/// O primeiro sink de áudio que este sistema consegue criar. `PAPO_AUDIO_SINK`
/// força um deles, para quando o padrão do sistema não coopera.
fn audio_sink() -> Option<gst::Element> {
    if let Ok(forced) = std::env::var("PAPO_AUDIO_SINK") {
        match gst::ElementFactory::make(&forced).build() {
            Ok(sink) => {
                log::info!("saída de áudio forçada: {forced}");
                return Some(sink);
            }
            Err(error) => log::warn!("saída de áudio {forced} não existe: {error}"),
        }
    }
    // A única diferença de plataforma da reprodução mora aqui. Todo o
    // resto — o `playbin`, o `appsink` que vira textura, posição, duração,
    // busca — é o mesmo nos dois lados, e por isso o arquivo é um só.
    #[cfg(target_os = "android")]
    const SINKS: &[&str] = &["openslessink", "autoaudiosink"];
    #[cfg(not(target_os = "android"))]
    const SINKS: &[&str] = &["autoaudiosink", "pipewiresink", "pulsesink", "alsasink"];

    // ABERTO: no Android o som sai, mas picotado. Três tentativas foram
    // feitas no aparelho e nenhuma mudou nada — nenhuma ficou no código:
    //
    //   1. folga no buffer do sink (`buffer-time`, `latency-time`);
    //   2. entregar 48 kHz em estéreo, que é o que o aparelho toca, em vez
    //      do mono dos recados;
    //   3. uma `queue` entre a decodificação e o sink — que foi o que
    //      resolveu o picote da **gravação** na área de trabalho (f1c08eb).
    //
    // O decodificador já é o de software, e Opus é barato: não é custo de
    // decodificação. O próximo passo não é tentar outra peça no pipeline, é
    // medir onde o tempo se perde — a thread de mídia também extrai capas e
    // formas de onda, e essas sim são caras.
    for name in SINKS {
        if let Ok(sink) = gst::ElementFactory::make(name).build() {
            log::debug!("saída de áudio: {name}");
            return Some(sink);
        }
    }
    log::warn!("sem saída de áudio: o som não vai tocar");
    None
}

/// Largura máxima do quadro entregue à janela. Acima disso o vídeo é
/// reduzido: a tela cheia ainda tem resolução de sobra e a memória por
/// quadro deixa de acompanhar o tamanho do original.
const MAX_FRAME_W: i32 = 1280;

/// `videoconvert ! videoscale ! appsink` empacotado como sink do playbin.
fn video_sink(shared: Arc<Shared>, repaint: egui::Context) -> Option<gst::Element> {
    // A largura entra como intervalo: vídeo menor passa intacto, maior é
    // reduzido antes de virar quadro. Cada quadro é copiado para a memória e
    // subido como textura, então 1080p custava 8 MiB por cópia para ser
    // desenhado numa coluna de mensagem com menos de 500 px de largura.
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .field("width", gst::IntRange::new(1, MAX_FRAME_W))
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
                    for [r, g, b, a] in line.as_chunks::<4>().0 {
                        pixels.push(egui::Color32::from_rgba_premultiplied(*r, *g, *b, *a));
                    }
                }

                if let Ok(mut slot) = shared.frame.lock() {
                    *slot = Some(Frame {
                        width,
                        height,
                        pixels,
                    });
                }
                if height > 0 {
                    shared.aspect.store(
                        (width as f32 / height as f32).to_bits(),
                        Ordering::Relaxed,
                    );
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
    let scale = gst::ElementFactory::make("videoscale").build().ok()?;
    let element = sink.upcast_ref::<gst::Element>().clone();
    bin.add_many([&convert, &scale, &element]).ok()?;
    gst::Element::link_many([&convert, &scale, &element]).ok()?;
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
    // O appsink devolve `Err` no fim do arquivo: é o que encerra a leitura.
    while let Ok(sample) = sink.pull_sample() {
        let Some(buffer) = sample.buffer() else { continue };
        let Ok(map) = buffer.map_readable() else { continue };
        for sample in map.as_chunks::<2>().0 {
            let value = i16::from_le_bytes(*sample);
            samples.push((value as f32 / i16::MAX as f32).abs());
        }
        // Uma hora de áudio não precisa virar um vetor de 30 milhões.
        if samples.len() > 8_000 * 60 * 30 {
            break;
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
        // Gravar ainda não existe no Android: depende da permissão de
        // microfone, que é assunto do PR de captura. As fontes abaixo são
        // do Linux e nem existem lá — melhor recusar aqui, com o motivo,
        // do que falhar procurando uma a uma.
        #[cfg(target_os = "android")]
        {
            let _ = dir;
            log::warn!("gravar áudio ainda não existe no Android");
            return None;
        }

        #[cfg(not(target_os = "android"))]
        {
        let _ = std::fs::create_dir_all(dir);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = dir.join(format!("recado-{stamp}.ogg"));

        // A fonte varia com o sistema: PipeWire onde existe, ALSA como
        // reserva. Sem microfone, nada disso abre e a gravação nem começa.
        for source in ["pipewiresrc", "alsasrc"] {
            // O `queue` logo depois da fonte é o que separa a captura da
            // codificação: sem ele, converter, resamplear e codificar em
            // Opus acontece na thread que está lendo o microfone, e cada
            // atraso do encoder vira amostra perdida — som picotado no
            // arquivo, não na hora de tocar. O `audiorate` costura os buracos
            // que mesmo assim apareçam, para o Ogg não sair com o tempo
            // torto. `do-timestamp` garante carimbo de hora na fonte viva.
            let live = if source == "pipewiresrc" {
                "pipewiresrc do-timestamp=true"
            } else {
                "alsasrc do-timestamp=true"
            };
            let description = format!(
                "{live} ! queue max-size-time=2000000000 leaky=no ! \
                 audioconvert ! audioresample ! audiorate ! \
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

/// Um quadro do vídeo para servir de capa, antes de alguém dar play.
///
/// Busca um pouco adiante em vez de pegar o primeiro quadro: vídeo costuma
/// abrir no preto, e uma capa preta não é capa nenhuma — é o que já se via
/// sem esta função.
pub fn poster(path: &Path) -> Option<egui::ColorImage> {
    if !init() {
        return None;
    }
    let uri = gst::glib::filename_to_uri(path, None).ok()?;
    // O `playbin` com a bandeira de só-vídeo é o caminho certo aqui, e é o
    // mesmo elemento que o player usa. Tentar podar o `uridecodebin` pelas
    // caps não serve: pedindo `video/x-raw` o ramo de áudio não tem como
    // chegar lá e o decodebin desiste com "no suitable plugins found" — no
    // Android, o vídeo nem chega a abrir. Com a bandeira, o áudio nem é
    // considerado, que era o objetivo: nada de decodificador de som moendo
    // contra um pad solto só para tirar uma imagem parada.
    //
    // A capa é desenhada num cartão estreito; `POSTER_MAX_W` de largura é
    // folga de sobra e poupa memória por anexo.
    let sink_bin = gst::parse::bin_from_description(
        &format!(
            "videoconvert ! videoscale ! \
             video/x-raw,format=RGBA,pixel-aspect-ratio=1/1,width=[1,{POSTER_MAX_W}] ! \
             appsink name=capa sync=false max-buffers=1 drop=false"
        ),
        true,
    )
    .ok()?;
    let sink = sink_bin
        .by_name("capa")?
        .downcast::<gst_app::AppSink>()
        .ok()?;

    let pipeline = gst::ElementFactory::make("playbin3")
        .build()
        .or_else(|_| gst::ElementFactory::make("playbin").build())
        .ok()?;
    pipeline.set_property("uri", &uri);
    pipeline.set_property("video-sink", &sink_bin);
    // Só o vídeo: sem áudio, sem legenda.
    pipeline.set_property_from_str("flags", "video");

    // `Paused` já decodifica o primeiro quadro e é o que dá a duração; não
    // é preciso tocar nada para tirar uma capa.
    if pipeline.set_state(gst::State::Paused).is_err() {
        return None;
    }
    // Esperar a mudança de estado terminar: é nela que o primeiro quadro é
    // decodificado, e é dela que sai a duração.
    let _ = pipeline.state(PREROLL_WAIT);

    // Toda espera aqui tem prazo. O `pull_preroll` sem prazo espera para
    // sempre quando o quadro não vem — arquivo sem vídeo, decodificador que
    // não assume — e levava a thread junto.
    let first = sink.try_pull_preroll(PREROLL_WAIT);
    if first.is_none() {
        // Sem quadro no prazo: o porquê está no barramento, e sem ler dali
        // a capa some em silêncio.
        if let Some(bus) = pipeline.bus() {
            while let Some(message) = bus.pop() {
                if let gst::MessageView::Error(error) = message.view() {
                    log::warn!(
                        "capa de {}: {} ({:?})",
                        path.display(),
                        error.error(),
                        error.debug()
                    );
                }
            }
        }
    }
    let mut chosen = first;

    if let Some(duration) = pipeline.query_duration::<gst::ClockTime>()
        && duration > gst::ClockTime::ZERO
    {
        // Um terço adiante, no máximo três segundos: longe do preto da
        // abertura e ainda perto do começo. É melhoria, não obrigação — se
        // a busca falhar, ou o quadro de lá não vier, fica o primeiro, que
        // já é melhor que capa nenhuma.
        let at = (duration / 3).min(gst::ClockTime::from_seconds(3));
        if pipeline
            .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, at)
            .is_ok()
            && let Some(sample) = sink.try_pull_preroll(PREROLL_WAIT)
        {
            chosen = Some(sample);
        }
    }

    let image = chosen.as_ref().and_then(frame_image);
    let _ = pipeline.set_state(gst::State::Null);
    image
}

/// Prazo de cada espera por um quadro da capa.
const PREROLL_WAIT: gst::ClockTime = gst::ClockTime::from_seconds(5);

/// Largura máxima da capa.
const POSTER_MAX_W: i32 = 640;

/// Converte um quadro RGBA do GStreamer numa imagem do egui.
fn frame_image(sample: &gst::Sample) -> Option<egui::ColorImage> {
    let buffer = sample.buffer()?;
    let info = gst_video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info).ok()?;
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    if width == 0 || height == 0 {
        return None;
    }
    let stride = frame.plane_stride()[0] as usize;
    let data = frame.plane_data(0).ok()?;

    // O stride raramente bate com a largura: copiamos linha a linha.
    let mut pixels = Vec::with_capacity(width * height);
    for row in 0..height {
        let start = row * stride;
        let line = data.get(start..start + width * 4)?;
        for [r, g, b, a] in line.as_chunks::<4>().0 {
            pixels.push(egui::Color32::from_rgba_premultiplied(*r, *g, *b, *a));
        }
    }
    Some(egui::ColorImage {
        size: [width, height],
        pixels,
        source_size: egui::vec2(width as f32, height as f32),
    })
}
