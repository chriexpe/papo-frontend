//! Pure admission decisions for server, channel and role groups, plus fixtures shared
//! with `people`. Loopback producers and SQLite retention are covered by
//! `tests/resource_admission.rs`.

use super::{Admission, AdmissionFault, Diagnostics, ResourceRejection, Roster, RosterPage};
use crate::{CacheResource, Generation, ResourceSnapshot, Uuid, permissions};
use papo_api::wire::{
    Channel, ChannelList, ChannelPermissionEntry, ChannelPermissions, MemberListResponse, Role,
    RoleListResponse, RolePermissions, RoleSummary, Server, UserSummary,
};
use std::sync::Arc;

mod people;

const OWN: u128 = 4;

#[expect(
    clippy::unnecessary_wraps,
    reason = "every wire identity field is optional; tests build them directly"
)]
fn id(value: u128) -> Option<String> {
    Some(Uuid::from_u128(value).to_string())
}

fn channel(value: u128) -> Channel {
    Channel {
        id: id(value),
        r#type: Some("text".into()),
        permissions: Some(Vec::new()),
        ..Channel::default()
    }
}

fn overwrite(role: Option<String>, read: Option<bool>) -> ChannelPermissionEntry {
    ChannelPermissionEntry {
        role_id: role,
        permissions: Some(ChannelPermissions {
            read_channel: read,
            ..ChannelPermissions::default()
        }),
        ..ChannelPermissionEntry::default()
    }
}

fn member(value: u128, roles: &[Option<String>]) -> UserSummary {
    UserSummary {
        id: id(value),
        created_at: Some("2026-01-01T00:00:00Z".into()),
        roles: Some(
            roles
                .iter()
                .map(|role| RoleSummary {
                    id: role.clone(),
                    ..RoleSummary::default()
                })
                .collect(),
        ),
        ..UserSummary::default()
    }
}

fn page(users: Option<Vec<UserSummary>>, has_more: Option<bool>) -> MemberListResponse {
    MemberListResponse { users, has_more }
}

/// Wire records have no `Debug`, so `unwrap_err` is unavailable on admitted values.
trait Rejected {
    fn rejected(self) -> ResourceRejection;
}

impl<T> Rejected for Result<T, ResourceRejection> {
    fn rejected(self) -> ResourceRejection {
        match self {
            Ok(_) => panic!("malformed input was admitted"),
            Err(rejection) => rejection,
        }
    }
}

fn rejected(group: CacheResource, fault: AdmissionFault) -> ResourceRejection {
    ResourceRejection { group, fault }
}

fn channels(log: &Diagnostics, list: ChannelList) -> Result<Vec<Channel>, ResourceRejection> {
    Admission::for_test(Uuid::from_u128(OWN), log).channels(list)
}

fn roster(
    log: &Diagnostics,
    users: Vec<UserSummary>,
) -> Result<Vec<UserSummary>, ResourceRejection> {
    let mut roster = Roster::new(Admission::for_test(Uuid::from_u128(OWN), log));
    match roster.page(page(Some(users), Some(false)))? {
        RosterPage::Complete(users) => Ok(users),
        RosterPage::More(_) => unreachable!("a has_more=false page completes the roster"),
    }
}

#[test]
fn absent_and_null_channel_lists_are_rejected_but_an_explicit_empty_list_is_admitted() {
    let log = Diagnostics::default();
    for json in [r"{}", r#"{"channels":null}"#] {
        let list: ChannelList = serde_json::from_str(json).unwrap();
        assert_eq!(
            channels(&log, list).rejected(),
            rejected(CacheResource::Channels, AdmissionFault::MissingList)
        );
    }
    let empty: ChannelList = serde_json::from_str(r#"{"channels":[]}"#).unwrap();
    assert!(channels(&log, empty).unwrap().is_empty());
    let diagnostics = log.snapshot();
    assert_eq!(diagnostics.rejected_replacements, 2);
    assert_eq!(
        diagnostics.last_rejection,
        Some(rejected(
            CacheResource::Channels,
            AdmissionFault::MissingList
        ))
    );
}

#[test]
fn empty_and_duplicate_channel_ids_are_wire_rejections_before_the_cache_encoder() {
    let log = Diagnostics::default();
    let mut blank = channel(3);
    blank.id = Some(String::new());
    let cases = [
        (vec![blank], AdmissionFault::InvalidId),
        (vec![channel(3), channel(3)], AdmissionFault::DuplicateId),
        (
            vec![Channel {
                id: Some(Uuid::from_u128(0xab).to_string().to_uppercase()),
                ..channel(0xab)
            }],
            AdmissionFault::InvalidId,
        ),
    ];
    for (list, fault) in cases {
        let list = ChannelList {
            channels: Some(list),
        };
        assert_eq!(
            channels(&log, list).rejected(),
            rejected(CacheResource::Channels, fault)
        );
    }
}

#[test]
fn malformed_nested_overwrite_rejects_channels_because_dropping_it_would_open_the_channel() {
    let log = Diagnostics::default();
    let restricted = Channel {
        permissions: Some(vec![overwrite(Some(String::new()), Some(false))]),
        ..channel(3)
    };
    let list = ChannelList {
        channels: Some(vec![restricted.clone()]),
    };
    assert_eq!(
        channels(&log, list).rejected(),
        rejected(CacheResource::Channels, AdmissionFault::InvalidId)
    );
    // Why it cannot be dropped: the existing evaluator reads the one-entry list as
    // closed, and the same channel without it as open.
    let mut image = ResourceSnapshot::empty(Generation(1));
    image.server = Some(Arc::new(Server {
        id: id(1),
        owner_id: id(2),
        ..Server::default()
    }));
    image.members = Arc::new(vec![member(OWN, &[])]);
    let own = Uuid::from_u128(OWN);
    assert!(
        !permissions::channel_grants(&image, own, &restricted)
            .unwrap()
            .read
    );
    assert!(
        permissions::channel_grants(&image, own, &channel(3))
            .unwrap()
            .read
    );
}

#[test]
fn channel_security_fields_must_be_present() {
    let log = Diagnostics::default();
    let cases = [
        (
            Channel {
                r#type: None,
                ..channel(3)
            },
            AdmissionFault::MissingField,
        ),
        (
            Channel {
                permissions: None,
                ..channel(3)
            },
            AdmissionFault::MissingField,
        ),
        (
            Channel {
                permissions: Some(vec![overwrite(id(9), None)]),
                ..channel(3)
            },
            AdmissionFault::MissingField,
        ),
        (
            Channel {
                permissions: Some(vec![overwrite(Some("role".into()), Some(true))]),
                ..channel(3)
            },
            AdmissionFault::InvalidId,
        ),
        (
            Channel {
                permissions: Some(vec![
                    overwrite(id(9), Some(true)),
                    overwrite(id(9), Some(false)),
                ]),
                ..channel(3)
            },
            AdmissionFault::DuplicateId,
        ),
    ];
    for (malformed, fault) in cases {
        let list = ChannelList {
            channels: Some(vec![channel(5), malformed]),
        };
        assert_eq!(
            channels(&log, list).rejected(),
            rejected(CacheResource::Channels, fault)
        );
    }
    // The backend keys an overwrite by the role text its route received; any UUID
    // spelling is valid and is kept verbatim.
    let uppercase = id(0xab).map(|value| value.to_uppercase());
    let spelled = Channel {
        permissions: Some(vec![overwrite(uppercase.clone(), Some(false))]),
        ..channel(4)
    };
    let list = ChannelList {
        channels: Some(vec![spelled]),
    };
    let admitted = channels(&log, list).unwrap();
    let kept = admitted[0].permissions.as_ref().unwrap()[0].role_id.clone();
    assert_eq!(kept, uppercase);
    let valid = Channel {
        permissions: Some(vec![
            overwrite(id(9), Some(true)),
            overwrite(id(10), Some(false)),
        ]),
        ..channel(3)
    };
    let list = ChannelList {
        channels: Some(vec![valid]),
    };
    assert_eq!(channels(&log, list).unwrap().len(), 1);
}

#[test]
fn a_full_server_of_500_channels_is_admitted_but_a_longer_list_is_not() {
    let log = Diagnostics::default();
    // The pinned backend refuses to create a 501st channel, so 500 is a legitimate maximum.
    let full: Vec<_> = (0..500).map(|index| channel(1000 + index)).collect();
    let mut over = full.clone();
    over.push(channel(2000));
    let admitted = channels(
        &log,
        ChannelList {
            channels: Some(full),
        },
    )
    .unwrap();
    assert_eq!(admitted.len(), 500);
    assert_eq!(
        channels(
            &log,
            ChannelList {
                channels: Some(over)
            }
        )
        .rejected(),
        rejected(CacheResource::Channels, AdmissionFault::ExceedsLimit)
    );
}

#[test]
fn an_unsupported_channel_type_is_rejected_because_the_evaluator_would_read_it_as_open() {
    let log = Diagnostics::default();
    for kind in ["unexpected_type", "dm", "Text"] {
        let list = ChannelList {
            channels: Some(vec![Channel {
                r#type: Some(kind.into()),
                ..channel(3)
            }]),
        };
        assert_eq!(
            channels(&log, list).rejected(),
            rejected(CacheResource::Channels, AdmissionFault::UnsupportedValue),
            "{kind}"
        );
    }
    for kind in ["text", "category", "voice"] {
        let list = ChannelList {
            channels: Some(vec![Channel {
                r#type: Some(kind.into()),
                ..channel(3)
            }]),
        };
        assert_eq!(channels(&log, list).unwrap().len(), 1, "{kind}");
    }
    // Why: with no overwrites, the existing evaluator grants read on any non-DM type.
    let mut image = ResourceSnapshot::empty(Generation(1));
    image.server = Some(Arc::new(Server {
        id: id(1),
        owner_id: id(2),
        ..Server::default()
    }));
    image.members = Arc::new(vec![member(OWN, &[])]);
    let unknown = Channel {
        r#type: Some("unexpected_type".into()),
        ..channel(3)
    };
    let grants = permissions::channel_grants(&image, Uuid::from_u128(OWN), &unknown).unwrap();
    assert!(grants.read);
}

#[test]
fn server_requires_its_id_and_a_valid_owner_when_one_is_named() {
    let log = Diagnostics::default();
    let admission = Admission::for_test(Uuid::from_u128(OWN), &log);
    let server = Server {
        id: id(1),
        owner_id: id(2),
        ..Server::default()
    };
    assert!(admission.server(server.clone()).is_ok());
    // The backend column is nullable: no owner grants nothing.
    assert!(
        admission
            .server(Server {
                owner_id: None,
                ..server.clone()
            })
            .is_ok()
    );
    for malformed in [
        Server {
            id: None,
            ..server.clone()
        },
        Server {
            owner_id: Some(String::new()),
            ..server
        },
    ] {
        assert_eq!(
            admission.server(malformed).rejected(),
            rejected(CacheResource::Server, AdmissionFault::InvalidId)
        );
    }
}

#[test]
fn role_definitions_require_a_list_unique_ids_and_permissions() {
    let log = Diagnostics::default();
    let admission = Admission::for_test(Uuid::from_u128(OWN), &log);
    let role = |value| Role {
        id: id(value),
        permissions: Some(RolePermissions::default()),
        ..Role::default()
    };
    let cases = [
        (None, AdmissionFault::MissingList),
        (
            Some(vec![Role {
                id: Some("role".into()),
                ..role(1)
            }]),
            AdmissionFault::InvalidId,
        ),
        (Some(vec![role(1), role(1)]), AdmissionFault::DuplicateId),
        (
            Some(vec![Role {
                permissions: None,
                ..role(1)
            }]),
            AdmissionFault::MissingField,
        ),
    ];
    for (roles, fault) in cases {
        assert_eq!(
            admission.roles(RoleListResponse { roles }).rejected(),
            rejected(CacheResource::Roles, fault)
        );
    }
    let empty = RoleListResponse {
        roles: Some(Vec::new()),
    };
    assert!(admission.roles(empty).unwrap().is_empty());
}
