//! Global DM sorting uses instants and account-scoped identities; block filtering never crosses accounts.
use super::*;
use papo_sdk::{DirectConversation, ServerAddress, UserSummary, Uuid};
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
        (a, image(1, "2026-01-01T00:00:00.000002Z", false)),
        (b, image(2, "2026-01-01T01:00:00.000001+01:00", false)),
    ]);
    let rows = project(&images, &Vector::new(), |_, _| true);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id.account, a);
    assert_eq!(rows[1].id.account, b);
    assert_ne!(rows[0].id, rows[1].id);
    images.insert(a, image(1, "2026-01-01T00:00:00.000002Z", true));
    let next = project(&images, &rows, |account, _| account == a);
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].id.account, b);
    assert!(Arc::ptr_eq(&next[0], &rows[1]));
}
