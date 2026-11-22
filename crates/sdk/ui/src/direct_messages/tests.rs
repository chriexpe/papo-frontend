//! Global DM sorting, read/badge overlay alignment, block filtering and dynamic account tracking.

use super::projection::{AccountImages, project};
use super::*;
use papo_sdk::{
    BadgesSnapshot, ChannelBadges, Client, ClientConfig, DataRoot, DirectConversation, Generation,
    ReadState, ReadingSnapshot, ResourceSnapshot, ServerAddress, UnreadCount, UserSummary, Uuid,
    secret::test_store::ScriptedSecretStore,
};
use std::collections::BTreeMap;

fn image(generation: u64, timestamp: &str, blocked: bool) -> Arc<ResourceSnapshot> {
    let peer = UserSummary {
        id: Some(Uuid::from_u128(2).to_string()),
        ..UserSummary::default()
    };
    let dm = DirectConversation {
        id: Uuid::from_u128(1).to_string(),
        user: peer.clone(),
        created_at: timestamp.into(),
        ..DirectConversation::default()
    };
    Arc::new(
        ResourceSnapshot::fixture(
            Generation::for_test(generation),
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![dm],
            if blocked { vec![peer] } else { vec![] },
            Uuid::from_u128(3),
        )
        .unwrap(),
    )
}

fn images_with(
    resources: Arc<ResourceSnapshot>,
    reading_gen: u64,
    unread_count: u64,
    badge_mentions: u64,
) -> AccountImages {
    let dm_id = Uuid::from_u128(1);
    let reading = Arc::new(ReadingSnapshot {
        version: 1,
        generation: Generation::for_test(reading_gen),
        channels: BTreeMap::from([(
            dm_id,
            ReadState {
                unread_count: UnreadCount::Exact(unread_count),
                ..ReadState::default()
            },
        )]),
    });
    let badges = Arc::new(BadgesSnapshot {
        version: 1,
        generation: Generation::for_test(reading_gen),
        channels_complete: true,
        channels: BTreeMap::from([(
            dm_id,
            Arc::new(ChannelBadges {
                has_unread: Some(unread_count > 0),
                mention_count: UnreadCount::Exact(badge_mentions),
                direct_message: true,
            }),
        )]),
    });
    AccountImages {
        resources,
        reading,
        badges,
    }
}

#[test]
fn equal_remote_ids_remain_separate_and_unread_badges_respect_generation() {
    let a = AccountId::new(
        ServerAddress::parse("https://example.invalid/a")
            .unwrap()
            .id(),
        Uuid::from_u128(3),
    );
    let b = AccountId::new(
        ServerAddress::parse("https://example.invalid/b")
            .unwrap()
            .id(),
        Uuid::from_u128(3),
    );
    let mut images = BTreeMap::from([
        (
            a,
            images_with(image(1, "2026-01-01T00:00:00.000002Z", false), 1, 3, 1),
        ),
        (
            b,
            images_with(image(2, "2026-01-01T01:00:00.000001+01:00", false), 1, 5, 2),
        ),
    ]);
    let rows = project(&images, &Vector::new(), |_, _| true);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id.account, a);
    assert_eq!(
        rows[0].unread.as_ref().unwrap().unread_count,
        UnreadCount::Exact(3)
    );
    assert_eq!(
        rows[0].badges.as_ref().unwrap().mention_count,
        UnreadCount::Exact(1)
    );
    // Stale reading/badges generation (1 != 2) must not overlay onto row b.
    assert_eq!(rows[1].id.account, b);
    assert_eq!(rows[1].unread, None);
    assert_eq!(rows[1].badges, None);

    images.insert(
        a,
        images_with(image(1, "2026-01-01T00:00:00.000002Z", true), 1, 0, 0),
    );
    let next = project(&images, &rows, |account, _| account == a);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].id.account, b);
    assert!(Arc::ptr_eq(&next[0], &rows[1]));
}

#[test]
fn for_client_closes_stream_cleanly_on_client_shutdown() {
    use futures_util::StreamExt;
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::open(
        DataRoot::new(dir.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig::default(),
    )
    .unwrap();
    let service = DirectMessageService::for_client(client.handle());
    let mut sub = service.subscribe();
    assert!(sub.snapshot.is_empty());
    client.shutdown().unwrap();
    let item = futures_util::FutureExt::now_or_never(sub.stream.next()).unwrap();
    assert!(item.is_none());
}
