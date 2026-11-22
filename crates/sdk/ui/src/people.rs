//! Member/profile/role/block projections; fetching is on demand in the SDK, avatars and mutations are later services.

use crate::{
    PeopleListSubscription,
    resource_list::{self, List},
};
use papo_sdk::{ClientError, Generated, ResourceSnapshot, RoleFlags, Session, UserProfile, Uuid};

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

/// Complete role definition including admitted backend permissions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleItem {
    /// Identity, name, and color metadata.
    pub label: RoleLabel,
    /// Admitted backend role capability flags.
    pub permissions: RoleFlags,
    /// Backend creation timestamp, if supplied.
    pub created_at: Option<String>,
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
    /// Backend avatar format hint for revisioned avatar fetches, if supplied.
    pub avatar_format: Option<String>,
    /// Backend banner media hash/identifier for `MediaSource::Blob`, if supplied.
    pub banner_media: Option<String>,
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
    /// Current backend role definitions including admitted permission flags.
    #[must_use]
    pub fn role_definitions(&self) -> PeopleListSubscription<RoleItem> {
        resource_list::subscribe(self.session.subscribe_resources(), List::Roles, role_item)
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

fn role_item(snapshot: &ResourceSnapshot, id: Uuid) -> RoleItem {
    let record = snapshot.roles.get(id);
    RoleItem {
        label: role(snapshot, id),
        permissions: record.map_or(
            RoleFlags {
                manage_server: None,
                manage_channels: None,
                manage_roles: None,
                manage_webhooks: None,
                ban_members: None,
                pin_message: None,
                everyone_message: None,
                send_attachment: None,
            },
            |role| role.permissions.clone(),
        ),
        created_at: record.and_then(|role| role.created_at.clone()),
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

fn profile(snapshot: &ResourceSnapshot, id: Uuid) -> ProfileItem {
    let profile_record = snapshot.profiles.get(id);
    ProfileItem {
        member: member(snapshot, id),
        description: profile_record.and_then(|profile| profile.description.clone()),
        typing_phrase: snapshot.users.get(id).and_then(|user| user.typing.clone()),
        avatar_format: profile_record.and_then(|profile| profile.avatar_format.clone()),
        banner_media: profile_record.and_then(|profile| profile.banner_media.clone()),
    }
}

#[cfg(test)]
mod tests {
    //! Verifies profile media fields and full role definitions project cleanly without raw snapshots.
    use super::*;
    use papo_sdk::{Generation, Role, RolePermissions, UserProfile};

    #[test]
    fn profile_and_role_item_projections_retain_avatar_banner_and_role_flags() {
        let user_id = Uuid::from_u128(10);
        let role_id = Uuid::from_u128(20);
        let role_wire = Role {
            id: Some(role_id.to_string()),
            name: Some("Moderator".into()),
            color: Some("#112233".into()),
            created_at: Some("2026-01-01T00:00:00Z".into()),
            permissions: Some(RolePermissions {
                ban_members: Some(true),
                pin_message: Some(true),
                ..RolePermissions::default()
            }),
        };
        let profile_wire = UserProfile {
            id: Some(user_id.to_string()),
            username: Some("alice".into()),
            description: Some("bio".into()),
            avatar_format: Some("png".into()),
            banner_media: Some("a".repeat(64)),
            ..UserProfile::default()
        };
        let snapshot = ResourceSnapshot::fixture(
            Generation::for_test(1),
            None,
            vec![],
            vec![role_wire],
            vec![],
            vec![profile_wire],
            vec![],
            vec![],
            user_id,
        )
        .unwrap();

        let projected_role = role_item(&snapshot, role_id);
        assert_eq!(projected_role.label.name.as_deref(), Some("Moderator"));
        assert_eq!(projected_role.permissions.ban_members, Some(true));
        assert_eq!(projected_role.permissions.pin_message, Some(true));
        assert_eq!(
            projected_role.created_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );

        let projected_profile = profile(&snapshot, user_id);
        assert_eq!(projected_profile.avatar_format.as_deref(), Some("png"));
        assert_eq!(
            projected_profile.banner_media.as_deref(),
            Some("a".repeat(64).as_str())
        );
    }
}
