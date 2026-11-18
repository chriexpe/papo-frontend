//! Read decisions from accepted resources: the existing evaluator's outcomes become
//! Allowed/Denied/Unknown, DMs use membership/block evidence, and nothing malformed grants.

use super::*;
use papo_api::wire::{
    Channel, ChannelPermissionEntry, ChannelPermissions, DirectConversation, RoleSummary, Server,
    UserSummary,
};
use std::sync::Arc;

const OWNER: u128 = 1;
const ME: u128 = 2;
const PEER: u128 = 3;
const ROLE: &str = "30000000-0000-4000-8000-000000000001";

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn member(roles: Vec<&str>) -> UserSummary {
    UserSummary {
        id: Some(id(ME).to_string()),
        roles: Some(
            roles
                .into_iter()
                .map(|role| RoleSummary {
                    id: Some(role.into()),
                    ..RoleSummary::default()
                })
                .collect(),
        ),
        ..UserSummary::default()
    }
}

fn restricted(channel: u128, read: bool) -> Channel {
    Channel {
        id: Some(id(channel).to_string()),
        r#type: Some("text".into()),
        permissions: Some(vec![ChannelPermissionEntry {
            role_id: Some(ROLE.into()),
            permissions: Some(ChannelPermissions {
                read_channel: Some(read),
                ..ChannelPermissions::default()
            }),
            ..ChannelPermissionEntry::default()
        }]),
        ..Channel::default()
    }
}

fn dm(channel: u128, peer: Option<u128>) -> DirectConversation {
    DirectConversation {
        id: id(channel).to_string(),
        user: UserSummary {
            id: peer.map(|peer| id(peer).to_string()),
            ..UserSummary::default()
        },
        created_at: "2026-01-01T00:00:00Z".into(),
        last_message: None,
        last_read_message: None,
        last_read_at: None,
        unread_count: 0,
    }
}

fn snapshot(owner: u128, roles: Vec<&str>) -> ResourceSnapshot {
    let mut snapshot = ResourceSnapshot::empty(crate::Generation(1));
    snapshot.server = Some(Arc::new(Server {
        owner_id: Some(id(owner).to_string()),
        ..Server::default()
    }));
    snapshot.members = Arc::new(vec![member(roles)]);
    snapshot
}

fn decide(snapshot: &ResourceSnapshot) -> ReadDecisions {
    ReadDecisions::evaluate(snapshot, id(ME)).unwrap()
}

#[test]
fn role_overwrites_decide_allowed_and_denied_without_a_second_evaluator() {
    let mut granted = snapshot(OWNER, vec![ROLE]);
    granted.channels = Arc::new(vec![restricted(10, true), restricted(11, false)]);
    let decisions = decide(&granted);
    assert_eq!(decisions.get(id(10)), Some(ReadAccess::Allowed));
    assert_eq!(decisions.get(id(11)), Some(ReadAccess::Denied));

    // Losing the role denies a channel whose only grant came from it.
    let mut removed = granted.clone();
    removed.members = Arc::new(vec![member(Vec::new())]);
    assert_eq!(decide(&removed).get(id(10)), Some(ReadAccess::Denied));
}

#[test]
fn owner_bypasses_ordinary_overwrites_but_not_dm_evidence() {
    let mut owned = snapshot(ME, Vec::new());
    owned.channels = Arc::new(vec![restricted(10, false)]);
    owned.dms = Arc::new(vec![dm(20, Some(PEER))]);
    owned.blocks = Arc::new(vec![UserSummary {
        id: Some(id(PEER).to_string()),
        ..UserSummary::default()
    }]);
    let decisions = decide(&owned);
    assert_eq!(decisions.get(id(10)), Some(ReadAccess::Allowed));
    assert_eq!(decisions.get(id(20)), Some(ReadAccess::Denied));
}

#[test]
fn missing_permission_inputs_are_unknown_only_for_channels_that_need_them() {
    let mut incomplete = snapshot(OWNER, vec![ROLE]);
    let mut absent = restricted(10, true);
    absent.permissions = None;
    let open = Channel {
        permissions: Some(Vec::new()),
        ..restricted(11, true)
    };
    incomplete.channels = Arc::new(vec![absent, open, restricted(12, true)]);
    let decisions = decide(&incomplete);
    assert_eq!(decisions.get(id(10)), Some(ReadAccess::Unknown));
    assert_eq!(decisions.get(id(11)), Some(ReadAccess::Allowed));
    assert_eq!(decisions.get(id(12)), Some(ReadAccess::Allowed));

    // Without the account's member row only role-restricted channels are undecidable.
    let mut no_member = incomplete.clone();
    no_member.members = Arc::default();
    let decisions = decide(&no_member);
    assert_eq!(decisions.get(id(11)), Some(ReadAccess::Allowed));
    assert_eq!(decisions.get(id(12)), Some(ReadAccess::Unknown));

    // Without server metadata, the owner bypass cannot be ruled out or in.
    let mut no_server = incomplete;
    no_server.server = None;
    assert_eq!(decide(&no_server).get(id(11)), Some(ReadAccess::Unknown));
}

#[test]
fn listed_dms_are_allowed_unless_blocked_and_unattributable_evidence_is_unknown() {
    let mut social = snapshot(OWNER, Vec::new());
    social.dms = Arc::new(vec![dm(20, Some(PEER)), dm(21, None)]);
    let decisions = decide(&social);
    assert_eq!(decisions.get(id(20)), Some(ReadAccess::Allowed));
    assert_eq!(decisions.get(id(21)), Some(ReadAccess::Unknown));

    social.blocks = Arc::new(vec![UserSummary::default()]);
    assert_eq!(decide(&social).get(id(20)), Some(ReadAccess::Unknown));
}

#[test]
fn unlisted_channels_have_no_decision_and_duplicates_never_grant() {
    let mut duplicated = snapshot(OWNER, vec![ROLE]);
    duplicated.channels = Arc::new(vec![restricted(10, true), restricted(10, true)]);
    let decisions = decide(&duplicated);
    assert_eq!(decisions.get(id(10)), Some(ReadAccess::Unknown));
    assert_eq!(decisions.get(id(99)), None);
}

#[test]
fn unparseable_ids_reject_the_admission_instead_of_reading_as_removal() {
    let mut malformed = snapshot(OWNER, Vec::new());
    malformed.channels = Arc::new(vec![Channel {
        id: None,
        ..restricted(10, true)
    }]);
    assert!(matches!(
        ReadDecisions::evaluate(&malformed, id(ME)),
        Err(ClientError::ResourceData)
    ));
    let mut malformed_dm = snapshot(OWNER, Vec::new());
    malformed_dm.dms = Arc::new(vec![DirectConversation {
        id: "not-a-uuid".into(),
        ..dm(20, Some(PEER))
    }]);
    assert!(matches!(
        ReadDecisions::evaluate(&malformed_dm, id(ME)),
        Err(ClientError::ResourceData)
    ));
}
