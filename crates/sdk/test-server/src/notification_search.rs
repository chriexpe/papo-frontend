//! Scripted account-scoped inbox/search pages and real push ownership shapes; not a search or permission engine.

use crate::{
    Error, TestServer,
    response::{Reply, problem},
    server::State,
};
use papo_api::{
    Uuid,
    wire::{
        MarkNotificationsReadRequest, Notification, PushDeviceRequest, SearchRequest, SearchResult,
    },
};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct NotificationSearch {
    inbox: BTreeMap<String, Vec<Notification>>,
    search: BTreeMap<String, Vec<SearchResult>>,
    requests: Vec<(String, SearchRequest, String)>,
    devices: BTreeMap<String, String>,
}
impl TestServer {
    /// Injects a one-shot REST failure after setup, without changing other accounts.
    /// # Errors
    /// Rejects invalid status or non-absolute route.
    pub async fn fail_next_rest(&self, method: &str, path: &str, status: u16) -> Result<(), Error> {
        if !(400..=599).contains(&status) || !path.starts_with('/') {
            return Err(Error::Configuration);
        }
        self.state.lock().await.rest.push_back(crate::RestStep {
            method: method.into(),
            path: path.into(),
            delay_ms: 0,
            failure: Some(crate::Failure {
                status,
                slug: "internal".into(),
            }),
        });
        Ok(())
    }

    /// Replaces one account's authoritative notification rows without generating live events.
    /// # Errors
    /// Returns malformed UUID/timestamp records.
    pub async fn set_notifications(
        &self,
        user: Uuid,
        rows: Vec<Notification>,
    ) -> Result<(), Error> {
        for row in &rows {
            key(row.created_at.as_deref(), row.id.as_deref()).ok_or(Error::Configuration)?;
        }
        let mut state = self.state.lock().await;
        for row in &rows {
            let id = row
                .message_id
                .as_deref()
                .and_then(|id| id.parse().ok())
                .ok_or(Error::Configuration)?;
            if !state.history.contains(id) {
                state.history.insert(papo_api::wire::Message {
                    id: row.message_id.clone(),
                    channel_id: row.channel_id.clone(),
                    author_id: row.author_id.clone(),
                    created_at: row.created_at.clone(),
                    content: row.message_content.clone(),
                    ..papo_api::wire::Message::default()
                })?;
            }
        }
        state
            .notification_search
            .inbox
            .insert(user.to_string(), rows);
        Ok(())
    }
    /// Scripts already-authorized backend search results; filters/full-text authorization are not simulated.
    /// # Errors
    /// Returns malformed tuple records.
    pub async fn set_search_results(
        &self,
        user: Uuid,
        rows: Vec<SearchResult>,
    ) -> Result<(), Error> {
        for row in &rows {
            key(row.created_at.as_deref(), row.id.as_deref()).ok_or(Error::Configuration)?;
        }
        self.state
            .lock()
            .await
            .notification_search
            .search
            .insert(user.to_string(), rows);
        Ok(())
    }
    /// Consumes account/body/exact query observations without logging contents.
    pub async fn take_search_requests(&self) -> Vec<(String, SearchRequest, String)> {
        std::mem::take(&mut self.state.lock().await.notification_search.requests)
    }
    /// Returns only the owner identity for an explicitly supplied test token.
    pub async fn push_owner(&self, token: &str) -> Option<String> {
        self.state
            .lock()
            .await
            .notification_search
            .devices
            .get(token)
            .cloned()
    }
}
fn key(at: Option<&str>, id: Option<&str>) -> Option<(i64, Uuid)> {
    Some((
        chrono::DateTime::parse_from_rfc3339(at?)
            .ok()?
            .timestamp_micros(),
        id?.parse().ok()?,
    ))
}
fn boundary(query: &str) -> Option<(i64, Uuid)> {
    let params = url::form_urlencoded::parse(query.as_bytes()).collect::<Vec<_>>();
    key(
        params
            .iter()
            .find(|(k, _)| k == "since")
            .map(|(_, v)| v.as_ref()),
        params
            .iter()
            .find(|(k, _)| k == "last_id")
            .map(|(_, v)| v.as_ref()),
    )
}
fn ascending(query: &str) -> bool {
    url::form_urlencoded::parse(query.as_bytes()).any(|(k, v)| k == "order" && v == "asc")
}
fn page<T: Clone>(
    rows: &[T],
    query: &str,
    asc: bool,
    key_of: impl Fn(&T) -> Option<(i64, Uuid)>,
) -> (Vec<T>, bool) {
    let boundary = boundary(query);
    let mut rows = rows
        .iter()
        .filter_map(|r| Some((key_of(r)?, r.clone())))
        .filter(|(key, _)| boundary.is_none_or(|b| if asc { *key > b } else { *key < b }))
        .collect::<Vec<_>>();
    rows.sort_by_key(|(key, _)| *key);
    if !asc {
        rows.reverse();
    }
    let more = rows.len() > 100;
    rows.truncate(100);
    (rows.into_iter().map(|(_, row)| row).collect(), more)
}
pub(crate) fn handle(
    state: &mut State,
    method: &str,
    path: &str,
    query: &str,
    user: Option<&str>,
    body: &[u8],
) -> Option<Reply> {
    let inbox_user = path
        .strip_prefix("/users/")
        .and_then(|s| s.strip_suffix("/notifications"));
    let read_user = path
        .strip_prefix("/users/")
        .and_then(|s| s.strip_suffix("/read_notification"));
    let relevant = path == "/search"
        || path == "/users/me/push-device"
        || inbox_user.is_some()
        || read_user.is_some();
    if !relevant {
        return None;
    }
    let Some(user) = user else {
        return Some(problem(401, "unauthorized", path));
    };
    let data = &mut state.notification_search;
    if method == "GET"
        && let Some(owner) = inbox_user
    {
        if owner != user {
            return Some(problem(403, "forbidden", path));
        }
        let (rows, more) = page(
            data.inbox.get(user).map_or(&[], Vec::as_slice),
            query,
            ascending(query),
            |r| key(r.created_at.as_deref(), r.id.as_deref()),
        );
        return Some(Reply::json(
            200,
            json!({"notifications":rows,"has_more":more}),
        ));
    }
    if method == "PUT"
        && let Some(owner) = read_user
    {
        if owner != user {
            return Some(problem(403, "forbidden", path));
        }
        let Ok(request) = serde_json::from_slice::<MarkNotificationsReadRequest>(body) else {
            return Some(problem(400, "invalid-param", path));
        };
        let mut marked = 0;
        if let Some(rows) = data.inbox.get_mut(user) {
            for row in rows {
                if row
                    .id
                    .as_ref()
                    .is_some_and(|id| request.notification_ids.contains(id))
                {
                    row.read = Some(true);
                    marked += 1;
                }
            }
        }
        return Some(Reply::json(200, json!({"updated":marked})));
    }
    if method == "POST" && path == "/search" {
        let Ok(request) = serde_json::from_slice::<SearchRequest>(body) else {
            return Some(problem(400, "invalid-param", path));
        };
        let asc = request.order.as_deref() == Some("asc");
        data.requests.push((user.into(), request, query.into()));
        let (rows, more) = page(
            data.search.get(user).map_or(&[], Vec::as_slice),
            query,
            asc,
            |r| key(r.created_at.as_deref(), r.id.as_deref()),
        );
        return Some(Reply::json(200, json!({"results":rows,"has_more":more})));
    }
    if path == "/users/me/push-device" {
        let Ok(request) = serde_json::from_slice::<PushDeviceRequest>(body) else {
            return Some(problem(400, "invalid-param", path));
        };
        if method == "PUT" {
            data.devices.insert(request.token, user.into());
            return Some(Reply::empty());
        }
        if method == "DELETE" {
            if data.devices.get(&request.token).map(String::as_str) != Some(user) {
                return Some(problem(404, "not-found", path));
            }
            data.devices.remove(&request.token);
            return Some(Reply::empty());
        }
    }
    Some(problem(404, "not-found", path))
}
