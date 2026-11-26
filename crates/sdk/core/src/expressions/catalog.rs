//! Cookie-free optional GIPHY catalog transport; durable favorites and outgoing recents live elsewhere.

use super::{ExpressionError, ExpressionService, Gif};
use crate::Generated;
use serde::Deserialize;

/// Explicit provider configuration. The key never enters SQLite, logs or Papo requests.
#[derive(Clone)]
pub struct GiphyConfig {
    pub(crate) key: String,
    #[cfg(feature = "test-support")]
    endpoint: Option<String>,
}
impl GiphyConfig {
    /// Accepts an externally supplied nonempty API key, without performing a request.
    /// # Errors
    /// Returns invalid data for empty, oversized or control-character keys.
    pub fn new(key: String) -> Result<Self, ExpressionError> {
        if key.is_empty() || key.len() > 1024 || key.chars().any(char::is_control) {
            return Err(ExpressionError::InvalidData);
        }
        Ok(Self {
            key,
            #[cfg(feature = "test-support")]
            endpoint: None,
        })
    }
    /// Installs a literal loopback provider endpoint for integration proofs only.
    /// # Errors
    /// Rejects non-loopback/credentialed URLs and invalid keys before any request.
    #[cfg(feature = "test-support")]
    pub fn loopback(key: String, endpoint: String) -> Result<Self, ExpressionError> {
        let url = url::Url::parse(&endpoint).map_err(|_| ExpressionError::InvalidData)?;
        let loopback = url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if url.scheme() != "http"
            || !loopback
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ExpressionError::InvalidData);
        }
        let mut config = Self::new(key)?;
        config.endpoint = Some(endpoint);
        Ok(config)
    }

    /// The loopback trending searches endpoint: the catalog endpoint with `-searches` after
    /// it (`/catalog/trending` → `/catalog/trending-searches`).
    #[cfg(feature = "test-support")]
    fn searches_endpoint(&self) -> Option<String> {
        self.endpoint
            .as_ref()
            .map(|endpoint| format!("{endpoint}-searches"))
    }
}

/// GIFs a page asks for: GIPHY's most (its beta keys' limit).
const PAGE_LEN: u32 = 50;
/// The last offset GIPHY answers for trending, and for a search.
const TRENDING_MAX_OFFSET: u32 = 499;
const SEARCH_MAX_OFFSET: u32 = 4999;
/// The most trending searches kept, and the longest one.
const MAX_TRENDING_SEARCHES: usize = 20;
const MAX_SEARCH_CHARS: usize = 50;

/// One provider page, in provider order. Clients own attribution and presentation.
#[derive(Clone)]
pub struct CatalogPage {
    /// Provider GIFs, at most 50 per request.
    pub gifs: Vec<Gif>,
    /// Exact next provider offset, if another supported page exists.
    pub next_offset: Option<u32>,
}

#[derive(Deserialize)]
struct Response {
    data: Vec<Item>,
    pagination: Pagination,
    meta: Meta,
}
#[derive(Deserialize)]
struct Meta {
    status: u16,
}
#[derive(Deserialize)]
struct Pagination {
    offset: u32,
    count: u32,
    total_count: u32,
}
#[derive(Deserialize)]
struct Item {
    id: String,
    url: String,
    title: String,
    images: Images,
}
#[derive(Deserialize)]
struct Images {
    original: Image,
    #[serde(default)]
    fixed_width: Option<Image>,
}
/// A rendition; GIPHY writes its sizes as strings.
#[derive(Deserialize)]
struct Image {
    url: String,
    #[serde(default)]
    width: Option<String>,
    #[serde(default)]
    height: Option<String>,
}
impl Image {
    /// Its width and height, or zeros when GIPHY gave none that read.
    fn size(&self) -> (u32, u32) {
        let read = |side: Option<&String>| side.and_then(|side| side.parse().ok()).unwrap_or(0);
        (read(self.width.as_ref()), read(self.height.as_ref()))
    }
}
#[derive(Deserialize)]
struct SearchesResponse {
    data: Vec<String>,
    meta: Meta,
}

impl ExpressionService {
    /// Retrieves GIPHY trending with no Papo credentials. Missing configuration is typed unavailable.
    /// # Errors
    /// Returns unavailable, rate limited, invalid response/input or stale account generation.
    pub async fn trending(&self, offset: u32) -> Result<Generated<CatalogPage>, ExpressionError> {
        self.catalog(None, offset).await
    }
    /// Searches the exact user query; no SDK spelling changes, reordering or synthetic results.
    /// # Errors
    /// Returns unavailable, rate limited, invalid response/input or stale account generation.
    pub async fn search(
        &self,
        query: String,
        offset: u32,
    ) -> Result<Generated<CatalogPage>, ExpressionError> {
        if query.is_empty() || query.chars().count() > 50 {
            return Err(ExpressionError::InvalidData);
        }
        self.catalog(Some(query), offset).await
    }
    /// GIPHY's trending searches, in its order, at most 20: the picker's search chips.
    /// # Errors
    /// Returns unavailable, rate limited, invalid response or stale account generation.
    pub async fn trending_searches(&self) -> Result<Generated<Vec<String>>, ExpressionError> {
        self.provider(|config| async move { fetch_searches(&config).await })
            .await
    }
    async fn catalog(
        &self,
        query: Option<String>,
        offset: u32,
    ) -> Result<Generated<CatalogPage>, ExpressionError> {
        let max_offset = if query.is_some() {
            SEARCH_MAX_OFFSET
        } else {
            TRENDING_MAX_OFFSET
        };
        if offset > max_offset {
            return Err(ExpressionError::InvalidData);
        }
        self.provider(move |config| async move { fetch(&config, query.as_deref(), offset).await })
            .await
    }
    /// Runs `request` against the configured provider on this account's executor, superseded
    /// with its connection generation.
    async fn provider<T, F, Fut>(&self, request: F) -> Result<Generated<T>, ExpressionError>
    where
        T: Send + 'static,
        F: FnOnce(GiphyConfig) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T, ExpressionError>> + Send,
    {
        let session = self.session.clone();
        let config = session
            .client
            .inner
            .config
            .giphy
            .clone()
            .ok_or(ExpressionError::Unavailable)?;
        let token = session.inner.tasks.token();
        self.session
            .client
            .dispatch(async move {
                let _token = token;
                let epoch = session.inner.epochs.current();
                session.inner.epochs.validate(epoch.generation)?;
                let work = request(config);
                let result = tokio::select! {
                    biased;
                    () = epoch.cancel.cancelled() => Err(crate::ClientError::Superseded.into()),
                    result = work => result,
                };
                session.inner.epochs.validate(epoch.generation)?;
                Ok(result.map(|value| Generated {
                    generation: epoch.generation,
                    value,
                }))
            })
            .await?
    }
}

async fn fetch(
    config: &GiphyConfig,
    query: Option<&str>,
    offset: u32,
) -> Result<CatalogPage, ExpressionError> {
    let endpoint = if query.is_some() {
        "https://api.giphy.com/v1/gifs/search"
    } else {
        "https://api.giphy.com/v1/gifs/trending"
    };
    #[cfg(feature = "test-support")]
    let endpoint = config.endpoint.as_deref().unwrap_or(endpoint);
    let client = crate::media::anonymous_client().map_err(|_| ExpressionError::Unavailable)?;
    let mut request = client.get(endpoint).query(&[
        ("api_key", config.key.as_str()),
        ("limit", &PAGE_LEN.to_string()),
        ("offset", &offset.to_string()),
    ]);
    if let Some(query) = query {
        request = request.query(&[("q", query)]);
    }
    let max_offset = if query.is_some() {
        SEARCH_MAX_OFFSET
    } else {
        TRENDING_MAX_OFFSET
    };
    parse(&read_body(request).await?, offset, max_offset)
}

async fn fetch_searches(config: &GiphyConfig) -> Result<Vec<String>, ExpressionError> {
    let endpoint = "https://api.giphy.com/v1/trending/searches";
    #[cfg(feature = "test-support")]
    let endpoint = config
        .searches_endpoint()
        .unwrap_or_else(|| endpoint.to_owned());
    let client = crate::media::anonymous_client().map_err(|_| ExpressionError::Unavailable)?;
    let request = client
        .get(endpoint)
        .query(&[("api_key", config.key.as_str())]);
    parse_searches(&read_body(request).await?)
}

/// The body of `request`'s answer, at most 2 MiB.
async fn read_body(request: reqwest::RequestBuilder) -> Result<Vec<u8>, ExpressionError> {
    let mut response = request
        .send()
        .await
        .map_err(|_| ExpressionError::Unavailable)?;
    if response.status().as_u16() == 429 {
        return Err(ExpressionError::RateLimited);
    }
    if !response.status().is_success() {
        return Err(ExpressionError::Unavailable);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ExpressionError::Unavailable)?
    {
        if chunk.len() > (2 * 1024 * 1024_usize).saturating_sub(bytes.len()) {
            return Err(ExpressionError::InvalidData);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn parse_searches(bytes: &[u8]) -> Result<Vec<String>, ExpressionError> {
    let response: SearchesResponse =
        serde_json::from_slice(bytes).map_err(|_| ExpressionError::InvalidData)?;
    if response.meta.status != 200 {
        return Err(ExpressionError::InvalidData);
    }
    Ok(response
        .data
        .into_iter()
        .map(|term| term.trim().to_owned())
        .filter(|term| {
            !term.is_empty()
                && term.chars().count() <= MAX_SEARCH_CHARS
                && !term.chars().any(char::is_control)
        })
        .take(MAX_TRENDING_SEARCHES)
        .collect())
}

fn parse(bytes: &[u8], offset: u32, max_offset: u32) -> Result<CatalogPage, ExpressionError> {
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| ExpressionError::InvalidData)?;
    if response.meta.status != 200
        || response.data.len() > PAGE_LEN as usize
        || response.pagination.offset != offset
        || response.pagination.count as usize != response.data.len()
    {
        return Err(ExpressionError::InvalidData);
    }
    let mut gifs = Vec::new();
    for item in response.data {
        let (width, height) = item.images.original.size();
        let (preview_url, (preview_width, preview_height)) = item.images.fixed_width.map_or_else(
            || (String::new(), (0, 0)),
            |preview| {
                let size = preview.size();
                (preview.url, size)
            },
        );
        let gif = Gif {
            id: item.id,
            url: item.images.original.url,
            page_url: item.url,
            title: item.title,
            width,
            height,
            preview_url,
            preview_width,
            preview_height,
        };
        gif.validate()?;
        gifs.push(gif);
    }
    let next = offset
        .checked_add(response.pagination.count)
        .ok_or(ExpressionError::InvalidData)?;
    let next_offset =
        (next > offset && next < response.pagination.total_count && next <= max_offset)
            .then_some(next);
    Ok(CatalogPage { gifs, next_offset })
}

/// [`parse`] for a trending page (`search` false) or a search page, for the unit tests.
#[cfg(test)]
pub(super) fn parse_page_for_test(
    bytes: &[u8],
    offset: u32,
    search: bool,
) -> Result<CatalogPage, ExpressionError> {
    let max_offset = if search {
        SEARCH_MAX_OFFSET
    } else {
        TRENDING_MAX_OFFSET
    };
    parse(bytes, offset, max_offset)
}

/// [`parse_searches`], for the unit tests.
#[cfg(test)]
pub(super) fn parse_searches_for_test(bytes: &[u8]) -> Result<Vec<String>, ExpressionError> {
    parse_searches(bytes)
}
