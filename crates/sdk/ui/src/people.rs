//! Member/profile/role/block projections; fetching is on demand in the SDK, avatars and mutations are later services.

use crate::{
    PeopleListSubscription,
    people_list::{self, Identified},
};
use papo_sdk::{ClientError, Generated, ResourceSnapshot, Session, UserProfile, Uuid};
use std::{collections::BTreeMap, sync::Arc};

/// A role label uses the current definition when available, falling back to backend summary metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleLabel {
    /// Stable backend role identity.
    pub id: String,
    /// Backend role name.
    pub name: Option<String>,
    /// Optional backend color. Native parsing/rendering belongs to the client.
    pub color: Option<String>,
}
impl Identified for RoleLabel {
    fn id(&self) -> &str {
        &self.id
    }
}

/// Lightweight member presentation; no inline avatar bytes, presence guesses or eager full profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberItem {
    /// Backend user identity, never an external author's bridge identity.
    pub id: String,
    /// Backend username.
    pub username: Option<String>,
    /// Backend nickname.
    pub nickname: Option<String>,
    /// Explicit role labels in backend summary order.
    pub roles: Vec<RoleLabel>,
    /// Backend banned flag, absent if not supplied.
    pub banned: Option<bool>,
}
impl Identified for MemberItem {
    fn id(&self) -> &str {
        &self.id
    }
}

/// Requested profile metadata with byte-heavy avatar data intentionally excluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileItem {
    /// Person and role presentation.
    pub member: MemberItem,
    /// Backend biography, if supplied.
    pub description: Option<String>,
    /// Backend custom typing phrase, distinct from active channel typing.
    pub typing_phrase: Option<String>,
}
impl Identified for ProfileItem {
    fn id(&self) -> &str {
        &self.member.id
    }
}

/// Observable people resources bound to exactly one account.
#[derive(Clone)]
pub struct PeopleService {
    session: Session,
}
impl PeopleService {
    /// Shares existing account reconciliation and lifecycle.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self { session }
    }
    /// Lightweight members only; never starts a profile request.
    #[must_use]
    pub fn members(&self) -> PeopleListSubscription<MemberItem> {
        people_list::subscribe(
            self.session.subscribe_resources(),
            |snapshot| members(snapshot, false),
            |old, next| {
                !Arc::ptr_eq(&old.members, &next.members) || !Arc::ptr_eq(&old.roles, &next.roles)
            },
        )
    }
    /// Backend block list; membership is independent of a currently visible DM.
    #[must_use]
    pub fn blocks(&self) -> PeopleListSubscription<MemberItem> {
        people_list::subscribe(
            self.session.subscribe_resources(),
            |snapshot| members(snapshot, true),
            |old, next| {
                !Arc::ptr_eq(&old.blocks, &next.blocks) || !Arc::ptr_eq(&old.roles, &next.roles)
            },
        )
    }
    /// Current backend role definitions projected for labels; no editing operations.
    #[must_use]
    pub fn roles(&self) -> PeopleListSubscription<RoleLabel> {
        people_list::subscribe(
            self.session.subscribe_resources(),
            |snapshot| {
                snapshot
                    .roles
                    .iter()
                    .filter_map(|role| {
                        Some(RoleLabel {
                            id: role.id.clone()?,
                            name: role.name.clone(),
                            color: role.color.clone(),
                        })
                    })
                    .collect()
            },
            |old, next| !Arc::ptr_eq(&old.roles, &next.roles),
        )
    }
    /// Requested profiles only. Individual member events invalidate only the affected entry.
    #[must_use]
    pub fn profiles(&self) -> PeopleListSubscription<ProfileItem> {
        people_list::subscribe(
            self.session.subscribe_resources(),
            |snapshot| {
                let roles = definitions(snapshot);
                snapshot
                    .profiles
                    .iter()
                    .filter_map(|profile| profile_item(profile, &roles))
                    .collect()
            },
            |old, next| {
                !Arc::ptr_eq(&old.profiles, &next.profiles) || !Arc::ptr_eq(&old.roles, &next.roles)
            },
        )
    }
    /// Loads only caller-requested people through SDK batches of at most fifty.
    /// # Errors
    /// Returns typed transport, generation, account or cache errors. No background roster hydration.
    pub async fn load_profiles(
        &self,
        ids: &[Uuid],
    ) -> Result<Generated<Vec<UserProfile>>, ClientError> {
        self.session.profiles(ids).await
    }
}

fn definitions(snapshot: &ResourceSnapshot) -> BTreeMap<&str, RoleLabel> {
    snapshot
        .roles
        .iter()
        .filter_map(|role| {
            Some((
                role.id.as_deref()?,
                RoleLabel {
                    id: role.id.clone()?,
                    name: role.name.clone(),
                    color: role.color.clone(),
                },
            ))
        })
        .collect()
}
fn labels(
    roles: Option<&[papo_sdk::RoleSummary]>,
    definitions: &BTreeMap<&str, RoleLabel>,
) -> Vec<RoleLabel> {
    roles
        .unwrap_or_default()
        .iter()
        .filter_map(|role| {
            let id = role.id.as_deref()?;
            Some(definitions.get(id).cloned().unwrap_or_else(|| RoleLabel {
                id: id.into(),
                name: role.name.clone(),
                color: role.color.clone(),
            }))
        })
        .collect()
}
fn members(snapshot: &ResourceSnapshot, blocked: bool) -> Vec<MemberItem> {
    let definitions = definitions(snapshot);
    let source = if blocked {
        &snapshot.blocks
    } else {
        &snapshot.members
    };
    source
        .iter()
        .filter_map(|member| {
            Some(MemberItem {
                id: member.id.clone()?,
                username: member.username.clone(),
                nickname: member.nickname.clone(),
                roles: labels(member.roles.as_deref(), &definitions),
                banned: member.banned,
            })
        })
        .collect()
}
fn profile_item(
    profile: &UserProfile,
    definitions: &BTreeMap<&str, RoleLabel>,
) -> Option<ProfileItem> {
    Some(ProfileItem {
        member: MemberItem {
            id: profile.id.clone()?,
            username: profile.username.clone(),
            nickname: profile.nickname.clone(),
            roles: labels(profile.roles.as_deref(), definitions),
            banned: None,
        },
        description: profile.description.clone(),
        typing_phrase: profile.typing.clone(),
    })
}
