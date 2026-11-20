//! A bounded exhaustive reference model for relationship, membership and role observations.

use super::*;
use crate::{
    Generation, Uuid,
    resource_admission::{Admission, Diagnostics},
};
use papo_api::wire;
use serde_json::json;

const USER: Uuid = Uuid::from_u128(2);
const OWN: Uuid = Uuid::from_u128(1);
const ROLE: Uuid = Uuid::from_u128(3);

#[derive(Clone, Copy)]
enum Event {
    Member,
    Absent,
    Block,
    Unblock,
    Profile,
    Expire,
    Define,
    Delete,
}

#[derive(Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "The oracle tracks six independently observable facts."
)]
struct Model {
    member: bool,
    blocked: bool,
    details: bool,
    user: bool,
    defined: bool,
    assigned: bool,
}

impl Model {
    fn observe(&mut self, event: Event) {
        match event {
            Event::Member => {
                self.member = true;
                self.user = true;
                self.assigned = true;
            }
            Event::Absent => {
                self.member = false;
                self.user = false;
                self.details = false;
                self.assigned = false;
            }
            Event::Block => {
                self.blocked = true;
                self.user = true;
            }
            Event::Unblock => self.blocked = false,
            Event::Profile => {
                self.details = true;
                self.user = true;
            }
            Event::Expire => self.details = false,
            Event::Define => self.defined = true,
            Event::Delete => {
                if self.defined {
                    self.assigned = false;
                }
                self.defined = false;
            }
        }
        if !self.member && !self.blocked && !self.details {
            self.user = false;
            self.assigned = false;
        }
    }

    fn compare(self, root: &ResourceSnapshot) {
        assert_eq!(root.members.contains(USER), self.member);
        assert_eq!(root.blocks.contains(USER), self.blocked);
        assert_eq!(root.profiles.get(USER).is_some(), self.details);
        assert_eq!(root.users.get(USER).is_some(), self.user);
        assert_eq!(root.roles.get(ROLE).is_some(), self.defined);
        assert_eq!(
            root.users
                .get(USER)
                .and_then(|user| user.roles.as_ref())
                .is_some_and(|roles| roles.contains(&ROLE)),
            self.assigned
        );
    }
}

fn observe(event: Event, admit: Admission<'_>) -> AdmittedMutation {
    let mut update = AdmittedMutation::default();
    let summary = || wire::UserSummary {
        id: Some(USER.to_string()),
        ..wire::UserSummary::default()
    };
    match event {
        Event::Member | Event::Absent => {
            let rows = if matches!(event, Event::Member) {
                vec![wire::UserSummary {
                    roles: Some(vec![wire::RoleSummary {
                        id: Some(ROLE.to_string()),
                        ..wire::RoleSummary::default()
                    }]),
                    ..summary()
                }]
            } else {
                vec![]
            };
            update
                .summaries
                .push(admit.summaries(&[USER], rows).unwrap());
        }
        Event::Block | Event::Unblock => {
            let users = if matches!(event, Event::Block) {
                vec![summary()]
            } else {
                vec![]
            };
            update.blocks = Some(admit.blocks(wire::BlockListResponse { users }).unwrap());
        }
        Event::Profile | Event::Expire => {
            let rows = if matches!(event, Event::Profile) {
                vec![wire::UserProfile {
                    id: Some(USER.to_string()),
                    ..wire::UserProfile::default()
                }]
            } else {
                vec![]
            };
            update
                .profiles
                .push(admit.profiles(&[USER], Some(rows)).unwrap());
        }
        Event::Define | Event::Delete => {
            let roles = if matches!(event, Event::Define) {
                vec![serde_json::from_value(json!({"id":ROLE,"permissions":{}})).unwrap()]
            } else {
                vec![]
            };
            update.roles = Some(
                admit
                    .roles(wire::RoleListResponse { roles: Some(roles) })
                    .unwrap(),
            );
        }
    }
    update
}

#[test]
fn all_length_four_sequences_agree_with_a_flat_relationship_model() {
    fn visit(base: &ResourceSnapshot, model: Model, depth: u8, admit: Admission<'_>) {
        if depth == 0 {
            return;
        }
        for event in [
            Event::Member,
            Event::Absent,
            Event::Block,
            Event::Unblock,
            Event::Profile,
            Event::Expire,
            Event::Define,
            Event::Delete,
        ] {
            let mut expected = model;
            expected.observe(event);
            let next = observe(event, admit).apply(base, OWN).unwrap();
            expected.compare(&next);
            model.compare(base);
            visit(&next, expected, depth - 1, admit);
        }
    }
    let diagnostics = Diagnostics::default();
    visit(
        &ResourceSnapshot::empty(Generation(1)),
        Model::default(),
        4,
        Admission::for_test(OWN, &diagnostics),
    );
}

#[test]
fn profile_and_social_observations_cannot_assign_or_unban_the_authenticated_user() {
    let diagnostics = Diagnostics::default();
    let admit = Admission::for_test(OWN, &diagnostics);
    let user = |banned, roles| {
        serde_json::from_value(json!({"id":OWN,"banned":banned,"roles":roles})).unwrap()
    };
    let base = AdmittedMutation {
        roster: Some(admit.roster(vec![user(true, json!([]))]).unwrap()),
        ..Default::default()
    }
    .apply(&ResourceSnapshot::empty(Generation(1)), OWN)
    .unwrap();
    let mutation = AdmittedMutation {
        blocks: Some(
            admit
                .blocks(wire::BlockListResponse {
                    users: vec![user(false, json!([{"id":ROLE}]))],
                })
                .unwrap(),
        ),
        profiles: vec![
            admit
                .profiles(
                    &[OWN],
                    Some(vec![
                        serde_json::from_value(json!({"id":OWN,"roles":[{"id":ROLE}]})).unwrap(),
                    ]),
                )
                .unwrap(),
        ],
        ..Default::default()
    };
    let next = mutation.apply(&base, OWN).unwrap();
    let summary = next.users.get(OWN).unwrap();
    assert_eq!(summary.roles.as_deref(), Some([].as_slice()));
    assert_eq!(summary.banned, Some(true));
    assert!(Arc::ptr_eq(
        &next.users.get_arc(OWN).unwrap(),
        &base.users.get_arc(OWN).unwrap()
    ));
}
