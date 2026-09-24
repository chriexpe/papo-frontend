//! Resolução, deduplicação e cache persistente de previews ricos.
//!
//! O coordenador é UI-neutral: recebe uma URL pública HTTPS e produz metadados
//! que a apresentação pode transformar em cartão, imagem, vídeo direto ou
//! player HTML/iframe. Nenhum host é allowlistado aqui. As capacidades
//! publicadas pela própria resposta (Content-Type, Open Graph, Twitter Cards,
//! oEmbed, <video>/<source> e iframe) decidem o resultado.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{CONTENT_TYPE, LOCATION, RETRY_AFTER};
use tokio::sync::{mpsc, Semaphore};
use url::{Host, Url};

use crate::cache::{now_millis, CachedPreview, ClientDb, PreviewCacheState};

const HTML_MAX: usize = 2 << 20;
const OEMBED_MAX: usize = 512 << 10;
const OEMBED_REGISTRY_MAX: usize = 2 << 20;
const OEMBED_REGISTRY_URL: &str = "https://oembed.com/providers.json";
const MAX_REDIRECTS: usize = 5;
const READY_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const NEGATIVE_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const RETRY_BASE_MS: i64 = 5 * 60 * 1000;
const RETRY_MAX_MS: i64 = 6 * 60 * 60 * 1000;
const RESOLVER_CONCURRENCY: usize = 4;
const QUEUE_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Link,
    Image,
    Video,
    Embed,
}

impl PreviewKind {
    fn as_db(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Image => "image",
            Self::Video => "video",
            Self::Embed => "embed",
        }
    }

    fn from_db(raw: &str) -> Option<Self> {
        match raw {
            "link" => Some(Self::Link),
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "embed" => Some(Self::Embed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPreview {
    /// URL original/canônica da mensagem, nunca um helper de apresentação.
    pub source_url: String,
    pub kind: PreviewKind,
    /// Arquivo/stream diretamente tocável quando a página realmente o expõe.
    pub media_url: Option<String>,
    /// Imagem de capa ou a própria imagem quando o recurso é visual.
    pub image_url: Option<String>,
    /// Player HTML/iframe declarado pelo site. O HTML bruto nunca é persistido.
    pub embed_url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub provider_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewState {
    Loading,
    Ready(ResolvedPreview),
    Negative,
    RetryLater { retry_after: i64 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewStatsSnapshot {
    pub cache_ready: u64,
    pub cache_negative: u64,
    pub queued: u64,
    pub network_resolves: u64,
    pub transient_failures: u64,
    pub stale_served: u64,
}

#[derive(Default)]
struct PreviewStats {
    cache_ready: AtomicU64,
    cache_negative: AtomicU64,
    queued: AtomicU64,
    network_resolves: AtomicU64,
    transient_failures: AtomicU64,
    stale_served: AtomicU64,
}

impl PreviewStats {
    fn snapshot(&self) -> PreviewStatsSnapshot {
        PreviewStatsSnapshot {
            cache_ready: self.cache_ready.load(Ordering::Relaxed),
            cache_negative: self.cache_negative.load(Ordering::Relaxed),
            queued: self.queued.load(Ordering::Relaxed),
            network_resolves: self.network_resolves.load(Ordering::Relaxed),
            transient_failures: self.transient_failures.load(Ordering::Relaxed),
            stale_served: self.stale_served.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone)]
struct MemoryEntry {
    state: PreviewState,
    resolved_at: i64,
    retry_after: Option<i64>,
    in_flight: bool,
}

impl MemoryEntry {
    fn loading() -> Self {
        Self {
            state: PreviewState::Loading,
            resolved_at: 0,
            retry_after: None,
            in_flight: false,
        }
    }

    fn wants_refresh(&self, now: i64) -> bool {
        if self.in_flight {
            return false;
        }
        if self.retry_after.is_some_and(|deadline| deadline > now) {
            return false;
        }
        match self.state {
            PreviewState::Loading => true,
            PreviewState::Ready(_) | PreviewState::Negative => {
                now.saturating_sub(self.resolved_at) >= READY_TTL_MS
            }
            PreviewState::RetryLater { retry_after } => retry_after <= now,
        }
    }
}

struct Inner {
    db: Arc<ClientDb>,
    entries: Mutex<HashMap<String, MemoryEntry>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    stats: PreviewStats,
}

pub struct PreviewCoordinator {
    inner: Arc<Inner>,
    queue: mpsc::Sender<String>,
}

impl PreviewCoordinator {
    pub fn new(db: Arc<ClientDb>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let inner = Arc::new(Inner {
            db,
            entries: Mutex::new(HashMap::new()),
            wake,
            stats: PreviewStats::default(),
        });
        let (queue_tx, queue_rx) = mpsc::channel(QUEUE_CAPACITY);
        let worker_inner = Arc::clone(&inner);

        if let Err(error) = std::thread::Builder::new()
            .name("papo-preview".to_owned())
            .spawn(move || preview_worker(worker_inner, queue_rx))
        {
            log::warn!("preview: worker não abriu: {error}");
        }

        Self {
            inner,
            queue: queue_tx,
        }
    }

    /// Retorna o estado já conhecido e, quando necessário, agenda exatamente
    /// uma resolução. Não bloqueia a thread da UI em SQL nem rede.
    pub fn get_or_request(&self, raw_url: &str) -> Option<PreviewState> {
        let key = canonical_url(raw_url)?;
        let now = now_millis();
        let mut entries = self.inner.entries.lock().ok()?;
        let entry = entries.entry(key.clone()).or_insert_with(MemoryEntry::loading);

        if entry.wants_refresh(now) {
            match self.queue.try_send(key) {
                Ok(()) => {
                    entry.in_flight = true;
                    self.inner.stats.queued.fetch_add(1, Ordering::Relaxed);
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // A fila é propositalmente limitada. Um próximo paint
                    // pode tentar de novo; não existe storm de tasks.
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
        }

        Some(entry.state.clone())
    }

    pub fn peek(&self, raw_url: &str) -> Option<PreviewState> {
        let key = canonical_url(raw_url)?;
        self.inner
            .entries
            .lock()
            .ok()?
            .get(&key)
            .map(|entry| entry.state.clone())
    }

    pub fn stats(&self) -> PreviewStatsSnapshot {
        self.inner.stats.snapshot()
    }
}

fn preview_worker(inner: Arc<Inner>, mut queue: mpsc::Receiver<String>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::warn!("preview: runtime não abriu: {error}");
            return;
        }
    };

    runtime.block_on(async move {
        let client = match reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(12))
            .user_agent("Papo/0.2 rich-preview")
            .build()
        {
            Ok(client) => Arc::new(client),
            Err(error) => {
                log::warn!("preview: cliente HTTP não abriu: {error}");
                return;
            }
        };
        let permits = Arc::new(Semaphore::new(RESOLVER_CONCURRENCY));
        let oembed_registry = Arc::new(tokio::sync::OnceCell::new());

        while let Some(key) = queue.recv().await {
            let Ok(permit) = Arc::clone(&permits).acquire_owned().await else {
                break;
            };
            let task_inner = Arc::clone(&inner);
            let task_client = Arc::clone(&client);
            let task_registry = Arc::clone(&oembed_registry);
            tokio::spawn(async move {
                process_request(task_inner, task_client, task_registry, key).await;
                drop(permit);
            });
        }
    });
}

async fn process_request(
    inner: Arc<Inner>,
    client: Arc<reqwest::Client>,
    oembed_registry: Arc<tokio::sync::OnceCell<Vec<OEmbedRegistryEndpoint>>>,
    key: String,
) {
    let cached = {
        let db = Arc::clone(&inner.db);
        let lookup_key = key.clone();
        tokio::task::spawn_blocking(move || db.load_preview(&lookup_key))
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
    };

    let now = now_millis();
    let mut stale_ready = None;

    if let Some(row) = cached {
        match row_to_state(&row) {
            Some(PreviewState::Ready(preview)) => {
                let fresh = now.saturating_sub(row.resolved_at) < READY_TTL_MS;
                if fresh {
                    inner.stats.cache_ready.fetch_add(1, Ordering::Relaxed);
                    publish(
                        &inner,
                        &key,
                        PreviewState::Ready(preview),
                        row.resolved_at,
                        row.retry_after,
                        false,
                    );
                    return;
                }
                if row.retry_after.is_some_and(|deadline| deadline > now) {
                    inner.stats.stale_served.fetch_add(1, Ordering::Relaxed);
                    publish(
                        &inner,
                        &key,
                        PreviewState::Ready(preview),
                        row.resolved_at,
                        row.retry_after,
                        false,
                    );
                    return;
                }
                stale_ready = Some((preview.clone(), row.clone()));
                inner.stats.stale_served.fetch_add(1, Ordering::Relaxed);
                publish(
                    &inner,
                    &key,
                    PreviewState::Ready(preview),
                    row.resolved_at,
                    row.retry_after,
                    true,
                );
            }
            Some(PreviewState::Negative)
                if now.saturating_sub(row.resolved_at) < NEGATIVE_TTL_MS =>
            {
                inner.stats.cache_negative.fetch_add(1, Ordering::Relaxed);
                publish(
                    &inner,
                    &key,
                    PreviewState::Negative,
                    row.resolved_at,
                    row.retry_after,
                    false,
                );
                return;
            }
            Some(PreviewState::RetryLater { retry_after }) if retry_after > now => {
                publish(
                    &inner,
                    &key,
                    PreviewState::RetryLater { retry_after },
                    row.resolved_at,
                    Some(retry_after),
                    false,
                );
                return;
            }
            _ => {}
        }
    }

    inner
        .stats
        .network_resolves
        .fetch_add(1, Ordering::Relaxed);

    match resolve_url(&client, &oembed_registry, &key, 0).await {
        Ok(preview) => {
            let row = preview_row(&key, &preview, now);
            persist(&inner, row).await;
            publish(
                &inner,
                &key,
                PreviewState::Ready(preview),
                now,
                None,
                false,
            );
        }
        Err(error) if error.class == FailureClass::Transient => {
            inner
                .stats
                .transient_failures
                .fetch_add(1, Ordering::Relaxed);
            let retry_after = error.retry_after.unwrap_or_else(|| now + RETRY_BASE_MS);
            if let Some((preview, mut old_row)) = stale_ready {
                old_row.retry_after = Some(retry_after);
                old_row.failure_class = Some("transient".to_owned());
                old_row.last_used_at = now;
                persist(&inner, old_row.clone()).await;
                publish(
                    &inner,
                    &key,
                    PreviewState::Ready(preview),
                    old_row.resolved_at,
                    Some(retry_after),
                    false,
                );
            } else {
                let row = CachedPreview {
                    url_key: key.clone(),
                    source_url: key.clone(),
                    state: PreviewCacheState::RetryAfter,
                    kind: None,
                    media_url: None,
                    image_url: None,
                    embed_url: None,
                    title: None,
                    description: None,
                    provider_name: None,
                    resolved_at: now,
                    retry_after: Some(retry_after),
                    failure_class: Some("transient".to_owned()),
                    last_used_at: now,
                };
                persist(&inner, row).await;
                publish(
                    &inner,
                    &key,
                    PreviewState::RetryLater { retry_after },
                    now,
                    Some(retry_after),
                    false,
                );
            }
        }
        Err(error) => {
            if let Some((preview, mut old_row)) = stale_ready {
                // Uma falha ao refrescar nunca apaga um preview útil. Para
                // rejeição de segurança, espera-se o TTL negativo antes de
                // reconsiderar metadados novos.
                let retry_after = now + NEGATIVE_TTL_MS;
                old_row.retry_after = Some(retry_after);
                old_row.failure_class = Some(error.class.as_str().to_owned());
                old_row.last_used_at = now;
                persist(&inner, old_row.clone()).await;
                publish(
                    &inner,
                    &key,
                    PreviewState::Ready(preview),
                    old_row.resolved_at,
                    Some(retry_after),
                    false,
                );
            } else {
                let row = CachedPreview {
                    url_key: key.clone(),
                    source_url: key.clone(),
                    state: PreviewCacheState::Negative,
                    kind: None,
                    media_url: None,
                    image_url: None,
                    embed_url: None,
                    title: None,
                    description: None,
                    provider_name: None,
                    resolved_at: now,
                    retry_after: None,
                    failure_class: Some(error.class.as_str().to_owned()),
                    last_used_at: now,
                };
                persist(&inner, row).await;
                publish(
                    &inner,
                    &key,
                    PreviewState::Negative,
                    now,
                    None,
                    false,
                );
            }
            log::debug!(
                "preview {}: {}",
                safe_key(&key),
                error.message
            );
        }
    }
}

async fn persist(inner: &Arc<Inner>, row: CachedPreview) {
    let db = Arc::clone(&inner.db);
    let _ = tokio::task::spawn_blocking(move || db.store_preview(row)).await;
}

fn publish(
    inner: &Arc<Inner>,
    key: &str,
    state: PreviewState,
    resolved_at: i64,
    retry_after: Option<i64>,
    in_flight: bool,
) {
    if let Ok(mut entries) = inner.entries.lock() {
        entries.insert(
            key.to_owned(),
            MemoryEntry {
                state,
                resolved_at,
                retry_after,
                in_flight,
            },
        );
    }
    (inner.wake)();
}

fn row_to_state(row: &CachedPreview) -> Option<PreviewState> {
    match row.state {
        PreviewCacheState::Ready => {
            let kind = PreviewKind::from_db(row.kind.as_deref()?)?;
            Some(PreviewState::Ready(ResolvedPreview {
                source_url: row.source_url.clone(),
                kind,
                media_url: row.media_url.clone(),
                image_url: row.image_url.clone(),
                embed_url: row.embed_url.clone(),
                title: row.title.clone(),
                description: row.description.clone(),
                provider_name: row.provider_name.clone(),
            }))
        }
        PreviewCacheState::Negative => Some(PreviewState::Negative),
        PreviewCacheState::RetryAfter => Some(PreviewState::RetryLater {
            retry_after: row.retry_after?,
        }),
    }
}

fn preview_row(key: &str, preview: &ResolvedPreview, now: i64) -> CachedPreview {
    CachedPreview {
        url_key: key.to_owned(),
        source_url: preview.source_url.clone(),
        state: PreviewCacheState::Ready,
        kind: Some(preview.kind.as_db().to_owned()),
        media_url: preview.media_url.clone(),
        image_url: preview.image_url.clone(),
        embed_url: preview.embed_url.clone(),
        title: preview.title.clone(),
        description: preview.description.clone(),
        provider_name: preview.provider_name.clone(),
        resolved_at: now,
        retry_after: None,
        failure_class: None,
        last_used_at: now,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureClass {
    Negative,
    Security,
    Transient,
}

impl FailureClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Negative => "negative",
            Self::Security => "security",
            Self::Transient => "transient",
        }
    }
}

#[derive(Debug)]
struct ResolveError {
    class: FailureClass,
    message: String,
    retry_after: Option<i64>,
}

impl ResolveError {
    fn negative(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Negative,
            message: message.into(),
            retry_after: None,
        }
    }

    fn security(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Security,
            message: message.into(),
            retry_after: None,
        }
    }

    fn transient(message: impl Into<String>) -> Self {
        Self {
            class: FailureClass::Transient,
            message: message.into(),
            retry_after: None,
        }
    }
}

async fn resolve_url(
    client: &reqwest::Client,
    oembed_registry: &tokio::sync::OnceCell<Vec<OEmbedRegistryEndpoint>>,
    source_url: &str,
    depth: usize,
) -> Result<ResolvedPreview, ResolveError> {
    if depth > 2 {
        return Err(ResolveError::negative("profundidade de embed excedida"));
    }
    let source = Url::parse(source_url)
        .map_err(|error| ResolveError::negative(format!("URL inválida: {error}")))?;
    let response = get_following_safe_redirects(client, source.clone()).await?;
    let final_url = response.url().clone();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let header_oembed = oembed_header_endpoint(response.headers(), &final_url);

    if is_video_content_type(&content_type) {
        return Ok(ResolvedPreview {
            source_url: source_url.to_owned(),
            kind: PreviewKind::Video,
            media_url: Some(final_url.to_string()),
            image_url: None,
            embed_url: None,
            title: file_name_title(&final_url),
            description: None,
            provider_name: final_url.host_str().map(str::to_owned),
        });
    }
    if content_type.starts_with("image/") {
        let media = final_url.to_string();
        return Ok(ResolvedPreview {
            source_url: source_url.to_owned(),
            kind: PreviewKind::Image,
            media_url: Some(media.clone()),
            image_url: Some(media),
            embed_url: None,
            title: file_name_title(&final_url),
            description: None,
            provider_name: final_url.host_str().map(str::to_owned),
        });
    }

    if !content_type.is_empty()
        && !content_type.starts_with("text/html")
        && !content_type.starts_with("application/xhtml")
    {
        return Err(ResolveError::negative(format!(
            "conteúdo não é HTML nem mídia visual: {content_type}"
        )));
    }

    let html = read_limited(response, HTML_MAX, "página").await?;
    let mut preview = parse_html_preview(source_url, &final_url, &html).await?;

    // Preferimos discovery publicado pela própria página. Quando ela não
    // publica, usamos a registry oficial do oEmbed como fallback de dados,
    // em vez de codificar YouTube/TikTok/etc. no cliente.
    let declared_oembed = oembed_endpoint(&html, &final_url).or(header_oembed);
    let oembed = if let Some(endpoint) = declared_oembed {
        resolve_oembed(client, oembed_registry, source_url, endpoint, depth)
            .await
            .ok()
    } else {
        // A registry oficial é a tabela de capacidades, não uma allowlist
        // codificada pelo Papo. Consultá-la mesmo quando já existe OG permite
        // promover páginas como Instagram de "card rico" para "embed rico".
        match registry_oembed_endpoint(client, oembed_registry, source_url).await {
            Some(endpoint) => resolve_oembed(
                client,
                oembed_registry,
                source_url,
                endpoint,
                depth,
            )
            .await
            .ok(),
            None => None,
        }
    };
    if let Some(oembed) = oembed {
        merge_preview(&mut preview, oembed);
    }

    // Um player HTML/iframe pode, por sua vez, publicar um stream direto.
    // Tentamos uma profundidade curta; se continuar sendo HTML, preservamos
    // o embed para uma apresentação WebView/sandbox sem inventar um MP4.
    if preview.media_url.is_none()
        && let Some(embed) = preview.embed_url.clone()
        && canonical_url(&embed).as_deref() != canonical_url(source_url).as_deref()
        && depth < 2
        && let Ok(nested) = Box::pin(resolve_url(client, oembed_registry, &embed, depth + 1)).await
    {
        if nested.media_url.is_some() {
            preview.media_url = nested.media_url;
            preview.kind = nested.kind;
        }
        if preview.image_url.is_none() {
            preview.image_url = nested.image_url;
        }
    }

    sanitize_preview_targets(&mut preview).await;

    if preview.kind == PreviewKind::Link
        && preview.title.is_none()
        && preview.description.is_none()
        && preview.image_url.is_none()
    {
        return Err(ResolveError::negative("página sem metadados ricos"));
    }

    Ok(preview)
}

async fn sanitize_preview_targets(preview: &mut ResolvedPreview) {
    for target in [
        &mut preview.media_url,
        &mut preview.image_url,
        &mut preview.embed_url,
    ] {
        let Some(raw) = target.as_deref() else {
            continue;
        };
        let valid = match Url::parse(raw) {
            Ok(url) => validate_destination(&url).await.is_ok(),
            Err(_) => false,
        };
        if !valid {
            *target = None;
        }
    }

    preview.kind = if preview.media_url.is_some() {
        PreviewKind::Video
    } else if preview.embed_url.is_some() {
        PreviewKind::Embed
    } else if preview.image_url.is_some()
        && preview.title.is_none()
        && preview.description.is_none()
    {
        PreviewKind::Image
    } else {
        PreviewKind::Link
    };
}

async fn resolve_oembed(
    client: &reqwest::Client,
    oembed_registry: &tokio::sync::OnceCell<Vec<OEmbedRegistryEndpoint>>,
    source_url: &str,
    endpoint: Url,
    depth: usize,
) -> Result<ResolvedPreview, ResolveError> {
    let response = get_following_safe_redirects(client, endpoint).await?;
    let bytes = read_limited_bytes(response, OEMBED_MAX, "oEmbed").await?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| ResolveError::negative(format!("oEmbed inválido: {error}")))?;

    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("link");
    let mut preview = ResolvedPreview {
        source_url: source_url.to_owned(),
        kind: PreviewKind::Link,
        media_url: None,
        image_url: json_string(&value, "thumbnail_url"),
        embed_url: None,
        title: json_string(&value, "title"),
        description: None,
        provider_name: json_string(&value, "provider_name"),
    };

    match kind {
        "photo" => {
            if let Some(url) = json_string(&value, "url")
                .and_then(|raw| resolve_candidate_url(source_url, &raw))
                .filter(|url| safe_remote_url(url))
            {
                preview.kind = PreviewKind::Image;
                preview.media_url = Some(url.clone());
                preview.image_url = Some(url);
            }
        }
        "video" | "rich" => {
            if let Some(html) = value.get("html").and_then(|v| v.as_str()) {
                if let Some(media) = html_media_url(html, source_url) {
                    preview.kind = PreviewKind::Video;
                    preview.media_url = Some(media);
                } else if let Some(embed) = html_embed_url(html, source_url) {
                    preview.kind = PreviewKind::Embed;
                    preview.embed_url = Some(embed);
                } else if !html.trim().is_empty() {
                    // Alguns oEmbed (ex.: blockquote + script) não oferecem
                    // iframe URL. Registramos a página original como alvo do
                    // futuro web player sem persistir HTML executável.
                    preview.kind = PreviewKind::Embed;
                    preview.embed_url = canonical_url(source_url);
                }
            }
        }
        _ => {}
    }

    if preview.media_url.is_none()
        && let Some(embed) = preview.embed_url.clone()
        && depth < 2
        && let Ok(nested) = Box::pin(resolve_url(client, oembed_registry, &embed, depth + 1)).await
        && nested.media_url.is_some()
    {
        preview.kind = nested.kind;
        preview.media_url = nested.media_url;
        if preview.image_url.is_none() {
            preview.image_url = nested.image_url;
        }
    }

    Ok(preview)
}

async fn parse_html_preview(
    source_url: &str,
    base: &Url,
    html: &str,
) -> Result<ResolvedPreview, ResolveError> {
    let title = meta_content(html, &["og:title", "twitter:title"])
        .or_else(|| html_title(html))
        .map(|value| decode_html_text(&value));
    let description = meta_content(html, &["og:description", "twitter:description"])
        .map(|value| decode_html_text(&value));
    let provider_name = meta_content(html, &["og:site_name", "application-name"])
        .map(|value| decode_html_text(&value))
        .or_else(|| base.host_str().map(str::to_owned));

    let image_url = meta_content(
        html,
        &[
            "og:image:secure_url",
            "og:image:url",
            "og:image",
            "twitter:image",
            "twitter:image:src",
        ],
    )
    .and_then(|raw| resolve_meta_url(base, &raw))
    .filter(|url| safe_remote_url(url));

    let video_type = meta_content(
        html,
        &["og:video:type", "twitter:player:stream:content_type"],
    )
    .unwrap_or_default()
    .to_ascii_lowercase();

    let direct_video = meta_content(
        html,
        &[
            "og:video:secure_url",
            "og:video:url",
            "og:video",
            "twitter:player:stream",
        ],
    )
    .and_then(|raw| resolve_meta_url(base, &raw))
    .filter(|url| safe_remote_url(url))
    .filter(|_| !video_type.starts_with("text/html"))
    .or_else(|| html_media_url(html, base.as_str()));

    let embed_url = meta_content(html, &["twitter:player"])
        .and_then(|raw| resolve_meta_url(base, &raw))
        .filter(|url| safe_remote_url(url))
        .or_else(|| {
            if video_type.starts_with("text/html") {
                meta_content(html, &["og:video:secure_url", "og:video:url", "og:video"])
                    .and_then(|raw| resolve_meta_url(base, &raw))
                    .filter(|url| safe_remote_url(url))
            } else {
                None
            }
        })
        .or_else(|| html_embed_url(html, base.as_str()));

    let kind = if direct_video.is_some() {
        PreviewKind::Video
    } else if embed_url.is_some() {
        PreviewKind::Embed
    } else if image_url.is_some() && title.is_none() && description.is_none() {
        PreviewKind::Image
    } else {
        PreviewKind::Link
    };

    let mut preview = ResolvedPreview {
        source_url: source_url.to_owned(),
        kind,
        media_url: direct_video,
        image_url,
        embed_url,
        title,
        description,
        provider_name,
    };

    // JSON-LD/Schema.org is another provider-neutral capability signal used
    // by many video sites. It often exposes VideoObject.embedUrl/contentUrl
    // even when no Open Graph player URL is present.
    if let Some(structured) = json_ld_preview(html, source_url, base) {
        merge_preview(&mut preview, structured);
    }

    Ok(preview)
}

fn page_wants_player(html: &str) -> bool {
    meta_content(html, &["og:type"]).is_some_and(|kind| {
        kind.to_ascii_lowercase().starts_with("video")
    }) || meta_content(html, &["twitter:card"]).is_some_and(|card| {
        card.eq_ignore_ascii_case("player")
    }) || html.to_ascii_lowercase().contains("\"@type\":\"videoobject\"")
        || html.to_ascii_lowercase().contains("\"@type\": \"videoobject\"")
}

fn is_video_content_type(content_type: &str) -> bool {
    content_type.starts_with("video/")
        || matches!(
            content_type,
            "application/vnd.apple.mpegurl"
                | "application/x-mpegurl"
                | "application/mpegurl"
                | "application/dash+xml"
        )
}

fn json_ld_preview(html: &str, source_url: &str, base: &Url) -> Option<ResolvedPreview> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0usize;

    while let Some(rel) = lower[from..].find("<script") {
        let start = from + rel;
        let tag_end = lower[start..].find('>')? + start;
        let tag = &html[start + 1..tag_end];
        let mime = html_attr(tag, "type").unwrap_or_default();
        from = tag_end + 1;
        if !mime.eq_ignore_ascii_case("application/ld+json") {
            continue;
        }
        let Some(close_rel) = lower[from..].find("</script>") else {
            break;
        };
        let close = from + close_rel;
        let raw = html[from..close].trim();
        from = close + "</script>".len();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            continue;
        };
        if let Some(preview) = json_ld_value_preview(&value, source_url, base) {
            return Some(preview);
        }
    }
    None
}

fn json_ld_value_preview(
    value: &serde_json::Value,
    source_url: &str,
    base: &Url,
) -> Option<ResolvedPreview> {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                if let Some(preview) = json_ld_value_preview(value, source_url, base) {
                    return Some(preview);
                }
            }
            None
        }
        serde_json::Value::Object(map) => {
            if let Some(graph) = map.get("@graph")
                && let Some(preview) = json_ld_value_preview(graph, source_url, base)
            {
                return Some(preview);
            }

            let type_is = |wanted: &str| {
                map.get("@type").is_some_and(|kind| match kind {
                    serde_json::Value::String(kind) => kind.eq_ignore_ascii_case(wanted),
                    serde_json::Value::Array(kinds) => kinds.iter().any(|kind| {
                        kind.as_str().is_some_and(|kind| kind.eq_ignore_ascii_case(wanted))
                    }),
                    _ => false,
                })
            };

            if !type_is("VideoObject") && !type_is("ImageObject") && !type_is("Article") {
                for nested in map.values() {
                    if let Some(preview) = json_ld_value_preview(nested, source_url, base) {
                        return Some(preview);
                    }
                }
                return None;
            }

            let media_url = json_ld_url(map.get("contentUrl"), base)
                .filter(|url| safe_remote_url(url));
            let embed_url = json_ld_url(map.get("embedUrl"), base)
                .filter(|url| safe_remote_url(url));
            let image_url = json_ld_url(map.get("thumbnailUrl"), base)
                .or_else(|| json_ld_url(map.get("image"), base))
                .filter(|url| safe_remote_url(url));
            let title = map
                .get("name")
                .or_else(|| map.get("headline"))
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            let description = map
                .get("description")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            let provider_name = map
                .get("publisher")
                .and_then(|publisher| publisher.get("name"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
                .or_else(|| base.host_str().map(str::to_owned));

            let kind = if type_is("VideoObject") && media_url.is_some() {
                PreviewKind::Video
            } else if type_is("VideoObject") && embed_url.is_some() {
                PreviewKind::Embed
            } else if type_is("ImageObject") && image_url.is_some() {
                PreviewKind::Image
            } else {
                PreviewKind::Link
            };

            Some(ResolvedPreview {
                source_url: source_url.to_owned(),
                kind,
                media_url,
                image_url,
                embed_url,
                title,
                description,
                provider_name,
            })
        }
        _ => None,
    }
}

fn json_ld_url(value: Option<&serde_json::Value>, base: &Url) -> Option<String> {
    let raw = match value? {
        serde_json::Value::String(raw) => Some(raw.as_str()),
        serde_json::Value::Array(values) => values.iter().find_map(|value| value.as_str()),
        serde_json::Value::Object(map) => map
            .get("url")
            .or_else(|| map.get("contentUrl"))
            .and_then(|value| value.as_str()),
        _ => None,
    }?;
    resolve_meta_url(base, raw)
}

fn merge_preview(base: &mut ResolvedPreview, extra: ResolvedPreview) {
    if base.media_url.is_none() {
        base.media_url = extra.media_url;
    }
    if base.image_url.is_none() {
        base.image_url = extra.image_url;
    }
    if base.embed_url.is_none() {
        base.embed_url = extra.embed_url;
    }
    if base.title.is_none() {
        base.title = extra.title;
    }
    if base.description.is_none() {
        base.description = extra.description;
    }
    if base.provider_name.is_none() {
        base.provider_name = extra.provider_name;
    }
    if base.media_url.is_some() {
        base.kind = PreviewKind::Video;
    } else if base.embed_url.is_some() {
        base.kind = PreviewKind::Embed;
    } else if base.kind == PreviewKind::Link && extra.kind == PreviewKind::Image {
        base.kind = PreviewKind::Image;
    }
}

async fn get_following_safe_redirects(
    client: &reqwest::Client,
    mut url: Url,
) -> Result<reqwest::Response, ResolveError> {
    for hop in 0..=MAX_REDIRECTS {
        validate_destination(&url).await?;
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(classify_reqwest)?;

        if !response.status().is_redirection() {
            if response.status().as_u16() == 429 {
                let mut error = ResolveError::transient("HTTP 429");
                error.retry_after = retry_after(response.headers().get(RETRY_AFTER));
                return Err(error);
            }
            if response.status().is_server_error() {
                return Err(ResolveError::transient(format!(
                    "HTTP {}",
                    response.status()
                )));
            }
            if !response.status().is_success() {
                return Err(ResolveError::negative(format!(
                    "HTTP {}",
                    response.status()
                )));
            }
            return Ok(response);
        }

        if hop == MAX_REDIRECTS {
            return Err(ResolveError::negative("redirecionamentos demais"));
        }
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| ResolveError::negative("redirect sem Location"))?;
        url = url
            .join(location)
            .map_err(|error| ResolveError::negative(format!("redirect inválido: {error}")))?;
    }
    unreachable!()
}

async fn validate_destination(url: &Url) -> Result<(), ResolveError> {
    if !safe_remote_url(url.as_str()) {
        return Err(ResolveError::security("destino remoto recusado"));
    }
    let port = url.port_or_known_default().unwrap_or(443);
    match url.host() {
        Some(Host::Ipv4(ip)) if !public_v4(ip) => {
            Err(ResolveError::security("IPv4 privado/local recusado"))
        }
        Some(Host::Ipv6(ip)) if !public_v6(ip) => {
            Err(ResolveError::security("IPv6 privado/local recusado"))
        }
        Some(Host::Domain(host)) => {
            let resolved = tokio::net::lookup_host((host, port))
                .await
                .map_err(|error| ResolveError::transient(format!("DNS: {error}")))?;
            let mut any = false;
            for addr in resolved {
                any = true;
                match addr.ip() {
                    IpAddr::V4(ip) if !public_v4(ip) => {
                        return Err(ResolveError::security(
                            "DNS resolveu para IPv4 privado/local",
                        ));
                    }
                    IpAddr::V6(ip) if !public_v6(ip) => {
                        return Err(ResolveError::security(
                            "DNS resolveu para IPv6 privado/local",
                        ));
                    }
                    _ => {}
                }
            }
            if any {
                Ok(())
            } else {
                Err(ResolveError::transient("DNS sem endereços"))
            }
        }
        Some(_) => Ok(()),
        None => Err(ResolveError::security("URL sem host")),
    }
}

/// Validação lexical barata usada também pela camada de mídia. A resolução
/// DNS completa acontece antes de cada fetch do coordenador.
pub fn safe_remote_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if url.scheme() != "https" || url.username() != "" || url.password().is_some() {
        return false;
    }
    match url.host() {
        Some(Host::Domain(host)) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            host != "localhost" && !host.ends_with(".localhost") && !host.ends_with(".local")
        }
        // Mantemos a política histórica mais estrita: mesmo um IP literal
        // público não é origem válida de preview. Hostnames passam por DNS
        // validation imediatamente antes de cada fetch.
        Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) => false,
        None => false,
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, d] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224
        || (a == 255 && b == 255 && c == 255 && d == 255))
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return false;
    }
    let first = ip.segments()[0];
    // fc00::/7 unique-local e fe80::/10 link-local.
    if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 {
        return false;
    }
    // IPv4-mapped IPv6 inherits the IPv4 policy.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    true
}

/// Extrai candidatos HTTPS do texto sem assumir que o link ocupa o token
/// inteiro (markdown e pontuação ao redor são comuns em mensagens).
pub fn extract_https_urls(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0usize;
    let mut urls = Vec::new();

    while let Some(relative) = lower[from..].find("https://") {
        let start = from + relative;
        let tail = &text[start..];
        let end = tail
            .char_indices()
            .find_map(|(index, ch)| {
                (ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\''))
                    .then_some(index)
            })
            .unwrap_or(tail.len());
        let raw = tail[..end].trim_end_matches(|ch: char| {
            matches!(ch, ')' | ']' | '}' | ',' | ';' | '!' | '?' | '.')
        });

        if let Some(url) = canonical_url(raw) {
            urls.push(url);
        }

        from = start.saturating_add("https://".len());
        if from >= text.len() {
            break;
        }
    }

    urls
}

pub fn canonical_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if !safe_remote_url(url.as_str()) {
        return None;
    }
    if url.port() == Some(443) {
        let _ = url.set_port(None);
    }
    Some(url.to_string())
}

/// Download limitado para a apresentação de imagens remotas. Redirecionamentos
/// recebem a mesma validação SSRF do resolver.
pub async fn fetch_bounded_remote_bytes(
    client: &reqwest::Client,
    raw_url: &str,
    max: usize,
) -> Result<Vec<u8>, String> {
    let url = Url::parse(raw_url).map_err(|error| error.to_string())?;
    let response = get_following_safe_redirects(client, url)
        .await
        .map_err(|error| error.message)?;
    read_limited_bytes(response, max, "mídia remota")
        .await
        .map_err(|error| error.message)
}

async fn read_limited(
    response: reqwest::Response,
    max: usize,
    label: &str,
) -> Result<String, ResolveError> {
    let bytes = read_limited_bytes(response, max, label).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn read_limited_bytes(
    response: reqwest::Response,
    max: usize,
    label: &str,
) -> Result<Vec<u8>, ResolveError> {
    if response.content_length().is_some_and(|len| len > max as u64) {
        return Err(ResolveError::negative(format!("{label} grande demais")));
    }
    let mut stream = response.bytes_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(classify_reqwest)?;
        if out.len().saturating_add(chunk.len()) > max {
            return Err(ResolveError::negative(format!("{label} grande demais")));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn classify_reqwest(error: reqwest::Error) -> ResolveError {
    if error.is_timeout() || error.is_connect() || error.is_request() {
        ResolveError::transient(error.to_string())
    } else {
        ResolveError::negative(error.to_string())
    }
}

fn retry_after(value: Option<&reqwest::header::HeaderValue>) -> Option<i64> {
    let seconds = value?.to_str().ok()?.parse::<i64>().ok()?;
    Some(now_millis() + (seconds * 1000).clamp(RETRY_BASE_MS, RETRY_MAX_MS))
}

#[derive(Debug, serde::Deserialize)]
struct OEmbedRegistryProvider {
    provider_name: String,
    #[serde(default)]
    endpoints: Vec<OEmbedRegistryRawEndpoint>,
}

#[derive(Debug, serde::Deserialize)]
struct OEmbedRegistryRawEndpoint {
    url: String,
    #[serde(default)]
    schemes: Vec<String>,
    #[serde(default)]
    formats: Vec<String>,
}

#[derive(Clone, Debug)]
struct OEmbedRegistryEndpoint {
    provider_name: String,
    url: String,
    schemes: Vec<String>,
}

async fn registry_oembed_endpoint(
    client: &reqwest::Client,
    registry: &tokio::sync::OnceCell<Vec<OEmbedRegistryEndpoint>>,
    source_url: &str,
) -> Option<Url> {
    let entries = registry
        .get_or_try_init(|| async { load_oembed_registry(client).await })
        .await
        .ok()?;
    let source = canonical_url(source_url)?;

    for entry in entries {
        if !entry
            .schemes
            .iter()
            .any(|scheme| wildcard_url_match(scheme, &source))
        {
            continue;
        }

        let raw_endpoint = entry.url.replace("{format}", "json");
        let Ok(mut endpoint) = Url::parse(&raw_endpoint) else {
            continue;
        };
        if !safe_remote_url(endpoint.as_str()) {
            continue;
        }
        let has_format = endpoint
            .query_pairs()
            .any(|(key, _)| key.eq_ignore_ascii_case("format"))
            || raw_endpoint.contains(".json");
        {
            let mut query = endpoint.query_pairs_mut();
            query.append_pair("url", &source);
            if !has_format {
                query.append_pair("format", "json");
            }
        }
        log::debug!("preview oembed registry: provider={}", entry.provider_name);
        return Some(endpoint);
    }
    None
}

async fn load_oembed_registry(
    client: &reqwest::Client,
) -> Result<Vec<OEmbedRegistryEndpoint>, ResolveError> {
    let url = Url::parse(OEMBED_REGISTRY_URL)
        .map_err(|error| ResolveError::negative(error.to_string()))?;
    let response = get_following_safe_redirects(client, url).await?;
    let bytes = read_limited_bytes(response, OEMBED_REGISTRY_MAX, "oEmbed registry").await?;
    let providers: Vec<OEmbedRegistryProvider> = serde_json::from_slice(&bytes)
        .map_err(|error| ResolveError::negative(format!("oEmbed registry inválida: {error}")))?;

    let mut entries = Vec::new();
    for provider in providers {
        for endpoint in provider.endpoints {
            if endpoint.schemes.is_empty()
                || (!endpoint.formats.is_empty()
                    && !endpoint
                        .formats
                        .iter()
                        .any(|format| format.eq_ignore_ascii_case("json")))
            {
                continue;
            }
            let endpoint_url = endpoint.url.replace("{format}", "json");
            if !safe_remote_url(&endpoint_url) {
                continue;
            }
            entries.push(OEmbedRegistryEndpoint {
                provider_name: provider.provider_name.clone(),
                url: endpoint.url,
                schemes: endpoint.schemes,
            });
        }
    }
    Ok(entries)
}

fn wildcard_url_match(pattern: &str, candidate: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let candidate = candidate.to_ascii_lowercase();
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return candidate == pattern;
    }

    let mut cursor = 0usize;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 && !pattern.starts_with('*') {
            if !candidate[cursor..].starts_with(part) {
                return false;
            }
            cursor += part.len();
            continue;
        }
        let Some(found) = candidate[cursor..].find(part) else {
            return false;
        };
        cursor += found + part.len();
    }

    pattern.ends_with('*')
        || parts
            .last()
            .is_none_or(|last| candidate.ends_with(last))
}

fn oembed_header_endpoint(headers: &reqwest::header::HeaderMap, base: &Url) -> Option<Url> {
    for value in headers.get_all(reqwest::header::LINK) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for item in value.split(',') {
            let lower = item.to_ascii_lowercase();
            if !lower.contains("rel=\"alternate\"")
                || !lower.contains("application/json+oembed")
            {
                continue;
            }
            let Some(start) = item.find('<') else {
                continue;
            };
            let Some(end_rel) = item[start + 1..].find('>') else {
                continue;
            };
            let href = &item[start + 1..start + 1 + end_rel];
            let Ok(url) = base.join(href) else {
                continue;
            };
            if safe_remote_url(url.as_str()) {
                return Some(url);
            }
        }
    }
    None
}

fn oembed_endpoint(html: &str, base: &Url) -> Option<Url> {
    for raw in html.split('<').skip(1) {
        let tag = raw.split_once('>').map(|(tag, _)| tag).unwrap_or(raw);
        if !tag_name_is(tag, "link") {
            continue;
        }
        let rel = html_attr(tag, "rel").unwrap_or_default();
        let mime = html_attr(tag, "type").unwrap_or_default();
        if !rel
            .split_ascii_whitespace()
            .any(|part| part.eq_ignore_ascii_case("alternate"))
            || !mime.eq_ignore_ascii_case("application/json+oembed")
        {
            continue;
        }
        let href = html_attr(tag, "href")?;
        let url = base.join(&decode_html_url(&href)).ok()?;
        if safe_remote_url(url.as_str()) {
            return Some(url);
        }
    }
    None
}

fn html_media_url(html: &str, base: &str) -> Option<String> {
    let base = Url::parse(base).ok()?;
    for tag_name in ["video", "source"] {
        for raw in html.split('<').skip(1) {
            let tag = raw.split_once('>').map(|(tag, _)| tag).unwrap_or(raw);
            if !tag_name_is(tag, tag_name) {
                continue;
            }
            let mime = html_attr(tag, "type").unwrap_or_default().to_ascii_lowercase();
            if tag_name == "source" && !mime.is_empty() && !mime.starts_with("video/") {
                continue;
            }
            if let Some(src) = html_attr(tag, "src")
                .and_then(|raw| resolve_meta_url(&base, &raw))
                .filter(|url| safe_remote_url(url))
            {
                return Some(src);
            }
        }
    }
    None
}

fn html_embed_url(html: &str, base: &str) -> Option<String> {
    let base = Url::parse(base).ok()?;
    for raw in html.split('<').skip(1) {
        let tag = raw.split_once('>').map(|(tag, _)| tag).unwrap_or(raw);
        if !tag_name_is(tag, "iframe") {
            continue;
        }
        let src = html_attr(tag, "src")
            .and_then(|raw| resolve_meta_url(&base, &raw))
            .filter(|url| safe_remote_url(url))?;
        let lower = tag.to_ascii_lowercase();
        let looks_like_player = lower.contains("allowfullscreen")
            || lower.contains("player")
            || lower.contains("video")
            || lower.contains("embed");
        if looks_like_player {
            return Some(src);
        }
    }
    None
}

fn meta_content(html: &str, names: &[&str]) -> Option<String> {
    for raw in html.split('<').skip(1) {
        let tag = raw.split_once('>').map(|(tag, _)| tag).unwrap_or(raw);
        if !tag_name_is(tag, "meta") {
            continue;
        }
        let key = html_attr(tag, "property")
            .or_else(|| html_attr(tag, "name"))
            .unwrap_or_default();
        if names.iter().any(|name| key.eq_ignore_ascii_case(name))
            && let Some(content) = html_attr(tag, "content")
            && !content.trim().is_empty()
        {
            return Some(content);
        }
    }
    None
}

fn html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let after_open = lower[start..].find('>')? + start + 1;
    let end = lower[after_open..].find("</title>")? + after_open;
    let title = html[after_open..end].trim();
    (!title.is_empty()).then(|| title.to_owned())
}

fn tag_name_is(tag: &str, wanted: &str) -> bool {
    let trimmed = tag.trim_start();
    let end = trimmed
        .find(|ch: char| ch.is_ascii_whitespace() || ch == '/' || ch == '>')
        .unwrap_or(trimmed.len());
    trimmed[..end].eq_ignore_ascii_case(wanted)
}

fn html_attr(tag: &str, wanted: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let bytes = tag.as_bytes();
    let mut from = 0;
    while let Some(rel) = lower[from..].find(wanted) {
        let start = from + rel;
        let before_ok = start == 0 || !lower.as_bytes()[start - 1].is_ascii_alphanumeric();
        let after = start + wanted.len();
        let after_ok = after >= lower.len() || !lower.as_bytes()[after].is_ascii_alphanumeric();
        if !before_ok || !after_ok {
            from = after;
            continue;
        }

        let mut index = after;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            from = after;
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            return None;
        }

        if matches!(bytes[index], b'\'' | b'"') {
            let quote = bytes[index];
            index += 1;
            let value_start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            return (index <= bytes.len()).then(|| tag[value_start..index].to_owned());
        }

        let value_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && bytes[index] != b'>'
        {
            index += 1;
        }
        return (index > value_start).then(|| tag[value_start..index].to_owned());
    }
    None
}

fn resolve_meta_url(base: &Url, value: &str) -> Option<String> {
    let value = decode_html_url(value.trim());
    base.join(&value).ok().map(|url| url.to_string())
}

fn resolve_candidate_url(base: &str, value: &str) -> Option<String> {
    let base = Url::parse(base).ok()?;
    resolve_meta_url(&base, value)
}

fn decode_html_url(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#38;", "&")
        .replace("&#x26;", "&")
}

fn decode_html_text(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn file_name_title(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut parts| parts.next_back())
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
}

fn safe_key(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let host = Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "invalid".to_owned());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    format!("{host}#{:08x}", hasher.finish() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_keeps_query_and_removes_default_https_port() {
        assert_eq!(
            canonical_url("https://EXAMPLE.com:443/a?token=1").as_deref(),
            Some("https://example.com/a?token=1")
        );
    }

    #[test]
    fn extracts_links_from_markdown_and_punctuation() {
        assert_eq!(
            extract_https_urls(
                "veja [repo](https://example.com/a?x=1), e <HTTPS://example.org/b>."
            ),
            vec![
                "https://example.com/a?x=1".to_owned(),
                "https://example.org/b".to_owned(),
            ]
        );
    }

    #[test]
    fn private_and_local_targets_are_rejected() {
        for url in [
            "http://example.com/x",
            "https://localhost/x",
            "https://router.local/x",
            "https://127.0.0.1/x",
            "https://10.0.0.1/x",
            "https://8.8.8.8/x",
            "https://[::1]/x",
        ] {
            assert!(!safe_remote_url(url), "{url}");
        }
        assert!(safe_remote_url("https://example.com/x"));
    }

    #[tokio::test]
    async fn parses_generic_og_card_without_any_host_table() {
        let base = Url::parse("https://code.example/project").unwrap();
        let html = r#"
            <html><head>
              <meta property="og:title" content="Papo">
              <meta property="og:description" content="chat client">
              <meta property="og:image" content="/cover.png">
              <meta property="og:site_name" content="Code Forge">
            </head></html>
        "#;
        let preview = parse_html_preview(base.as_str(), &base, html).await.unwrap();
        assert_eq!(preview.kind, PreviewKind::Link);
        assert_eq!(preview.title.as_deref(), Some("Papo"));
        assert_eq!(preview.image_url.as_deref(), Some("https://code.example/cover.png"));
        assert_eq!(preview.provider_name.as_deref(), Some("Code Forge"));
    }

    #[tokio::test]
    async fn parses_direct_video_and_iframe_capabilities() {
        let base = Url::parse("https://media.example/watch/1").unwrap();
        let direct = r#"<meta property="og:video" content="/clip.mp4">
                         <meta property="og:video:type" content="video/mp4">"#;
        let preview = parse_html_preview(base.as_str(), &base, direct).await.unwrap();
        assert_eq!(preview.kind, PreviewKind::Video);
        assert_eq!(preview.media_url.as_deref(), Some("https://media.example/clip.mp4"));

        let player = r#"<meta name="twitter:player" content="/embed/1">"#;
        let preview = parse_html_preview(base.as_str(), &base, player).await.unwrap();
        assert_eq!(preview.kind, PreviewKind::Embed);
        assert_eq!(preview.embed_url.as_deref(), Some("https://media.example/embed/1"));
    }

    #[test]
    fn only_videoish_pages_need_registry_enhancement() {
        assert!(page_wants_player(
            r#"<meta property="og:type" content="video.other">"#
        ));
        assert!(page_wants_player(
            r#"<meta name="twitter:card" content="player">"#
        ));
        assert!(!page_wants_player(
            r#"<meta property="og:type" content="object"><meta property="og:image" content="/x.png">"#
        ));
    }

    #[tokio::test]
    async fn parses_schema_org_video_object_without_provider_hardcoding() {
        let base = Url::parse("https://video.example/watch/1").unwrap();
        let html = r#"
            <script type="application/ld+json">
            {
              "@context": "https://schema.org",
              "@type": "VideoObject",
              "name": "Demo",
              "thumbnailUrl": "/poster.jpg",
              "embedUrl": "/embed/1"
            }
            </script>
        "#;
        let preview = parse_html_preview(base.as_str(), &base, html).await.unwrap();
        assert_eq!(preview.kind, PreviewKind::Embed);
        assert_eq!(preview.title.as_deref(), Some("Demo"));
        assert_eq!(preview.image_url.as_deref(), Some("https://video.example/poster.jpg"));
        assert_eq!(preview.embed_url.as_deref(), Some("https://video.example/embed/1"));
    }

    fn isolated_coordinator() -> (PreviewCoordinator, mpsc::Receiver<String>) {
        let (queue, receiver) = mpsc::channel(8);
        let inner = Arc::new(Inner {
            db: Arc::new(ClientDb::open(None)),
            entries: Mutex::new(HashMap::new()),
            wake: Arc::new(|| {}),
            stats: PreviewStats::default(),
        });
        (PreviewCoordinator { inner, queue }, receiver)
    }

    fn ready_preview(url: &str) -> ResolvedPreview {
        ResolvedPreview {
            source_url: url.to_owned(),
            kind: PreviewKind::Link,
            media_url: None,
            image_url: Some("https://cdn.example/cover.png".to_owned()),
            embed_url: None,
            title: Some("Preview".to_owned()),
            description: None,
            provider_name: Some("example".to_owned()),
        }
    }

    #[test]
    fn repeated_url_is_single_flight_before_worker_runs() {
        let (coordinator, mut queue) = isolated_coordinator();
        let url = "https://example.com/post?keep=1";
        assert_eq!(coordinator.get_or_request(url), Some(PreviewState::Loading));
        assert_eq!(coordinator.get_or_request(url), Some(PreviewState::Loading));
        assert_eq!(
            queue.try_recv().expect("one queued resolution"),
            canonical_url(url).unwrap()
        );
        assert!(queue.try_recv().is_err(), "same URL must not queue twice");
    }

    #[test]
    fn stale_ready_is_served_while_exactly_one_refresh_is_queued() {
        let (coordinator, mut queue) = isolated_coordinator();
        let url = canonical_url("https://example.com/stale").unwrap();
        coordinator.inner.entries.lock().unwrap().insert(
            url.clone(),
            MemoryEntry {
                state: PreviewState::Ready(ready_preview(&url)),
                resolved_at: now_millis() - READY_TTL_MS - 1,
                retry_after: None,
                in_flight: false,
            },
        );

        assert!(matches!(
            coordinator.get_or_request(&url),
            Some(PreviewState::Ready(_))
        ));
        assert!(matches!(
            coordinator.get_or_request(&url),
            Some(PreviewState::Ready(_))
        ));
        assert_eq!(queue.try_recv().unwrap(), url);
        assert!(queue.try_recv().is_err());
    }

    #[test]
    fn retry_deadline_suppresses_then_reenables_resolution() {
        let (coordinator, mut queue) = isolated_coordinator();
        let url = canonical_url("https://example.com/retry").unwrap();
        let future = now_millis() + 60_000;
        coordinator.inner.entries.lock().unwrap().insert(
            url.clone(),
            MemoryEntry {
                state: PreviewState::RetryLater {
                    retry_after: future,
                },
                resolved_at: now_millis(),
                retry_after: Some(future),
                in_flight: false,
            },
        );

        assert!(matches!(
            coordinator.get_or_request(&url),
            Some(PreviewState::RetryLater { .. })
        ));
        assert!(queue.try_recv().is_err());

        {
            let mut entries = coordinator.inner.entries.lock().unwrap();
            let entry = entries.get_mut(&url).unwrap();
            let past = now_millis() - 1;
            entry.state = PreviewState::RetryLater { retry_after: past };
            entry.retry_after = Some(past);
        }
        assert!(matches!(
            coordinator.get_or_request(&url),
            Some(PreviewState::RetryLater { .. })
        ));
        assert_eq!(queue.try_recv().unwrap(), url);
    }

    #[test]
    fn oembed_registry_matching_is_data_driven() {
        assert!(wildcard_url_match(
            "https://*.example.com/watch*",
            "https://video.example.com/watch?v=123"
        ));
        assert!(wildcard_url_match(
            "https://example.com/v/*",
            "https://example.com/v/abc"
        ));
        assert!(!wildcard_url_match(
            "https://example.com/v/*",
            "https://example.com/other/abc"
        ));
    }

    #[test]
    fn discovers_oembed_from_http_link_header() {
        let base = Url::parse("https://video.example/watch/1").unwrap();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::LINK,
            reqwest::header::HeaderValue::from_static(
                "</oembed?url=x>; rel=\"alternate\"; type=\"application/json+oembed\"",
            ),
        );
        assert_eq!(
            oembed_header_endpoint(&headers, &base)
                .map(|url| url.to_string())
                .as_deref(),
            Some("https://video.example/oembed?url=x")
        );
    }

    #[test]
    fn discovers_oembed_without_provider_hardcoding() {
        let base = Url::parse("https://video.example/watch/1").unwrap();
        let html = r#"<link rel="alternate" type="application/json+oembed"
                         href="/oembed?url=https%3A%2F%2Fvideo.example%2Fwatch%2F1">"#;
        assert_eq!(
            oembed_endpoint(html, &base).map(|url| url.to_string()).as_deref(),
            Some("https://video.example/oembed?url=https%3A%2F%2Fvideo.example%2Fwatch%2F1")
        );
    }
}
