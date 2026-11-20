//! The evaluator consumes typed identities; sparse and ambiguous evidence cannot authorize history.

use super::*;
use crate::resource_admission::{Admission, Diagnostics};
use crate::resources::AdmittedMutation;
use papo_api::wire;
use serde_json::{Value, json};

const ME: Uuid = Uuid::from_u128(2);
const PEER: Uuid = Uuid::from_u128(3);
const ROLE: Uuid = Uuid::from_u128(0xabc);
const CHANNEL: Uuid = Uuid::from_u128(10);
fn snapshot(assigned: bool, owner: bool, overwrite: &Value) -> ResourceSnapshot {
    let roles = if assigned {
        vec![json!({"id":ROLE})]
    } else {
        vec![]
    };
    let server = json!({"id":Uuid::nil(),"owner_id":owner.then_some(ME)});
    let chan = json!({"id":CHANNEL,"type":"text","permissions":overwrite});
    let member = json!({"id":ME,"roles":roles});
    ResourceSnapshot::fixture(
        crate::Generation(1),
        Some(serde_json::from_value(server).unwrap()),
        vec![serde_json::from_value(chan).unwrap()],
        vec![serde_json::from_value(json!({"id":ROLE,"permissions":{}})).unwrap()],
        vec![serde_json::from_value(member).unwrap()],
        vec![],
        vec![],
        vec![],
        ME,
    )
    .unwrap()
}

fn read(root: &ResourceSnapshot) -> Option<ReadAccess> {
    ReadDecisions::evaluate(root, ME).get(CHANNEL)
}

#[test]
fn only_exact_keys_grant_the_assigned_role() {
    for (key, expected) in [
        (ROLE.to_string(), ReadAccess::Allowed),
        (ROLE.to_string().to_uppercase(), ReadAccess::Denied),
    ] {
        let overwrite = json!([{"role_id":key,"permissions":{"read_channel":true}}]);
        assert_eq!(read(&snapshot(true, false, &overwrite)), Some(expected));
        assert_eq!(
            read(&snapshot(false, false, &overwrite)),
            Some(ReadAccess::Denied)
        );
        assert_eq!(
            read(&snapshot(false, true, &overwrite)),
            Some(ReadAccess::Allowed)
        );
    }
}

#[test]
fn missing_member_or_server_is_unknown_only_where_needed() {
    let mut restricted = snapshot(
        false,
        false,
        &json!([{"role_id":ROLE,"permissions":{"read_channel":true}}]),
    );
    restricted.members = crate::ResourceIds::default();
    assert_eq!(read(&restricted), Some(ReadAccess::Unknown));
    let mut open = snapshot(false, false, &json!([]));
    open.members = crate::ResourceIds::default();
    assert_eq!(read(&open), Some(ReadAccess::Allowed));
    open.server = None;
    assert_eq!(read(&open), Some(ReadAccess::Unknown));
    assert_eq!(ReadDecisions::evaluate(&open, ME).get(PEER), None);
}

#[test]
fn owner_does_not_grant_a_blocked_unresolved_or_ambiguous_dm() {
    let diagnostics = Diagnostics::default();
    let admission = Admission::for_test(ME, &diagnostics);
    let mut base = snapshot(false, true, &json!([]));
    let dm = json!({"dms":[{"id":CHANNEL,"user":{"id":PEER},"unread_count":0,"created_at":""}]});
    let mutation = AdmittedMutation {
        dms: Some(admission.dms(serde_json::from_value(dm).unwrap()).unwrap()),
        ..Default::default()
    };
    base = mutation.apply(&base, ME).unwrap();
    assert_eq!(read(&base), Some(ReadAccess::Unknown));
    base.channels = crate::ResourceTable::default();
    assert_eq!(read(&base), Some(ReadAccess::Allowed));
    base.blocks.insert(0, PEER);
    assert_eq!(read(&base), Some(ReadAccess::Denied));
    base.users.remove(PEER);
    assert_eq!(read(&base), Some(ReadAccess::Unknown));
}

#[test]
fn absent_or_malformed_overwrites_reject_the_entire_group() {
    let log = Diagnostics::default();
    let admission = Admission::for_test(ME, &log);
    for raw in [
        json!({"id":CHANNEL,"type":"text"}),
        json!({"id":CHANNEL,"type":"text","permissions":[{}]}),
        json!({"id":"?","type":"text","permissions":[]}),
    ] {
        let list: wire::ChannelList = serde_json::from_value(json!({"channels":[raw]})).unwrap();
        assert!(admission.channels(list).is_err());
    }
}
