//! Direct GIPHY GIF integration.
//!
//! GIPHY API and media requests are made directly by the client. Papo persists
//! only stable GIPHY IDs in message content/favourites; provider media URLs are
//! resolved on demand and remain transient.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::Duration;

use serde::Deserialize;

const API_ROOT: &str = "https://api.giphy.com/v1/gifs";
const PAGE_SIZE: usize = 24;
const SEARCH_DEBOUNCE: f64 = 0.28;
const ITEM_BATCH: usize = 32;
const RATING: &str = "pg-13";

pub fn api_key() -> Option<&'static str> {
    option_env!("PAPO_GIPHY_API_KEY").filter(|key| {
        let key = key.trim();
        !key.is_empty() && key != "@PAPO_GIPHY_API_KEY@"
    })
}

pub fn available() -> bool {
    api_key().is_some()
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub fn encode_message(id: &str) -> String {
    format!("giphy:{id}")
}

/// Official Papo wire marker for a GIPHY-backed GIF.
pub fn message_id(content: &str) -> Option<&str> {
    let id = content.trim().strip_prefix("giphy:")?;
    valid_id(id).then_some(id)
}

#[derive(Clone, Debug)]
pub struct GifItem {
    pub id: String,
    pub title: String,
    /// Original/highest-resolution GIF used by the fullscreen viewer.
    pub gif_url: String,
    /// Higher-quality bounded rendition used by sent/chat GIFs.
    pub display_url: String,
    /// Grid rendition used by the picker/category tiles.
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
    categories_loaded: bool,
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
            categories_loaded: false,
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
    Categories,
    Page {
        generation: u64,
        purpose: PagePurpose,
        source: PageSource,
        page: usize,
        per_page: usize,
        locale: String,
    },
    Items {
        ids: Vec<String>,
    },
}

enum Event {
    Categories {
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
        let key = api_key().map(str::to_owned);
        let _ = std::thread::Builder::new()
            .name("papo-giphy".into())
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

    pub fn open_picker(&mut self, locale: &str) {
        self.browser.locale = locale.to_owned();
        self.show_home();

        if !self.browser.categories_loaded {
            self.browser.categories.clear();
            let _ = self.requests.send(Request::Categories);
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
        });
        if sent.is_err() {
            self.browser.loading = false;
            self.browser.error = Some("GIPHY worker unavailable".into());
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

    pub fn item(&mut self, id: &str, ctx: &egui::Context) -> Option<GifItem> {
        if let Some(item) = self.items.get(id) {
            return Some(item.clone());
        }
        if valid_id(id)
            && !self.pending_items.contains(id)
            && !self.missing_items.contains(id)
            && self.wanted_items.insert(id.to_owned())
        {
            ctx.request_repaint_after(Duration::from_millis(20));
        }
        None
    }

    /// Drain API completions and coalesce ID lookups requested by message
    /// cards/favourites during the previous frame.
    pub fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Categories { result } => match result {
                    Ok(categories) => {
                        self.browser.categories = categories;
                        self.browser.categories_loaded = true;
                    }
                    Err(error) => self.browser.error = Some(error),
                },
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
                            self.items.insert(item.id.clone(), item.clone());
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
                                    self.items.insert(item.id.clone(), item.clone());
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
                    for id in &requested {
                        self.pending_items.remove(id);
                    }
                    match result {
                        Ok(items) => {
                            let found: HashSet<_> =
                                items.iter().map(|item| item.id.clone()).collect();
                            for item in items {
                                self.items.insert(item.id.clone(), item);
                            }
                            for id in requested {
                                if !found.contains(&id) {
                                    self.missing_items.insert(id);
                                }
                            }
                        }
                        Err(error) => {
                            log::warn!("GIPHY items: {error}");
                            self.missing_items.extend(requested);
                        }
                    }
                }
            }
            ctx.request_repaint();
        }

        if !self.wanted_items.is_empty() {
            let ids: Vec<String> = self.wanted_items.iter().take(ITEM_BATCH).cloned().collect();
            for id in &ids {
                self.wanted_items.remove(id);
                self.pending_items.insert(id.clone());
            }
            if self.requests.send(Request::Items { ids: ids.clone() }).is_err() {
                for id in ids {
                    self.pending_items.remove(&id);
                    self.missing_items.insert(id);
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
        .user_agent(concat!("Papo/", env!("CARGO_PKG_VERSION"), " GIPHY"))
        .build();

    while let Ok(request) = requests.recv() {
        let result = match (&key, &client) {
            (Some(key), Ok(client)) => Some((key.as_str(), client)),
            (None, _) => None,
            (_, Err(error)) => {
                log::warn!("GIPHY HTTP client unavailable: {error}");
                None
            }
        };

        match request {
            Request::Categories => {
                let response = result
                    .ok_or_else(|| unavailable().to_owned())
                    .and_then(|(key, client)| fetch_categories(client, key));
                if events.send(Event::Categories { result: response }).is_ok() {
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
            } => {
                let response = result
                    .ok_or_else(|| unavailable().to_owned())
                    .and_then(|(key, client)| {
                        fetch_page(client, key, &source, page, per_page, &locale)
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
            Request::Items { ids } => {
                let response = result
                    .ok_or_else(|| unavailable().to_owned())
                    .and_then(|(key, client)| fetch_items(client, key, &ids));
                if events
                    .send(Event::Items {
                        requested: ids,
                        result: response,
                    })
                    .is_ok()
                {
                    repaint.request_repaint();
                }
            }
        }
    }
}

fn unavailable() -> &'static str {
    "GIPHY API key is not configured for this build"
}

fn fetch_categories(
    client: &reqwest::blocking::Client,
    key: &str,
) -> Result<Vec<Category>, String> {
    let payload: ListResponse<RawCategory> = client
        .get(format!("{API_ROOT}/categories"))
        .query(&[("api_key", key)])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .json()
        .map_err(|error| error.to_string())?;

    Ok(payload
        .data
        .into_iter()
        .filter_map(|category| {
            let name = category.name.trim().to_owned();
            if name.is_empty() {
                return None;
            }
            let preview_url = convert_item(category.gif?)?.preview_url;
            Some(Category {
                query: name.clone(),
                name,
                preview_url,
            })
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
) -> Result<Vec<GifItem>, String> {
    let offset = page.saturating_sub(1).saturating_mul(per_page);
    let mut params = vec![
        ("api_key".to_owned(), key.to_owned()),
        ("limit".to_owned(), per_page.to_string()),
        ("offset".to_owned(), offset.to_string()),
        ("rating".to_owned(), RATING.to_owned()),
    ];
    let endpoint = match source {
        PageSource::Trending => format!("{API_ROOT}/trending"),
        PageSource::Search(query) => {
            params.push(("q".to_owned(), query.clone()));
            if !locale.is_empty() {
                params.push(("lang".to_owned(), locale.to_owned()));
            }
            format!("{API_ROOT}/search")
        }
    };

    let payload: ListResponse<RawGif> = client
        .get(endpoint)
        .query(&params)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .json()
        .map_err(|error| error.to_string())?;

    Ok(payload.data.into_iter().filter_map(convert_item).collect())
}

fn fetch_items(
    client: &reqwest::blocking::Client,
    key: &str,
    ids: &[String],
) -> Result<Vec<GifItem>, String> {
    let payload: ListResponse<RawGif> = client
        .get(API_ROOT)
        .query(&[("api_key", key.to_owned()), ("ids", ids.join(","))])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?
        .json()
        .map_err(|error| error.to_string())?;

    Ok(payload.data.into_iter().filter_map(convert_item).collect())
}

fn convert_item(raw: RawGif) -> Option<GifItem> {
    if !valid_id(&raw.id) {
        return None;
    }

    let full = raw
        .images
        .original
        .as_ref()
        .or(raw.images.downsized.as_ref())
        .or(raw.images.fixed_width.as_ref())?;
    // GIPHY documents fixed_width_small as a 100px "nano" rendition. Our
    // two-column picker tiles are roughly 200px wide, so using it first caused
    // visible upscaling/blurring. fixed_width is the intended grid rendition.
    let preview = raw
        .images
        .fixed_width
        .as_ref()
        .or(raw.images.downsized.as_ref())
        .unwrap_or(full);

    // Once selected, GIPHY recommends a higher-quality downsized rendition for
    // chat/messaging. Prefer the <=5MB medium tier here: substantially sharper
    // than the 100/200px grid renditions without forcing every visible message
    // to fetch/decode the original asset.
    let display = raw
        .images
        .downsized_medium
        .as_ref()
        .or(raw.images.downsized.as_ref())
        .or(raw.images.downsized_large.as_ref())
        .unwrap_or(full);

    if !papo_core::preview::safe_remote_url(&full.url)
        || !papo_core::preview::safe_remote_url(&display.url)
        || !papo_core::preview::safe_remote_url(&preview.url)
    {
        return None;
    }

    let still_url = raw
        .images
        .fixed_width_still
        .as_ref()
        .or(raw.images.original_still.as_ref())
        .filter(|image| papo_core::preview::safe_remote_url(&image.url))
        .map(|image| image.url.clone());

    Some(GifItem {
        id: raw.id,
        title: if raw.title.trim().is_empty() {
            "GIF".into()
        } else {
            raw.title
        },
        gif_url: full.url.clone(),
        display_url: display.url.clone(),
        preview_url: preview.url.clone(),
        still_url,
        width: parse_dimension(&full.width).max(1),
        height: parse_dimension(&full.height).max(1),
    })
}

fn parse_dimension(value: &str) -> u32 {
    value.parse().unwrap_or(0)
}

#[derive(Deserialize)]
struct ListResponse<T> {
    #[serde(default)]
    data: Vec<T>,
}

#[derive(Default, Deserialize)]
struct RawCategory {
    #[serde(default)]
    name: String,
    #[serde(default)]
    gif: Option<RawGif>,
}

#[derive(Default, Deserialize)]
struct RawGif {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    images: RawImages,
}

#[derive(Default, Deserialize)]
struct RawImages {
    #[serde(default)]
    original: Option<RawImage>,
    #[serde(default)]
    downsized: Option<RawImage>,
    #[serde(default)]
    downsized_large: Option<RawImage>,
    #[serde(default)]
    downsized_medium: Option<RawImage>,
    #[serde(default)]
    fixed_width: Option<RawImage>,
    #[serde(default)]
    fixed_width_still: Option<RawImage>,
    #[serde(default)]
    original_still: Option<RawImage>,
}

#[derive(Deserialize)]
struct RawImage {
    #[serde(default)]
    url: String,
    #[serde(default)]
    width: String,
    #[serde(default)]
    height: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_round_trip_matches_official_papo_contract() {
        let encoded = encode_message("xT4uQulxzV39haRFjG");
        assert_eq!(message_id(&encoded), Some("xT4uQulxzV39haRFjG"));
        assert_eq!(message_id(" giphy:xT4uQulxzV39haRFjG\n"), Some("xT4uQulxzV39haRFjG"));
    }

    #[test]
    fn unrelated_text_is_not_a_gif_marker() {
        assert_eq!(message_id("hello giphy:xT4uQulxzV39haRFjG"), None);
        assert_eq!(message_id("@gif(klipy:wave)"), None);
    }
}
