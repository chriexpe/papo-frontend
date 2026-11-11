//! Owns channel-list history/read summaries; leaves authoritative resources, ordering and permission policy outside.

use papo_api::wire::{ChannelLastMessage, Message};
use serde_json::{Value, json};

use crate::{
    history::History,
    resources::Resources,
    response::{Reply, problem},
    server::State,
};

pub(crate) fn handle(state: &State, method: &str, path: &str, user: Option<&str>) -> Option<Reply> {
    if method != "GET" || path != "/channels" {
        return None;
    }
    Some(user.map_or_else(
        || problem(401, "unauthorized", path),
        |user| list(&state.resources, &state.history, user),
    ))
}

fn list(resources: &Resources, history: &History, user: &str) -> Reply {
    let mut channels: Vec<_> = resources.channels.values().collect();
    channels.sort_by(|left, right| (left.position, &left.id).cmp(&(right.position, &right.id)));
    let channels: Vec<_> = channels
        .into_iter()
        .map(|channel| {
            let id = channel.id.as_deref().unwrap_or_default();
            let mark = resources.read_marks.get(&(user.to_owned(), id.to_owned()));
            let mut value = json!(channel);
            value["last_message"] = history
                .latest(id)
                .map_or(Value::Null, |message| summary(resources, message));
            value["last_read_message"] = json!(mark.map(|mark| &mark.0));
            value["last_read_at"] = json!(mark.map(|mark| &mark.1));
            value
        })
        .collect();
    Reply::json(200, json!({"channels": channels}))
}

fn summary(resources: &Resources, message: &Message) -> Value {
    let mut value = json!(ChannelLastMessage {
        id: message.id.clone(),
        content: message.content.clone(),
        author_id: message.author_id.clone(),
        author_username: message
            .author_id
            .as_ref()
            .and_then(|id| resources.profiles.get(id))
            .and_then(|profile| profile.username.clone()),
        created_at: message.created_at.clone(),
    });
    // Go emits these nullable keys even though generated OpenAPI records omit None.
    for field in ["content", "author_id", "author_username"] {
        if value.get(field).is_none() {
            value[field] = Value::Null;
        }
    }
    value
}

#[cfg(test)]
mod tests;
