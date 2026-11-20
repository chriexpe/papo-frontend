//! Component measurements on retained resource versions. Counts are correctness assertions; times are observations.

use super::{AdmittedMutation, ResourceSnapshot, tables::EntityTable};
use crate::{
    Generation, Uuid,
    permissions::read_access::{ReadAccess, ReadDecisions},
    resource_admission::{Admission, Diagnostics},
};
use papo_api::wire;
use serde_json::json;
use std::{sync::Arc, time::Instant};

#[test]
#[ignore = "reports large-roster structure and permission evaluation measurements"]
fn retained_rosters_and_five_hundred_restricted_channels() {
    let log = Diagnostics::default();
    let own = Uuid::from_u128(1);
    let role = Uuid::from_u128(51_001);
    let admit = Admission::for_test(own, &log);
    let rows: Vec<_> = (0..500)
        .map(|index| {
            json!({
                "id": Uuid::from_u128(52_000 + index),
                "type": "text",
                "permissions": [{"role_id": role, "permissions": {"read_channel": true}}]
            })
        })
        .collect();
    let list = serde_json::from_value(json!({"channels": rows})).unwrap();
    let channels = admit.channels(list).unwrap();
    for size in [10_000_u32, 50_000] {
        let roster = (1..=size)
            .map(|n| wire::UserSummary {
                id: Some(Uuid::from_u128(n.into()).to_string()),
                roles: Some(if n == 1 {
                    vec![wire::RoleSummary {
                        id: Some(role.to_string()),
                        ..wire::RoleSummary::default()
                    }]
                } else {
                    vec![]
                }),
                ..wire::UserSummary::default()
            })
            .collect();
        let base = AdmittedMutation {
            roster: Some(admit.roster(roster).unwrap()),
            server: Some(
                admit
                    .server(serde_json::from_value(json!({"id":Uuid::nil()})).unwrap())
                    .unwrap(),
            ),
            channels: Some(channels.clone()),
            roles: Some(
                admit
                    .roles(
                        serde_json::from_value(json!({"roles":[{"id":role,"permissions":{}}]}))
                            .unwrap(),
                    )
                    .unwrap(),
            ),
            ..Default::default()
        }
        .apply(&ResourceSnapshot::empty(Generation(1)), own)
        .unwrap();
        let before = ReadDecisions::evaluate(&base, own);
        assert_eq!(before.with(ReadAccess::Allowed).len(), 500);
        let user = serde_json::from_value(json!({"id":own,"roles":[]})).unwrap();
        let update = AdmittedMutation {
            summaries: vec![admit.summaries(&[own], vec![user]).unwrap()],
            ..Default::default()
        };
        let started = Instant::now();
        let next = update.apply(&base, own).unwrap();
        let apply_us = started.elapsed().as_micros();
        let started = Instant::now();
        let dirty = next.changed_groups(&base);
        let changed_groups_us = started.elapsed().as_micros();
        assert_eq!(dirty, [crate::CacheResource::Members]);
        let started = Instant::now();
        let after = ReadDecisions::evaluate(&next, own);
        let evaluate_us = started.elapsed().as_micros();
        assert_eq!(after.with(ReadAccess::Denied).len(), 500);
        assert_eq!(next.users.diff_keys(&base.users).collect::<Vec<_>>(), [own]);
        let shared_entities = base
            .members
            .iter()
            .filter(|id| {
                Arc::ptr_eq(
                    &base.users.get_arc(*id).unwrap(),
                    &next.users.get_arc(*id).unwrap(),
                )
            })
            .count();
        let shared_slots = EntityTable::shared_value_slots(&base.users, &next.users);
        assert_eq!(shared_entities, size as usize - 1);
        assert!(shared_slots > 0 && shared_slots < shared_entities);
        assert!(next.members.shares_order_root(&base.members));
        assert!(next.channels.shares_entity_root(&base.channels));
        assert!(next.roles.shares_entity_root(&base.roles));
        eprintln!(
            "roster={size} shared_entities={shared_entities} shared_btree_value_slots={shared_slots} shared_order_root=true shared_channel_and_role_roots=true changed_access=500 apply_us={apply_us} changed_groups_us={changed_groups_us} evaluate_us={evaluate_us}"
        );
    }
}
