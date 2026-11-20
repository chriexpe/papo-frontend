//! Precedence and identity checks with literal JSON so omission differs from an explicit null.

use super::{AdmittedMutation, ResourceSnapshot};
use crate::{
    Generation, Uuid,
    resource_admission::{Admission, Diagnostics},
};
use serde_json::{Value, json};
use std::sync::Arc;

const OWN: Uuid = Uuid::from_u128(1);
const USER: Uuid = Uuid::from_u128(2);
const ROLE: Uuid = Uuid::from_u128(3);
const DM: Uuid = Uuid::from_u128(4);

fn member(admit: Admission<'_>, value: Value) -> AdmittedMutation {
    AdmittedMutation {
        summaries: vec![
            admit
                .summaries(&[USER], vec![serde_json::from_value(value).unwrap()])
                .unwrap(),
        ],
        ..Default::default()
    }
}

fn initial(admit: Admission<'_>) -> ResourceSnapshot {
    let mut mutation = member(
        admit,
        json!({
            "id":USER, "nickname":"current","typing":"phrase","username":"user","banned":true,"roles":[]
        }),
    );
    mutation.profiles.push(
        admit
            .profiles(
                &[USER],
                Some(vec![
                    serde_json::from_value(json!({
                        "id":USER, "description":"detail"
                    }))
                    .unwrap(),
                ]),
            )
            .unwrap(),
    );
    mutation
        .apply(&ResourceSnapshot::empty(Generation(1)), OWN)
        .unwrap()
}

#[test]
fn omitted_summary_fields_preserve_profile_freshness_and_explicit_null_clears_them() {
    let diagnostics = Diagnostics::default();
    let admit = Admission::for_test(OWN, &diagnostics);
    let initial = initial(admit);
    let update = member(admit, json!({"id":USER,"roles":[{"id":ROLE}]}));
    let changed = update.apply(&initial, OWN).unwrap();
    let user = changed.users.get(USER).unwrap();
    assert_eq!(user.nickname.as_deref(), Some("current"));
    assert_eq!(user.typing.as_deref(), Some("phrase"));
    assert_eq!(user.roles.as_deref(), Some([ROLE].as_slice()));
    assert!(Arc::ptr_eq(
        &initial.profiles.get_arc(USER).unwrap(),
        &changed.profiles.get_arc(USER).unwrap()
    ));

    let clear = AdmittedMutation {
        blocks: Some(
            admit
                .blocks(
                    serde_json::from_value(json!({"users":[{
                        "id":USER, "nickname":null, "typing":null, "banned":false
                    }]}))
                    .unwrap(),
                )
                .unwrap(),
        ),
        ..Default::default()
    }
    .apply(&changed, OWN)
    .unwrap();
    let cleared = clear.users.get(USER).unwrap();
    assert!(cleared.nickname.is_none());
    assert!(cleared.typing.is_none());
    assert_eq!(cleared.banned, Some(true));
    assert!(clear.profiles.get(USER).is_none());
}

#[test]
fn joint_readback_resolves_each_supplied_field_before_allocating_or_invalidating() {
    let diagnostics = Diagnostics::default();
    let admit = Admission::for_test(OWN, &diagnostics);
    let initial = initial(admit);
    let mut update = member(admit, json!({"id":USER,"roles":[],"typing":"phrase"}));
    update.blocks = Some(
        admit
            .blocks(
                serde_json::from_value(json!({"users":[{
                    "id":USER, "nickname":"block", "typing":null,"username":"block"
                }]}))
                .unwrap(),
            )
            .unwrap(),
    );
    update.dms = Some(admit.dms(serde_json::from_value(json!({"dms":[{
        "id":DM,"created_at":"","unread_count":0,"user":{"id":USER,"nickname":"dm","typing":"dm","username":"user"}
    }]})).unwrap()).unwrap());
    update.profiles.push(
        admit
            .profiles(
                &[USER],
                Some(vec![
                    serde_json::from_value(json!({
                        "id":USER,"nickname":"current","typing":null,"description":"detail"
                    }))
                    .unwrap(),
                ]),
            )
            .unwrap(),
    );
    let new = update.apply(&initial, OWN).unwrap();
    assert!(new.blocks.contains(USER));
    assert!(new.dms.get(DM).is_some());
    assert!(Arc::ptr_eq(
        &initial.users.get_arc(USER).unwrap(),
        &new.users.get_arc(USER).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &initial.profiles.get_arc(USER).unwrap(),
        &new.profiles.get_arc(USER).unwrap()
    ));
}

#[test]
fn joint_readback_fills_only_unobserved_fields_from_lower_priority_sources() {
    let diagnostics = Diagnostics::default();
    let admit = Admission::for_test(OWN, &diagnostics);
    let mut update = member(admit, json!({"id":USER,"roles":[],"typing":"member"}));
    update.dms = Some(admit.dms(serde_json::from_value(json!({"dms":[{
        "id":DM,"created_at":"","unread_count":0,"user":{"id":USER,"username":"dm","nickname":"dm","typing":"dm"}
    }]})).unwrap()).unwrap());
    update.profiles.push(
        admit
            .profiles(
                &[USER],
                Some(vec![
                    serde_json::from_value(json!({
                        "id":USER,"nickname":"profile","typing":"profile"
                    }))
                    .unwrap(),
                ]),
            )
            .unwrap(),
    );
    let new = update.apply(&initial(admit), OWN).unwrap();
    let user = new.users.get(USER).unwrap();
    assert_eq!(user.username.as_deref(), Some("dm"));
    assert_eq!(user.nickname.as_deref(), Some("profile"));
    assert_eq!(user.typing.as_deref(), Some("member"));
    assert_eq!(user.banned, Some(true));
}

#[test]
fn fresh_profile_revalidates_equal_detail_identity_despite_changed_shared_fields() {
    let diagnostics = Diagnostics::default();
    let admit = Admission::for_test(OWN, &diagnostics);
    let initial = initial(admit);
    let mut update = member(admit, json!({"id":USER,"nickname":"new","roles":[]}));
    update.profiles.push(
        admit
            .profiles(
                &[USER],
                Some(vec![
                    serde_json::from_value(json!({
                        "id":USER,"description":"detail"
                    }))
                    .unwrap(),
                ]),
            )
            .unwrap(),
    );
    update.invalidate_profiles.push(USER);
    let new = update.apply(&initial, OWN).unwrap();
    assert_eq!(
        new.users.get(USER).unwrap().nickname.as_deref(),
        Some("new")
    );
    assert!(Arc::ptr_eq(
        &initial.profiles.get_arc(USER).unwrap(),
        &new.profiles.get_arc(USER).unwrap()
    ));
}

#[test]
fn embedded_assignments_in_a_joint_role_deletion_cannot_reintroduce_the_deleted_role() {
    let diagnostics = Diagnostics::default();
    let admit = Admission::for_test(OWN, &diagnostics);
    let initial = AdmittedMutation {
        roles: Some(
            admit
                .roles(
                    serde_json::from_value(json!({"roles":[{"id":ROLE,"permissions":{}}]}))
                        .unwrap(),
                )
                .unwrap(),
        ),
        ..Default::default()
    }
    .apply(&initial(admit), OWN)
    .unwrap();
    let updated = AdmittedMutation {
        roles: Some(Vec::new()),
        blocks: Some(
            admit
                .blocks(
                    serde_json::from_value(json!({"users":[{
                        "id":USER,"roles":[{"id":ROLE}]
                    }]}))
                    .unwrap(),
                )
                .unwrap(),
        ),
        ..Default::default()
    }
    .apply(&initial, OWN)
    .unwrap();
    let old = initial.users.get_arc(USER).unwrap();
    let new = updated.users.get_arc(USER).unwrap();
    assert_eq!(new.roles.as_deref(), Some([].as_slice()));
    assert!(Arc::ptr_eq(&old, &new));
}
