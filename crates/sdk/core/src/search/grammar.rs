//! Search filter grammar and unambiguous name resolution; server execution stays authoritative.

use super::SearchError;
use crate::{HistoryOrder, ResourceSnapshot, Uuid};
use papo_api::wire::SearchRequest;

/// Explicit backend filters; absence of a channel means server-wide non-DM search.
#[derive(Clone, Default)]
pub struct SearchQuery {
    /// Backend full-text terms.
    pub text: Option<String>,
    /// Exact author UUID.
    pub author: Option<Uuid>,
    /// Explicit channel/DM scope.
    pub channel: Option<Uuid>,
    /// Exact mentioned user UUID.
    pub mention: Option<Uuid>,
    /// Require HTTP(S) links.
    pub has_link: bool,
    /// Presence or absence of attachments, including false.
    pub contains_attachment: Option<bool>,
    /// Inclusive UTC date YYYY-MM-DD.
    pub date_start: Option<String>,
    /// Inclusive UTC date YYYY-MM-DD.
    pub date_end: Option<String>,
    /// Tuple ordering; default newest first.
    pub order: Option<HistoryOrder>,
}

/// Borrowed lightweight labels used for parsing/local filtering; never fetches profiles or a roster.
pub struct SearchNames<'a> {
    /// Current account's lightweight resource image.
    pub resources: &'a ResourceSnapshot,
}
impl SearchQuery {
    /// Parses de/from, em/in, tem/has, mentions and quoted names. Ambiguous names fail.
    /// # Errors
    /// Returns invalid/unknown/ambiguous filters or unmatched quotes.
    pub fn parse(raw: &str, names: &SearchNames<'_>) -> Result<Self, SearchError> {
        if raw.len() > 8192 {
            return Err(SearchError::InvalidData);
        }
        let mut query = Self::default();
        let mut words = Vec::new();
        for token in tokens(raw)? {
            let Some((prefix, value)) = token.split_once(':') else {
                words.push(token);
                continue;
            };
            match prefix.to_lowercase().as_str() {
                "de" | "from" => set(&mut query.author, names.person(value)?)?,
                "em" | "in" => set(&mut query.channel, names.channel(value)?)?,
                "mentions" => set(&mut query.mention, names.person(value)?)?,
                "tem" | "has" => match value.to_lowercase().as_str() {
                    "link" => query.has_link = true,
                    "anexo" | "arquivo" | "attachment" | "file" => {
                        query.contains_attachment = Some(true);
                    }
                    _ => return Err(SearchError::InvalidData),
                },
                _ => words.push(token),
            }
        }
        let text = words.join(" ");
        query.text = (!text.is_empty()).then_some(text);
        query.clone().into_request()?;
        Ok(query)
    }
    pub(super) fn into_request(self) -> Result<SearchRequest, SearchError> {
        if self.text.as_ref().is_some_and(|s| s.len() > 8192) {
            return Err(SearchError::InvalidData);
        }
        let text = self.text.filter(|s| !s.trim().is_empty());
        if text.is_none()
            && self.author.is_none()
            && self.channel.is_none()
            && self.mention.is_none()
            && !self.has_link
            && self.contains_attachment.is_none()
            && self.date_start.is_none()
            && self.date_end.is_none()
        {
            return Err(SearchError::InvalidData);
        }
        let start = self.date_start.as_deref().map(date).transpose()?;
        let end = self.date_end.as_deref().map(date).transpose()?;
        if start.zip(end).is_some_and(|(a, b)| a > b) {
            return Err(SearchError::InvalidData);
        }
        Ok(SearchRequest {
            text,
            author: self.author.map(|id| id.to_string()),
            channel_id: self.channel.map(|id| id.to_string()),
            mention: self.mention.map(|id| id.to_string()),
            has: self.has_link.then(|| "link".into()),
            order: Some(
                if self.order == Some(HistoryOrder::Asc) {
                    "asc"
                } else {
                    "desc"
                }
                .into(),
            ),
            date_start: self.date_start,
            date_end: self.date_end,
            contains_attachment: self.contains_attachment,
        })
    }
}
fn date(raw: &str) -> Result<chrono::NaiveDate, SearchError> {
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| SearchError::InvalidData)
}
fn set<T>(slot: &mut Option<T>, value: T) -> Result<(), SearchError> {
    if slot.replace(value).is_some() {
        return Err(SearchError::InvalidData);
    }
    Ok(())
}
impl SearchNames<'_> {
    fn person(&self, raw: &str) -> Result<Uuid, SearchError> {
        let name = raw.trim_start_matches('@');
        if let Ok(id) = name.parse() {
            return Ok(id);
        }
        unique(
            self.resources
                .members
                .iter()
                .filter(|p| {
                    p.username
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(name))
                        || p.nickname
                            .as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case(name))
                })
                .filter_map(|p| p.id.as_deref().and_then(|id| id.parse().ok())),
        )
    }
    fn channel(&self, raw: &str) -> Result<Uuid, SearchError> {
        let name = raw.trim_start_matches('#');
        if let Ok(id) = name.parse() {
            return Ok(id);
        }
        unique(
            self.resources
                .channels
                .iter()
                .filter(|c| {
                    c.name
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(name))
                })
                .filter_map(|c| c.id.as_deref().and_then(|id| id.parse().ok())),
        )
    }
    /// Finds channel and person metadata by name without loading additional members/profiles.
    #[must_use]
    pub fn matching(&self, text: &str) -> (Vec<Uuid>, Vec<Uuid>) {
        let needle = text.to_lowercase();
        let channels = self
            .resources
            .channels
            .iter()
            .filter(|c| {
                c.name
                    .as_ref()
                    .is_some_and(|s| s.to_lowercase().contains(&needle))
            })
            .filter_map(|c| c.id.as_deref().and_then(|s| s.parse().ok()))
            .take(100)
            .collect();
        let people = self
            .resources
            .members
            .iter()
            .filter(|p| {
                [p.username.as_ref(), p.nickname.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|s| s.to_lowercase().contains(&needle))
            })
            .filter_map(|p| p.id.as_deref().and_then(|s| s.parse().ok()))
            .take(100)
            .collect();
        (channels, people)
    }
}
fn unique(mut ids: impl Iterator<Item = Uuid>) -> Result<Uuid, SearchError> {
    let id = ids.next().ok_or(SearchError::InvalidData)?;
    if ids.any(|next| next != id) {
        return Err(SearchError::InvalidData);
    }
    Ok(id)
}
fn tokens(raw: &str) -> Result<Vec<String>, SearchError> {
    let mut words = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in raw.chars() {
        if escaped {
            token.push(ch);
            escaped = false;
        } else if ch == '\\' && quoted {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            if !token.is_empty() {
                words.push(std::mem::take(&mut token));
            }
        } else {
            token.push(ch);
        }
    }
    if quoted || escaped {
        return Err(SearchError::InvalidData);
    }
    if !token.is_empty() {
        words.push(token);
    }
    Ok(words)
}
