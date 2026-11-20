//! Work counts and identity checks across retained baselines and coalesced member updates.

use super::*;
use crate::people::{MemberItem, member};
use futures_util::FutureExt;
use papo_sdk::{Role, RoleSummary, UserSummary};
use std::cell::Cell;

thread_local! {
    static PROJECTED: Cell<usize> = const { Cell::new(0) };
}

fn measured(snapshot: &ResourceSnapshot, id: Uuid) -> MemberItem {
    PROJECTED.set(PROJECTED.get() + 1);
    member(snapshot, id)
}

fn summary(id: Uuid, nickname: &str, role: Option<Uuid>) -> UserSummary {
    UserSummary {
        id: Some(id.to_string()),
        nickname: nickname.into(),
        roles: Some(
            role.into_iter()
                .map(|id| RoleSummary {
                    id: Some(id.to_string()),
                    ..Default::default()
                })
                .collect(),
        ),
        ..Default::default()
    }
}

#[test]
fn active_roster_projects_only_changed_people_across_skipped_versions() {
    for size in [10_000, 50_000] {
        exercise(size, false);
    }
}

#[test]
#[ignore = "reports host-dependent latency and memory, run on its own"]
fn measure_retained_roster_subscriptions() {
    for size in [10_000, 50_000] {
        exercise(size, true);
    }
}

fn exercise(size: u128, measure: bool) {
    let clock = || measure.then(std::time::Instant::now);
    let own = Uuid::from_u128(1);
    let changed = Uuid::from_u128(size / 2);
    let role = Uuid::from_u128(size + 1);
    let rows = (1..=size)
        .map(|id| {
            let id = Uuid::from_u128(id);
            summary(id, "", (id == changed).then_some(role))
        })
        .collect();
    let base = Arc::new(roster(own, role, "before", rows));
    let (sender, receiver) = watch::channel(Arc::clone(&base));
    let mut subscription = subscribe(receiver, List::Members, measured);
    let original = subscription.snapshot.clone();
    let rss_before = measure.then(rss_kib).flatten();
    let start = clock();
    let mut edited = base
        .with_summary_for_test(own, changed, summary(changed, "revision", Some(role)))
        .unwrap();
    let mutation = start.map(|instant| instant.elapsed());
    edited.version = 1;
    sender.send_replace(Arc::new(edited.clone()));
    edited.version = 2;
    sender.send_replace(Arc::new(edited.clone()));
    let start = clock();
    PROJECTED.set(0);
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    let projection = start.map(|instant| instant.elapsed());
    assert_eq!(PROJECTED.get(), 1);
    assert!(
        matches!(update.diffs.as_slice(), [VectorDiff::Set { index, .. }] if *index == usize::try_from(size/2-1).unwrap())
    );
    let mut projected = original.clone();
    for diff in update.diffs {
        diff.apply(&mut projected);
    }
    let same_rows = original
        .iter()
        .zip(&projected)
        .filter(|(a, b)| Arc::ptr_eq(a, b))
        .count();
    let same_entities = base
        .members
        .iter()
        .filter(|&id| {
            Arc::ptr_eq(
                &base.users.get_arc(id).unwrap(),
                &edited.users.get_arc(id).unwrap(),
            )
        })
        .count();
    let size_usize = usize::try_from(size).unwrap();
    assert_eq!(same_rows, size_usize - 1);
    assert_eq!(same_entities, size_usize - 1);
    assert!(edited.members.same_order(&base.members));
    let rss_after = measure.then(rss_kib).flatten();
    edited.roles = roster(own, role, "after", Vec::new()).roles;
    edited.version = 3;
    sender.send_replace(Arc::new(edited.clone()));
    PROJECTED.set(0);
    let start = clock();
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    let role_projection = start.map(|instant| instant.elapsed());
    assert_eq!(PROJECTED.get(), 1);
    assert!(
        matches!(update.diffs.as_slice(), [VectorDiff::Set { value,.. }] if value.roles[0].name.as_deref()==Some("after"))
    );
    edited.version = 4;
    sender.send_replace(Arc::new(edited));
    PROJECTED.set(0);
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.diffs.len(), 0);
    assert_eq!(PROJECTED.get(), 0);
    if let (Some(mutation), Some(projection), Some(role_projection)) =
        (mutation, projection, role_projection)
    {
        let retained_user = base.users.get(changed).unwrap();
        let retained_item = &original[usize::try_from(size / 2 - 1).unwrap()];
        // Each superseded Arc has two counters. Dynamic capacities below belong only to its
        // old payload in this fixture. Persistent tree nodes and allocator metadata are separate.
        let counters = 4 * size_of::<usize>();
        let user_bytes = size_of_val(retained_user)
            + retained_user.roles.as_ref().unwrap().capacity() * size_of::<Uuid>();
        let row_bytes = size_of_val(retained_item.as_ref())
            + retained_item.roles.capacity() * size_of::<crate::RoleLabel>()
            + retained_item.roles[0].name.as_ref().unwrap().capacity();
        eprintln!(
            "roster={size} user_arcs_shared={same_entities} row_arcs_shared={same_rows} member_us={} projection_us={} role_projection_us={} projected_per_edit=1 diffs_per_edit=1 old_user_and_row_payload_with_arc_headers_bytes={} rss_before_kib={rss_before:?} rss_after_kib={rss_after:?}",
            mutation.as_micros(),
            projection.as_micros(),
            role_projection.as_micros(),
            counters + user_bytes + row_bytes
        );
    }
}

fn roster(own: Uuid, role: Uuid, name: &str, users: Vec<UserSummary>) -> ResourceSnapshot {
    let mut definition = Role {
        id: Some(role.to_string()),
        name: Some(name.into()),
        ..Role::default()
    };
    definition.permissions.get_or_insert_default();
    ResourceSnapshot::fixture(
        Generation::for_test(1),
        None,
        vec![],
        vec![definition],
        users,
        vec![],
        vec![],
        vec![],
        own,
    )
    .unwrap()
}

fn rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[test]
fn unresolved_assignment_has_an_identity_and_only_current_definitions_supply_its_label() {
    let own = Uuid::from_u128(1);
    let peer = Uuid::from_u128(2);
    let role = Uuid::from_u128(3);
    let without_definition = ResourceSnapshot::fixture(
        Generation::for_test(1),
        None,
        vec![],
        vec![],
        vec![summary(peer, "peer", Some(role))],
        vec![],
        vec![],
        vec![],
        own,
    )
    .unwrap();
    let assigned = member(&without_definition, peer);
    assert_eq!(assigned.roles.len(), 1);
    assert_eq!(assigned.roles[0].id, role);
    assert!(assigned.roles[0].name.is_none());
    let with_definition = roster(own, role, "new", vec![summary(peer, "peer", Some(role))]);
    assert_eq!(
        member(&with_definition, peer).roles[0].name.as_deref(),
        Some("new")
    );
    let removed = roster(own, role, "new", vec![summary(peer, "peer", None)]);
    assert_eq!(member(&removed, peer).roles.len(), 0);
}

#[test]
fn ordered_membership_edits_cover_sparse_changes_and_coalesced_reorders() {
    let source: Vector<_> = (0..50_000).map(Uuid::from_u128).collect();
    let newcomer = Uuid::from_u128(50_000);
    for index in [0, 25_000, 50_000] {
        let mut joined = source.clone();
        joined.insert(index, newcomer);
        let edits = order_edits(&source, &joined);
        assert_eq!(edits.len(), 1);
        let mut actual = source.clone();
        for edit in edits {
            edit.apply(&mut actual);
        }
        assert_eq!(actual, joined);
        assert_eq!(
            order_edits(&joined, &source),
            [VectorDiff::Remove { index }]
        );
    }
    let mut coalesced = source.clone();
    let moved = coalesced.remove(10);
    coalesced.insert(2, moved);
    coalesced.push_back(newcomer);
    let mut applied = source;
    for edit in order_edits(&applied, &coalesced) {
        edit.apply(&mut applied);
    }
    assert_eq!(applied, coalesced);
}
