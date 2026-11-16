//! REST identities, message sends and reading-state waits for the badge proof.

use papo_api::{
    MessageUpload, RestClient,
    wire::{CreateChannelRequest, LoginRequest, RegisterRequest},
};
use papo_sdk::{ConnectionState, ReadState, Session, UnreadCount, Uuid};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

pub async fn text_channel(owner: &RestClient, name: &str) -> Result<Uuid> {
    Ok(owner
        .create_channel(&CreateChannelRequest {
            name: name.into(),
            r#type: Some("text".into()),
            ..Default::default()
        })
        .await?
        .id
        .ok_or("channel")?
        .parse()?)
}

pub async fn rest_as(
    address: &str,
    username: &str,
    password: &str,
    register: bool,
) -> Result<RestClient> {
    let rest = RestClient::loopback(address)?;
    if register {
        rest.register(&RegisterRequest {
            username: username.into(),
            password: password.to_owned(),
        })
        .await?;
    }
    rest.login(&LoginRequest {
        username: username.into(),
        password: password.to_owned(),
    })
    .await?;
    Ok(rest)
}

pub async fn send(
    rest: &RestClient,
    channel: Uuid,
    content: &str,
    reply_to: Option<&str>,
) -> Result<Uuid> {
    let sent = rest
        .create_message(&MessageUpload {
            channel_id: &channel.to_string(),
            content: Some(content),
            reply_to,
            embeds: None,
            attachments: &[],
        })
        .await?;
    Ok(sent.id.ok_or("message id")?.parse()?)
}

const WAIT: std::time::Duration = std::time::Duration::from_secs(8);

/// Waits for the channel's reading state to satisfy `ready`, or fails naming `what` and the last
/// counts seen, so a stalled step is identifiable.
async fn wait_for_state(
    session: &Session,
    channel: Uuid,
    what: &str,
    ready: impl Fn(&ReadState) -> bool,
) -> Result<ReadState> {
    let mut reading = session.subscribe_reading();
    let mut last = None;
    let found = tokio::time::timeout(WAIT, async {
        loop {
            if let Some(state) = reading.borrow_and_update().channels.get(&channel) {
                last = Some((state.unread_count, state.mention_count));
                if ready(state) {
                    return Ok::<_, Box<dyn std::error::Error>>(state.clone());
                }
            }
            reading.changed().await?;
        }
    })
    .await;
    found.map_err(|_| format!("timed out waiting for {what}; last (unread, mentions): {last:?}"))?
}

/// Waits until the channel's unread total reaches `rows`: the arrival and its recount happen in
/// one publication, so the mention count is final for that row.
pub async fn settled(session: &Session, channel: Uuid, rows: u64, what: &str) -> Result<ReadState> {
    wait_for_state(session, channel, what, |state| {
        matches!(state.unread_count, UnreadCount::Exact(n) | UnreadCount::AtLeast(n) if n == rows)
    })
    .await
}

pub async fn mentions_reach(
    session: &Session,
    channel: Uuid,
    count: UnreadCount,
    what: &str,
) -> Result {
    wait_for_state(session, channel, what, |state| state.mention_count == count)
        .await
        .map(drop)
}

pub async fn online(session: &Session) -> Result {
    let mut connection = session.subscribe_connection();
    loop {
        let status = *connection.borrow_and_update();
        if status.generation == session.generation() && status.state == ConnectionState::Online {
            return Ok(());
        }
        connection.changed().await?;
    }
}
