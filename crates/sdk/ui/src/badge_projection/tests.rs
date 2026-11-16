//! Deterministic account-isolated badge aggregation; lifecycle watches are tested separately.

use super::*;

use papo_sdk::{
    Generation, ReadState, SavedAccount, SavedServer, ServerAddress, UnixMicros,
    secret::CredentialGeneration,
};

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn server() -> SavedServer {
    SavedServer {
        address: ServerAddress::parse("https://example.invalid").unwrap(),
        added_at: UnixMicros(0),
    }
}

fn account(user: u128, active: bool) -> SavedAccount {
    SavedAccount {
        id: AccountId::new(server().address.id(), uuid(user)),
        display_name: None,
        avatar_ref: None,
        active,
        created_at: UnixMicros(0),
        credential_generation: CredentialGeneration::new(1),
    }
}

fn registry(accounts: Vec<SavedAccount>) -> RegistrySnapshot {
    RegistrySnapshot {
        version: 0,
        servers: vec![server()],
        accounts,
        removals: vec![],
    }
}

fn image(
    channels: &[(u128, UnreadCount, UnreadCount)],
    dms: &[(u128, UnreadCount, UnreadCount)],
) -> AccountImage {
    let generation = Generation::for_test(1);
    AccountImage {
        resources: Arc::new(ResourceSnapshot {
            version: 1,
            generation,
            server: Some(Arc::default()),
            channels: Arc::new(
                channels
                    .iter()
                    .map(|(id, _, _)| {
                        serde_json::from_value(serde_json::json!({"id": uuid(*id)})).unwrap()
                    })
                    .collect(),
            ),
            roles: Arc::default(),
            members: Arc::default(),
            profiles: Arc::default(),
            dms: Arc::new(
                dms.iter()
                    .map(|(id, _, _)| {
                        serde_json::from_value(serde_json::json!({
                            "id": uuid(*id), "user": {}, "created_at": "", "unread_count": 0,
                        }))
                        .unwrap()
                    })
                    .collect(),
            ),
            blocks: Arc::default(),
        }),
        reading: Arc::new(ReadingSnapshot {
            generation,
            version: 1,
            channels: channels
                .iter()
                .chain(dms)
                .map(|(id, unread, mentions)| {
                    (
                        uuid(*id),
                        ReadState {
                            unread_count: *unread,
                            mention_count: *mentions,
                            ..ReadState::default()
                        },
                    )
                })
                .collect(),
        }),
    }
}

fn totals(has_unread: Option<bool>, mentions: UnreadCount) -> BadgeTotals {
    BadgeTotals {
        has_unread,
        mentions,
    }
}

#[test]
fn reused_channel_ids_stay_account_isolated_and_server_uses_active_account() {
    let first = account(1, true);
    let second = account(2, false);
    let images = BTreeMap::from([
        (
            first.id,
            image(&[(10, UnreadCount::Exact(4), UnreadCount::Exact(2))], &[]),
        ),
        (
            second.id,
            image(&[(10, UnreadCount::Exact(0), UnreadCount::Exact(0))], &[]),
        ),
    ]);
    let mut registry = registry(vec![first.clone(), second.clone()]);
    let snapshot = project(&registry, &images);
    assert_eq!(
        snapshot.accounts[&first.id],
        Some(totals(Some(true), UnreadCount::Exact(2)))
    );
    assert_eq!(
        snapshot.accounts[&second.id],
        Some(totals(Some(false), UnreadCount::Exact(0)))
    );
    assert_eq!(
        snapshot.servers[&server().address.id()],
        snapshot.accounts[&first.id]
    );

    registry.accounts[0].active = false;
    registry.accounts[1].active = true;
    let switched = project(&registry, &images);
    assert_eq!(switched.accounts, snapshot.accounts);
    assert_eq!(
        switched.servers[&server().address.id()],
        snapshot.accounts[&second.id]
    );
}

#[test]
fn dm_totals_sum_visible_accounts_without_polluting_channel_totals() {
    let first = account(1, true);
    let second = account(2, false);
    let images = BTreeMap::from([
        (
            first.id,
            image(
                &[(10, UnreadCount::Exact(0), UnreadCount::Exact(0))],
                &[(20, UnreadCount::Exact(2), UnreadCount::Exact(2))],
            ),
        ),
        (
            second.id,
            image(&[], &[(20, UnreadCount::Exact(3), UnreadCount::AtLeast(1))]),
        ),
    ]);
    let snapshot = project(&registry(vec![first.clone(), second.clone()]), &images);
    assert_eq!(
        snapshot.accounts[&first.id],
        Some(totals(Some(false), UnreadCount::Exact(0)))
    );
    assert_eq!(
        snapshot.accounts[&second.id],
        Some(totals(Some(false), UnreadCount::Exact(0)))
    );
    assert_eq!(
        snapshot.direct_messages,
        Some(totals(Some(true), UnreadCount::AtLeast(3)))
    );
}

#[test]
fn absent_or_closed_session_is_unknown_not_zero() {
    let first = account(1, true);
    let registry = registry(vec![first.clone()]);
    let images = BTreeMap::from([(first.id, image(&[], &[]))]);
    let known = project(&registry, &images);
    assert_eq!(
        known.accounts[&first.id],
        Some(totals(Some(false), UnreadCount::Exact(0)))
    );

    let closed = project(&registry, &BTreeMap::new());
    assert_eq!(closed.accounts[&first.id], None);
    assert_eq!(closed.servers[&server().address.id()], None);
    assert_eq!(closed.direct_messages, None);
}

#[test]
fn unreconciled_or_generation_mismatched_image_is_unknown() {
    let first = account(1, true);
    let registry = registry(vec![first.clone()]);
    let mut missing_server = image(&[], &[]);
    Arc::make_mut(&mut missing_server.resources).server = None;
    assert_eq!(
        project(&registry, &BTreeMap::from([(first.id, missing_server)])).accounts[&first.id],
        None
    );

    let mut stale = image(&[], &[]);
    Arc::make_mut(&mut stale.reading).generation = Generation::for_test(2);
    assert_eq!(
        project(&registry, &BTreeMap::from([(first.id, stale)])).accounts[&first.id],
        None
    );
}

#[test]
fn gaps_preserve_lower_bound_and_unknown_unread_dot() {
    let first = account(1, true);
    let images = BTreeMap::from([(
        first.id,
        image(
            &[
                (10, UnreadCount::AtLeast(0), UnreadCount::AtLeast(2)),
                (11, UnreadCount::Exact(0), UnreadCount::Exact(3)),
            ],
            &[],
        ),
    )]);
    let snapshot = project(&registry(vec![first.clone()]), &images);
    assert_eq!(
        snapshot.accounts[&first.id],
        Some(totals(None, UnreadCount::AtLeast(5)))
    );
}

#[test]
fn inactive_account_never_fills_server_badge_and_removed_accounts_stop_contributing() {
    let first = account(1, false);
    let images = BTreeMap::from([(
        first.id,
        image(&[], &[(20, UnreadCount::Exact(1), UnreadCount::Exact(1))]),
    )]);
    let snapshot = project(&registry(vec![first]), &images);
    assert_eq!(snapshot.servers[&server().address.id()], None);
    let removed = project(&registry(vec![]), &images);
    assert!(removed.accounts.is_empty());
    assert_eq!(
        removed.direct_messages,
        Some(totals(Some(false), UnreadCount::Exact(0)))
    );
}

#[test]
fn duplicate_metadata_counts_once_and_missing_reading_state_stays_unknown() {
    let first = account(1, true);
    let mut image = image(&[(10, UnreadCount::Exact(2), UnreadCount::Exact(1))], &[]);
    let channel = image.resources.channels[0].clone();
    Arc::make_mut(&mut Arc::make_mut(&mut image.resources).channels).push(channel);
    assert_eq!(
        account_totals(&image).unwrap().0,
        totals(Some(true), UnreadCount::Exact(1))
    );
    Arc::make_mut(&mut image.reading).channels.clear();
    assert!(account_totals(&image).is_none());
    assert_eq!(
        project(
            &registry(vec![first.clone()]),
            &BTreeMap::from([(first.id, image)])
        )
        .accounts[&first.id],
        None
    );
}
