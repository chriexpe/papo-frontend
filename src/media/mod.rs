//! Mídia das mensagens: download, cache em disco, texturas e players.
//!
//! O anexo chega por uma rota autenticada, então nada pode ser entregue ao
//! GStreamer ou ao egui como URL: tudo passa por aqui, é gravado no cache do
//! usuário e só então vira textura ou arquivo para tocar.

/// Conferência do GStreamer no Android, escrita no logcat na abertura.
#[cfg(target_os = "android")]
pub mod gst_check;
pub mod player;
pub mod prepare;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as sync_mpsc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use egui::{ColorImage, TextureHandle, TextureOptions};
use tokio::sync::mpsc;

use crate::api::client::{Api, Session};
use crate::api::models::Attachment;
use crate::ui::emoji_raster::EmojiRaster;

/// Lado maior de uma textura de mensagem; o visualizador pede a versão cheia.
/// Quantos players ficam vivos ao mesmo tempo. Cada um carrega uma thread,
/// um decodificador e um contexto de vídeo, e isso não aparece no tamanho do
/// arquivo: um clipe de meio mega em 1080p custa quase o mesmo que um de
/// quinze. Guardar pipeline para mídia que ninguém está ouvindo é o
/// desperdício mais caro que havia aqui.
const MAX_PLAYERS: usize = 4;

/// Teto do que fica decodificado em textura. O custo é o pixel, não o
/// arquivo: uma imagem de 1600² ocupa 10 MiB abertos venha ela de 200 KiB
/// de JPEG ou de 4 MiB de PNG.
const TEXTURE_BUDGET: usize = 192 * 1024 * 1024;

/// Teto do cache em disco e idade máxima de um arquivo parado. Nada aqui
/// era apagado antes: a pasta só crescia, para sempre.
const CACHE_BUDGET: u64 = 512 * 1024 * 1024;
const CACHE_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Gravações são do usuário, não mídia baixada, e só saem quando velhas
/// demais para alguma ainda estar esperando no campo de escrever.
const RECORDING_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

const INLINE_MAX: u32 = 1600;
const FULL_MAX: u32 = 4096;

#[derive(Debug, Clone)]
pub enum Request {
    /// Miniatura (ou a própria imagem, quando não há miniatura).
    Thumb { id: String, thumb_id: Option<String> },
    /// Imagem em tamanho cheio, para o visualizador.
    Full { id: String },
    /// Garante o arquivo no cache e devolve o caminho (vídeo, áudio, outros).
    File { id: String, name: String },
    /// Copia o anexo para fora do cache.
    Save { id: String, name: String, dest: PathBuf },
    /// Picos do áudio para desenhar a forma de onda.
    Waveform { id: String, path: PathBuf },
    /// Um quadro do vídeo para servir de capa antes do play.
    Poster { id: String, path: PathBuf },
    /// Emoji custom do servidor, que chega em base64 junto da listagem.
    Emoji { id: String, blob: String },
}

pub enum Loaded {
    Image {
        key: String,
        image: Box<ColorImage>,
    },
    /// GIF animado: quadros e a espera de cada um, em segundos.
    Animation {
        key: String,
        frames: Vec<(ColorImage, f32)>,
    },
    File {
        id: String,
        path: PathBuf,
    },
    Saved {
        name: String,
        path: PathBuf,
    },
    Waveform {
        id: String,
        peaks: Vec<f32>,
    },
    Failed {
        key: String,
        error: String,
    },
}

pub struct Media {
    requests: mpsc::UnboundedSender<Request>,
    results: sync_mpsc::Receiver<Loaded>,
}

impl Media {
    pub fn spawn(base_url: String, session: Arc<Session>, repaint: egui::Context) -> Option<Self> {
        let (requests_tx, requests_rx) = mpsc::unbounded_channel();
        let (results_tx, results_rx) = sync_mpsc::channel();

        std::thread::Builder::new()
            .name("papo-media".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        log::error!("mídia sem runtime: {error}");
                        return;
                    }
                };
                runtime.block_on(worker(base_url, session, requests_rx, results_tx, repaint));
            })
            .ok()?;

        Some(Self {
            requests: requests_tx,
            results: results_rx,
        })
    }

    pub fn request(&self, request: Request) {
        let _ = self.requests.send(request);
    }

    pub fn try_recv(&self) -> Option<Loaded> {
        self.results.try_recv().ok()
    }
}

async fn worker(
    base_url: String,
    session: Arc<Session>,
    mut requests: mpsc::UnboundedReceiver<Request>,
    results: sync_mpsc::Sender<Loaded>,
    repaint: egui::Context,
) {
    let Ok(api) = Api::new(&base_url, session) else {
        log::error!("mídia sem cliente HTTP");
        return;
    };

    // O cache em disco não tinha quem o limpasse. Uma varrida na partida,
    // fora da thread da janela.
    tokio::task::spawn_blocking(|| sweep_cache(&cache_root()));

    while let Some(request) = requests.recv().await {
        let api = api.clone();
        let results = results.clone();
        let repaint = repaint.clone();
        tokio::spawn(async move {
            let outcome = run(&api, request).await;
            if results.send(outcome).is_ok() {
                repaint.request_repaint();
            }
        });
    }
}

async fn run(api: &Api, request: Request) -> Loaded {
    match request {
        Request::Thumb { id, thumb_id } => {
            let key = thumb_key(&id);
            // A miniatura do servidor evita baixar o original inteiro; sem
            // ela, a própria imagem serve.
            let path = match &thumb_id {
                Some(_) => format!("/attachments/{id}/thumbnail"),
                None => format!("/attachments/{id}"),
            };
            match cached_fetch(api, &cache_path("thumbs", &id, ""), &path).await {
                Ok(bytes) => decode(key, &bytes, INLINE_MAX),
                Err(error) => Loaded::Failed { key, error },
            }
        }
        Request::Full { id } => {
            let key = full_key(&id);
            match cached_fetch(api, &cache_path("files", &id, ""), &format!("/attachments/{id}")).await
            {
                Ok(bytes) => decode(key, &bytes, FULL_MAX),
                Err(error) => Loaded::Failed { key, error },
            }
        }
        Request::File { id, name } => {
            let path = cache_path("files", &id, &name);
            match cached_file(api, &path, &format!("/attachments/{id}")).await {
                Ok(()) => Loaded::File { id, path },
                Err(error) => Loaded::Failed {
                    key: file_key(&id),
                    error,
                },
            }
        }
        Request::Save { id, name, dest } => {
            let source = cache_path("files", &id, &name);
            match cached_file(api, &source, &format!("/attachments/{id}")).await {
                // `copy` vai em pedaços: salvar um vídeo grande não precisa
                // dele inteiro na memória.
                Ok(()) => match tokio::fs::copy(&source, &dest).await {
                    Ok(_) => Loaded::Saved { name, path: dest },
                    Err(error) => Loaded::Failed {
                        key: file_key(&id),
                        error: error.to_string(),
                    },
                },
                Err(error) => Loaded::Failed {
                    key: file_key(&id),
                    error,
                },
            }
        }
        // Capa e forma de onda são trabalho **bloqueante**: o GStreamer
        // decodifica de forma síncrona, e cada uma segura a thread por
        // segundos. Rodá-las direto aqui prendia as threads do runtime, e
        // com elas os downloads dos outros anexos. `spawn_blocking` as tira
        // do caminho.
        Request::Poster { id, path } => {
            let key = poster_key(&id);
            // A capa fica em disco depois de tirada. Decodificar vídeo é
            // caro, e sem isto cada abertura do aplicativo montava um
            // decodificador por vídeo da conversa outra vez — que é
            // justamente o que o cartão de vídeo evita não abrindo player
            // sozinho.
            let cached = cache_path("thumbs", &id, "capa.png");
            if let Ok(bytes) = tokio::fs::read(&cached).await
                && !bytes.is_empty()
            {
                return decode(key, &bytes, INLINE_MAX);
            }
            // Uma capa por vez. `spawn_blocking` tira a decodificação das
            // threads do runtime, mas não limita quantas acontecem juntas:
            // uma conversa com cinco vídeos à vista montava cinco pipelines
            // do GStreamer ao mesmo tempo, cada um com seus decodificadores
            // do aparelho. Capa é enfeite — pode esperar a vez.
            let _turn = poster_queue().acquire().await;
            match tokio::task::spawn_blocking(move || {
                let image = player::poster(&path)?;
                save_poster(&cached, &image);
                Some(image)
            })
            .await
            {
                Ok(Some(image)) => Loaded::Image {
                    key,
                    image: Box::new(image),
                },
                Ok(None) => Loaded::Failed {
                    key,
                    error: "sem capa".into(),
                },
                Err(error) => Loaded::Failed {
                    key,
                    error: format!("capa: {error}"),
                },
            }
        }
        Request::Waveform { id, path } => {
            match tokio::task::spawn_blocking(move || player::waveform(&path)).await {
                Ok(Some(peaks)) => Loaded::Waveform { id, peaks },
                _ => Loaded::Failed {
                    key: waveform_key(&id),
                    error: "sem forma de onda".into(),
                },
            }
        }
        Request::Emoji { id, blob } => {
            use base64::Engine as _;
            let key = emoji_key(&id);
            match base64::engine::general_purpose::STANDARD.decode(blob.as_bytes()) {
                Ok(bytes) => decode(key, &bytes, 128),
                Err(error) => Loaded::Failed {
                    key,
                    error: error.to_string(),
                },
            }
        }
    }
}

/// Lê do cache quando já existe; senão busca, grava e devolve.
async fn cached_fetch(api: &Api, path: &Path, route: &str) -> Result<Vec<u8>, String> {
    if let Ok(bytes) = tokio::fs::read(path).await
        && !bytes.is_empty()
    {
        return Ok(bytes);
    }
    let (bytes, _) = api
        .fetch_bytes(route)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(path, &bytes).await;
    Ok(bytes)
}

/// Garante o arquivo no cache, baixando direto para o disco quando falta.
/// Diferente de [`cached_fetch`], nada aqui precisa dos bytes em memória: o
/// player lê do arquivo, então não há motivo para carregar um vídeo inteiro
/// só para gravá-lo.
async fn cached_file(api: &Api, path: &Path, route: &str) -> Result<(), String> {
    if let Ok(meta) = tokio::fs::metadata(path).await
        && meta.len() > 0
    {
        return Ok(());
    }
    api.fetch_to_file(route, path)
        .await
        .map_err(|error| error.to_string())
}

/// Decodifica bytes em textura; GIF vira animação.
fn decode(key: String, bytes: &[u8], max: u32) -> Loaded {
    if bytes.starts_with(b"GIF8")
        && let Some(frames) = decode_gif(bytes, max)
    {
        if frames.len() > 1 {
            return Loaded::Animation { key, frames };
        }
        if let Some((image, _)) = frames.into_iter().next() {
            return Loaded::Image {
                key,
                image: Box::new(image),
            };
        }
    }

    match image::load_from_memory(bytes) {
        Ok(image) => Loaded::Image {
            key,
            image: Box::new(to_color_image(image, max)),
        },
        Err(error) => Loaded::Failed {
            key,
            error: error.to_string(),
        },
    }
}

fn decode_gif(bytes: &[u8], max: u32) -> Option<Vec<(ColorImage, f32)>> {
    use image::AnimationDecoder;
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let frames = decoder.into_frames().take(240);
    let mut out = Vec::new();
    for frame in frames {
        let Ok(frame) = frame else { break };
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = if denominator == 0 {
            0.1
        } else {
            (numerator as f32 / denominator as f32 / 1000.0).max(0.02)
        };
        let buffer = image::DynamicImage::ImageRgba8(frame.into_buffer());
        out.push((to_color_image(buffer, max), delay));
    }
    (!out.is_empty()).then_some(out)
}

fn to_color_image(image: image::DynamicImage, max: u32) -> ColorImage {
    let image = if image.width().max(image.height()) > max {
        image.resize(max, max, image::imageops::FilterType::CatmullRom)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
}

// ---------------------------------------------------------------------------
// Cache em disco
// ---------------------------------------------------------------------------

pub fn cache_root() -> PathBuf {
    crate::platform::dirs::cache_dir()
}

/// `~/.cache/papo/<bucket>/<id>-<nome>`; o nome ajuda o player a adivinhar o
/// formato e deixa o cache legível para quem for espiar.
pub fn cache_path(bucket: &str, id: &str, name: &str) -> PathBuf {
    let mut file = id.to_owned();
    let name: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .take(64)
        .collect();
    if !name.is_empty() {
        file.push('-');
        file.push_str(&name);
    }
    cache_root().join(bucket).join(file)
}

/// Poda o cache em disco: apaga restos de download interrompido, o que está
/// parado há tempo demais e, se ainda passar do teto, o mais antigo até
/// caber. Recebe a raiz para poder ser testada fora da pasta do usuário.
fn sweep_cache(root: &Path) {
    sweep_cache_with(root, CACHE_BUDGET, CACHE_MAX_AGE, RECORDING_MAX_AGE);
}

fn sweep_cache_with(root: &Path, budget: u64, max_age: Duration, recording_max_age: Duration) {
    let now = SystemTime::now();
    let mut kept: Vec<(SystemTime, u64, PathBuf)> = Vec::new();

    for bucket in ["thumbs", "files"] {
        let Ok(entries) = std::fs::read_dir(root.join(bucket)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            // Sobra de download interrompido: nunca vai ser completada.
            if path.extension().is_some_and(|ext| ext == "parcial") {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let used = meta.accessed().or_else(|_| meta.modified()).unwrap_or(now);
            if now.duration_since(used).unwrap_or_default() > max_age {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            kept.push((used, meta.len(), path));
        }
    }

    if let Ok(entries) = std::fs::read_dir(root.join("recordings")) {
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            let used = meta.modified().unwrap_or(now);
            if now.duration_since(used).unwrap_or_default() > recording_max_age {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    let mut total: u64 = kept.iter().map(|(_, size, _)| *size).sum();
    if total <= budget {
        return;
    }
    // Do mais antigo para o mais novo, até caber.
    kept.sort_unstable_by_key(|(used, _, _)| *used);
    for (_, size, path) in kept {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

pub fn thumb_key(id: &str) -> String {
    format!("thumb:{id}")
}
pub fn full_key(id: &str) -> String {
    format!("full:{id}")
}
pub fn file_key(id: &str) -> String {
    format!("file:{id}")
}
/// A fila das capas: só uma extração de cada vez.
fn poster_queue() -> &'static tokio::sync::Semaphore {
    static QUEUE: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    QUEUE.get_or_init(|| tokio::sync::Semaphore::new(1))
}

/// Guarda a capa em disco para não decodificar o vídeo de novo amanhã.
fn save_poster(dest: &Path, image: &ColorImage) {
    let (width, height) = (image.size[0] as u32, image.size[1] as u32);
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_srgba_unmultiplied())
        .collect();
    let Some(buffer) = image::RgbaImage::from_raw(width, height, bytes) else {
        return;
    };
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = buffer.save_with_format(dest, image::ImageFormat::Png) {
        log::warn!("capa não foi guardada: {error}");
    }
}

pub fn poster_key(id: &str) -> String {
    format!("poster:{id}")
}

pub fn waveform_key(id: &str) -> String {
    format!("wave:{id}")
}
pub fn emoji_key(id: &str) -> String {
    format!("emoji:{id}")
}

// ---------------------------------------------------------------------------
// Estado do lado da interface
// ---------------------------------------------------------------------------

pub enum Texture {
    Loading,
    Ready(TextureHandle),
    Animated {
        frames: Vec<TextureHandle>,
        delays: Vec<f32>,
    },
    Failed,
}

impl Texture {
    /// Quadro a mostrar agora; a animação avança com o relógio do egui.
    pub fn frame(&self, ctx: &egui::Context) -> Option<&TextureHandle> {
        match self {
            Texture::Ready(handle) => Some(handle),
            Texture::Animated { frames, delays } => {
                let total: f32 = delays.iter().sum();
                if total <= 0.0 {
                    return frames.first();
                }
                let mut time = (ctx.input(|input| input.time) as f32) % total;
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
                for (index, delay) in delays.iter().enumerate() {
                    if time < *delay {
                        return frames.get(index);
                    }
                    time -= delay;
                }
                frames.last()
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum FileState {
    Loading,
    Ready(PathBuf),
    Failed,
}

/// Quanto um item custa em memória. O mapa guarda um identificador, mas os
/// pixels seguem alocados enquanto o `TextureHandle` viver.
fn texture_bytes(texture: &Texture) -> usize {
    fn one(handle: &TextureHandle) -> usize {
        let size = handle.size();
        size[0] * size[1] * 4
    }
    match texture {
        Texture::Ready(handle) => one(handle),
        Texture::Animated { frames, .. } => frames.iter().map(one).sum(),
        Texture::Loading | Texture::Failed => 0,
    }
}

/// Tudo que a interface precisa saber sobre a mídia já pedida.
pub struct MediaStore {
    media: Option<Media>,
    textures: HashMap<String, Texture>,
    files: HashMap<String, FileState>,
    waveforms: HashMap<String, Vec<f32>>,
    players: HashMap<String, player::Player>,
    /// Última vez que alguém pediu cada chave, para saber quem sai quando o
    /// teto aperta. Guarda textura e player no mesmo mapa: as chaves de
    /// textura vêm prefixadas (`thumb:`, `full:`…) e as de player são o id
    /// cru do anexo, então não se cruzam.
    used: HashMap<String, u64>,
    tick: u64,
    /// Anexos cuja moderação marcou como sensível e o usuário revelou.
    revealed: std::collections::HashSet<String>,
    /// Último arquivo salvo, para o aviso flutuante.
    pub saved: Option<(String, PathBuf, f64)>,
    /// Emojis unicode em imagem colorida, tirados da fonte do sistema.
    emoji_raster: EmojiRaster,
}

impl MediaStore {
    pub fn new(media: Option<Media>) -> Self {
        Self {
            media,
            textures: HashMap::new(),
            files: HashMap::new(),
            waveforms: HashMap::new(),
            players: HashMap::new(),
            used: HashMap::new(),
            tick: 0,
            revealed: std::collections::HashSet::new(),
            saved: None,
            emoji_raster: EmojiRaster::new(),
        }
    }

    /// Consome o que a thread de mídia terminou. Devolve `true` quando algo
    /// novo entrou — o que costuma mudar a altura da conversa.
    pub fn pump(&mut self, ctx: &egui::Context) -> bool {
        let Some(media) = &self.media else {
            return false;
        };
        let mut loaded = Vec::new();
        while let Some(item) = media.try_recv() {
            loaded.push(item);
        }
        let changed = !loaded.is_empty();
        for item in loaded {
            match item {
                Loaded::Image { key, image } => {
                    let handle = ctx.load_texture(key.clone(), *image, TextureOptions::LINEAR);
                    self.textures.insert(key, Texture::Ready(handle));
                }
                Loaded::Animation { key, frames } => {
                    let mut handles = Vec::with_capacity(frames.len());
                    let mut delays = Vec::with_capacity(frames.len());
                    for (index, (image, delay)) in frames.into_iter().enumerate() {
                        handles.push(ctx.load_texture(
                            format!("{key}#{index}"),
                            image,
                            TextureOptions::LINEAR,
                        ));
                        delays.push(delay);
                    }
                    self.textures.insert(
                        key,
                        Texture::Animated {
                            frames: handles,
                            delays,
                        },
                    );
                }
                Loaded::File { id, path } => {
                    self.files.insert(file_key(&id), FileState::Ready(path));
                }
                Loaded::Saved { name, path } => {
                    self.saved = Some((name, path, ctx.input(|input| input.time)));
                }
                Loaded::Waveform { id, peaks } => {
                    self.waveforms.insert(waveform_key(&id), peaks);
                }
                Loaded::Failed { key, error } => {
                    log::warn!("mídia {key}: {error}");
                    if key.starts_with("file:") {
                        self.files.insert(key, FileState::Failed);
                    } else {
                        self.textures.insert(key, Texture::Failed);
                    }
                }
            }
        }
        self.evict();
        changed
    }

    /// Marca a chave como usada agora.
    fn touch(&mut self, key: &str) {
        self.tick += 1;
        self.used.insert(key.to_owned(), self.tick);
    }

    /// Devolve o que ninguém está olhando. Sem isto os mapas só cresciam:
    /// todo vídeo, áudio e imagem que passasse pela tela ficava carregado
    /// até o programa fechar.
    fn evict(&mut self) {
        // Players primeiro, que são o item caro. Um que esteja tocando nunca
        // sai — parar o som no meio por causa de uma conta de memória seria
        // trocar um defeito por outro pior.
        if self.players.len() > MAX_PLAYERS {
            let mut idle: Vec<(u64, String)> = self
                .players
                .iter()
                .filter(|(_, player)| !player.is_playing())
                .map(|(key, _)| (self.used.get(key).copied().unwrap_or(0), key.clone()))
                .collect();
            idle.sort_unstable();
            let excess = self.players.len().saturating_sub(MAX_PLAYERS);
            for (_, key) in idle.into_iter().take(excess) {
                self.players.remove(&key);
                self.used.remove(&key);
            }
        }

        // Texturas, do mais antigo para o mais novo. `Loading` fica: tirar a
        // marca faria o pedido em voo voltar para um mapa que não o espera
        // mais, e o download recomeçaria do zero.
        let mut total: usize = self.textures.values().map(texture_bytes).sum();
        if total <= TEXTURE_BUDGET {
            return;
        }
        let mut aged: Vec<(u64, String)> = self
            .textures
            .iter()
            .filter(|(_, texture)| !matches!(texture, Texture::Loading))
            .map(|(key, _)| (self.used.get(key).copied().unwrap_or(0), key.clone()))
            .collect();
        aged.sort_unstable();
        for (_, key) in aged {
            if total <= TEXTURE_BUDGET {
                break;
            }
            if let Some(texture) = self.textures.remove(&key) {
                // Soltar o `TextureHandle` é o que devolve a memória da
                // placa de vídeo; o mapa só guardava o identificador.
                total = total.saturating_sub(texture_bytes(&texture));
                self.used.remove(&key);
            }
        }
    }

    fn ask(&self, request: Request) {
        if let Some(media) = &self.media {
            media.request(request);
        }
    }

    /// Miniatura do anexo, pedindo o download na primeira vez.
    pub fn thumb(&mut self, attachment: &Attachment) -> Option<&Texture> {
        let key = thumb_key(&attachment.id);
        if !self.textures.contains_key(&key) {
            self.textures.insert(key.clone(), Texture::Loading);
            self.ask(Request::Thumb {
                id: attachment.id.clone(),
                thumb_id: attachment.thumbnail_id.clone(),
            });
        }
        self.touch(&key);
        self.textures.get(&key)
    }

    /// Capa do vídeo: um quadro tirado do arquivo já em cache.
    ///
    /// Sem ela o cartão do vídeo é um retângulo preto até alguém dar play.
    pub fn poster(&mut self, id: &str, path: &Path) -> Option<&Texture> {
        let key = poster_key(id);
        if !self.textures.contains_key(&key) {
            self.textures.insert(key.clone(), Texture::Loading);
            self.ask(Request::Poster {
                id: id.to_owned(),
                path: path.to_owned(),
            });
        }
        self.touch(&key);
        self.textures.get(&key)
    }

    pub fn full(&mut self, id: &str) -> Option<&Texture> {
        let key = full_key(id);
        if !self.textures.contains_key(&key) {
            self.textures.insert(key.clone(), Texture::Loading);
            self.ask(Request::Full { id: id.to_owned() });
        }
        self.touch(&key);
        self.textures.get(&key)
    }

    pub fn emoji(&mut self, id: &str, blob: Option<&str>) -> Option<&Texture> {
        let key = emoji_key(id);
        if !self.textures.contains_key(&key) {
            let blob = blob?;
            self.textures.insert(key.clone(), Texture::Loading);
            self.ask(Request::Emoji {
                id: id.to_owned(),
                blob: blob.to_owned(),
            });
        }
        self.touch(&key);
        self.textures.get(&key)
    }

    /// Foto de perfil. Chega em base64 junto com o perfil, igual à
    /// figurinha, então usa o mesmo caminho — só com a chave separada, para
    /// que um id de pessoa nunca colida com um id de figurinha.
    pub fn avatar(&mut self, user_id: &str, blob: Option<&str>) -> Option<&Texture> {
        self.emoji(&format!("avatar:{user_id}"), blob)
    }

    /// Arquivo local do anexo, baixando na primeira vez.
    pub fn file(&mut self, id: &str, name: &str) -> FileState {
        let key = file_key(id);
        if let Some(state) = self.files.get(&key) {
            return state.clone();
        }
        self.files.insert(key, FileState::Loading);
        self.ask(Request::File {
            id: id.to_owned(),
            name: name.to_owned(),
        });
        FileState::Loading
    }

    pub fn waveform(&mut self, id: &str, path: &Path) -> Option<&[f32]> {
        let key = waveform_key(id);
        if !self.waveforms.contains_key(&key) {
            // Um vetor vazio marca "já pedido".
            self.waveforms.insert(key.clone(), Vec::new());
            self.ask(Request::Waveform {
                id: id.to_owned(),
                path: path.to_owned(),
            });
        }
        self.waveforms
            .get(&key)
            .map(Vec::as_slice)
            .filter(|peaks| !peaks.is_empty())
    }

    pub fn save(&self, id: &str, name: &str, dest: PathBuf) {
        self.ask(Request::Save {
            id: id.to_owned(),
            name: name.to_owned(),
            dest,
        });
    }

    /// Emoji unicode como imagem colorida; `None` quando o sistema não tem
    /// uma fonte de emoji em cores.
    pub fn unicode_emoji(&mut self, ctx: &egui::Context, emoji: &str) -> Option<TextureHandle> {
        self.emoji_raster.texture(ctx, emoji).cloned()
    }

    pub fn sensitive_hidden(&self, id: &str) -> bool {
        !self.revealed.contains(id)
    }

    pub fn reveal(&mut self, id: &str) {
        self.revealed.insert(id.to_owned());
    }

    // -- Players -----------------------------------------------------------

    /// Abre o player do anexo, se ainda não houver. Caro: monta thread,
    /// decodificador e contexto de vídeo. Só quem sabe que o usuário pediu a
    /// mídia chama isto — desenhar o cartão não é pedir.
    pub fn start_player(
        &mut self,
        id: &str,
        path: &Path,
        video: bool,
        ctx: &egui::Context,
    ) -> Option<&mut player::Player> {
        if !self.players.contains_key(id) {
            let player = player::Player::open(path, video, ctx.clone())?;
            self.players.insert(id.to_owned(), player);
        }
        self.touch(id);
        self.players.get_mut(id)
    }

    /// Liga ou desliga o anexo, abrindo o player se for a primeira vez. Um
    /// player recém-aberto começa parado, então aqui ele já sai tocando: o
    /// clique que o criou era um pedido de play.
    pub fn toggle_player(&mut self, id: &str, path: &Path, video: bool, ctx: &egui::Context) {
        let fresh = !self.players.contains_key(id);
        let Some(player) = self.start_player(id, path, video, ctx) else {
            return;
        };
        if fresh {
            player.play();
        } else {
            player.toggle();
        }
    }

    /// Player já aberto, ou nada. Quem só desenha o cartão usa isto e aceita
    /// não ter duração nem quadro antes do primeiro play.
    pub fn existing_player(&mut self, id: &str) -> Option<&mut player::Player> {
        if self.players.contains_key(id) {
            self.touch(id);
        }
        self.players.get_mut(id)
    }

    /// Pausa todos os outros — dois áudios ao mesmo tempo nunca é o que se
    /// quer.
    pub fn solo(&mut self, id: &str) {
        for (key, player) in self.players.iter_mut() {
            if key != id {
                player.pause();
            }
        }
    }

    pub fn pause_all(&mut self) {
        for player in self.players.values_mut() {
            player.pause();
        }
    }
}

#[cfg(test)]
mod limpeza {
    use super::*;

    fn raiz(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("papo-teste-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for bucket in ["thumbs", "files", "recordings"] {
            std::fs::create_dir_all(dir.join(bucket)).unwrap();
        }
        dir
    }

    fn escreve(path: &Path, bytes: usize, idade: Duration) {
        std::fs::write(path, vec![0u8; bytes]).unwrap();
        let quando = SystemTime::now() - idade;
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_times(
            std::fs::FileTimes::new()
                .set_accessed(quando)
                .set_modified(quando),
        )
        .unwrap();
    }

    const HORA: Duration = Duration::from_secs(3600);

    #[test]
    fn resto_de_download_interrompido_sai() {
        let raiz = raiz("parcial");
        let sobra = raiz.join("files/video.mp4.parcial");
        let bom = raiz.join("files/video.mp4");
        escreve(&sobra, 10, HORA);
        escreve(&bom, 10, HORA);

        sweep_cache_with(&raiz, 1 << 30, HORA * 24, HORA * 24);

        assert!(!sobra.exists(), "o arquivo parcial devia ter saído");
        assert!(bom.exists(), "o arquivo inteiro devia ter ficado");
    }

    #[test]
    fn o_que_esta_parado_ha_tempo_demais_sai() {
        let raiz = raiz("idade");
        let velho = raiz.join("thumbs/velho");
        let novo = raiz.join("thumbs/novo");
        escreve(&velho, 10, HORA * 50);
        escreve(&novo, 10, HORA);

        sweep_cache_with(&raiz, 1 << 30, HORA * 24, HORA * 24);

        assert!(!velho.exists());
        assert!(novo.exists());
    }

    #[test]
    fn passando_do_teto_o_mais_antigo_sai_primeiro() {
        let raiz = raiz("teto");
        let antigo = raiz.join("files/antigo");
        let medio = raiz.join("files/medio");
        let recente = raiz.join("files/recente");
        escreve(&antigo, 1000, HORA * 3);
        escreve(&medio, 1000, HORA * 2);
        escreve(&recente, 1000, HORA);

        // Cabem dois dos três.
        sweep_cache_with(&raiz, 2000, HORA * 24, HORA * 24);

        assert!(!antigo.exists(), "o mais antigo devia sair primeiro");
        assert!(medio.exists());
        assert!(recente.exists());
    }

    #[test]
    fn gravacao_recente_do_usuario_nao_e_apagada() {
        let raiz = raiz("gravacao");
        let pendente = raiz.join("recordings/audio.ogg");
        // Bem acima do teto de tamanho: gravação não entra nessa conta, ela
        // só sai por idade. Uma esperando no campo de escrever não pode
        // sumir por causa de um vídeo baixado.
        escreve(&pendente, 5000, HORA);

        sweep_cache_with(&raiz, 0, HORA * 24, HORA * 24);

        assert!(pendente.exists());
    }

    #[test]
    fn gravacao_esquecida_ha_semanas_sai() {
        let raiz = raiz("gravacao-velha");
        let esquecida = raiz.join("recordings/audio.ogg");
        escreve(&esquecida, 10, HORA * 50);

        sweep_cache_with(&raiz, 1 << 30, HORA * 24, HORA * 24);

        assert!(!esquecida.exists());
    }
}
