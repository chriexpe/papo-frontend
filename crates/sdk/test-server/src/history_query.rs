//! Owns pinned-backend history query semantics; leaves storage and timestamp-string preservation outside.

use crate::history::{Key, instant};

pub(crate) struct HistoryQuery {
    pub(crate) ascending: bool,
    pub(crate) limit: usize,
    pub(crate) invalid_last_id: bool,
    since: Option<i128>,
    last_id: Option<papo_api::Uuid>,
}

impl HistoryQuery {
    pub(crate) fn parse(query: &str) -> Result<Self, u16> {
        let params: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
        // Echo's QueryParam uses the first value, including an explicitly empty value.
        let get = |key: &str| {
            params
                .iter()
                .find(|(name, _)| name == key)
                .map_or("", |(_, value)| value.as_ref())
        };
        let since = match get("since") {
            "" => None,
            value => Some(instant(value).map_err(|_| 400_u16)?),
        };
        let limit = match get("limit") {
            "" => 30,
            value => value
                .parse::<usize>()
                .ok()
                .filter(|limit| (1..=100).contains(limit))
                .ok_or(400_u16)?,
        };
        let ascending = match get("order") {
            "" | "desc" => false,
            "asc" => true,
            _ => return Err(400),
        };
        // The backend passes last_id to PostgreSQL only with since. Invalid UUIDs there
        // surface as an internal error, not the handler's invalid-param error.
        let use_last_id = since.is_some() && !get("last_id").is_empty();
        let last_id = use_last_id
            .then(|| papo_api::Uuid::parse_str(get("last_id")).ok())
            .flatten();
        let invalid_last_id = use_last_id && last_id.is_none();
        Ok(Self {
            ascending,
            limit,
            invalid_last_id,
            since,
            last_id,
        })
    }

    pub(crate) fn includes(&self, key: Key) -> bool {
        match (self.since, self.last_id) {
            (Some(since), Some(id)) if self.ascending => key > (since, id),
            (Some(since), Some(id)) => key < (since, id),
            (Some(since), None) => key.0 > since,
            (None, _) => true,
        }
    }
}
