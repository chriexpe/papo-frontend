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

#[test]
fn silent_accounts_do_not_advance_versions_or_hide_other_accounts_changes() {
    use futures_util::FutureExt;
    use tokio::sync::watch;

    let server = ServerAddress::parse("https://example.invalid")
        .unwrap()
        .id();
    let a = AccountId::new(server, Uuid::from_u128(3));
    let b = AccountId::new(server, Uuid::from_u128(4));
    let timestamp = "2026-01-01T00:00:00Z";
    let (first, first_receiver) = watch::channel(image(1, timestamp, true));
    let (second, second_receiver) = watch::channel(image(1, timestamp, false));
    let mut list = subscribe_accounts([(a, first_receiver), (b, second_receiver)].into_iter());
    assert_eq!(list.snapshot.len(), 1);
    let mut hidden = image(1, timestamp, true);
    Arc::make_mut(&mut hidden).version = 1;
    first.send_replace(hidden);
    assert!(list.stream.next().now_or_never().is_none());

    // A changed, blocked row is still invisible; its account must not mask B's update.
    first.send_replace(image(1, "2026-01-01T00:00:01Z", true));
    second.send_replace(image(1, "2026-01-01T00:00:02Z", false));
    let update = list.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, 1);
    assert!(
        matches!(update.diffs.as_slice(), [VectorDiff::Set { index: 0, value }] if value.id.account == b)
    );
    assert!(list.stream.next().now_or_never().is_none());

    first.send_replace(image(2, timestamp, true));
    let reset = list.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(reset.version, 2);
    assert!(matches!(reset.diffs.as_slice(), [VectorDiff::Reset { values }] if values.len() == 1));
    drop(first);
    assert!(list.stream.next().now_or_never().is_none());
    drop(second);
    let removed = list.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(removed.version, 3);
    assert!(matches!(
        removed.diffs.as_slice(),
        [VectorDiff::Remove { index: 0 }]
    ));
    assert!(list.stream.next().now_or_never().unwrap().is_none());
}
