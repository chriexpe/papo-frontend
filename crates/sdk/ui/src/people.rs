//! Member/profile/role/block projections; fetching is on demand in the SDK, avatars and mutations are later services.

use crate::{
    PeopleListSubscription,
    resource_list::{self, List},
};
use papo_sdk::{ClientError, Generated, ResourceSnapshot, Session, UserProfile, Uuid};

/// Label from the current role definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleLabel {
    /// Stable backend role identity.
    pub id: Uuid,
    /// Backend role name.
    pub name: Option<String>,
    /// Optional backend color. Native parsing/rendering belongs to the client.
    pub color: Option<String>,
}

/// Lightweight member presentation; no inline avatar bytes, presence guesses or eager full profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberItem {
    /// Backend user identity, never an external author's bridge identity.
    pub id: Uuid,
    /// Backend username.
    pub username: Option<String>,
    /// Backend nickname.
    pub nickname: Option<String>,
    /// Explicit role labels in backend summary order.
    pub roles: Vec<RoleLabel>,
    /// Backend banned flag, absent if not supplied.
    pub banned: Option<bool>,
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
    /// The profile's saved status message, as the last profile read returned it; live
    /// presence may show another for this connection.
    pub status_message: Option<String>,
    /// When the person's account was created (backend RFC 3339 `created_at`), for "Member
    /// since"; clients format it.
    pub member_since: Option<String>,
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
        resource_list::subscribe(self.session.subscribe_resources(), List::Members, member)
    }
    /// Backend block list; membership is independent of a currently visible DM.
    #[must_use]
    pub fn blocks(&self) -> PeopleListSubscription<MemberItem> {
        resource_list::subscribe(self.session.subscribe_resources(), List::Blocks, member)
    }
    /// Current backend role definitions projected for labels; no editing operations.
    #[must_use]
    pub fn roles(&self) -> PeopleListSubscription<RoleLabel> {
        resource_list::subscribe(self.session.subscribe_resources(), List::Roles, role)
    }
    /// Requested profiles only. Individual member events invalidate only the affected entry.
    #[must_use]
    pub fn profiles(&self) -> PeopleListSubscription<ProfileItem> {
        resource_list::subscribe(self.session.subscribe_resources(), List::Profiles, profile)
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

fn role(snapshot: &ResourceSnapshot, id: Uuid) -> RoleLabel {
    let record = snapshot.roles.get(id);
    RoleLabel {
        id,
        name: record.and_then(|role| role.name.clone()),
        color: record.and_then(|role| role.color.clone()),
    }
}

pub(crate) fn member(snapshot: &ResourceSnapshot, id: Uuid) -> MemberItem {
    let user = snapshot.users.get(id);
    let roles = user
        .and_then(|user| user.roles.as_ref())
        .into_iter()
        .flatten()
        .map(|&id| role(snapshot, id))
        .collect();
    MemberItem {
        id,
        username: user.and_then(|user| user.username.clone()),
        nickname: user.and_then(|user| user.nickname.clone()),
        banned: user.and_then(|user| user.banned),
        roles,
    }
}

/// Person `id` as a snapshot knows them, whether or not their full profile was requested.
#[must_use]
pub fn person(snapshot: &ResourceSnapshot, id: Uuid) -> MemberItem {
    member(snapshot, id)
}

/// Person `id`'s profile, once a request for it (`PeopleService::load_profiles`) reached the
/// snapshot; none before.
#[must_use]
pub fn requested_profile(snapshot: &ResourceSnapshot, id: Uuid) -> Option<ProfileItem> {
    snapshot.profiles.get(id)?;
    Some(profile(snapshot, id))
}

fn profile(snapshot: &ResourceSnapshot, id: Uuid) -> ProfileItem {
    ProfileItem {
        member: member(snapshot, id),
        description: snapshot
            .profiles
            .get(id)
            .and_then(|profile| profile.description.clone()),
        typing_phrase: snapshot.users.get(id).and_then(|user| user.typing.clone()),
        status_message: snapshot
            .users
            .get(id)
            .and_then(|user| user.status_message.clone()),
        member_since: snapshot
            .users
            .get(id)
            .and_then(|user| user.created_at.clone()),
    }
}
