//! Mídia das mensagens: download, cache em disco, texturas e players.
//!
//! O anexo chega por uma rota autenticada, então nada pode ser entregue ao
//! GStreamer ou ao egui como URL: tudo passa por aqui, é gravado no cache do
//! usuário e só então vira textura ou arquivo para tocar.

/// Conferência do GStreamer no Android, escrita no logcat na abertura.
#[cfg(target_os = "android")]
pub mod gst_check;
pub mod backend;
pub mod platform;
pub mod player;
pub mod prepare;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Once};
use std::time::{Duration, Instant, SystemTime};


use egui::{ColorImage, TextureHandle, TextureOptions};
use tokio::sync::mpsc;

use crate::api::client::{Api, Session};
use crate::api::models::{Attachment, LinkPreview};
use crate::ui::emoji_raster::EmojiRaster;
use backend::{
    DirectMediaKind, DirectMediaPlayer, DirectMediaSource, GStreamerBackend, PlaybackBackend,
};

/// Limites de mídia ficam em uma política única, em vez de constantes
/// espalhadas pela apresentação. PR37 pode trocar estes defaults por alvo.
#[derive(Clone, Debug)]
pub struct MediaLimits {
    pub max_players: usize,
    pub player_idle_ttl: Duration,
    pub texture_budget: usize,
    pub disk_budget: u64,
    pub disk_max_age: Duration,
}

impl Default for MediaLimits {
    fn default() -> Self {
        Self {
            max_players: 4,
            player_idle_ttl: Duration::from_secs(3 * 60),
            texture_budget: 192 * 1024 * 1024,
            disk_budget: 512 * 1024 * 1024,
            disk_max_age: Duration::from_secs(30 * 24 * 60 * 60),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimLevel {
    Light,
    Moderate,
    Critical,
}

/// Gravações são do usuário, não mídia baixada, e só saem quando velhas
/// demais para alguma ainda estar esperando no campo de escrever.
const RECORDING_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Um `.parcial`/`.tmp` mais novo que isto ainda pode estar sendo baixado.
/// A varredura não o toca para não abortar o download no meio do caminho.
const PARTIAL_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Downloads autenticados simultâneos por worker de mídia. O cartão deixou de
/// baixar sozinho, mas várias miniaturas visíveis ainda podem coincidir — e o
/// backend castiga rajadas de `/attachments/:id` com 429. Fica na faixa de
/// 2–4 pedida pela política de mídia; o teste `fetch_gate_limits_...` prova
/// que o portão impõe o teto.
const AUTH_FETCH_CONCURRENCY: usize = 3;

/// A limpeza de partida pertence ao processo, não a cada workspace. Sem esta
/// guarda, dez servidores disparavam dez varreduras concorrentes da mesma raiz.
static STARTUP_CACHE_SWEEP: Once = Once::new();

const INLINE_MAX: u32 = 1600;
const FULL_MAX: u32 = 4096;

#[derive(Debug, Clone)]
pub enum Request {
    /// Miniatura do servidor. Só existe quando a listagem anuncia um
    /// `thumbnail_id`; sem ele o cartão nem chega aqui, e o arquivo inteiro
    /// nunca é baixado só para virar capa.
    Thumb { id: String },
    /// Imagem em tamanho cheio, para o visualizador.
    Full { id: String },
    /// Garante o arquivo no cache e devolve o caminho (vídeo, áudio, outros).
    File { id: String, name: String },
    /// Verifica se o arquivo já está em disco, **sem** baixar. É a única
    /// consulta que desenhar um cartão pode disparar: uma ida ao disco local
    /// não é um download, e é assim que mídia guardada por uma sessão anterior
    /// reaparece.
    Probe { id: String, name: String },
    /// Copia o anexo para fora do cache.
    Save { id: String, name: String, dest: PathBuf },
    /// Picos do áudio para desenhar a forma de onda.
    Waveform { id: String, path: PathBuf },
    /// Um quadro do vídeo para servir de capa antes do play.
    Poster { id: String, path: PathBuf },
    /// Emoji custom do servidor, que chega em base64 junto da listagem.
    Emoji { id: String, blob: String },
    /// Thumbnail de link preview. Em mensagens históricas a listagem traz os
    /// metadados, mas a imagem fica no endpoint autenticado do preview.
    Preview {
        id: String,
        blob: Option<String>,
    },
    /// Imagem pública de preview. A chave é derivada da URL canônica e
    /// portanto é compartilhada entre mensagens e reinícios.
    RemoteImage {
        id: String,
        url: String,
    },
}

impl Request {
    /// Rótulo curto, para o log de diagnóstico saber o que foi pedido.
    fn kind(&self) -> &'static str {
        match self {
            Self::Thumb { .. } => "thumb",
            Self::Full { .. } => "full",
            Self::File { .. } => "file",
            Self::Probe { .. } => "probe",
            Self::Save { .. } => "save",
            Self::Waveform { .. } => "waveform",
            Self::Poster { .. } => "poster",
            Self::Emoji { .. } => "emoji",
            Self::Preview { .. } => "preview",
            Self::RemoteImage { .. } => "remote-image",
        }
    }
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
    /// O `Probe` não achou o arquivo em disco — e não deve baixá-lo sozinho.
    Absent {
        id: String,
    },
    /// Ausência já esperada (ex.: vídeo sem miniatura): marca a textura como
    /// falha sem poluir o log — 404 de miniatura é resposta normal.
    Missing {
        key: String,
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
        // A URL nunca entra no caminho em disco. Calcula uma vez por worker e
        // usa a mesma identidade estável que ClientDb/SecretStore já usam.
        let server_key = papo_core::server_key(&base_url);
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
                runtime.block_on(worker(
                    base_url,
                    server_key,
                    session,
                    requests_rx,
                    results_tx,
                    repaint,
                ));
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
    server_key: String,
    session: Arc<Session>,
    mut requests: mpsc::UnboundedReceiver<Request>,
    results: sync_mpsc::Sender<Loaded>,
    repaint: egui::Context,
) {
    let Ok(api) = Api::new(&base_url, session) else {
        log::error!("mídia sem cliente HTTP");
        return;
    };
    let remote_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .user_agent("Papo/0.2 remote-media")
        .build()
        .ok();

    // Uma única varrida por processo. Cada workspace tem seu próprio worker
    // de mídia, mas todos compartilham a mesma raiz/budget em disco.
    STARTUP_CACHE_SWEEP.call_once(|| {
        std::mem::drop(tokio::task::spawn_blocking(|| sweep_cache(&cache_root())));
    });

    let fetch_gate = Arc::new(FetchGate::new(AUTH_FETCH_CONCURRENCY));

    while let Some(request) = requests.recv().await {
        let api = api.clone();
        let server_key = server_key.clone();
        let remote_client = remote_client.clone();
        let results = results.clone();
        let repaint = repaint.clone();
        let fetch_gate = fetch_gate.clone();
        tokio::spawn(async move {
            let outcome =
                run(&api, &server_key, remote_client.as_ref(), &fetch_gate, request).await;
            if results.send(outcome).is_ok() {
                repaint.request_repaint();
            }
        });
    }
}

async fn run(
    api: &Api,
    server_key: &str,
    remote_client: Option<&reqwest::Client>,
    fetch_gate: &FetchGate,
    request: Request,
) -> Loaded {
    match request {
        Request::Thumb { id } => {
            let _slot = fetch_gate.acquire().await;
            let key = thumb_key(&id);
            // Só o endpoint de miniatura, nunca o arquivo inteiro.
            match cached_fetch(
                api,
                &authenticated_cache_path(server_key, "thumbs", &id, ""),
                &format!("/attachments/{id}/thumbnail"),
            )
            .await
            {
                Ok(bytes) => decode(key, &bytes, INLINE_MAX),
                // Ausência esperada (miniatura some do cache, por exemplo):
                // não polui o log.
                Err(_) => Loaded::Missing { key },
            }
        }
        Request::Full { id } => {
            let _slot = fetch_gate.acquire().await;
            let key = full_key(&id);
            match cached_fetch(
                api,
                &authenticated_cache_path(server_key, "files", &id, ""),
                &format!("/attachments/{id}"),
            )
            .await
            {
                Ok(bytes) => decode(key, &bytes, FULL_MAX),
                Err(error) => Loaded::Failed { key, error },
            }
        }
        Request::File { id, name } => {
            let _slot = fetch_gate.acquire().await;
            let path = authenticated_cache_path(server_key, "files", &id, &name);
            match cached_file(api, &path, &format!("/attachments/{id}")).await {
                Ok(()) => Loaded::File { id, path },
                Err(error) => Loaded::Failed {
                    key: file_key(&id),
                    error,
                },
            }
        }
        // Só olha o disco: nenhum byte sai pela rede por causa disto. É o que
        // permite um cartão reaproveitar o cache sem pedir o anexo.
        Request::Probe { id, name } => {
            let path = authenticated_cache_path(server_key, "files", &id, &name);
            match tokio::fs::metadata(&path).await {
                Ok(meta) if meta.len() > 0 => Loaded::File { id, path },
                _ => Loaded::Absent { id },
            }
        }
        Request::Save { id, name, dest } => {
            let _slot = fetch_gate.acquire().await;
            let source = authenticated_cache_path(server_key, "files", &id, &name);
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
            let cached = authenticated_cache_path(server_key, "thumbs", &id, "capa.png");
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
                sweep_cache(&cache_root());
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
        Request::Preview { id, blob } => {
            use base64::Engine as _;
            let key = preview_key(&id);
            let blob = match blob {
                Some(blob) => Ok(blob),
                None => api
                    .link_preview(&id)
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|preview| {
                        preview
                            .image_data
                            .ok_or_else(|| "preview sem imagem".to_owned())
                    }),
            };
            match blob.and_then(|blob| {
                base64::engine::general_purpose::STANDARD
                    .decode(blob.as_bytes())
                    .map_err(|error| error.to_string())
            }) {
                Ok(bytes) => decode(key, &bytes, INLINE_MAX),
                Err(error) => Loaded::Failed { key, error },
            }
        }
        Request::RemoteImage { id, url } => {
            let key = remote_image_key(&id);
            let path = public_remote_cache_path(&id);
            if let Ok(bytes) = tokio::fs::read(&path).await
                && !bytes.is_empty()
            {
                return decode(key, &bytes, FULL_MAX);
            }
            let Some(client) = remote_client else {
                return Loaded::Failed {
                    key,
                    error: "cliente de rich embed indisponível".into(),
                };
            };
            match papo_core::preview::fetch_bounded_remote_bytes(client, &url, 12 << 20).await {
                Ok(bytes) => {
                    if let Err(error) = atomic_cache_write(&path, &bytes).await {
                        log::warn!("remote-media {}: cache write: {error}", safe_resource_id(&id));
                    } else {
                        let _ = tokio::task::spawn_blocking(|| sweep_cache(&cache_root())).await;
                    }
                    decode(key, &bytes, FULL_MAX)
                }
                Err(error) => Loaded::Failed { key, error },
            }
        }
    }
}

async fn atomic_cache_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }
    let tmp = path.with_extension("parcial");
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|error| error.to_string())?;
    if tokio::fs::rename(&tmp, path).await.is_err() {
        let _ = tokio::fs::remove_file(path).await;
        tokio::fs::rename(&tmp, path)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn safe_resource_id(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
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
    if tokio::fs::write(path, &bytes).await.is_ok() {
        std::mem::drop(tokio::task::spawn_blocking(|| sweep_cache(&cache_root())));
    }
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
        .map_err(|error| error.to_string())?;
    std::mem::drop(tokio::task::spawn_blocking(|| sweep_cache(&cache_root())));
    Ok(())
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

fn cache_file_name(id: &str, name: &str) -> String {
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
    file
}

/// Mídia vinda de endpoints autenticados pertence a um servidor. IDs de anexo
/// só são únicos dentro desse backend e portanto nunca podem ser usados como
/// identidade global de disco.
fn authenticated_cache_path_at(
    root: &Path,
    server_key: &str,
    bucket: &str,
    id: &str,
    name: &str,
) -> PathBuf {
    debug_assert!(matches!(bucket, "files" | "thumbs"));
    root.join("servers")
        .join(server_key)
        .join(bucket)
        .join(cache_file_name(id, name))
}

pub fn authenticated_cache_path(server_key: &str, bucket: &str, id: &str, name: &str) -> PathBuf {
    authenticated_cache_path_at(&cache_root(), server_key, bucket, id, name)
}

/// Recursos públicos usam a URL canônica como identidade (hash em `id`) e
/// podem ser compartilhados com segurança entre workspaces.
fn public_remote_cache_path_at(root: &Path, resource_id: &str) -> PathBuf {
    root.join("remote").join(resource_id)
}

fn public_remote_cache_path(resource_id: &str) -> PathBuf {
    public_remote_cache_path_at(&cache_root(), resource_id)
}

fn safe_server_key(server_key: &str) -> bool {
    !server_key.is_empty()
        && server_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Função síncrona/testável. A camada de UI chama o wrapper abaixo em uma
/// thread separada para não bloquear um frame com muitos arquivos.
fn clear_server_media_cache_at(root: &Path, server_key: &str) -> std::io::Result<bool> {
    if !safe_server_key(server_key) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsafe server cache key",
        ));
    }
    match std::fs::remove_dir_all(root.join("servers").join(server_key)) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Remoção explícita de servidor é destrutiva para a mídia autenticada daquele
/// backend. Falha de cache não deve impedir a remoção do workspace.
pub fn clear_server_media_cache(server_key: &str) {
    let server_key = server_key.to_owned();
    let spawn = std::thread::Builder::new()
        .name("papo-media-clear".into())
        .spawn(move || match clear_server_media_cache_at(&cache_root(), &server_key) {
            Ok(true) => log::info!("media cache server namespace cleared server={server_key}"),
            Ok(false) => {}
            Err(error) => {
                log::warn!("media cache server namespace cleanup failed server={server_key}: {error}")
            }
        });
    if let Err(error) = spawn {
        log::warn!("media cache cleanup worker failed to start: {error}");
    }
}

/// Cache autenticado antigo era plano (`files/*`, `thumbs/*`) e não contém
/// informação suficiente para saber a qual servidor pertencia. Ele é
/// descartável: nunca o "adotamos" para o primeiro servidor que pedir o ID.
fn discard_legacy_authenticated_cache(root: &Path) {
    let mut removed = false;
    for bucket in ["files", "thumbs"] {
        let path = root.join(bucket);
        match std::fs::remove_dir_all(&path) {
            Ok(()) => removed = true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("media cache legacy cleanup failed bucket={bucket}: {error}"),
        }
    }
    if removed {
        log::info!("media cache legacy authenticated entries removed");
    }
}

fn collect_cache_bucket(
    dir: &Path,
    now: SystemTime,
    max_age: Duration,
    kept: &mut Vec<(SystemTime, u64, PathBuf)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if !kind.is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let used = meta.accessed().or_else(|_| meta.modified()).unwrap_or(now);

        // `.parcial`/`.tmp` é o rascunho de um download em andamento: o
        // arquivo é escrito nele e só então renomeado. Apagá-lo entre a
        // escrita e o rename faz o download falhar com ENOENT — e como cada
        // download bem-sucedido dispara uma varredura, uma conversa com
        // vários anexos à vista derrubava os que ainda estavam baixando.
        // Só some depois que já não pode estar em curso.
        if path
            .extension()
            .is_some_and(|ext| ext == "parcial" || ext == "tmp")
        {
            if now.duration_since(used).unwrap_or_default() > PARTIAL_MAX_AGE {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        if now.duration_since(used).unwrap_or_default() > max_age {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        kept.push((used, meta.len(), path));
    }
}

/// Poda somente a árvore conhecida do Papo:
/// `servers/<server-key>/{files,thumbs}` + `remote`. O teto é um só para
/// todos os servidores, não um teto por workspace.
fn sweep_cache(root: &Path) {
    let limits = MediaLimits::default();
    sweep_cache_with(
        root,
        limits.disk_budget,
        limits.disk_max_age,
        RECORDING_MAX_AGE,
    );
}

fn sweep_cache_with(root: &Path, budget: u64, max_age: Duration, recording_max_age: Duration) {
    discard_legacy_authenticated_cache(root);

    let now = SystemTime::now();
    let mut kept: Vec<(SystemTime, u64, PathBuf)> = Vec::new();

    collect_cache_bucket(&root.join("remote"), now, max_age, &mut kept);

    if let Ok(servers) = std::fs::read_dir(root.join("servers")) {
        for server in servers.flatten() {
            let Ok(kind) = server.file_type() else { continue };
            if !kind.is_dir() {
                continue;
            }
            let server_root = server.path();
            for bucket in ["files", "thumbs"] {
                collect_cache_bucket(&server_root.join(bucket), now, max_age, &mut kept);
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(root.join("recordings")) {
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if !kind.is_file() {
                continue;
            }
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

pub fn remote_player_key(url: &str) -> Option<String> {
    let canonical = papo_core::preview::canonical_url(url)?;
    Some(format!("remote-player:{}", remote_resource_id(&canonical)))
}

fn remote_resource_id(canonical_url: &str) -> String {
    // FNV-1a 128: deterministic across processes/platforms and sufficient for
    // cache identity. Correctness never depends on a file extension or raw URL.
    let mut hash = 0x6c62272e07bb014262b821756295c58du128;
    const PRIME: u128 = 0x0000000001000000000000000000013bu128;
    for byte in canonical_url.as_bytes() {
        hash ^= *byte as u128;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:032x}")
}

fn texture_eviction_class(key: &str) -> u8 {
    if key.starts_with("full:") || key.starts_with("remote-image:") {
        0
    } else if key.starts_with("poster:") {
        1
    } else if key.starts_with("thumb:") || key.starts_with("preview:") {
        2
    } else {
        3
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

/// Portão dos downloads autenticados. Um teto explícito evita que uma conversa
/// cheia de anexos dispare dezenas de `/attachments/:id` de uma vez — que é
/// justamente o que faz o backend responder 429. Cada worker de mídia tem o
/// seu, então o limite é por workspace, não global.
struct FetchGate {
    slots: tokio::sync::Semaphore,
}

impl FetchGate {
    fn new(permits: usize) -> Self {
        Self {
            slots: tokio::sync::Semaphore::new(permits),
        }
    }

    async fn acquire(&self) -> tokio::sync::SemaphorePermit<'_> {
        self.slots
            .acquire()
            .await
            .expect("o portão de mídia nunca fecha")
    }
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
pub fn preview_key(id: &str) -> String {
    format!("preview:{id}")
}
fn remote_image_key(id: &str) -> String {
    format!("remote-image:{id}")
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
    players: HashMap<String, DirectMediaPlayer>,
    playback: Arc<dyn PlaybackBackend>,
    limits: MediaLimits,
    player_last_used: HashMap<String, Instant>,
    /// Última vez que alguém pediu cada chave, para saber quem sai quando o
    /// teto aperta. Guarda textura e player no mesmo mapa: as chaves de
    /// textura vêm prefixadas (`thumb:`, `full:`…) e as de player são o id
    /// cru do anexo, então não se cruzam.
    used: HashMap<String, u64>,
    tick: u64,
    /// Anexos que o usuário mandou tocar antes de o arquivo existir: assim que
    /// o download chega em `pump`, o player abre e toca. O `bool` diz se é
    /// vídeo. Sem isto, pedir play só poderia funcionar com o arquivo em mãos.
    pending_play: HashMap<String, bool>,
    /// Anexos cujo disco já foi consultado uma vez, para um cartão desenhado
    /// a cada quadro não repetir a mesma pergunta. Não guarda o resultado:
    /// quem o tem é o mapa `files`.
    probed: std::collections::HashSet<String>,
    /// Anexos cuja moderação marcou como sensível e o usuário revelou.
    revealed: std::collections::HashSet<String>,
    /// Último arquivo salvo, para o aviso flutuante.
    pub saved: Option<(String, PathBuf, f64)>,
    /// Emojis unicode em imagem colorida, tirados da fonte do sistema.
    emoji_raster: EmojiRaster,
}

impl MediaStore {
    pub fn new(media: Option<Media>) -> Self {
        Self::with_backend_and_limits(
            media,
            Arc::new(GStreamerBackend),
            MediaLimits::default(),
        )
    }

    pub(crate) fn with_backend_and_limits(
        media: Option<Media>,
        playback: Arc<dyn PlaybackBackend>,
        limits: MediaLimits,
    ) -> Self {
        log::debug!("media backend={}", playback.name());
        Self {
            media,
            textures: HashMap::new(),
            files: HashMap::new(),
            waveforms: HashMap::new(),
            players: HashMap::new(),
            playback,
            limits,
            player_last_used: HashMap::new(),
            used: HashMap::new(),
            tick: 0,
            pending_play: HashMap::new(),
            probed: std::collections::HashSet::new(),
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
                    self.files.insert(file_key(&id), FileState::Ready(path.clone()));
                    // Um play pedido antes do arquivo existir só pode ser
                    // honrado agora: o clique era o pedido, o download foi o
                    // caminho.
                    if let Some(video) = self.pending_play.remove(&id) {
                        self.toggle_player(&id, &path, video, ctx);
                        self.solo(&id);
                    }
                }
                Loaded::Absent { .. } => {}
                Loaded::Missing { key } => {
                    self.textures.insert(key, Texture::Failed);
                }
                Loaded::Saved { name, path } => {
                    self.saved = Some((name, path, ctx.input(|input| input.time)));
                }
                Loaded::Waveform { id, peaks } => {
                    self.waveforms.insert(waveform_key(&id), peaks);
                }
                Loaded::Failed { key, error } => {
                    log::warn!("mídia {key}: {error}");
                    if let Some(id) = key.strip_prefix("file:") {
                        self.pending_play.remove(id);
                        self.files.insert(key, FileState::Failed);
                    } else {
                        self.textures.insert(key, Texture::Failed);
                    }
                }
            }
        }
        self.housekeep();
        changed
    }

    /// Marca a chave como usada agora.
    fn touch(&mut self, key: &str) {
        self.tick += 1;
        self.used.insert(key.to_owned(), self.tick);
    }

    fn touch_player(&mut self, key: &str) {
        self.touch(key);
        self.player_last_used.insert(key.to_owned(), Instant::now());
    }

    pub fn housekeep(&mut self) {
        self.housekeep_at(Instant::now());
    }

    fn housekeep_at(&mut self, now: Instant) {
        let ttl = self.limits.player_idle_ttl;
        let stale: Vec<String> = self
            .players
            .iter()
            .filter(|(_, player)| !player.is_playing())
            .filter_map(|(key, _)| {
                let last = self.player_last_used.get(key).copied()?;
                now.checked_duration_since(last)
                    .filter(|age| *age >= ttl)
                    .map(|_| key.clone())
            })
            .collect();
        for key in stale {
            self.remove_player(&key);
            log::debug!("media evicted_player reason=idle");
        }
        self.enforce_player_cap();
        self.trim_textures_to(self.limits.texture_budget);
    }

    fn remove_player(&mut self, key: &str) {
        self.players.remove(key);
        self.player_last_used.remove(key);
        self.used.remove(key);
    }

    fn enforce_player_cap(&mut self) {
        while self.players.len() > self.limits.max_players {
            let Some(key) = self.oldest_idle_player() else { break };
            self.remove_player(&key);
            log::debug!("media evicted_player reason=cap");
        }
    }

    fn oldest_idle_player(&self) -> Option<String> {
        self.players
            .iter()
            .filter(|(_, player)| !player.is_playing())
            .min_by_key(|(key, _)| self.used.get(*key).copied().unwrap_or(0))
            .map(|(key, _)| key.clone())
    }

    fn make_player_room(&mut self) -> bool {
        self.housekeep();
        while self.players.len() >= self.limits.max_players {
            let Some(key) = self.oldest_idle_player() else {
                return false;
            };
            self.remove_player(&key);
            log::debug!("media evicted_player reason=cap");
        }
        true
    }

    fn trim_textures_to(&mut self, budget: usize) {
        let mut total: usize = self.textures.values().map(texture_bytes).sum();
        if total <= budget {
            return;
        }
        let mut aged: Vec<(u8, u64, String)> = self
            .textures
            .iter()
            .filter(|(_, texture)| !matches!(texture, Texture::Loading))
            .map(|(key, _)| {
                (
                    texture_eviction_class(key),
                    self.used.get(key).copied().unwrap_or(0),
                    key.clone(),
                )
            })
            .collect();
        aged.sort_unstable();
        for (_, _, key) in aged {
            if total <= budget {
                break;
            }
            if let Some(texture) = self.textures.remove(&key) {
                total = total.saturating_sub(texture_bytes(&texture));
                self.used.remove(&key);
                log::debug!("media evicted_texture");
            }
        }
    }

    pub fn trim(&mut self, level: TrimLevel) {
        log::debug!("media trim level={level:?}");
        match level {
            TrimLevel::Light => {
                self.housekeep();
                self.trim_textures_to(self.limits.texture_budget.saturating_mul(3) / 4);
            }
            TrimLevel::Moderate => {
                let idle: Vec<String> = self
                    .players
                    .iter()
                    .filter(|(_, player)| !player.is_playing())
                    .map(|(key, _)| key.clone())
                    .collect();
                for key in idle {
                    self.remove_player(&key);
                }
                self.trim_textures_to(self.limits.texture_budget / 2);
            }
            TrimLevel::Critical => {
                let idle: Vec<String> = self
                    .players
                    .iter()
                    .filter(|(_, player)| !player.is_playing())
                    .map(|(key, _)| key.clone())
                    .collect();
                for key in idle {
                    self.remove_player(&key);
                }
                self.trim_textures_to(0);
            }
        }
    }

    fn ask(&self, request: Request) {
        log::trace!("media ask kind={}", request.kind());
        if let Some(media) = &self.media {
            media.request(request);
        }
    }

    /// Miniatura do servidor, pedindo o download na primeira vez. Sem
    /// `thumbnail_id` não há miniatura: devolve `None` sem pedir nada — o
    /// cartão desenha o quadro vazio e a imagem inteira só vem no visualizador.
    pub fn thumb(&mut self, attachment: &Attachment) -> Option<&Texture> {
        attachment.thumbnail_id.as_deref()?;
        let key = thumb_key(&attachment.id);
        if !self.textures.contains_key(&key) {
            self.textures.insert(key.clone(), Texture::Loading);
            self.ask(Request::Thumb {
                id: attachment.id.clone(),
            });
        }
        self.touch(&key);
        self.textures.get(&key)
    }

    /// Textura da miniatura já em memória, **sem disparar pedido**. Deixa o
    /// cartão medir o espaço antes de decidir buscar — e não pede nada fora
    /// da área visível.
    pub fn loaded_thumb(&self, id: &str) -> Option<&Texture> {
        self.textures.get(&thumb_key(id))
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

    /// Imagem de um link preview. Se o evento já trouxe `image_data`, evita
    /// a ida extra à rede; para mensagens antigas busca GET /link-previews/:id.
    pub fn preview(&mut self, preview: &LinkPreview) -> Option<&Texture> {
        let key = preview_key(&preview.id);
        if !self.textures.contains_key(&key) {
            // Sem MIME/tamanho e sem blob o backend já disse que não há imagem:
            // não vale disparar uma requisição que só voltaria vazia.
            if preview.image_data.is_none()
                && preview.image_mime_type.is_none()
                && preview.image_size_bytes.is_none()
            {
                return None;
            }
            self.textures.insert(key.clone(), Texture::Loading);
            self.ask(Request::Preview {
                id: preview.id.clone(),
                blob: preview.image_data.clone(),
            });
        }
        self.touch(&key);
        self.textures.get(&key)
    }

    pub fn remote_image(&mut self, _id: &str, url: &str) -> Option<&Texture> {
        let canonical = papo_core::preview::canonical_url(url)?;
        let id = remote_resource_id(&canonical);
        let key = remote_image_key(&id);
        if !self.textures.contains_key(&key) {
            self.textures.insert(key.clone(), Texture::Loading);
            self.ask(Request::RemoteImage { id, url: canonical });
        }
        self.touch(&key);
        self.textures.get(&key)
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

    /// Estado do arquivo **sem pedir nada**. É o que um cartão usa ao ser
    /// desenhado: renderizar não é pedir o anexo. `None` = nunca pedido.
    pub fn file_state(&self, id: &str) -> Option<FileState> {
        self.files.get(&file_key(id)).cloned()
    }

    /// Pergunta ao disco se o arquivo já existe, **sem baixá-lo**. É o que um
    /// cartão faz ao ser desenhado: uma ida ao cache local não é download, e é
    /// assim que uma mídia guardada numa sessão anterior reaparece sozinha.
    /// Idempotente por anexo.
    pub fn probe_file(&mut self, id: &str, name: &str) {
        let key = file_key(id);
        if self.files.contains_key(&key) || !self.probed.insert(key) {
            return;
        }
        self.ask(Request::Probe {
            id: id.to_owned(),
            name: name.to_owned(),
        });
    }

    /// Caminho do arquivo se ele já estiver em cache; não dispara download.
    pub fn file_ready(&self, id: &str) -> Option<PathBuf> {
        match self.files.get(&file_key(id)) {
            Some(FileState::Ready(path)) => Some(path.clone()),
            _ => None,
        }
    }

    /// Pedido explícito do usuário para tocar o anexo. Diferente de desenhar o
    /// cartão: só um clique/tap chega aqui. Com o arquivo em mãos, toca;
    /// sem ele, baixa e toca assim que chegar — o play é honrado, não perdido.
    pub fn toggle_play(&mut self, id: &str, name: &str, video: bool, ctx: &egui::Context) {
        if let Some(path) = self.file_ready(id) {
            self.toggle_player(id, &path, video, ctx);
            self.solo(id);
            return;
        }
        let key = file_key(id);
        // Uma tentativa anterior falhou: deixa pedir de novo.
        if matches!(self.files.get(&key), Some(FileState::Failed)) {
            self.files.remove(&key);
        }
        let mut need_fetch = false;
        match self.files.entry(key) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(FileState::Loading);
                need_fetch = true;
            }
            std::collections::hash_map::Entry::Occupied(_) => {}
        }
        if need_fetch {
            self.ask(Request::File {
                id: id.to_owned(),
                name: name.to_owned(),
            });
        }
        self.pending_play.insert(id.to_owned(), video);
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
    ) -> Option<&mut DirectMediaPlayer> {
        if !self.players.contains_key(id) {
            if !self.make_player_room() {
                return None;
            }
            let kind = if video { DirectMediaKind::Video } else { DirectMediaKind::Audio };
            let player = self.playback.open(
                DirectMediaSource::File(path.to_owned()),
                kind,
                ctx.clone(),
            )?;
            self.players.insert(id.to_owned(), player);
        }
        self.touch_player(id);
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

    pub fn start_remote_player(
        &mut self,
        id: &str,
        url: &str,
        ctx: &egui::Context,
    ) -> Option<&mut DirectMediaPlayer> {
        if !papo_core::preview::safe_remote_url(url) {
            return None;
        }
        if !self.players.contains_key(id) {
            if !self.make_player_room() {
                return None;
            }
            let player = self.playback.open(
                DirectMediaSource::RemoteUri(url.to_owned()),
                DirectMediaKind::Video,
                ctx.clone(),
            )?;
            self.players.insert(id.to_owned(), player);
        }
        self.touch_player(id);
        self.players.get_mut(id)
    }

    pub fn toggle_remote_player(&mut self, id: &str, url: &str, ctx: &egui::Context) {
        let fresh = !self.players.contains_key(id);
        let Some(player) = self.start_remote_player(id, url, ctx) else {
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
    pub fn existing_player(&mut self, id: &str) -> Option<&mut DirectMediaPlayer> {
        if self.players.contains_key(id) {
            self.touch_player(id);
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
mod lifecycle_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct FakeBackend {
        opens: AtomicUsize,
        sources: std::sync::Mutex<Vec<DirectMediaSource>>,
    }

    impl PlaybackBackend for FakeBackend {
        fn name(&self) -> &'static str { "fake" }

        fn open(
            &self,
            source: DirectMediaSource,
            _kind: DirectMediaKind,
            _repaint: egui::Context,
        ) -> Option<DirectMediaPlayer> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            self.sources.lock().unwrap().push(source);
            Some(DirectMediaPlayer::new(Box::new(FakePlayer::default())))
        }
    }

    #[derive(Default)]
    struct FakePlayer {
        playing: bool,
        muted: bool,
    }

    impl backend::PlaybackPlayer for FakePlayer {
        fn play(&mut self) { self.playing = true; }
        fn pause(&mut self) { self.playing = false; }
        fn toggle(&mut self) { self.playing = !self.playing; }
        fn seek(&mut self, _seconds: f64) { self.playing = true; }
        fn set_muted(&mut self, muted: bool) { self.muted = muted; }
        fn is_playing(&self) -> bool { self.playing }
        fn position(&self) -> f64 { 0.0 }
        fn duration(&self) -> f64 { 1.0 }
        fn aspect(&self) -> f32 { 16.0 / 9.0 }
        fn error(&self) -> Option<String> { None }
        fn update(&mut self) {}
        fn frame<'a>(&'a mut self, _ctx: &egui::Context) -> Option<&'a TextureHandle> { None }
    }

    fn store(backend: Arc<FakeBackend>, max_players: usize, ttl: Duration) -> MediaStore {
        MediaStore::with_backend_and_limits(
            None,
            backend,
            MediaLimits {
                max_players,
                player_idle_ttl: ttl,
                ..MediaLimits::default()
            },
        )
    }

    #[test]
    fn rendering_without_explicit_play_does_not_open_player() {
        let backend = Arc::new(FakeBackend::default());
        let mut media = store(Arc::clone(&backend), 4, Duration::from_secs(60));
        assert!(media.existing_player("x").is_none());
        assert_eq!(backend.opens.load(Ordering::SeqCst), 0);
    }

    /// Espiar o estado é o que o cartão faz ao ser desenhado. Não pode virar
    /// pedido de download — essa era justamente a regressão que enchia o
    /// backend de `/attachments/:id` a cada rolagem.
    #[test]
    fn peeking_file_state_never_starts_a_download() {
        let backend = Arc::new(FakeBackend::default());
        let media = store(backend, 4, Duration::from_secs(60));
        assert!(media.file_state("x").is_none());
        assert!(media.file_ready("x").is_none());
    }

    /// Play pedido antes do arquivo existir não inventa um player: registra a
    /// intenção e espera o download. Quem abre o player é a chegada do arquivo.
    #[test]
    fn play_without_file_waits_instead_of_opening_player() {
        let backend = Arc::new(FakeBackend::default());
        let mut media = store(Arc::clone(&backend), 4, Duration::from_secs(60));
        let ctx = egui::Context::default();
        media.toggle_play("x", "video.mp4", true, &ctx);
        assert!(media.existing_player("x").is_none());
        assert_eq!(backend.opens.load(Ordering::SeqCst), 0);
        assert!(matches!(media.file_state("x"), Some(FileState::Loading)));
    }

    #[test]
    fn explicit_open_is_single_flight_per_resource() {
        let backend = Arc::new(FakeBackend::default());
        let mut media = store(Arc::clone(&backend), 4, Duration::from_secs(60));
        let ctx = egui::Context::default();
        let path = Path::new("/tmp/fake.mp4");
        assert!(media.start_player("x", path, true, &ctx).is_some());
        assert!(media.start_player("x", path, true, &ctx).is_some());
        assert_eq!(backend.opens.load(Ordering::SeqCst), 1);
    }

    /// O portão existe para rajada não virar 429: prova que o teto é de fato
    /// respeitado, em vez de confiar num número escolhido a dedo.
    #[tokio::test]
    async fn fetch_gate_limits_concurrent_downloads() {
        use tokio::task::JoinSet;

        let gate = Arc::new(FetchGate::new(AUTH_FETCH_CONCURRENCY));
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();
        for _ in 0..24 {
            let gate = Arc::clone(&gate);
            let live = Arc::clone(&live);
            let peak = Arc::clone(&peak);
            tasks.spawn(async move {
                let _slot = gate.acquire().await;
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(2)).await;
                live.fetch_sub(1, Ordering::SeqCst);
            });
        }
        while let Some(outcome) = tasks.join_next().await {
            outcome.unwrap();
        }
        assert!(
            peak.load(Ordering::SeqCst) <= AUTH_FETCH_CONCURRENCY,
            "o portão deixou passar mais downloads simultâneos do que o teto"
        );
    }

    #[test]
    fn paused_idle_player_expires_but_playing_player_survives() {
        let backend = Arc::new(FakeBackend::default());
        let mut media = store(backend, 4, Duration::from_secs(10));
        let ctx = egui::Context::default();
        let path = Path::new("/tmp/fake.mp4");

        media.start_player("idle", path, true, &ctx).unwrap();
        let stale = Instant::now() - Duration::from_secs(20);
        media.player_last_used.insert("idle".into(), stale);
        media.housekeep_at(Instant::now());
        assert!(!media.players.contains_key("idle"));

        media.start_player("playing", path, true, &ctx).unwrap().play();
        media.player_last_used
            .insert("playing".into(), Instant::now() - Duration::from_secs(20));
        media.housekeep_at(Instant::now());
        assert!(media.players.contains_key("playing"));
    }

    #[test]
    fn hard_cap_evicts_oldest_idle_and_never_interrupts_active() {
        let backend = Arc::new(FakeBackend::default());
        let mut media = store(backend, 2, Duration::from_secs(3600));
        let ctx = egui::Context::default();
        let path = Path::new("/tmp/fake.mp4");

        media.start_player("a", path, true, &ctx).unwrap();
        media.start_player("b", path, true, &ctx).unwrap();
        media.start_player("c", path, true, &ctx).unwrap();
        assert_eq!(media.players.len(), 2);
        assert!(!media.players.contains_key("a"));

        media.players.get_mut("b").unwrap().play();
        media.players.get_mut("c").unwrap().play();
        assert!(media.start_player("d", path, true, &ctx).is_none());
        assert!(media.players.contains_key("b"));
        assert!(media.players.contains_key("c"));
    }

    #[test]
    fn solo_pauses_competing_playback() {
        let backend = Arc::new(FakeBackend::default());
        let mut media = store(backend, 4, Duration::from_secs(3600));
        let ctx = egui::Context::default();
        let path = Path::new("/tmp/fake.mp4");
        media.start_player("a", path, true, &ctx).unwrap().play();
        media.start_player("b", path, true, &ctx).unwrap().play();
        media.solo("b");
        assert!(!media.players.get("a").unwrap().is_playing());
        assert!(media.players.get("b").unwrap().is_playing());
    }

    #[test]
    fn moderate_and_critical_trim_drop_idle_players() {
        let backend = Arc::new(FakeBackend::default());
        let mut media = store(backend, 4, Duration::from_secs(3600));
        let ctx = egui::Context::default();
        let path = Path::new("/tmp/fake.mp4");
        media.start_player("idle", path, true, &ctx).unwrap();
        media.start_player("playing", path, true, &ctx).unwrap().play();

        media.trim(TrimLevel::Moderate);
        assert!(!media.players.contains_key("idle"));
        assert!(media.players.contains_key("playing"));

        media.players.get_mut("playing").unwrap().pause();
        media.trim(TrimLevel::Critical);
        assert!(media.players.is_empty());
    }

    fn store_com_budget(budget: usize) -> MediaStore {
        MediaStore::with_backend_and_limits(
            None,
            Arc::new(FakeBackend::default()),
            MediaLimits {
                texture_budget: budget,
                ..MediaLimits::default()
            },
        )
    }

    fn textura(ctx: &egui::Context, key: &str, w: usize, h: usize) -> Texture {
        let bytes = vec![0u8; w * h * 4];
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &bytes);
        Texture::Ready(ctx.load_texture(key, image, egui::TextureOptions::NEAREST))
    }

    fn total_bytes(store: &MediaStore) -> usize {
        store.textures.values().map(texture_bytes).sum()
    }

    // Light encolhe na direção de 75% do teto, sem zerar tudo.
    #[test]
    fn light_reduz_texturas_a_tres_quartos() {
        let ctx = egui::Context::default();
        let mut media = store_com_budget(1000);
        for key in ["thumb:a", "thumb:b", "thumb:c"] {
            let t = textura(&ctx, key, 10, 10); // 400 bytes cada
            media.textures.insert(key.into(), t);
        }
        assert_eq!(total_bytes(&media), 1200);

        media.trim(TrimLevel::Light);

        assert!(total_bytes(&media) <= 750, "devia caber em 3/4 do teto");
        assert!(total_bytes(&media) > 0, "não é para zerar no Light");
    }

    // Moderate encolhe na direção de metade do teto.
    #[test]
    fn moderate_reduz_texturas_a_metade() {
        let ctx = egui::Context::default();
        let mut media = store_com_budget(1000);
        for key in ["thumb:a", "thumb:b", "thumb:c"] {
            let t = textura(&ctx, key, 10, 10);
            media.textures.insert(key.into(), t);
        }

        media.trim(TrimLevel::Moderate);

        assert!(total_bytes(&media) <= 500, "devia caber em metade do teto");
    }

    // Critical solta todas as texturas decodificadas reconstruíveis.
    #[test]
    fn critical_solta_todas_as_texturas() {
        let ctx = egui::Context::default();
        let mut media = store_com_budget(1000);
        for key in ["thumb:a", "full:d", "remote-image:e"] {
            let t = textura(&ctx, key, 10, 10);
            media.textures.insert(key.into(), t);
        }

        media.trim(TrimLevel::Critical);

        assert_eq!(total_bytes(&media), 0);
    }

    // Uma textura ainda carregando não pode ser descartada: o pedido continua
    // a caminho e sumir com o marcador faria o quadro seguinte pedir de novo.
    #[test]
    fn textura_em_carregamento_sobrevive_ao_trim() {
        let ctx = egui::Context::default();
        let mut media = store_com_budget(1000);
        let pronta = textura(&ctx, "thumb:a", 10, 10);
        media.textures.insert("thumb:a".into(), pronta);
        media
            .textures
            .insert("thumb:pendente".into(), Texture::Loading);

        media.trim(TrimLevel::Critical);

        assert!(!media.textures.contains_key("thumb:a"));
        assert!(matches!(
            media.textures.get("thumb:pendente"),
            Some(Texture::Loading)
        ));
    }

    #[test]
    fn remote_video_goes_directly_to_backend_as_uri() {
        let backend = Arc::new(FakeBackend::default());
        let mut media = store(Arc::clone(&backend), 4, Duration::from_secs(60));
        let ctx = egui::Context::default();
        let url = "https://example.com/video.mp4?token=1";
        let key = remote_player_key(url).unwrap();

        assert!(media.start_remote_player(&key, url, &ctx).is_some());
        assert_eq!(backend.opens.load(Ordering::SeqCst), 1);
        assert_eq!(
            backend.sources.lock().unwrap().as_slice(),
            &[DirectMediaSource::RemoteUri(url.to_owned())]
        );
    }

    #[test]
    fn equivalent_remote_urls_share_player_identity() {
        assert_eq!(
            remote_player_key("https://EXAMPLE.com:443/video.mp4?q=1"),
            remote_player_key("https://example.com/video.mp4?q=1")
        );
    }

    #[test]
    fn same_canonical_remote_url_has_same_disk_identity() {
        let a = papo_core::preview::canonical_url("https://EXAMPLE.com:443/a?q=1").unwrap();
        let b = papo_core::preview::canonical_url("https://example.com/a?q=1").unwrap();
        assert_eq!(remote_resource_id(&a), remote_resource_id(&b));
    }
}

#[cfg(test)]
mod limpeza {
    use super::*;

    fn raiz(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("papo-teste-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for bucket in ["servers", "remote", "recordings"] {
            std::fs::create_dir_all(dir.join(bucket)).unwrap();
        }
        dir
    }

    fn escreve(path: &Path, bytes: usize, idade: Duration) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
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
    fn attachment_id_is_isolated_by_server_namespace() {
        let raiz = raiz("namespace");
        let a = authenticated_cache_path_at(&raiz, "srv-a", "files", "abc123", "video.mp4");
        let b = authenticated_cache_path_at(&raiz, "srv-b", "files", "abc123", "video.mp4");
        let a_again =
            authenticated_cache_path_at(&raiz, "srv-a", "files", "abc123", "video.mp4");

        assert_ne!(a, b);
        assert_eq!(a, a_again);
        assert!(a.starts_with(raiz.join("servers/srv-a/files")));
        assert!(b.starts_with(raiz.join("servers/srv-b/files")));
    }

    #[test]
    fn server_url_characters_never_enter_authenticated_paths() {
        let key = papo_core::server_key("https://example.com:8443/a/path?x=1");
        assert!(safe_server_key(&key));
        assert!(!key.contains("://"));
        assert!(!key.contains('?'));
        assert!(!key.contains('/'));
        assert!(!key.contains(':'));

        let raiz = raiz("safe-key");
        let path = authenticated_cache_path_at(&raiz, &key, "thumbs", "id", "capa.png");
        assert!(path.starts_with(raiz.join("servers").join(&key).join("thumbs")));
    }

    #[test]
    fn thumbs_files_and_posters_share_the_server_namespace() {
        let raiz = raiz("buckets");
        let key = "srv-a";
        let thumb = authenticated_cache_path_at(&raiz, key, "thumbs", "id", "");
        let full = authenticated_cache_path_at(&raiz, key, "files", "id", "");
        let poster = authenticated_cache_path_at(&raiz, key, "thumbs", "id", "capa.png");

        assert!(thumb.starts_with(raiz.join("servers/srv-a/thumbs")));
        assert!(poster.starts_with(raiz.join("servers/srv-a/thumbs")));
        assert!(full.starts_with(raiz.join("servers/srv-a/files")));
    }

    #[test]
    fn public_remote_cache_stays_global_and_canonical() {
        let raiz = raiz("remote-global");
        let a = papo_core::preview::canonical_url("https://EXAMPLE.com:443/image.png?q=1").unwrap();
        let b = papo_core::preview::canonical_url("https://example.com/image.png?q=1").unwrap();
        let a = public_remote_cache_path_at(&raiz, &remote_resource_id(&a));
        let b = public_remote_cache_path_at(&raiz, &remote_resource_id(&b));

        assert_eq!(a, b);
        assert!(a.starts_with(raiz.join("remote")));
        assert!(!a.starts_with(raiz.join("servers")));
    }

    #[test]
    fn cached_attachment_from_a_cannot_satisfy_b() {
        let raiz = raiz("collision");
        let a = authenticated_cache_path_at(&raiz, "srv-a", "files", "same-id", "");
        let b = authenticated_cache_path_at(&raiz, "srv-b", "files", "same-id", "");
        escreve(&a, 4, HORA);

        assert!(a.exists());
        assert!(!b.exists());
    }

    #[test]
    fn legacy_flat_authenticated_cache_is_discarded_not_adopted() {
        let raiz = raiz("legacy");
        let file = raiz.join("files/same-id");
        let thumb = raiz.join("thumbs/same-id");
        escreve(&file, 8, HORA);
        escreve(&thumb, 8, HORA);

        sweep_cache_with(&raiz, 1 << 30, HORA * 24, HORA * 24);

        assert!(!raiz.join("files").exists());
        assert!(!raiz.join("thumbs").exists());
    }

    #[test]
    fn limpar_servidor_remove_so_o_namespace_dele() {
        let raiz = raiz("clear-server");
        let a = authenticated_cache_path_at(&raiz, "srv-a", "files", "id", "");
        let b = authenticated_cache_path_at(&raiz, "srv-b", "files", "id", "");
        let remoto = public_remote_cache_path_at(&raiz, "public");
        escreve(&a, 10, HORA);
        escreve(&b, 10, HORA);
        escreve(&remoto, 10, HORA);

        assert!(clear_server_media_cache_at(&raiz, "srv-a").unwrap());
        assert!(!a.exists());
        assert!(b.exists());
        assert!(remoto.exists());
        assert!(!clear_server_media_cache_at(&raiz, "srv-a").unwrap());
    }

    #[test]
    fn limpar_servidor_recusa_chave_que_poderia_escapar_da_raiz() {
        let raiz = raiz("unsafe-clear");
        assert!(clear_server_media_cache_at(&raiz, "../fora").is_err());
        assert!(clear_server_media_cache_at(&raiz, "https://example.com").is_err());
    }

    #[test]
    fn resto_de_download_interrompido_sai() {
        let raiz = raiz("parcial");
        let sobra = raiz.join("servers/srv-a/files/video.mp4.parcial");
        let bom = raiz.join("servers/srv-a/files/video.mp4");
        // Velho o bastante para não poder mais estar em curso.
        escreve(&sobra, 10, PARTIAL_MAX_AGE + HORA);
        escreve(&bom, 10, HORA);

        sweep_cache_with(&raiz, 1 << 30, HORA * 24, HORA * 24);

        assert!(!sobra.exists(), "o arquivo parcial devia ter saído");
        assert!(bom.exists(), "o arquivo inteiro devia ter ficado");
    }

    /// Um `.parcial` recém-escrito pode ser um download em andamento: se a
    /// varredura o apagasse, o rename seguinte falhava com ENOENT.
    #[test]
    fn parcial_em_curso_sobrevive_a_varredura() {
        let raiz = raiz("parcial-em-curso");
        let em_curso = raiz.join("servers/srv-a/files/video.mp4.parcial");
        escreve(&em_curso, 10, Duration::from_secs(1));

        sweep_cache_with(&raiz, 1 << 30, HORA * 24, HORA * 24);

        assert!(em_curso.exists(), "não pode apagar download em andamento");
    }

    #[test]
    fn remoto_parcial_nunca_vira_objeto_valido() {
        let raiz = raiz("remote-parcial");
        let parcial = raiz.join("remote/abc.parcial");
        escreve(&parcial, 128, PARTIAL_MAX_AGE + HORA);

        sweep_cache_with(&raiz, 1 << 30, HORA * 24, HORA * 24);

        assert!(!parcial.exists());
    }

    #[test]
    fn remoto_e_servidores_compartilham_um_teto_global() {
        let raiz = raiz("remote-teto");
        let antigo = raiz.join("servers/srv-a/files/antigo");
        let medio = raiz.join("servers/srv-b/thumbs/medio");
        let recente = raiz.join("remote/recente");
        escreve(&antigo, 1000, HORA * 3);
        escreve(&medio, 1000, HORA * 2);
        escreve(&recente, 1000, HORA);

        sweep_cache_with(&raiz, 2000, HORA * 24, HORA * 24);

        assert!(!antigo.exists());
        assert!(medio.exists());
        assert!(recente.exists());
    }

    #[test]
    fn o_que_esta_parado_ha_tempo_demais_sai() {
        let raiz = raiz("idade");
        let velho = raiz.join("servers/srv-a/thumbs/velho");
        let novo = raiz.join("servers/srv-a/thumbs/novo");
        escreve(&velho, 10, HORA * 50);
        escreve(&novo, 10, HORA);

        sweep_cache_with(&raiz, 1 << 30, HORA * 24, HORA * 24);

        assert!(!velho.exists());
        assert!(novo.exists());
    }

    #[test]
    fn passando_do_teto_o_mais_antigo_sai_primeiro() {
        let raiz = raiz("teto");
        let antigo = raiz.join("servers/srv-a/files/antigo");
        let medio = raiz.join("servers/srv-b/files/medio");
        let recente = raiz.join("servers/srv-b/files/recente");
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

    // A pressão de memória é de RAM/GPU. O cache em disco tem política própria
    // (teto/idade) e apagá-lo sob pressão só forçaria mais rede e CPU depois.
    #[test]
    fn trim_de_memoria_nao_apaga_o_cache_de_disco() {
        let raiz = raiz("trim-nao-mexe-no-disco");
        let arquivo = raiz.join("servers/srv-a/files/baixado");
        escreve(&arquivo, 4096, HORA);

        let mut media = MediaStore::new(None);
        media.trim(TrimLevel::Critical);

        assert!(arquivo.exists(), "o trim de memória não toca no disco");
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
