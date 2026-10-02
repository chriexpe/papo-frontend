//! Direct KLIPY GIF integration.
//!
//! KLIPY's standard integration requires API and media requests to originate
//! from the end-user client. This module therefore keeps provider state in
//! memory, never proxies through the Papo backend and stores only stable item
//! slugs for messages/favourites. Media URLs are resolved fresh from KLIPY.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

const API_ROOT: &str = "https://api.klipy.com/api/v1";
const PAGE_SIZE: usize = 24;
const SEARCH_DEBOUNCE: f64 = 0.28;
const ITEM_BATCH: usize = 32;

pub fn app_key() -> Option<&'static str> {
    option_env!("PAPO_KLIPY_APP_KEY").filter(|key| !key.trim().is_empty())
}

pub fn available() -> bool {
    app_key().is_some()
}

/// Opaque device-local identifier used only for KLIPY personalization.
pub fn new_customer_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("papo-{nanos:032x}-{:x}-{serial:x}", std::process::id())
}

pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 200
        && !slug.chars().any(|c| c.is_whitespace() || matches!(c, ')' | '(' | ','))
}

pub fn encode_message(slug: &str) -> String {
    format!("@gif(klipy:{slug})")
}

/// Papo wire marker for a provider-backed GIF. The message keeps the stable
/// KLIPY slug rather than retaining or rewriting a media URL.
pub fn message_slug(content: &str) -> Option<&str> {
    let content = content.trim();
    let slug = content.strip_prefix("@gif(klipy:")?.strip_suffix(')')?;
    valid_slug(slug).then_some(slug)
}

#[derive(Clone, Debug)]
pub struct GifItem {
    pub slug: String,
    pub title: String,
    /// Full GIF used by the fullscreen viewer.
    pub gif_url: String,
    /// Small animated GIF used in the picker.
    pub preview_url: String,
    /// Provider-supplied still image when one exists.
    pub still_url: Option<String>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct Category {
    pub name: String,
    pub query: String,
    pub preview_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowseMode {
    Home,
    Trending,
    Favourites,
    Category { name: String, query: String },
    Search { query: String },
}

#[derive(Debug)]
pub struct Browser {
    pub mode: BrowseMode,
    pub query: String,
    pub categories: Vec<Category>,
    pub trending_cover: Option<GifItem>,
    pub results: Vec<GifItem>,
    pub loading: bool,
    pub has_more: bool,
    pub error: Option<String>,
    page: usize,
    generation: u64,
    locale: String,
    customer_id: String,
    categories_locale: String,
    cover_locale: String,
    search_due: Option<f64>,
    submitted_query: String,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            mode: BrowseMode::Home,
            query: String::new(),
            categories: Vec::new(),
            trending_cover: None,
            results: Vec::new(),
            loading: false,
            has_more: false,
            error: None,
            page: 0,
            generation: 0,
            locale: String::new(),
            customer_id: String::new(),
            categories_locale: String::new(),
            cover_locale: String::new(),
            search_due: None,
            submitted_query: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
enum PageSource {
    Trending,
    Search(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PagePurpose {
    Cover,
    Results,
}

enum Request {
    Categories {
        locale: String,
    },
    Page {
        generation: u64,
        purpose: PagePurpose,
        source: PageSource,
        page: usize,
        per_page: usize,
        locale: String,
        customer_id: String,
    },
    Items {
        slugs: Vec<String>,
    },
    Share {
        slug: String,
        customer_id: String,
        query: String,
    },
}

enum Event {
    Categories {
        locale: String,
        result: Result<Vec<Category>, String>,
    },
    Page {
        generation: u64,
        purpose: PagePurpose,
        page: usize,
        result: Result<Vec<GifItem>, String>,
    },
    Items {
        requested: Vec<String>,
        result: Result<Vec<GifItem>, String>,
    },
}

pub struct Store {
    requests: mpsc::Sender<Request>,
    events: mpsc::Receiver<Event>,
    pub browser: Browser,
    items: HashMap<String, GifItem>,
    wanted_items: HashSet<String>,
    pending_items: HashSet<String>,
    missing_items: HashSet<String>,
}

impl Store {
    pub fn new(repaint: egui::Context) -> Self {
        let (request_tx, request_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let key = app_key().map(str::to_owned);
        let _ = std::thread::Builder::new()
            .name("papo-klipy".into())
            .spawn(move || worker(key, request_rx, event_tx, repaint));

        Self {
            requests: request_tx,
            events: event_rx,
            browser: Browser::default(),
            items: HashMap::new(),
            wanted_items: HashSet::new(),
            pending_items: HashSet::new(),
            missing_items: HashSet::new(),
        }
    }

    pub fn open_picker(&mut self, locale: &str, customer_id: &str) {
        self.browser.locale = locale.to_owned();
        self.browser.customer_id = customer_id.to_owned();
        self.show_home();

        if self.browser.categories_locale != locale {
            self.browser.categories.clear();
            self.browser.categories_locale.clear();
            let _ = self.requests.send(Request::Categories {
                locale: locale.to_owned(),
            });
        }
        if self.browser.cover_locale != locale {
            self.browser.trending_cover = None;
            self.browser.cover_locale.clear();
            let _ = self.requests.send(Request::Page {
                generation: self.browser.generation,
                purpose: PagePurpose::Cover,
                source: PageSource::Trending,
                page: 1,
                per_page: 1,
                locale: locale.to_owned(),
                customer_id: customer_id.to_owned(),
            });
        }
    }

    pub fn show_home(&mut self) {
        self.browser.mode = BrowseMode::Home;
        self.browser.results.clear();
        self.browser.loading = false;
        self.browser.has_more = false;
        self.browser.page = 0;
        self.browser.error = None;
        self.browser.submitted_query.clear();
        self.browser.search_due = None;
    }

    pub fn show_favourites(&mut self) {
        self.browser.mode = BrowseMode::Favourites;
        self.browser.results.clear();
        self.browser.loading = false;
        self.browser.has_more = false;
        self.browser.page = 0;
        self.browser.error = None;
        self.browser.search_due = None;
    }

    pub fn show_trending(&mut self) {
        self.begin_results(PageSource::Trending, BrowseMode::Trending);
    }

    pub fn show_category(&mut self, name: String, query: String) {
        self.begin_results(
            PageSource::Search(query.clone()),
            BrowseMode::Category { name, query },
        );
    }

    fn begin_results(&mut self, source: PageSource, mode: BrowseMode) {
        self.browser.generation = self.browser.generation.wrapping_add(1);
        self.browser.mode = mode;
        self.browser.results.clear();
        self.browser.page = 0;
        self.browser.has_more = true;
        self.browser.loading = true;
        self.browser.error = None;
        self.request_page(source, 1);
    }

    fn request_page(&mut self, source: PageSource, page: usize) {
        let sent = self.requests.send(Request::Page {
            generation: self.browser.generation,
            purpose: PagePurpose::Results,
            source,
            page,
            per_page: PAGE_SIZE,
            locale: self.browser.locale.clone(),
            customer_id: self.browser.customer_id.clone(),
        });
        if sent.is_err() {
            self.browser.loading = false;
            self.browser.error = Some("KLIPY worker unavailable".into());
        }
    }

    pub fn query_changed(&mut self, now: f64) {
        self.browser.search_due = Some(now + SEARCH_DEBOUNCE);
        if self.browser.query.is_empty() {
            self.show_home();
        }
    }

    pub fn maybe_submit_search(&mut self, now: f64) {
        let Some(due) = self.browser.search_due else {
            return;
        };
        if now < due {
            return;
        }
        self.browser.search_due = None;
        if self.browser.query.is_empty() || self.browser.query == self.browser.submitted_query {
            return;
        }
        // KLIPY asks partners to pass the query exactly as typed. Only the
        // empty check above is semantic; punctuation/case/spacing stay intact.
        let query = self.browser.query.clone();
        self.browser.submitted_query = query.clone();
        self.begin_results(
            PageSource::Search(query.clone()),
            BrowseMode::Search { query },
        );
    }

    pub fn load_more(&mut self) {
        if self.browser.loading || !self.browser.has_more {
            return;
        }
        let source = match &self.browser.mode {
            BrowseMode::Trending => PageSource::Trending,
            BrowseMode::Category { query, .. } | BrowseMode::Search { query } => {
                PageSource::Search(query.clone())
            }
            BrowseMode::Home | BrowseMode::Favourites => return,
        };
        self.browser.loading = true;
        self.request_page(source, self.browser.page.saturating_add(1).max(1));
    }

    pub fn item(&mut self, slug: &str, ctx: &egui::Context) -> Option<GifItem> {
        if let Some(item) = self.items.get(slug) {
            return Some(item.clone());
        }
        if valid_slug(slug)
            && !self.pending_items.contains(slug)
            && !self.missing_items.contains(slug)
            && self.wanted_items.insert(slug.to_owned())
        {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        None
    }

    pub fn register_share(&self, slug: &str, query: &str) {
        if !valid_slug(slug) {
            return;
        }
        let _ = self.requests.send(Request::Share {
            slug: slug.to_owned(),
            customer_id: self.browser.customer_id.clone(),
            query: query.to_owned(),
        });
    }

    /// Drain API completions and coalesce item lookups requested by message
    /// cards/favourites during the previous frame.
    pub fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Categories { locale, result } => {
                    if locale == self.browser.locale {
                        match result {
                            Ok(categories) => {
                                self.browser.categories = categories;
                                self.browser.categories_locale = locale;
                            }
                            Err(error) => self.browser.error = Some(error),
                        }
                    }
                }
                Event::Page {
                    generation,
                    purpose,
                    page,
                    result,
                } => match purpose {
                    PagePurpose::Cover => {
                        if let Ok(items) = result
                            && let Some(item) = items.into_iter().next()
                        {
                            self.items.insert(item.slug.clone(), item.clone());
                            self.browser.trending_cover = Some(item);
                            self.browser.cover_locale = self.browser.locale.clone();
                        }
                    }
                    PagePurpose::Results if generation == self.browser.generation => {
                        self.browser.loading = false;
                        match result {
                            Ok(items) => {
                                let count = items.len();
                                for item in &items {
                                    self.items.insert(item.slug.clone(), item.clone());
                                }
                                if page == 1 {
                                    self.browser.results = items;
                                } else {
                                    self.browser.results.extend(items);
                                }
                                self.browser.page = page;
                                self.browser.has_more = count == PAGE_SIZE;
                            }
                            Err(error) => {
                                self.browser.error = Some(error);
                                self.browser.has_more = false;
                            }
                        }
                    }
                    PagePurpose::Results => {}
                },
                Event::Items { requested, result } => {
                    for slug in &requested {
                        self.pending_items.remove(slug);
                    }
                    match result {
                        Ok(items) => {
                            let found: HashSet<_> =
                                items.iter().map(|item| item.slug.clone()).collect();
                            for item in items {
                                self.items.insert(item.slug.clone(), item);
                            }
                            for slug in requested {
                                if !found.contains(&slug) {
                                    self.missing_items.insert(slug);
                                }
                            }
                        }
                        Err(error) => {
                            log::warn!("KLIPY items: {error}");
                            // Don't hammer a failing endpoint every frame.
                            self.missing_items.extend(requested);
                        }
                    }
                }
            }
            ctx.request_repaint();
        }

        if !self.wanted_items.is_empty() {
            let slugs: Vec<String> = self.wanted_items.iter().take(ITEM_BATCH).cloned().collect();
            for slug in &slugs {
                self.wanted_items.remove(slug);
                self.pending_items.insert(slug.clone());
            }
            if self.requests.send(Request::Items { slugs: slugs.clone() }).is_err() {
                for slug in slugs {
                    self.pending_items.remove(&slug);
                    self.missing_items.insert(slug);
                }
            }
        }
    }
}

fn worker(
    key: Option<String>,
    requests: mpsc::Receiver<Request>,
    events: mpsc::Sender<Event>,
    repaint: egui::Context,
) {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("Papo/", env!("CARGO_PKG_VERSION"), " KLIPY"))
        .build();

    while let Ok(request) = requests.recv() {
        let result = match (&key, &client) {
            (Some(key), Ok(client)) => Some((key.as_str(), client)),
            (None, _) => None,
            (_, Err(error)) => {
                log::warn!("KLIPY HTTP client unavailable: {error}");
                None
            }
        };

        match request {
            Request::Categories { locale } => {
                let response = result
                    .ok_or_else(|| unavailable().to_owned())
                    .and_then(|(key, client)| fetch_categories(client, key, &locale));
                if events
                    .send(Event::Categories {
                        locale,
                        result: response,
                    })
                    .is_ok()
                {
                    repaint.request_repaint();
                }
            }
            Request::Page {
                generation,
                purpose,
                source,
                page,
                per_page,
                locale,
                customer_id,
            } => {
                let response = result
                    .ok_or_else(|| unavailable().to_owned())
                    .and_then(|(key, client)| {
                        fetch_page(
                            client,
                            key,
                            &source,
                            page,
                            per_page,
                            &locale,
                            &customer_id,
                        )
                    });
                if events
                    .send(Event::Page {
                        generation,
                        purpose,
                        page,
                        result: response,
                    })
                    .is_ok()
                {
                    repaint.request_repaint();
                }
            }
            Request::Items { slugs } => {
                let response = result
                    .ok_or_else(|| unavailable().to_owned())
                    .and_then(|(key, client)| fetch_items(client, key, &slugs));
                if events
                    .send(Event::Items {
                        requested: slugs,
                        result: response,
                    })
                    .is_ok()
                {
                    repaint.request_repaint();
                }
            }
            Request::Share {
                slug,
                customer_id,
                query,
            } => {
                if let Some((key, client)) = result
                    && let Err(error) = register_share(client, key, &slug, &customer_id, &query)
                {
                    log::debug!("KLIPY share trigger failed: {error}");
                }
            }
        }
    }
}

fn unavailable() -> &'static str {
    "KLIPY API key is not configured for this build"
}

fn endpoint(key: &str, suffix: &str) -> String {
    format!("{API_ROOT}/{key}/gifs/{suffix}")
}

fn fetch_categories(
    client: &reqwest::blocking::Client,
    key: &str,
    locale: &str,
) -> Result<Vec<Category>, String> {
    let envelope: Envelope<CategoryPayload> = client
        .get(endpoint(key, "categories"))
        .query(&[("locale", locale)])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .json()
        .map_err(|error| error.to_string())?;
    if !envelope.result {
        return Err("KLIPY categories request was rejected".into());
    }
    Ok(envelope
        .data
        .categories
        .into_iter()
        .filter(|category| {
            !category.category.is_empty()
                && !category.query.is_empty()
                && papo_core::preview::safe_remote_url(&category.preview_url)
        })
        .map(|category| Category {
            name: category.category,
            query: category.query,
            preview_url: category.preview_url,
        })
        .collect())
}

fn fetch_page(
    client: &reqwest::blocking::Client,
    key: &str,
    source: &PageSource,
    page: usize,
    per_page: usize,
    locale: &str,
    customer_id: &str,
) -> Result<Vec<GifItem>, String> {
    let mut request = match source {
        PageSource::Trending => client.get(endpoint(key, "trending")),
        PageSource::Search(query) => client
            .get(endpoint(key, "search"))
            .query(&[("q", query.as_str())]),
    };
    request = request.query(&[
        ("page", page.to_string()),
        ("per_page", per_page.to_string()),
        ("customer_id", customer_id.to_owned()),
        ("locale", locale.to_owned()),
        ("format_filter", "gif,jpg".to_owned()),
    ]);

    let envelope: Envelope<PagePayload> = request
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .json()
        .map_err(|error| error.to_string())?;
    if !envelope.result {
        return Err("KLIPY GIF request was rejected".into());
    }
    convert_items(envelope.data.data)
}

fn fetch_items(
    client: &reqwest::blocking::Client,
    key: &str,
    slugs: &[String],
) -> Result<Vec<GifItem>, String> {
    let envelope: Envelope<PagePayload> = client
        .get(endpoint(key, "items"))
        .query(&[("slugs", slugs.join(","))])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .json()
        .map_err(|error| error.to_string())?;
    if !envelope.result {
        return Err("KLIPY item request was rejected".into());
    }
    convert_items(envelope.data.data)
}

fn register_share(
    client: &reqwest::blocking::Client,
    key: &str,
    slug: &str,
    customer_id: &str,
    query: &str,
) -> Result<(), String> {
    client
        .post(endpoint(key, &format!("share/{slug}")))
        .json(&serde_json::json!({
            "customer_id": customer_id,
            "q": query,
        }))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn convert_items(items: Vec<RawItem>) -> Result<Vec<GifItem>, String> {
    let mut out = Vec::with_capacity(items.len());
    for raw in items {
        if raw.kind.as_deref() == Some("ad") {
            return Err(
                "KLIPY ads are enabled for this key, but Papo has no KLIPY ad renderer yet".into(),
            );
        }
        let (Some(slug), Some(file)) = (raw.slug, raw.file) else {
            continue;
        };
        if !valid_slug(&slug) {
            continue;
        }
        let Some(gif) = pick_gif(&file) else {
            continue;
        };
        if !papo_core::preview::safe_remote_url(&gif.url) {
            continue;
        }
        let preview = pick_preview_gif(&file).unwrap_or(gif);
        let still = pick_still(&file)
            .filter(|media| papo_core::preview::safe_remote_url(&media.url))
            .map(|media| media.url.clone());

        out.push(GifItem {
            slug,
            title: raw.title.unwrap_or_else(|| "GIF".into()),
            gif_url: gif.url.clone(),
            preview_url: preview.url.clone(),
            still_url: still,
            width: gif.width.max(1),
            height: gif.height.max(1),
        });
    }
    Ok(out)
}

fn pick_gif(file: &RawFile) -> Option<&RawMedia> {
    file.hd
        .as_ref()
        .and_then(|variant| variant.gif.as_ref())
        .or_else(|| file.md.as_ref().and_then(|variant| variant.gif.as_ref()))
        .or_else(|| file.sm.as_ref().and_then(|variant| variant.gif.as_ref()))
        .or_else(|| file.xs.as_ref().and_then(|variant| variant.gif.as_ref()))
}

fn pick_preview_gif(file: &RawFile) -> Option<&RawMedia> {
    file.sm
        .as_ref()
        .and_then(|variant| variant.gif.as_ref())
        .or_else(|| file.xs.as_ref().and_then(|variant| variant.gif.as_ref()))
        .or_else(|| file.md.as_ref().and_then(|variant| variant.gif.as_ref()))
        .or_else(|| file.hd.as_ref().and_then(|variant| variant.gif.as_ref()))
}

fn pick_still(file: &RawFile) -> Option<&RawMedia> {
    file.sm
        .as_ref()
        .and_then(|variant| variant.jpg.as_ref())
        .or_else(|| file.xs.as_ref().and_then(|variant| variant.jpg.as_ref()))
        .or_else(|| file.md.as_ref().and_then(|variant| variant.jpg.as_ref()))
        .or_else(|| file.hd.as_ref().and_then(|variant| variant.jpg.as_ref()))
}

#[derive(Deserialize)]
struct Envelope<T> {
    result: bool,
    data: T,
}

#[derive(Deserialize)]
struct CategoryPayload {
    #[serde(default)]
    categories: Vec<RawCategory>,
}

#[derive(Deserialize)]
struct RawCategory {
    #[serde(default)]
    category: String,
    #[serde(default)]
    query: String,
    #[serde(default)]
    preview_url: String,
}

#[derive(Deserialize)]
struct PagePayload {
    #[serde(default)]
    data: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawItem {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    file: Option<RawFile>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Default, Deserialize)]
struct RawFile {
    #[serde(default)]
    hd: Option<RawVariant>,
    #[serde(default)]
    md: Option<RawVariant>,
    #[serde(default)]
    sm: Option<RawVariant>,
    #[serde(default)]
    xs: Option<RawVariant>,
}

#[derive(Default, Deserialize)]
struct RawVariant {
    #[serde(default)]
    gif: Option<RawMedia>,
    #[serde(default)]
    jpg: Option<RawMedia>,
}

#[derive(Deserialize)]
struct RawMedia {
    #[serde(default)]
    url: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_round_trip() {
        let encoded = encode_message("hello-there");
        assert_eq!(message_slug(&encoded), Some("hello-there"));
        assert_eq!(message_slug(" @gif(klipy:hello-there)\n"), Some("hello-there"));
    }

    #[test]
    fn unrelated_text_is_not_a_gif_marker() {
        assert_eq!(message_slug("hello @gif(klipy:wave)"), None);
        assert_eq!(message_slug("@gif(tenor:wave)"), None);
    }
}
