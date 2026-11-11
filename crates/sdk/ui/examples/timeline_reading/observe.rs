//! Public SDK/window exercise and bounded waits; owns no transport, durable state or message logging.

use super::{Checkpoint, Result};
use futures_util::StreamExt;
use papo_sdk::{
    Client, ClientError, ConnectionState, HistoryError, LoginRequest, LoginServerRequest, Message,
    ReadingError, RegisterRequest, Session, UnixMicros, Uuid,
};
use papo_sdk_ui::{
    Timeline, TimelineConfig, TimelineDirection, TimelineError, TimelineEvent, TimelineItem,
    TimelineSubscription,
};
use papo_test_server::TestServer;
use std::{env, time::Duration};

async fn sign_in(client: &Client, fixture: Option<&TestServer>) -> Result<Session> {
    let address =
        fixture.map_or_else(|| env::var("PAPO_SERVER"), |server| Ok(server.base_url()))?;
    let username = if fixture.is_some() {
        "timeline-demo".into()
    } else {
        env::var("PAPO_USER")?
    };
    let password = if fixture.is_some() {
        std::iter::once('A')
            .chain((0_u8..24).map(|index| char::from(b'a' + index)))
            .chain(std::iter::once('!'))
            .collect()
    } else {
        env::var("PAPO_PASSWORD")?
    };
    let handle = client.handle();
    let server = handle.add_server(&address, UnixMicros(0)).await?;
    let auth = handle.auth(server)?;
    if let Ok(server_password) = env::var("PAPO_SERVER_PASSWORD") {
        auth.server_login(LoginServerRequest { server_password })
            .await?;
    }
    if fixture.is_some() {
        auth.register(RegisterRequest {
            username: username.clone(),
            password: password.clone(),
        })
        .await?;
    }
    let session = auth
        .login(LoginRequest { username, password }, UnixMicros(0))
        .await?;
    Ok(session)
}

pub(super) async fn first(client: &Client, fixture: Option<&TestServer>) -> Result<Checkpoint> {
    let session = sign_in(client, fixture).await?;
    wait_connection(&session, true).await?;
    let channel = if fixture.is_some() {
        Uuid::from_u128(100)
    } else if let Ok(channel) = env::var("PAPO_CHANNEL") {
        channel.parse()?
    } else {
        session
            .subscribe_resources()
            .borrow()
            .channels
            .iter()
            .find(|channel| {
                channel.r#type.as_deref() == Some("text") && channel.last_message.is_some()
            })
            .and_then(|channel| channel.id.as_deref())
            .ok_or("populated text channel required")?
            .parse()?
    };
    let config = TimelineConfig {
        page_size: 20,
        ..TimelineConfig::default()
    };
    let live = Timeline::live(session.clone(), channel, config.clone()).await?;
    let initial = live.subscribe()?;
    let cursor = initial
        .items
        .iter()
        .filter_map(|item| match item.as_ref() {
            TimelineItem::Message { message, .. } => message.cursor(),
            _ => None,
        })
        .nth(4)
        .ok_or("at least five messages required")?;
    let focused = Timeline::focused(session.clone(), channel, cursor, config).await?;
    print_window("Live", &initial);
    let focused_before = focused.subscribe()?;
    print_window("Focused", &focused_before);
    live.paginate(TimelineDirection::Older).await?;
    let focused_after = focused.subscribe()?;
    if message_ids(&focused_before) != message_ids(&focused_after) {
        return Err("independent focused bounds changed".into());
    }
    focused.paginate(TimelineDirection::Older).await?;
    focused.paginate(TimelineDirection::Newer).await?;
    if let Some(server) = fixture {
        let new_id = Uuid::from_u128(2000);
        let mut changes = live.subscribe()?;
        let fixed = message_ids(&focused.subscribe()?);
        server
            .publish_message(Message {
                id: Some(new_id.to_string()),
                channel_id: Some(channel.to_string()),
                author_id: Some(Uuid::from_u128(101).to_string()),
                created_at: Some("2026-01-01T00:00:01.000001Z".into()),
                ..Message::default()
            })
            .await?;
        wait_message(&mut changes, new_id).await?;
        print_window("Followed", &changes);
        if message_ids(&focused.subscribe()?) != fixed {
            return Err("focused window followed live arrival".into());
        }
    }
    report(&focused).await?;
    if let Some(server) = fixture {
        server.set_gateway_accepting(false).await;
        server.disconnect_gateways();
        wait_connection(&session, false).await?;
    }
    let point = report(&live).await?;
    let state = session
        .reading_state(channel)
        .ok_or("missing reading state")?;
    if state.local_point != Some(point) {
        return Err("local point not acknowledged".into());
    }
    if fixture.is_some() && state.uploaded_point >= Some(point) {
        return Err("fixture point was not left pending".into());
    }
    println!("Durable count 1 generation {}", session.generation().get());
    Ok(Checkpoint {
        account: session.account(),
        channel,
        point,
    })
}

pub(super) async fn restarted(client: &Client, checkpoint: Checkpoint) -> Result<()> {
    let session = client.handle().session(checkpoint.account)?;
    wait_connection(&session, true).await?;
    let mut reading = session.subscribe_reading();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let state = reading
                .borrow_and_update()
                .channels
                .get(&checkpoint.channel)
                .cloned();
            if let Some(state) = state {
                if state.local_point < Some(checkpoint.point) {
                    return Err("read point did not survive restart");
                }
                if state.uploaded_point >= Some(checkpoint.point) {
                    return Ok(());
                }
            }
            reading
                .changed()
                .await
                .map_err(|_| "reading stream closed")?;
        }
    })
    .await??;
    let live = Timeline::live(
        session.clone(),
        checkpoint.channel,
        TimelineConfig::default(),
    )
    .await?;
    print_window("Restored", &live.subscribe()?);
    println!("Uploaded count 1 generation {}", session.generation().get());
    Ok(())
}

async fn report(window: &Timeline) -> Result<papo_sdk::ReadPoint> {
    for _ in 0..32 {
        let snapshot = window.subscribe()?;
        let ids: Vec<_> = snapshot
            .items
            .iter()
            .rev()
            .filter_map(|item| match item.as_ref() {
                TimelineItem::Message { id, .. } => Some(*id),
                _ => None,
            })
            .take(3)
            .collect();
        let point = snapshot
            .items
            .iter()
            .rev()
            .find_map(|item| match item.as_ref() {
                TimelineItem::Message { id, message } => message
                    .created_at
                    .as_ref()
                    .and_then(|created| chrono::DateTime::parse_from_rfc3339(created).ok())
                    .map(|created| papo_sdk::ReadPoint {
                        created_at_us: created.timestamp_micros(),
                        message_id: *id,
                    }),
                _ => None,
            })
            .ok_or("empty visible range")?;
        match window
            .report_visible(snapshot.version, snapshot.generation, &ids)
            .await
        {
            Ok(()) => {
                println!(
                    "Visible count {} version {} generation {}",
                    ids.len(),
                    snapshot.version,
                    snapshot.generation.get()
                );
                return Ok(point);
            }
            Err(
                TimelineError::StaleSnapshot
                | TimelineError::History(HistoryError::StaleSnapshot)
                | TimelineError::Reading(
                    ReadingError::History(HistoryError::StaleSnapshot)
                    | ReadingError::Session(ClientError::Superseded),
                ),
            ) => tokio::task::yield_now().await,
            Err(error) => return Err(error.into()),
        }
    }
    Err("visible reports remained stale".into())
}

async fn wait_connection(session: &Session, online: bool) -> Result<()> {
    let mut status = session.subscribe_connection();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let current = *status.borrow_and_update();
            if (online && current.state == ConnectionState::Online)
                || (!online && matches!(current.state, ConnectionState::Backoff { .. }))
            {
                return Ok::<_, Box<dyn std::error::Error>>(());
            }
            status.changed().await?;
        }
    })
    .await??;
    Ok(())
}

async fn wait_message(subscription: &mut TimelineSubscription, id: Uuid) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !subscription.items.iter().any(|item| matches!(item.as_ref(), TimelineItem::Message { id: current, .. } if *current == id)) {
            let event = subscription.stream.next().await.ok_or("timeline ended")?;
            match event {
                TimelineEvent::Update(update) => { for diff in update.diffs { diff.apply(&mut subscription.items); } subscription.version = update.version; subscription.generation = update.generation; }
                _ => return Err("timeline closed".into()),
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    }).await?
}

fn message_ids(snapshot: &TimelineSubscription) -> Vec<Uuid> {
    snapshot
        .items
        .iter()
        .filter_map(|item| match item.as_ref() {
            TimelineItem::Message { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

fn print_window(label: &str, snapshot: &TimelineSubscription) {
    println!(
        "{label} count {} version {} generation {}",
        snapshot.items.len(),
        snapshot.version,
        snapshot.generation.get()
    );
}
