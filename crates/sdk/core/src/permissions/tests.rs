//! Exercises missing-input safety and independent grants; production Go parity is a separate integration proof.
use super::*;
use papo_api::wire::{
    ChannelPermissionEntry, ChannelPermissions, Role, RoleSummary, Server, UserSummary,
};
use std::sync::Arc;

fn snapshot() -> ResourceSnapshot {
    let mut snapshot = ResourceSnapshot::empty(crate::Generation(1));
    snapshot.server = Some(Arc::new(Server {
        owner_id: Some(Uuid::from_u128(1).to_string()),
        ..Server::default()
    }));
    snapshot.members = Arc::new(vec![UserSummary {
        id: Some(Uuid::from_u128(2).to_string()),
        roles: Some(Vec::new()),
        ..UserSummary::default()
    }]);
    snapshot
}

#[test]
fn absent_overwrites_do_not_become_an_open_channel() {
    assert_eq!(
        channel_grants(&snapshot(), Uuid::from_u128(2), &Channel::default()),
        Err(PermissionDataError::Unavailable)
    );
    let open = Channel {
        permissions: Some(Vec::new()),
        ..Channel::default()
    };
    assert_eq!(
        channel_grants(&snapshot(), Uuid::from_u128(2), &open).unwrap(),
        ChannelGrants {
            read: true,
            send: true,
            voice: true,
            delete_others: false
        }
    );
}

#[test]
fn assigned_roles_combine_each_field_without_deny_or_administrator_implication() {
    let mut snapshot = snapshot();
    Arc::make_mut(&mut snapshot.members)[0].roles = Some(vec![
        RoleSummary {
            id: Some("a".into()),
            ..RoleSummary::default()
        },
        RoleSummary {
            id: Some("b".into()),
            ..RoleSummary::default()
        },
    ]);
    snapshot.roles = Arc::new(vec![
        Role {
            id: Some("a".into()),
            permissions: Some(RolePermissions {
                manage_server: Some(true),
                ..RolePermissions::default()
            }),
            ..Role::default()
        },
        Role {
            id: Some("b".into()),
            permissions: Some(RolePermissions {
                pin_message: Some(true),
                ..RolePermissions::default()
            }),
            ..Role::default()
        },
    ]);
    let global = global_permissions(&snapshot, Uuid::from_u128(2)).unwrap();
    assert!(global.manage_server && global.pin_message);
    assert!(!global.manage_roles && !global.send_attachment && !global.owner);
    let channel = Channel {
        permissions: Some(vec![
            ChannelPermissionEntry {
                role_id: Some("a".into()),
                permissions: Some(ChannelPermissions {
                    read_channel: Some(true),
                    send_messages: Some(false),
                    ..ChannelPermissions::default()
                }),
                ..ChannelPermissionEntry::default()
            },
            ChannelPermissionEntry {
                role_id: Some("b".into()),
                permissions: Some(ChannelPermissions {
                    read_channel: Some(false),
                    send_messages: Some(true),
                    ..ChannelPermissions::default()
                }),
                ..ChannelPermissionEntry::default()
            },
        ]),
        ..Channel::default()
    };
    let grants = channel_grants(&snapshot, Uuid::from_u128(2), &channel).unwrap();
    assert!(grants.read && grants.send);
    assert!(!grants.delete_others && !grants.voice);
}

#[test]
fn owner_cannot_bypass_direct_membership_or_blocks() {
    for (member, blocked) in [(false, false), (false, true), (true, true)] {
        assert_eq!(direct_grants(member, blocked), ChannelGrants::default());
    }
    assert!(
        global_permissions(&snapshot(), Uuid::from_u128(1))
            .unwrap()
            .owner
    );
    assert!(!direct_grants(true, false).delete_others);
}
