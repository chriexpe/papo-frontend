//! Desktop OS notifications (`docs/design.md` §8, "D9 additions"): consumes
//! claimed deliveries from each open SDK session (`NotificationService`),
//! formats title and body according to the approved notification text rules,
//! posts them through `rune_notify` (Notification portal in Flatpak,
//! `org.freedesktop.Notifications` on the host), and tracks posted IDs per
//! channel so they can be cleared when read or marked all read.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use papo_sdk::{
    AccountId, Attachment, ClientHandle, Message, NotificationDelivery, NotificationTarget,
    ResourceSnapshot, Session, Uuid,
};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::snippet;
use tokio::task::{AbortHandle, JoinSet};

use crate::strings;
use crate::timeline::replies::snippet_line;

/// Info keys carried on posted OS notifications for click routing.
const INFO_SERVER: &str = "server";
const INFO_USER: &str = "user";
const INFO_CHANNEL: &str = "channel";
const INFO_MESSAGE: &str = "message";
const INFO_DM: &str = "dm";

/// Formatted title and body ready for `rune_notify::Notification`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FormattedNotification {
    pub(crate) title: String,
    pub(crate) body: String,
}

/// Inputs to [`format_notification`], kept pure for unit tests.
pub(crate) struct FormatInput<'a> {
    pub(crate) author: Option<&'a str>,
    pub(crate) channel: Option<&'a str>,
    pub(crate) server: &'a str,
    pub(crate) multi_server: bool,
    pub(crate) is_dm: bool,
    pub(crate) preview: Option<&'a str>,
    pub(crate) attachments: &'a [Attachment],
    pub(crate) resources: Option<&'a ResourceSnapshot>,
}

/// Formats a notification's title and body per `docs/design.md` §8 "D9 additions":
/// - Channel title: `{author} · #{channel}` (plus ` ({server})` when 2+ servers are saved)
/// - DM title: `{author}` (plus ` ({server})` when 2+ servers are saved)
/// - Body with preview on: decoded message/attachment snippet
/// - Body with preview off: "New message" / "Direct message" (sender kept in title)
pub(crate) fn format_notification(input: &FormatInput<'_>) -> FormattedNotification {
    let author = input
        .author
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| strings::someone());
    let mut title = if input.is_dm {
        author.to_owned()
    } else if let Some(channel) = input.channel.map(str::trim).filter(|name| !name.is_empty()) {
        format!("{author} · #{channel}")
    } else {
        author.to_owned()
    };
    let server = input.server.trim();
    if input.multi_server && !server.is_empty() {
        title.push_str(" (");
        title.push_str(server);
        title.push(')');
    }

    let fallback = || {
        if input.is_dm {
            strings::notification_direct_message().to_owned()
        } else {
            strings::notification_new_message().to_owned()
        }
    };

    let body = match input.preview {
        Some(raw) => {
            let message = Message {
                content: Some(raw.to_owned()),
                attachments: (!input.attachments.is_empty()).then(|| input.attachments.to_vec()),
                ..Message::default()
            };
            let parsed = MessageContent::parse(raw);
            let snip = snippet(&message, &parsed);
            let line = snippet_line(&snip, input.resources).text;
            let collapsed = collapse_whitespace(&line);
            if collapsed.is_empty() {
                fallback()
            } else {
                collapsed
            }
        }
        None => fallback(),
    };

    FormattedNotification { title, body }
}

/// Collapses newlines and runs of whitespace into single spaces for a one-line
/// notification body.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Navigation target parsed from a clicked OS notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClickTarget {
    /// Hex-encoded `ServerId`.
    pub(crate) server: String,
    pub(crate) user: Uuid,
    pub(crate) channel: Uuid,
    pub(crate) message: Uuid,
    pub(crate) is_dm: bool,
}

/// Parses a `rune_notify::Click` into a [`ClickTarget`].
pub(crate) fn parse_click(click: &rune_notify::Click) -> Option<ClickTarget> {
    let server = click.get(INFO_SERVER)?.to_owned();
    if server.is_empty() {
        return None;
    }
    let user = Uuid::parse_str(click.get(INFO_USER)?).ok()?;
    let channel = Uuid::parse_str(click.get(INFO_CHANNEL)?).ok()?;
    let message = Uuid::parse_str(click.get(INFO_MESSAGE)?).ok()?;
    let is_dm = click.get(INFO_DM).is_some_and(|flag| flag == "1");
    Some(ClickTarget {
        server,
        user,
        channel,
        message,
        is_dm,
    })
}

/// Stable OS notification identifier for a message delivery.
fn notification_id(target: &NotificationTarget) -> String {
    format!("papo-{}-{}", target.channel, target.message)
}

/// Tracks posted OS notification IDs per `(AccountId, channel)` so they can be
/// dismissed when a channel becomes visible or is marked read.
#[derive(Default)]
struct PostedLedger {
    by_channel: HashMap<(AccountId, Uuid), HashSet<String>>,
}

static LEDGER: LazyLock<Mutex<PostedLedger>> =
    LazyLock::new(|| Mutex::new(PostedLedger::default()));

fn record_posted(account: AccountId, channel: Uuid, id: String) {
    let mut ledger = LEDGER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ledger
        .by_channel
        .entry((account, channel))
        .or_default()
        .insert(id);
}

/// Dismisses any OS notifications previously posted for `(account, channel)`.
pub(crate) fn clear_channel(account: AccountId, channel: Uuid) {
    let ids: Vec<String> = {
        let mut ledger = LEDGER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ledger
            .by_channel
            .remove(&(account, channel))
            .map(|set| set.into_iter().collect())
            .unwrap_or_default()
    };
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    if !refs.is_empty() {
        rune_notify::remove(&refs);
    }
}

/// Dismisses any OS notifications previously posted for `account`.
pub(crate) fn clear_account(account: AccountId) {
    let ids: Vec<String> = {
        let mut ledger = LEDGER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let keys: Vec<(AccountId, Uuid)> = ledger
            .by_channel
            .keys()
            .copied()
            .filter(|(acc, _)| *acc == account)
            .collect();
        let mut removed = Vec::new();
        for key in keys {
            if let Some(set) = ledger.by_channel.remove(&key) {
                removed.extend(set);
            }
        }
        removed
    };
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    if !refs.is_empty() {
        rune_notify::remove(&refs);
    }
}

/// Runs until the client closes: starts one notification delivery consumer per
/// open session, ensures desktop preview policy defers to the account setting,
/// and posts each claimed delivery to `rune_notify`.
pub(crate) async fn run(client: ClientHandle) {
    let mut sessions = client.subscribe_session_changes();
    let mut tasks = JoinSet::new();
    let mut running: HashMap<AccountId, AbortHandle> = HashMap::new();
    loop {
        sessions.borrow_and_update();
        let registry = Arc::clone(&client.subscribe_registry().borrow());
        let open: Vec<Session> = registry
            .accounts
            .iter()
            .filter_map(|account| client.session(account.id).ok())
            .collect();
        running.retain(|account, task| {
            let kept = open.iter().any(|session| session.account() == *account);
            if !kept {
                task.abort();
                clear_account(*account);
            }
            kept && !task.is_finished()
        });
        for session in open {
            let account = session.account();
            if let std::collections::hash_map::Entry::Vacant(slot) = running.entry(account) {
                slot.insert(tasks.spawn(consume_session(client.clone(), session)));
            }
        }
        loop {
            tokio::select! {
                changed = sessions.changed() => {
                    if changed.is_err() {
                        for account in running.keys().copied() {
                            clear_account(account);
                        }
                        return;
                    }
                    break;
                }
                Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
            }
        }
    }
}

async fn consume_session(client: ClientHandle, session: Session) {
    let service = session.notifications();
    // Desktop delegates "Show the message in the notification" to the account's
    // `notifications.messagePreview`; enable the device-side preview gate if still
    // at its mobile default (`false`).
    if let Ok(mut settings) = service.settings().await
        && !settings.preview
    {
        settings.preview = true;
        if let Err(error) = service.set_settings(settings).await {
            log::warn!("desktop notification preview flag was not saved: {error}");
        }
    }
    loop {
        let delivery = match service.next_delivery().await {
            Ok(delivery) => delivery,
            Err(error) => {
                log::debug!("notification consumer ended: {error}");
                return;
            }
        };
        if service.validate_target(&delivery.target).is_err() {
            continue;
        }
        post_delivery(&client, &session, &delivery);
    }
}

fn post_delivery(client: &ClientHandle, session: &Session, delivery: &NotificationDelivery) {
    let snapshot = Arc::clone(&session.subscribe_resources().borrow());
    let registry = Arc::clone(&client.subscribe_registry().borrow());
    let multi_server = registry.servers.len() > 1;
    let account = delivery.target.account;
    let server_name = snapshot
        .server
        .as_deref()
        .and_then(|server| server.name.as_deref())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            registry
                .servers
                .iter()
                .find(|saved| saved.address.id() == account.server())
                .map(|saved| crate::sign_in::shown_address(saved.address.as_str()).to_owned())
        })
        .unwrap_or_default();
    let channel_name = (!delivery.is_dm)
        .then(|| {
            snapshot
                .channels
                .get(delivery.target.channel)
                .and_then(|channel| channel.name.as_deref())
                .map(str::to_owned)
        })
        .flatten();
    let author_name = delivery.author_name.clone().or_else(|| {
        let author = delivery.author?;
        let user = snapshot.users.get(author)?;
        papo_sdk_ui::display_name(user.nickname.as_deref(), user.username.as_deref())
            .map(str::to_owned)
    });
    let formatted = format_notification(&FormatInput {
        author: author_name.as_deref(),
        channel: channel_name.as_deref(),
        server: &server_name,
        multi_server,
        is_dm: delivery.is_dm,
        preview: delivery.preview.as_deref(),
        attachments: &delivery.attachments,
        resources: Some(&snapshot),
    });
    let id = notification_id(&delivery.target);
    let notification = rune_notify::Notification::new(&id, formatted.title)
        .body(formatted.body)
        .sound(delivery.sound)
        .info(INFO_SERVER, account.server().to_string())
        .info(INFO_USER, account.user().to_string())
        .info(INFO_CHANNEL, delivery.target.channel.to_string())
        .info(INFO_MESSAGE, delivery.target.message.to_string())
        .info(INFO_DM, if delivery.is_dm { "1" } else { "0" });
    if !rune_notify::post(&notification) {
        log::debug!("OS notification was not posted (notifications unavailable)");
        return;
    }
    record_posted(account, delivery.target.channel, id);
}

#[cfg(test)]
mod tests {
    use papo_sdk::{Attachment, Uuid};
    use rune_notify::Click;

    use super::{
        ClickTarget, FormatInput, INFO_CHANNEL, INFO_DM, INFO_MESSAGE, INFO_SERVER, INFO_USER,
        format_notification, parse_click,
    };

    #[test]
    fn channel_notification_formats_title_and_decoded_body() {
        let formatted = format_notification(&FormatInput {
            author: Some("Lia"),
            channel: Some("general"),
            server: "Lumen",
            multi_server: false,
            is_dm: false,
            preview: Some("hello ||secret|| world\nsecond line"),
            attachments: &[],
            resources: None,
        });
        assert_eq!(formatted.title, "Lia · #general");
        assert_eq!(formatted.body, "hello ▮▮▮ world");
    }

    #[test]
    fn multi_server_appends_server_name_for_channel_and_dm() {
        let channel = format_notification(&FormatInput {
            author: Some("Lia"),
            channel: Some("general"),
            server: "Lumen",
            multi_server: true,
            is_dm: false,
            preview: Some("ping"),
            attachments: &[],
            resources: None,
        });
        assert_eq!(channel.title, "Lia · #general (Lumen)");

        let dm = format_notification(&FormatInput {
            author: Some("Lia"),
            channel: None,
            server: "Lumen",
            multi_server: true,
            is_dm: true,
            preview: Some("hi"),
            attachments: &[],
            resources: None,
        });
        assert_eq!(dm.title, "Lia (Lumen)");
        assert_eq!(dm.body, "hi");
    }

    #[test]
    fn preview_off_keeps_sender_and_uses_generic_body() {
        let channel = format_notification(&FormatInput {
            author: Some("Lia"),
            channel: Some("general"),
            server: "Lumen",
            multi_server: false,
            is_dm: false,
            preview: None,
            attachments: &[],
            resources: None,
        });
        assert_eq!(channel.title, "Lia · #general");
        assert_eq!(channel.body, "New message");

        let dm = format_notification(&FormatInput {
            author: Some("Lia"),
            channel: None,
            server: "Lumen",
            multi_server: false,
            is_dm: true,
            preview: None,
            attachments: &[],
            resources: None,
        });
        assert_eq!(dm.title, "Lia");
        assert_eq!(dm.body, "Direct message");
    }

    #[test]
    fn attachment_only_message_uses_media_snippet_when_preview_on() {
        let photo = Attachment {
            id: Some(Uuid::from_u128(1).to_string()),
            original_file_name: Some("sunset.png".to_owned()),
            mime_type: Some("image/png".to_owned()),
            ..Attachment::default()
        };
        let formatted = format_notification(&FormatInput {
            author: Some("Lia"),
            channel: Some("media"),
            server: "Lumen",
            multi_server: false,
            is_dm: false,
            preview: Some(""),
            attachments: &[photo],
            resources: None,
        });
        assert_eq!(formatted.body, "Image");

        let doc = Attachment {
            id: Some(Uuid::from_u128(2).to_string()),
            original_file_name: Some("report.pdf".to_owned()),
            mime_type: Some("application/pdf".to_owned()),
            ..Attachment::default()
        };
        let formatted = format_notification(&FormatInput {
            author: Some("Lia"),
            channel: Some("media"),
            server: "Lumen",
            multi_server: false,
            is_dm: false,
            preview: Some(""),
            attachments: &[doc],
            resources: None,
        });
        assert_eq!(formatted.body, "File · report.pdf");
    }

    #[test]
    fn click_info_round_trips() {
        let user = Uuid::from_u128(10);
        let channel = Uuid::from_u128(20);
        let message = Uuid::from_u128(30);
        let click = Click {
            id: "papo-test".to_owned(),
            info: vec![
                (
                    INFO_SERVER.to_owned(),
                    "00112233445566778899aabbccddeeff".to_owned(),
                ),
                (INFO_USER.to_owned(), user.to_string()),
                (INFO_CHANNEL.to_owned(), channel.to_string()),
                (INFO_MESSAGE.to_owned(), message.to_string()),
                (INFO_DM.to_owned(), "1".to_owned()),
            ],
        };
        assert_eq!(
            parse_click(&click),
            Some(ClickTarget {
                server: "00112233445566778899aabbccddeeff".to_owned(),
                user,
                channel,
                message,
                is_dm: true,
            })
        );
    }
}
