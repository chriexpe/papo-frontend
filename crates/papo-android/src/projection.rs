//! Read-only snapshots for the Android presentation layer. Timeline, pins and
//! search use the same message projection so media and reply metadata agree.
//! Projection must not hydrate channels or mutate reading/navigation state.
use papo_core::api::models::SearchResult;
use papo_core::api::ws::Connection;
use papo_core::cache::OutgoingState;
use papo_core::state::{ChannelKind, Emoji, Member, Message, Screen, Store};
use serde_json::{Value, json};

use crate::{discovery, markdown, media};

fn blob_revision(blob: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    blob.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

pub(crate) fn project_message(store: &Store, url: &str, message: &Message) -> Value {
    let author = message
        .external_author
        .as_ref()
        .map(|author| author.name.as_str())
        .or_else(|| {
            store
                .member(&message.author_id)
                .map(|user| user.name.as_str())
        })
        .unwrap_or(&message.author_id);
    let reactions: Vec<_> = message
        .reactions
        .iter()
        .map(|reaction| {
            json!({
                "emoji": match &reaction.emoji {
                    Emoji::Unicode(value) | Emoji::Custom(value) => value,
                },
                "count": reaction.count,
                "mine": reaction.mine,
                "custom": matches!(reaction.emoji, Emoji::Custom(_)),
            })
        })
        .collect();
    let external_author = message.external_author.as_ref().map(|author| {
        json!({
            "source": author.source,
            "id": author.id,
            "name": author.name,
            "avatarUrl": author.avatar_url.as_deref().unwrap_or_default(),
            "profileUrl": author.profile_url.as_deref().unwrap_or_default(),
        })
    });
    let delivery = store
        .outgoing_state(&message.id)
        .map(|state| match state {
            OutgoingState::Queued => "queued",
            OutgoingState::Sending => "sending",
            OutgoingState::UnknownOutcome => "unknown",
            OutgoingState::FailedPermanent => "failed",
        })
        .unwrap_or_default();
    let reply_content = message
        .reply_to
        .as_ref()
        .and_then(|id| store.message(id))
        .map(|message| message.content.as_str())
        .unwrap_or_default();
    json!({
        "id": message.id,
        "authorId": message.author_id,
        "author": author,
        "content": message.content,
        "pinned": message.pinned,
        "markdown": markdown::document(store, url, &message.content).as_ref(),
        "externalAuthor": external_author,
        "at": message.at.timestamp_millis(),
        "edited": message.edited,
        "mine": message.mine(&store.me),
        "canEdit": store.can_edit_message(message),
        "canDelete": store.can_delete_message(message),
        "replyTo": message.reply_to.as_deref().unwrap_or_default(),
        "replyContent": reply_content,
        "pending": message.pending,
        "delivery": delivery,
        "reactions": reactions,
        "attachments": message.attachments.iter().map(media::AttachmentView::from).collect::<Vec<_>>(),
        "embeds": message.previews.iter().map(media::EmbedView::from).collect::<Vec<_>>(),
    })
}

fn project_profile(store: &Store, user: &Member) -> Value {
    let details = store.profiles.get(&user.id);
    let revision = store
        .avatars
        .get(&user.id)
        .map(|blob| blob_revision(blob))
        .unwrap_or_default();
    let roles = details
        .map(|profile| {
            profile
                .roles
                .iter()
                .map(|role| {
                    json!({
                        "id": role.id,
                        "name": role.name,
                        "color": role.color.as_deref().unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "id": user.id,
        "avatarRevision": revision,
        "loaded": details.is_some(),
        "banner": details.and_then(|profile| profile.banner.as_deref()).unwrap_or_default(),
        "description": details.and_then(|profile| profile.description.as_deref()).unwrap_or_default(),
        "createdAt": details.and_then(|profile| profile.created_at).map(|at| at.to_rfc3339()).unwrap_or_default(),
        "statusMessage": user.status_message.as_deref().unwrap_or_default(),
        "roles": roles,
    })
}

fn project_search_result(store: &Store, url: &str, result: &SearchResult) -> Value {
    let message = store
        .search_message(&result.id)
        .filter(|message| message.channel_id == result.channel_id)
        .map(|message| project_message(store, url, message));
    let author = result
        .author_id
        .as_ref()
        .and_then(|id| store.member(id))
        .map(|user| user.name.as_str())
        .or(result.author_username.as_deref())
        .unwrap_or_default();
    json!({
        "message": message,
        "id": result.id,
        "kind": result.kind,
        "content": store.display_mentions(&result.content),
        "channelId": result.channel_id,
        "channelName": result.channel_name,
        "authorId": result.author_id.as_deref().unwrap_or_default(),
        "author": author,
        "at": result.created_at.map(|at| at.timestamp_millis()).unwrap_or_default(),
    })
}

fn project_reaction_details(store: &Store) -> Vec<Value> {
    store.reaction_details.iter().map(|(id, groups)| {
        let groups: Vec<_> = groups.iter().map(|group| {
            let users: Vec<_> = group.users.iter().map(|user| {
                json!({
                    "id": user.user_id,
                    "name": store.member(&user.user_id).map(|member| member.name.as_str()).unwrap_or(&user.user_id),
                })
            }).collect();
            json!({
                "emoji": group.emoji_id.as_ref().or(group.unicode.as_ref()).cloned().unwrap_or_default(),
                "custom": group.emoji_id.is_some(),
                "users": users,
            })
        }).collect();
        json!({
            "messageId": id,
            "groups": groups,
            "more": store.reaction_details_has_more.get(id).copied().unwrap_or(false),
        })
    }).collect()
}

fn project_reading(store: &Store, url: &str) -> Value {
    let channel = &store.selected_channel;
    let range = store.hourglass_range(channel);
    let initializing = store
        .pending_remote_read_anchors()
        .iter()
        .any(|(pending, _)| pending == channel);
    json!({
        "gapStart": if store.catch_up_needs_history(channel) {
            String::new()
        } else {
            range.as_ref().map(|range| range.0.clone()).unwrap_or_default()
        },
        "gapEnd": range.map(|range| range.1).unwrap_or_default(),
        "hourglassTarget": store.hourglass_navigation_target(channel).unwrap_or_default(),
        "unreadStart": store.first_badge_unread(channel).unwrap_or_default(),
        "mentions": store.unread_mention_ids(channel),
        "syncPending": !url.starts_with("demo:") && (
            !store.pending_read_receipts().is_empty()
            || !store.pending_notification_reads().is_empty()
            || initializing
        ),
        "initializing": initializing,
        "liveIds": store.messages.iter()
            .filter(|message| message.channel_id == *channel && !message.pending)
            .map(|message| message.id.clone()).collect::<Vec<_>>(),
    })
}

pub(crate) fn project(store: &Store, url: &str, key: &str) -> Value {
    let channels: Vec<_> = store
        .channels
        .iter()
        .map(|channel| {
            json!({
                "id": channel.id,
                "name": channel.name,
                "kind": match channel.kind {
                    ChannelKind::Text => "text",
                    ChannelKind::Voice => "voice",
                    ChannelKind::Category => "category",
                    ChannelKind::Direct => "direct",
                },
                "topic": channel.topic.as_deref().unwrap_or_default(),
                "unread": channel.unread,
                "mentions": channel.mentions,
                "notificationSettings": channel.notification_settings,
            })
        })
        .collect();
    let messages: Vec<_> = store
        .messages_in(&store.selected_channel)
        .map(|message| project_message(store, url, message))
        .collect();
    let members: Vec<_> = store.members.iter().map(|user| {
        json!({
            "id": user.id,
            "name": user.name,
            "username": user.username,
            "roleColor": user.role_color.map(|[r, g, b]| format!("#{r:02X}{g:02X}{b:02X}")).unwrap_or_default(),
            "presence": format!("{:?}", user.presence).to_lowercase(),
        })
    }).collect();
    let dms: Vec<_> = store
        .direct_messages
        .iter()
        .map(|dm| {
            json!({
                "id": dm.id,
                "userId": dm.user.id,
                "name": dm.user.display_name(),
                "kind": "direct",
                "topic": "",
                "unread": store.channel(&dm.id).is_some_and(|channel| channel.unread),
                "mentions": store.channel(&dm.id).map_or(0, |channel| channel.mentions),
            })
        })
        .collect();
    let roles: Vec<_> = store
        .roles
        .iter()
        .map(|role| {
            json!({
                "id": role.id,
                "name": role.name,
                "color": role.color.as_deref().unwrap_or_default(),
                "permissions": discovery::Capabilities::from(role.permissions),
            })
        })
        .collect();
    let emojis: Vec<_> = store
        .emojis
        .iter()
        .map(|emoji| {
            json!({
                "id": emoji.id,
                "name": emoji.name,
                "revision": emoji.blob.as_deref().map(blob_revision).unwrap_or_default(),
            })
        })
        .collect();
    let mut snapshot = json!({
        "version": 1,
        "url": url,
        "key": key,
        "screen": match store.screen {
            Screen::Starting => "starting",
            Screen::Auth => "auth",
            Screen::NeedsServer => "needs_server",
            Screen::Chat => "chat",
        },
        "connection": match store.connection {
            Connection::Online => "online",
            Connection::Connecting => "connecting",
            Connection::Offline => "offline",
        },
        "serverName": store.server.as_ref().map(|server| server.name.as_str()).unwrap_or(url),
        "serverDescription": store.server.as_ref().and_then(|server| server.description.as_deref()).unwrap_or_default(),
        "serverIconRevision": store.server.as_ref().and_then(|server| server.icon.as_deref()).map(blob_revision).unwrap_or_default(),
        "serverPublic": store.server.as_ref().is_some_and(|server| server.public),
        "capabilities": discovery::Capabilities::from_store(store),
        "canResetPasswords": store.is_server_owner(),
        "roles": roles,
        "searchQuery": "",
        "searching": store.searching,
        "searchHasMore": store.search_has_more,
        "searchResults": store.search_results.iter().map(|result| project_search_result(store, url, result)).collect::<Vec<_>>(),
        "notificationCount": store.pending_notification_count(),
        "me": store.me,
        "myName": store.my_name,
        "selectedChannel": store.selected_channel,
        "channels": channels,
        "emojis": emojis,
        "reactionDetails": project_reaction_details(store),
        "reactionLoading": store.reaction_details_loading.iter().collect::<Vec<_>>(),
        "profiles": store.members.iter().map(|user| project_profile(store, user)).collect::<Vec<_>>(),
        "messages": messages,
        "members": members,
        "dms": dms,
        "typing": store.typing_names(),
        "locked": store.locked,
        "busy": store.busy,
        "error": store.error.as_deref().unwrap_or_default(),
        "canLoadOlder": store.can_load_older(&store.selected_channel),
        "browsingHistory": store.browsing_history(&store.selected_channel),
        "loadingContext": store.loading_message_context(&store.selected_channel),
        "canLoadNewer": store.can_load_newer(&store.selected_channel),
        "navigationSerial": 0,
        "navigationTarget": "",
        "reading": project_reading(store, url),
        "loadingOlder": store.loading_older(&store.selected_channel),
        "timeline": format!("{:?}", store.timeline_status(&store.selected_channel)).to_lowercase(),
    });
    snapshot["notificationsEnabled"] = json!(
        store
            .user_settings
            .as_ref()
            .is_none_or(|settings| settings.config.notifications.enabled)
    );
    snapshot["pinnedMessages"] = json!(
        store
            .pinned_in(&store.selected_channel)
            .into_iter()
            .map(|message| project_message(store, url, message))
            .collect::<Vec<_>>()
    );
    snapshot["pinsLoading"] = json!(store.loading_pinned(&store.selected_channel));
    snapshot["pinsError"] = json!(
        store
            .pin_errors
            .get(&store.selected_channel)
            .cloned()
            .unwrap_or_default()
    );
    snapshot["pinsKnown"] = json!(store.pinned_messages.contains_key(&store.selected_channel));
    snapshot["canPin"] = json!(store.can_pin_in(&store.selected_channel));
    snapshot["typingPhrase"] = json!(store.typing_phrase().unwrap_or_default());
    snapshot
}
