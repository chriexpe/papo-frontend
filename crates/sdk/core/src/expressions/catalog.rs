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
}

/// One provider page, in provider order. Clients own attribution and presentation.
#[derive(Clone)]
pub struct CatalogPage {
    /// Provider GIFs, bounded to 25 per request.
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
}
#[derive(Deserialize)]
struct Image {
    url: String,
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
    async fn catalog(
        &self,
        query: Option<String>,
        offset: u32,
    ) -> Result<Generated<CatalogPage>, ExpressionError> {
        if offset > 499 {
            return Err(ExpressionError::InvalidData);
        }
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
                let work = fetch(&config, query.as_deref(), offset);
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
        ("limit", "25"),
        ("offset", &offset.to_string()),
    ]);
    if let Some(query) = query {
        request = request.query(&[("q", query)]);
    }
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
    parse(&bytes, offset)
}

fn parse(bytes: &[u8], offset: u32) -> Result<CatalogPage, ExpressionError> {
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| ExpressionError::InvalidData)?;
    if response.meta.status != 200
        || response.data.len() > 25
        || response.pagination.offset != offset
        || response.pagination.count as usize != response.data.len()
    {
        return Err(ExpressionError::InvalidData);
    }
    let mut gifs = Vec::new();
    for item in response.data {
        let gif = Gif {
            id: item.id,
            url: item.images.original.url,
            page_url: item.url,
            title: item.title,
        };
        gif.validate()?;
        gifs.push(gif);
    }
    let next = offset
        .checked_add(response.pagination.count)
        .ok_or(ExpressionError::InvalidData)?;
    let next_offset =
        (next > offset && next < response.pagination.total_count && next <= 499).then_some(next);
    Ok(CatalogPage { gifs, next_offset })
}
