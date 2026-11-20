//! Global grant composition and DM independence; malformed wire inputs are tested at admission.
use super::*;
use papo_api::wire;
use serde_json::json;

fn fixture(grants: &[(&str, bool, bool)]) -> ResourceSnapshot {
    let user = Uuid::from_u128(2);
    let roles = grants
        .iter()
        .map(|(id, manage, pin)| {
            serde_json::from_value(
                json!({"id":id,"permissions":{"manage_server":manage,"pin_message":pin}}),
            )
            .unwrap()
        })
        .collect();
    let assignments: Vec<_> = grants.iter().map(|(id, _, _)| json!({"id":id})).collect();
    let member: wire::UserSummary =
        serde_json::from_value(json!({"id":user,"roles":assignments})).unwrap();
    let overwrites:Vec<_>=grants.iter().map(|(id,read,send)|json!({"role_id":id,"permissions":{"read_channel":read,"send_messages":send}})).collect();
    ResourceSnapshot::fixture(
        crate::Generation(1),
        Some(
            serde_json::from_value(json!({"id":Uuid::nil(),"owner_id":Uuid::from_u128(1)}))
                .unwrap(),
        ),
        vec![
            serde_json::from_value(
                json!({"id":Uuid::from_u128(10),"type":"text","permissions":overwrites}),
            )
            .unwrap(),
        ],
        roles,
        vec![member],
        vec![],
        vec![],
        vec![],
        user,
    )
    .unwrap()
}

#[test]
fn assigned_roles_combine_each_field_without_administrator_implication() {
    let a = Uuid::from_u128(100).to_string();
    let b = Uuid::from_u128(101).to_string();
    let snapshot = fixture(&[(&a, true, false), (&b, false, true)]);
    let user = Uuid::from_u128(2);
    let global = global_permissions(&snapshot, user).unwrap();
    assert!(global.manage_server && global.pin_message);
    assert!(!global.manage_roles && !global.send_attachment && !global.owner);
    let grants = channel_grants(&snapshot, user, snapshot.channels.iter().next().unwrap()).unwrap();
    assert!(grants.read && grants.send);
    assert!(!grants.delete_others && !grants.voice);
}

#[test]
fn owner_cannot_bypass_direct_membership_or_blocks() {
    assert!(
        global_permissions(&fixture(&[]), Uuid::from_u128(1))
            .unwrap()
            .owner
    );
    for (member, blocked) in [(false, false), (false, true), (true, true)] {
        assert_eq!(direct_grants(member, blocked), ChannelGrants::default());
    }
    assert!(!direct_grants(true, false).delete_others);
}
