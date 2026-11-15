//! Parsed message-content cost fixtures (7c schema) covering cold parsing and cache reuse.

use super::fixture::{self, Result};
use papo_sdk::Client;
use papo_sdk_ui::{Timeline, TimelineConfig, TimelineItem, message_content::MessageContent};
use papo_test_server::{Config, TestServer};
use std::{hint::black_box, sync::Arc, time::Instant};

const SYNTHETIC_SAMPLES: &[&str] = &[
    "**heading** _italic_ ~~strike~~ ||spoiler|| `inline` @everyone [link](https://example.invalid)",
    "Plain message without Markdown delimiters",
    "> quote line\n> second line\n```rust\nlet x = 1;\n```",
    "| a | b |\n| - | - |\n| 1 | 2 |\n- [x] done\n@mention(<@11111111-2222-3333-4444-555555555555>)",
];

pub async fn run(client: &Client, count: usize) -> Result {
    if !(100..=100_000).contains(&count) {
        return Err("unsupported fixture row count".into());
    }
    let seed = fixture::seed(0, count);
    let channel: papo_sdk::Uuid = seed.channel_id.parse()?;
    let mut messages = seed.messages()?;
    for (index, message) in messages.iter_mut().enumerate() {
        message.content = Some(SYNTHETIC_SAMPLES[index % SYNTHETIC_SAMPLES.len()].to_owned());
    }
    let empty_seed = fixture::seed(0, 0);
    let server = TestServer::start(Config {
        history: vec![empty_seed],
        ..Config::default()
    })
    .await?;
    server.seed_messages(&seed.channel_id, messages).await?;
    let result = exercise(client, &server, channel, count).await;
    let stopped = server.shutdown().await;
    result?;
    stopped?;
    Ok(())
}

async fn exercise(
    client: &Client,
    server: &TestServer,
    channel: papo_sdk::Uuid,
    count: usize,
) -> Result {
    let session = fixture::sign_in(client, server).await?;
    let history = session.history(channel).await?;
    let initial = fixture::hydrate(&history, count).await?;
    let config = TimelineConfig {
        page_size: 30,
        ..TimelineConfig::default()
    };

    let started = Instant::now();
    let live = Timeline::live(session.clone(), channel, config.clone()).await?;
    fixture::report(&session, "content_live_30", count, started).await?;

    let focus = initial.messages[count - 1]
        .cursor()
        .ok_or("missing focus cursor")?;
    let started = Instant::now();
    let focused = Timeline::focused(session.clone(), channel, focus, config).await?;
    fixture::report(&session, "content_focused_30_hit", count, started).await?;

    let live_items = live.subscribe()?.items;
    let focused_items = focused.subscribe()?.items;
    verify_shared_arcs(&live_items, &focused_items)?;

    let target_id = initial.messages[count - 1]
        .id
        .as_deref()
        .ok_or("missing target id")?;
    let mut events = session.subscribe_events();
    let started = Instant::now();
    server
        .set_message_reaction(
            target_id,
            serde_json::from_value(serde_json::json!({"unicode": "x", "count": 1}))?,
        )
        .await?;
    fixture::event(&mut events, "react_update").await?;
    let after_react = live.subscribe()?.items;
    verify_shared_arcs(&live_items, &after_react)?;
    fixture::report(&session, "content_reaction_hit", count, started).await?;

    let started = Instant::now();
    server
        .edit_message(
            target_id,
            "**edited** _content_ ||new||",
            "2026-01-02T00:00:00.123456Z",
        )
        .await?;
    fixture::event(&mut events, "message_embeds_update").await?;
    let after_edit = live.subscribe()?.items;
    if verify_shared_arcs(&after_react, &after_edit).is_ok() {
        return Err("expected edit to allocate a new MessageContent Arc".into());
    }
    fixture::report(&session, "content_edit_reparse", count, started).await?;

    let started = Instant::now();
    let mut block_count = 0usize;
    for index in 0..count {
        let parsed = MessageContent::parse(SYNTHETIC_SAMPLES[index % SYNTHETIC_SAMPLES.len()]);
        block_count += black_box(parsed.blocks.len());
    }
    black_box(block_count);
    fixture::report(&session, "content_parse_batch", count, started).await?;
    Ok(())
}

fn verify_shared_arcs(
    left: &imbl::Vector<Arc<TimelineItem>>,
    right: &imbl::Vector<Arc<TimelineItem>>,
) -> Result {
    let left_last = left.iter().rev().find_map(|item| match item.as_ref() {
        TimelineItem::Message { id, content, .. } => Some((*id, Arc::clone(content))),
        _ => None,
    });
    let right_last = right.iter().rev().find_map(|item| match item.as_ref() {
        TimelineItem::Message { id, content, .. } => Some((*id, Arc::clone(content))),
        _ => None,
    });
    match (left_last, right_last) {
        (Some((l_id, l_arc)), Some((r_id, r_arc)))
            if l_id == r_id && Arc::ptr_eq(&l_arc, &r_arc) =>
        {
            Ok(())
        }
        _ => Err("expected shared MessageContent Arc across windows/reaction diffs".into()),
    }
}
