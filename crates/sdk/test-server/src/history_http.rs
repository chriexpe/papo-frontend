//! Owns authenticated history and pinned-list REST responses; leaves fixture mutations and permission policy outside.

use papo_api::wire::{Message, MessageListResponse, PinnedMessageList};
use serde_json::{Value, json};

use crate::{
    history_query::HistoryQuery,
    response::{Reply, problem},
    server::State,
};

pub(crate) fn handle(
    state: &State,
    method: &str,
    path: &str,
    query: &str,
    user: Option<&str>,
) -> Option<Reply> {
    if method != "GET" {
        return None;
    }
    let tail = path.strip_prefix("/channels/")?;
    let (channel, route) = tail.split_once('/')?;
    if !matches!(route, "messages" | "pinned") {
        return None;
    }
    let Some(user) = user else {
        return Some(problem(401, "unauthorized", path));
    };
    let parsed = if route == "messages" {
        match HistoryQuery::parse(query) {
            Ok(query) => Some(query),
            Err(status) => {
                return Some(problem(
                    status,
                    if status == 500 {
                        "internal"
                    } else {
                        "invalid-param"
                    },
                    path,
                ));
            }
        }
    } else {
        None
    };
    if !state.resources.channels.contains_key(channel) {
        return Some(problem(404, "not-found", path));
    }
    if !state.history.readable(user, channel) {
        return Some(problem(403, "forbidden", path));
    }
    if parsed.as_ref().is_some_and(|query| query.invalid_last_id) {
        return Some(problem(500, "internal", path));
    }
    let body = if let Some(query) = parsed {
        let (messages, has_more) = state.history.page(channel, user, &query);
        let mut body = json!(MessageListResponse {
            channel_id: Some(channel.to_owned()),
            messages: Some(Vec::new()),
            has_more: Some(has_more),
        });
        body["messages"] = messages.iter().map(wire_message).collect();
        body
    } else {
        let mut body = json!(PinnedMessageList {
            channel_id: Some(channel.to_owned()),
            pinned: Some(Vec::new())
        });
        body["pinned"] = state
            .history
            .pinned(channel, user)
            .iter()
            .map(wire_message)
            .collect();
        body
    };
    Some(Reply::json(200, body))
}

fn wire_message(message: &Message) -> Value {
    let mut value = json!(message);
    // Generated OpenAPI records omit None; Go's model requires these keys as null.
    for field in [
        "author_id",
        "content",
        "edited_at",
        "reply_to",
        "embeds",
        "reactions",
        "user_reactions",
    ] {
        if value.get(field).is_none() {
            value[field] = Value::Null;
        }
    }
    if let Some(attachments) = value["attachments"].as_array_mut() {
        for attachment in attachments {
            if attachment.get("thumbnail_id").is_none() {
                attachment["thumbnail_id"] = Value::Null;
            }
        }
    }
    for field in ["reactions", "user_reactions"] {
        if let Some(reactions) = value[field].as_array_mut() {
            for reaction in reactions {
                for key in ["emoji_id", "unicode"] {
                    if reaction.get(key).is_none() {
                        reaction[key] = Value::Null;
                    }
                }
            }
        }
    }
    value
}
