//! Deterministic message mutation fixture and payload-free request counts; production validation remains backend-owned.

use crate::{
    TestServer,
    response::{Reply, problem},
    server::State,
};
use papo_api::{
    Uuid,
    gateway::{KnownEvent, MessageDeleteEvent, MessageEditEvent, MessageEvent},
    wire::{Attachment, Message, UpdateMessageRequest},
};
use serde_json::json;
use std::collections::BTreeMap;

/// Counts only dispatch/acceptance, never credentials or content.
#[derive(Clone, Copy, Default, Debug)]
pub struct OutgoingRequestCounts {
    /// Mutation requests whose headers reached the fixture, including rejected ones.
    pub requests: u64,
    /// Successfully accepted message creations; identical requests increment separately.
    pub accepted: u64,
}
impl TestServer {
    /// Observes bounded payload-free dispatch counters.
    pub async fn outgoing_request_counts(&self) -> OutgoingRequestCounts {
        self.state.lock().await.outgoing
    }
}
pub(crate) fn relevant(method: &str, path: &str) -> bool {
    (path == "/messages" && method == "POST")
        || (path.starts_with("/messages/") && matches!(method, "PUT" | "DELETE"))
        || (path.ends_with("/reactions") && matches!(method, "POST" | "DELETE"))
}
pub(crate) fn handle(
    state: &mut State,
    method: &str,
    path: &str,
    user: Option<&str>,
    content_type: &str,
    body: &[u8],
) -> Option<Reply> {
    if !relevant(method, path) {
        return None;
    }
    let Some(user) = user else {
        return Some(problem(401, "unauthorized", path));
    };
    Some(if path == "/messages" {
        create(state, user, content_type, body, path)
    } else {
        mutate(state, user, method, path, body)
    })
}
struct Parts {
    fields: BTreeMap<String, String>,
    files: Vec<(String, Vec<u8>)>,
}
fn validate_create(state: &State, user: &str, parts: &Parts) -> Result<(), u16> {
    let channel = parts.fields.get("channel_id").ok_or(400_u16)?;
    if !state.resources.channels.contains_key(channel) {
        return Err(404);
    }
    if !state.history.readable(user, channel) {
        return Err(403);
    }
    let embeds = parts
        .fields
        .get("embeds")
        .map(|value| serde_json::from_str::<Vec<papo_api::wire::EmbedInput>>(value))
        .transpose()
        .map_err(|_| 400_u16)?
        .unwrap_or_default();
    if parts.fields.get("content").is_none_or(String::is_empty)
        && parts.files.is_empty()
        && embeds.is_empty()
    {
        return Err(400);
    }
    if let Some(reply) = parts.fields.get("reply_to") {
        let id = reply.parse::<Uuid>().map_err(|_| 400_u16)?;
        if state
            .history
            .message(id)
            .and_then(|message| message.channel_id.as_ref())
            != Some(channel)
        {
            return Err(404);
        }
    }
    Ok(())
}
fn parts(content_type: &str, body: &[u8]) -> Option<Parts> {
    let boundary = content_type
        .split(';')
        .map(str::trim)
        .find_map(|p| p.strip_prefix("boundary="))?
        .trim_matches('"');
    if boundary.is_empty() || boundary.len() > 200 {
        return None;
    }
    let delimiter = format!("\r\n--{boundary}").into_bytes();
    let prefix = format!("--{boundary}\r\n");
    let mut rest = body.strip_prefix(prefix.as_bytes())?;
    let mut parts = Parts {
        fields: BTreeMap::new(),
        files: Vec::new(),
    };
    for _ in 0..16 {
        let end = rest.windows(4).position(|v| v == b"\r\n\r\n")?;
        let headers = std::str::from_utf8(&rest[..end]).ok()?;
        rest = &rest[end + 4..];
        let end = rest.windows(delimiter.len()).position(|v| v == delimiter)?;
        let value = &rest[..end];
        let disposition = headers.lines().find(|line| {
            line.to_ascii_lowercase()
                .starts_with("content-disposition:")
        })?;
        let attr = |name: &str| {
            disposition
                .split(';')
                .map(str::trim)
                .find_map(|p| p.strip_prefix(name))
                .and_then(|p| p.strip_prefix('"'))
                .and_then(|p| p.strip_suffix('"'))
        };
        let name = attr("name=")?;
        if let Some(filename) = attr("filename=") {
            if name != "attachments" || parts.files.len() >= 10 {
                return None;
            }
            parts.files.push((filename.to_owned(), value.to_vec()));
        } else {
            parts
                .fields
                .insert(name.to_owned(), String::from_utf8(value.to_vec()).ok()?);
        }
        rest = &rest[end + delimiter.len()..];
        if rest.starts_with(b"--") {
            return Some(parts);
        }
        rest = rest.strip_prefix(b"\r\n")?;
    }
    None
}
fn create(state: &mut State, user: &str, content_type: &str, body: &[u8], path: &str) -> Reply {
    let Some(parts) = parts(content_type, body) else {
        return problem(400, "invalid-param", path);
    };
    let Some(channel) = parts.fields.get("channel_id") else {
        return problem(400, "invalid-param", path);
    };
    if let Err(status) = validate_create(state, user, &parts) {
        return problem(status, "invalid-param", path);
    }
    let Some(counter) = state.outgoing.accepted.checked_add(1) else {
        return problem(500, "internal", path);
    };
    let Some(created_at) = i64::try_from(counter)
        .ok()
        .and_then(|offset_us| 1_769_904_000_000_000_i64.checked_add(offset_us))
        .and_then(chrono::DateTime::from_timestamp_micros)
    else {
        return problem(500, "internal", path);
    };
    let id = Uuid::from_u128(0x9000_0000_0000_4000_8000_0000_0000_0000 + u128::from(counter))
        .to_string();
    let attachments = parts
        .files
        .iter()
        .enumerate()
        .map(|(index, (name, bytes))| Attachment {
            id: Some(
                Uuid::from_u128(
                    0xa000_0000_0000_4000_8000_0000_0000_0000
                        + u128::from(counter) * 10
                        + index as u128,
                )
                .to_string(),
            ),
            original_file_name: Some(name.clone()),
            size_bytes: i64::try_from(bytes.len()).ok(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let message = Message {
        id: Some(id.clone()),
        channel_id: Some(channel.clone()),
        author_id: Some(user.to_owned()),
        content: parts.fields.get("content").cloned(),
        reply_to: parts.fields.get("reply_to").cloned(),
        created_at: Some(created_at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)),
        attachments: Some(attachments.clone()),
        ..Default::default()
    };
    if state.history.insert(message.clone()).is_err() {
        return problem(500, "internal", path);
    }
    state.outgoing.accepted = counter;
    let event = KnownEvent::Message(Box::new(MessageEvent {
        id,
        channel_id: channel.clone(),
        author_id: user.to_owned(),
        content: message.content.clone().unwrap_or_default(),
        created_at: message.created_at.clone().unwrap_or_default(),
        reply_to: message.reply_to.clone(),
        attachments,
        ..Default::default()
    }));
    let _ = crate::history_controls::emit_channel(state, channel, event);
    Reply::json(201, json!(message))
}
fn mutate(state: &mut State, user: &str, method: &str, path: &str, body: &[u8]) -> Reply {
    let fields: Vec<_> = path.split('/').filter(|s| !s.is_empty()).collect();
    if fields.first() == Some(&"messages") {
        let Some(id) = fields.get(1).and_then(|id| id.parse::<Uuid>().ok()) else {
            return problem(400, "invalid-param", path);
        };
        let Some(message) = state.history.message(id) else {
            return problem(404, "not-found", path);
        };
        if message.author_id.as_deref() != Some(user) {
            return problem(403, "forbidden", path);
        }
        let channel = message.channel_id.clone().unwrap_or_default();
        if method == "DELETE" {
            state.history.remove(id);
            let _ = crate::history_controls::emit_channel(
                state,
                &channel,
                KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                    id: id.to_string(),
                    channel_id: channel.clone(),
                })),
            );
            return Reply::empty();
        }
        let Ok(request) = serde_json::from_slice::<UpdateMessageRequest>(body) else {
            return problem(400, "invalid-param", path);
        };
        let Some(message) = state.history.message_mut(id) else {
            return problem(404, "not-found", path);
        };
        message.content = Some(request.content.clone());
        message.edited_at = Some("2026-02-02T00:00:00Z".into());
        let reply = Reply::json(200, json!(message));
        let _ = crate::history_controls::emit_channel(
            state,
            &channel,
            KnownEvent::MessageEdit(Box::new(MessageEditEvent {
                id: id.to_string(),
                channel_id: channel.clone(),
                content: request.content,
                edited_at: "2026-02-02T00:00:00Z".into(),
            })),
        );
        return reply;
    }
    // SDK tests assert the resolved message ID via the same authoritative history map.
    let Some(id) = fields.get(3).and_then(|id| id.parse::<Uuid>().ok()) else {
        return problem(400, "invalid-param", path);
    };
    if !state.history.contains(id) {
        return problem(404, "not-found", path);
    }
    if method == "DELETE" {
        Reply::empty()
    } else {
        Reply::json(201, json!({}))
    }
}

#[cfg(test)]
mod tests {
    //! Byte parser tests use synthetic binary parts, with no network or clock.
    use super::*;
    #[test]
    fn multipart_keeps_binary_bytes_and_repeated_file_order() {
        let body=b"--b\r\nContent-Disposition: form-data; name=\"attachments\"; filename=\"one\"\r\n\r\n\0\xff\r\n--b\r\nContent-Disposition: form-data; name=\"attachments\"; filename=\"two\"\r\n\r\n\x01\r\n--b--\r\n";
        let parts = parts("multipart/form-data; boundary=b", body).unwrap();
        assert_eq!(
            parts.files,
            vec![("one".into(), vec![0, 255]), ("two".into(), vec![1])]
        );
        assert!(super::parts("multipart/form-data", body).is_none());
    }
}
