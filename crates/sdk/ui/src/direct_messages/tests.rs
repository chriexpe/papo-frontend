//! Global DM sorting uses instants and account-scoped identities; block filtering never crosses accounts.
use super::*;
use papo_sdk::{DirectConversation, ServerAddress, UserSummary, Uuid};
fn image(generation: u64, timestamp: &str) -> Arc<ResourceSnapshot> {
    Arc::new(ResourceSnapshot {
        version: 1,
        generation: Generation::for_test(generation),
        server: None,
        channels: Arc::default(),
        roles: Arc::default(),
        members: Arc::default(),
        profiles: Arc::default(),
        blocks: Arc::default(),
        presence: Arc::default(),
        dms: Arc::new(vec![DirectConversation {
            id: Uuid::from_u128(1).to_string(),
            user: UserSummary {
                id: Some(Uuid::from_u128(2).to_string()),
                ..UserSummary::default()
            },
            created_at: timestamp.into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        }]),
    })
}
#[test]
fn equal_remote_ids_remain_separate_and_activity_offsets_compare_as_instants() {
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
        (a, image(1, "2026-01-01T00:00:00.000002Z")),
        (b, image(2, "2026-01-01T01:00:00.000001+01:00")),
    ]);
    let rows = project(&images, &Vector::new());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id.account, a);
    assert_eq!(rows[1].id.account, b);
    assert_ne!(rows[0].id, rows[1].id);
    let mut blocked = (**images.get(&a).unwrap()).clone();
    blocked.blocks = Arc::new(vec![UserSummary {
        id: Some(Uuid::from_u128(2).to_string()),
        ..UserSummary::default()
    }]);
    images.insert(a, Arc::new(blocked));
    let next = project(&images, &rows);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].id.account, b);
    assert!(Arc::ptr_eq(&next[0], &rows[1]));
}
