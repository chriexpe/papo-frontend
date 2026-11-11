//! Unread reconnect selection uses ordered message heads, independently of open views.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use papo_api::Uuid;

use crate::{Generation, ReadPoint, ReadState, ReadingSnapshot, reading::Reading};

use super::reconnect::unread_channels;

#[tokio::test]
async fn unread_reconnect_uses_the_greater_local_and_uploaded_message_point() {
    let reading = Reading::new(Generation(1));
    let points: Vec<_> = (1..=3)
        .map(|id| ReadPoint {
            created_at_us: 100,
            message_id: Uuid::from_u128(id),
        })
        .collect();
    let channels: Vec<_> = (10..=13).map(Uuid::from_u128).collect();
    reading
        .heads
        .lock()
        .await
        .extend(channels.iter().map(|channel| (*channel, points[1])));
    reading.image.send_replace(Arc::new(ReadingSnapshot {
        generation: Generation(1),
        version: 1,
        channels: BTreeMap::from([
            (
                channels[0],
                ReadState {
                    local_point: Some(points[2]),
                    uploaded_point: Some(points[0]),
                    ..ReadState::default()
                },
            ),
            (
                channels[1],
                ReadState {
                    local_point: Some(points[0]),
                    uploaded_point: Some(points[2]),
                    ..ReadState::default()
                },
            ),
            (
                channels[2],
                ReadState {
                    local_point: Some(points[0]),
                    uploaded_point: None,
                    ..ReadState::default()
                },
            ),
        ]),
    }));
    assert_eq!(
        unread_channels(&reading).await,
        BTreeSet::from([channels[2], channels[3]])
    );
}
