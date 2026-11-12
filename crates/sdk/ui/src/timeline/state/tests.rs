//! Arrival between an empty page and registration must be present in the initial live window.

use super::*;
use papo_sdk::{Generation, HistoryUpdate};
use std::collections::BTreeMap;

#[test]
fn initial_live_snapshot_includes_arrival_after_empty_page_without_waiting_for_another_wake() {
    let generation = Generation::for_test(7);
    let messages: Vec<_> = (1..=3)
        .map(|id| {
            Arc::new(Message {
                id: Some(Uuid::from_u128(id).to_string()),
                created_at: Some("2026-01-01T00:00:00.123456Z".into()),
                ..Message::default()
            })
        })
        .collect();
    let raw = Arc::new(HistorySnapshot {
        version: 8,
        generation,
        messages: messages.into(),
        chunks: vec![],
        gaps: vec![],
        empty_complete: false,
    });
    let (_history, mut updates) = watch::channel(HistoryUpdate::Reset(Arc::clone(&raw)));
    updates.mark_unchanged();
    let (_reading, reading) = watch::channel(Arc::new(ReadingSnapshot {
        generation,
        version: 9,
        channels: BTreeMap::new(),
    }));
    let config = TimelineConfig {
        page_size: 2,
        ..TimelineConfig::default()
    };
    let mut window = WindowState::new(
        HistorySubscription {
            snapshot: raw,
            updates,
        },
        reading,
        vec![],
        true,
        Uuid::from_u128(4),
        &config,
    )
    .unwrap();
    window.refresh(&config).unwrap();
    let ids: Vec<_> = window
        .publisher
        .items
        .iter()
        .filter_map(|item| match item.as_ref() {
            TimelineItem::Message { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(ids, [Uuid::from_u128(2), Uuid::from_u128(3)]);
    assert!(!window.history.has_changed().unwrap());
}
