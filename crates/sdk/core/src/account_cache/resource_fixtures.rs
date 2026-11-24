//! Admitted resource fixtures shared by SQLite revision and history tests.

use crate::{
    Generation, ResourceSnapshot,
    resource_admission::{Admission, Diagnostics},
    resources::AdmittedMutation,
};
use papo_api::{
    Uuid,
    wire::{
        Channel, ChannelPermissionEntry, ChannelPermissions, DirectConversation, Role, RoleSummary,
        Server, UserProfile, UserSummary,
    },
};

pub(super) fn channel(id: Uuid) -> crate::ChannelRecord {
    let diagnostics = Diagnostics::default();
    let admission = Admission::for_test(Uuid::from_u128(1), &diagnostics);
    admission
        .channels(papo_api::wire::ChannelList {
            channels: Some(vec![Channel {
                id: Some(id.to_string()),
                r#type: Some("text".into()),
                permissions: Some(Vec::new()),
                ..Default::default()
            }]),
        })
        .unwrap()
        .pop()
        .unwrap()
}

pub(super) fn roster(count: u128, generation: Generation) -> ResourceSnapshot {
    let diagnostics = Diagnostics::default();
    let own = Uuid::from_u128(1);
    let admission = Admission::for_test(own, &diagnostics);
    let users = (1..=count)
        .map(|i| UserSummary {
            id: Some(Uuid::from_u128(i).to_string()),
            roles: Some(Vec::new()),
            username: Some(format!("fixture-{i}")),
            ..Default::default()
        })
        .collect();
    AdmittedMutation {
        roster: Some(admission.roster(users).unwrap()),
        ..Default::default()
    }
    .apply(&ResourceSnapshot::empty(generation), own)
    .unwrap()
}

pub(super) fn snapshot() -> ResourceSnapshot {
    let own = Uuid::from_u128(1);
    let diagnostics = Diagnostics::default();
    let admit = Admission::for_test(own, &diagnostics);
    let user = Uuid::from_u128(7);
    let role = Uuid::from_u128(6);
    let summary = member(user, role);
    let mut snapshot = AdmittedMutation {
        server: Some(
            admit
                .server(Server {
                    id: Some(Uuid::from_u128(3).to_string()),
                    name: Some("fixture".into()),
                    icon_blob: Some("image".into()),
                    ..Default::default()
                })
                .unwrap(),
        ),
        channels: Some(
            admit
                .channels(papo_api::wire::ChannelList {
                    channels: Some(vec![Channel {
                        id: Some(Uuid::from_u128(5).to_string()),
                        name: Some("fixture".into()),
                        r#type: Some("text".into()),
                        notification_settings: Some("future".into()),
                        permissions: Some(vec![ChannelPermissionEntry {
                            role_id: Some(role.simple().to_string()),
                            permissions: Some(ChannelPermissions {
                                read_channel: Some(false),
                                send_messages: Some(true),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    }]),
                })
                .unwrap(),
        ),
        roles: Some(
            admit
                .roles(papo_api::wire::RoleListResponse {
                    roles: Some(vec![Role {
                        id: Some(role.to_string()),
                        name: Some("original".into()),
                        permissions: Some(papo_api::wire::RolePermissions::default()),
                        ..Default::default()
                    }]),
                })
                .unwrap(),
        ),
        roster: Some(
            admit
                .roster(vec![
                    UserSummary {
                        id: Some(own.to_string()),
                        roles: Some(Vec::new()),
                        ..Default::default()
                    },
                    summary.clone(),
                ])
                .unwrap(),
        ),
        profiles: vec![
            admit
                .profiles(
                    &[user],
                    Some(vec![UserProfile {
                        id: Some(user.to_string()),
                        avatar_blob: Some("image".into()),
                        description: Some("detail".into()),
                        ..Default::default()
                    }]),
                )
                .unwrap(),
        ],
        dms: Some(
            admit
                .dms(papo_api::wire::DirectConversationList {
                    dms: vec![DirectConversation {
                        id: Uuid::from_u128(8).to_string(),
                        user: summary.clone(),
                        ..Default::default()
                    }],
                })
                .unwrap(),
        ),
        blocks: Some(
            admit
                .blocks(papo_api::wire::BlockListResponse {
                    users: vec![summary],
                })
                .unwrap(),
        ),
        ..Default::default()
    }
    .apply(&ResourceSnapshot::empty(Generation(4)), own)
    .unwrap();
    snapshot.version = 9;
    snapshot
}

fn member(user: Uuid, role: Uuid) -> UserSummary {
    UserSummary {
        id: Some(user.to_string()),
        nickname: "member".into(),
        roles: Some(vec![RoleSummary {
            id: Some(role.to_string()),
            name: Some("embedded".into()),
            ..Default::default()
        }]),
        status: Some("online".into()),
        status_message: "live".into(),
        typing: "live".into(),
        status_updated_at: Some("live".into()),
        ..Default::default()
    }
}
