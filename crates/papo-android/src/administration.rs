//! Acknowledged, account-scoped administration through the canonical core API.
use super::*;
use crate::{
    account::{self, ScopedApi},
    bindings::{Backend, CoreError, NativeClient},
    discovery::{Capabilities, RoleView},
};
use papo_core::{
    api::{
        client::{Api, ApiError},
        models,
    },
    state::{ChannelKind, Store},
};

#[derive(Clone, Debug, uniffi::Record)]
pub struct ManagedChannel {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub topic: String,
    pub parent_id: String,
    pub position: i32,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ManagedPerson {
    pub id: String,
    pub username: String,
    pub name: String,
    pub roles: Vec<String>,
    pub banned: bool,
    pub owner: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AdministrationState {
    pub channels: Vec<ManagedChannel>,
    pub people: Vec<ManagedPerson>,
    pub roles: Vec<RoleView>,
}

#[derive(Clone, Copy, Debug, Default, uniffi::Record)]
pub struct ChannelAccess {
    pub read: bool,
    pub send: bool,
    pub delete_messages: bool,
    pub connect_voice: bool,
}
impl From<models::ChannelPermissions> for ChannelAccess {
    fn from(p: models::ChannelPermissions) -> Self {
        Self {
            read: p.read_channel,
            send: p.send_messages,
            delete_messages: p.delete_messages,
            connect_voice: p.connect_voice,
        }
    }
}
impl From<ChannelAccess> for models::ChannelPermissions {
    fn from(p: ChannelAccess) -> Self {
        Self {
            read_channel: p.read,
            send_messages: p.send,
            delete_messages: p.delete_messages,
            connect_voice: p.connect_voice,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ChannelRule {
    pub role_id: String,
    pub role_name: String,
    pub access: ChannelAccess,
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum AdministrationChange {
    Server {
        name: String,
        public: bool,
        password: Option<String>,
    },
    Icon {
        blob: String,
        format: String,
    },
    CreateChannel {
        name: String,
        kind: String,
        topic: String,
        parent_id: Option<String>,
        access_roles: Vec<String>,
    },
    UpdateChannel {
        id: String,
        name: String,
        topic: String,
    },
    MoveChannel {
        id: String,
        anchor_id: String,
        parent_id: Option<String>,
    },
    DeleteChannel {
        id: String,
    },
    SetChannelRule {
        channel_id: String,
        role_id: String,
        access: ChannelAccess,
    },
    RemoveChannelRule {
        channel_id: String,
        role_id: String,
    },
    CreateRole {
        name: String,
        color: String,
        permissions: Capabilities,
    },
    UpdateRole {
        id: String,
        name: String,
        color: String,
        permissions: Capabilities,
    },
    DeleteRole {
        id: String,
    },
    AssignRole {
        user_id: String,
        role_id: String,
        assigned: bool,
    },
    Ban {
        user_id: String,
        banned: bool,
    },
    CreateSticker {
        name: String,
        blob: String,
        format: String,
    },
    DeleteSticker {
        id: String,
    },
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AdministrationOutcome {
    pub created_id: String,
    pub refresh_error: String,
}

fn state_view(s: &Store) -> AdministrationState {
    AdministrationState {
        roles: s
            .roles
            .iter()
            .map(|role| RoleView {
                id: role.id.clone(),
                name: role.name.clone(),
                color: role.color.clone().unwrap_or_default(),
                permissions: role.permissions.into(),
            })
            .collect(),
        channels: s
            .channels
            .iter()
            .filter(|c| c.kind != ChannelKind::Direct)
            .map(|c| ManagedChannel {
                id: c.id.clone(),
                name: c.name.clone(),
                topic: c.topic.clone().unwrap_or_default(),
                kind: match c.kind {
                    ChannelKind::Text => "text",
                    ChannelKind::Voice => "voice",
                    ChannelKind::Category => "category",
                    ChannelKind::Direct => "direct",
                }
                .into(),
                parent_id: c.parent_id.clone().unwrap_or_default(),
                position: c.position,
            })
            .collect(),
        people: s
            .members
            .iter()
            .map(|p| ManagedPerson {
                id: p.id.clone(),
                username: p.username.clone(),
                name: p.name.clone(),
                roles: p.roles.clone(),
                banned: p.banned,
                owner: s.server.as_ref().and_then(|s| s.owner_id.as_deref()) == Some(&p.id),
            })
            .collect(),
    }
}

fn validate(s: &Store, change: &AdministrationChange) -> Result<(), String> {
    use AdministrationChange::*;
    let p = Capabilities::from_store(s);
    let allowed = match change {
        Server { .. } | Icon { .. } | CreateSticker { .. } | DeleteSticker { .. } => {
            p.manage_server
        }
        CreateChannel { .. }
        | UpdateChannel { .. }
        | MoveChannel { .. }
        | DeleteChannel { .. }
        | SetChannelRule { .. }
        | RemoveChannelRule { .. } => p.manage_channels,
        CreateRole { .. } | UpdateRole { .. } | DeleteRole { .. } | AssignRole { .. } => {
            p.manage_roles
        }
        Ban { .. } => p.ban_members,
    };
    if !allowed {
        return Err("You don’t have permission for this change".into());
    }
    match change {
        Server {
            name,
            public,
            password,
        } => crate::discovery::check_server(s, name, *public, password.as_deref())?,
        CreateChannel { name, .. }
        | UpdateChannel { name, .. }
        | CreateRole { name, .. }
        | UpdateRole { name, .. }
            if name.trim().is_empty() || name.chars().count() > 32 =>
        {
            return Err("Use a name between 1 and 32 characters".into());
        }
        _ => {}
    }
    match change {
        CreateChannel { topic, .. } | UpdateChannel { topic, .. }
            if topic.chars().count() > 512
                || matches!(change,CreateChannel {kind,..} if kind=="category" && !topic.is_empty()) =>
        {
            return Err(
                "Descriptions support up to 512 characters; categories have no description".into(),
            );
        }
        _ => {}
    }
    match change {
        CreateChannel {
            kind, parent_id, ..
        } => {
            if !matches!(kind.as_str(), "text" | "voice" | "category") {
                return Err("Choose a valid channel type".into());
            }
            if kind == "category" && parent_id.as_ref().is_some_and(|id| !id.is_empty()) {
                return Err("Categories cannot be nested".into());
            }
            validate_parent(s, parent_id.as_deref())?;
        }
        MoveChannel {
            id,
            anchor_id,
            parent_id,
        } => {
            let channel = s.channel(id).ok_or("Channel no longer exists")?;
            if channel.kind == ChannelKind::Direct
                || (!anchor_id.is_empty() && s.channel(anchor_id).is_none())
            {
                return Err("Choose an existing channel".into());
            }
            if channel.kind == ChannelKind::Category
                && parent_id.as_ref().is_some_and(|id| !id.is_empty())
            {
                return Err("Categories cannot be nested".into());
            }
            validate_parent(s, parent_id.as_deref())?;
        }
        CreateRole { color, .. } | UpdateRole { color, .. } => {
            if !color.is_empty()
                && !(color.len() == 7
                    && color.starts_with('#')
                    && color[1..].bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err("Use a color such as #0088FF".into());
            }
        }
        Ban { user_id, .. }
            if user_id == &s.me
                || s.server.as_ref().and_then(|s| s.owner_id.as_deref()) == Some(user_id) =>
        {
            return Err("You cannot ban yourself or the server owner".into());
        }
        CreateSticker { name, blob, format } => {
            if name.is_empty()
                || name.len() > 32
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+'))
            {
                return Err("Use up to 32 letters, numbers, _ or - for the sticker name".into());
            }
            validate_image(blob, format, 256 * 1024)?;
        }
        Icon { blob, format } => validate_image(blob, format, 2 * 1024 * 1024)?,
        _ => {}
    }
    Ok(())
}
fn validate_parent(s: &Store, parent: Option<&str>) -> Result<(), String> {
    if parent.is_some_and(|id| {
        !id.is_empty()
            && s.channel(id)
                .is_none_or(|c| c.kind != ChannelKind::Category)
    }) {
        Err("Choose an existing category".into())
    } else {
        Ok(())
    }
}
fn validate_image(blob: &str, format: &str, max: usize) -> Result<(), String> {
    use base64::Engine;
    if !matches!(format, "PNG" | "JPEG" | "GIF" | "WEBP") {
        return Err("Choose a supported image".into());
    }
    if blob.len() > max.div_ceil(3) * 4 {
        return Err("The image exceeds the upload limit".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .map_err(|_| "The image is invalid")?;
    if bytes.is_empty() || bytes.len() > max {
        return Err("The image exceeds the upload limit".into());
    }
    Ok(())
}

impl NativeClient {
    pub(super) fn inspect_admin<T>(
        &self,
        owner: &str,
        inspect: impl FnOnce(&Store) -> Result<T, String>,
    ) -> Result<T, CoreError> {
        let backend = self
            .backend
            .lock()
            .map_err(|_| "Client lock failed".to_owned())?;
        let store = match backend.as_ref().ok_or("Client is closed".to_owned())? {
            Backend::Live(c) => &c.runtime.store,
            Backend::Demo(c) => &c.store,
        };
        if owner.is_empty() || store.me != owner {
            return Err("Account changed".to_owned().into());
        }
        inspect(store).map_err(Into::into)
    }
    fn refresh_administration(&self, request: &ScopedApi) -> Result<(), CoreError> {
        let updates = account::run(async {
            Ok(vec![
                Update::Server(request.api.server().await?.map(Box::new)),
                Update::Channels(request.api.channels().await?),
                Update::Roles(request.api.roles().await?),
                Update::Users(request.api.users().await?),
                Update::Emojis(request.api.emojis().await?),
            ])
        })?;
        self.finish_request(request, updates)
    }
}

#[uniffi::export]
impl NativeClient {
    pub fn administration(
        &self,
        owner: String,
        refresh: bool,
    ) -> Result<AdministrationState, CoreError> {
        self.inspect_admin(&owner, |s| {
            if s.can_open_server_admin()
                || s.can_manage_webhooks()
                || Capabilities::from_store(s).ban_members
            {
                Ok(())
            } else {
                Err("You don’t have access to server settings".into())
            }
        })?;
        if refresh {
            let demo = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?
                .as_ref()
                .is_some_and(|b| matches!(b, Backend::Demo(_)));
            if !demo {
                let request = self.scoped_api(&owner)?;
                self.refresh_administration(&request)?;
            }
        }
        self.inspect_admin(&owner, |s| Ok(state_view(s)))
    }
    pub fn administer(
        &self,
        owner: String,
        change: AdministrationChange,
    ) -> Result<AdministrationOutcome, CoreError> {
        self.inspect_admin(&owner, |s| validate(s, &change))?;
        {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            if let Some(Backend::Demo(c)) = backend.as_mut() {
                if c.store.me != owner {
                    return Err("Account changed".to_owned().into());
                }
                return apply_demo(&mut c.store, change)
                    .map(|created_id| AdministrationOutcome {
                        created_id,
                        refresh_error: String::new(),
                    })
                    .map_err(Into::into);
            }
        }
        let request = self.scoped_api(&owner)?;
        let (old_position, new_position) = self.inspect_admin(&owner, |s| {
            Ok(match &change {
                AdministrationChange::MoveChannel { id, anchor_id, .. } => {
                    let old = s.channel(id).map_or(0, |c| c.position);
                    (old, s.channel(anchor_id).map_or(old, |c| c.position))
                }
                _ => (0, 0),
            })
        })?;
        let created_id = account::run(apply(&request.api, change, old_position, new_position))?;
        self.finish_request(&request, vec![])?;
        // An acknowledged mutation must never be presented as a failed save
        // just because its subsequent metadata refresh failed.
        let refresh_error = self
            .refresh_administration(&request)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        Ok(AdministrationOutcome {
            created_id,
            refresh_error,
        })
    }
    pub fn channel_rules(
        &self,
        owner: String,
        channel_id: String,
    ) -> Result<Vec<ChannelRule>, CoreError> {
        self.inspect_admin(&owner, |s| {
            if s.can_manage_channels() {
                Ok(())
            } else {
                Err("You don’t have permission to edit channel access".into())
            }
        })?;
        {
            let backend = self
                .backend
                .lock()
                .map_err(|_| "Client lock failed".to_owned())?;
            if let Some(Backend::Demo(c)) = backend.as_ref() {
                return Ok(c
                    .store
                    .channel(&channel_id)
                    .ok_or("Channel no longer exists".to_owned())?
                    .permissions
                    .iter()
                    .map(rule_view)
                    .collect());
            }
        }
        let request = self.scoped_api(&owner)?;
        let response = account::run(request.api.channel_permissions(&channel_id))?;
        self.finish_request(&request, vec![])?;
        Ok(response.permissions.iter().map(rule_view).collect())
    }
}
fn rule_view(p: &models::ChannelPermissionEntry) -> ChannelRule {
    ChannelRule {
        role_id: p.role_id.clone(),
        role_name: p.role_name.clone(),
        access: p.permissions.into(),
    }
}
fn absent(result: Result<(), ApiError>) -> Result<(), ApiError> {
    match result {
        Err(ApiError::NotFound) => Ok(()),
        other => other,
    }
}
async fn apply(
    api: &Api,
    change: AdministrationChange,
    old: i32,
    new: i32,
) -> Result<String, ApiError> {
    use AdministrationChange::*;
    match change {
        Server {
            name,
            public,
            password,
        } => {
            api.patch_server(&models::PatchServerRequest {
                name: Some(name.trim().into()),
                public: Some(public),
                password,
                ..Default::default()
            })
            .await?;
        }
        Icon { blob, format } => {
            api.patch_server(&models::PatchServerRequest {
                icon_blob: Some(blob),
                icon_format: Some(format),
                ..Default::default()
            })
            .await?;
        }
        CreateChannel {
            name,
            kind,
            topic,
            parent_id,
            access_roles,
        } => {
            return Ok(api
                .create_channel(name.trim(), &kind, Some(&topic), parent_id, access_roles)
                .await?
                .id);
        }
        UpdateChannel { id, name, topic } => {
            api.update_channel(&id, name.trim(), Some(&topic)).await?;
        }
        MoveChannel { id, parent_id, .. } => {
            api.move_channel(&id, old, new, parent_id).await?;
        }
        DeleteChannel { id } => absent(api.delete_channel(&id).await)?,
        SetChannelRule {
            channel_id,
            role_id,
            access,
        } => {
            api.update_channel_permissions(&channel_id, &role_id, access.into())
                .await?;
        }
        RemoveChannelRule {
            channel_id,
            role_id,
        } => absent(
            api.remove_channel_role_permission(&channel_id, &role_id)
                .await,
        )?,
        CreateRole {
            name,
            color,
            permissions,
        } => {
            return Ok(api
                .create_role(
                    name.trim(),
                    (!color.is_empty()).then_some(color.as_str()),
                    permissions.into(),
                    vec![],
                )
                .await?
                .id);
        }
        UpdateRole {
            id,
            name,
            color,
            permissions,
        } => {
            api.update_role(
                &id,
                name.trim(),
                (!color.is_empty()).then_some(color.as_str()),
                permissions.into(),
            )
            .await?;
        }
        DeleteRole { id } => absent(api.delete_role(&id).await)?,
        AssignRole {
            user_id,
            role_id,
            assigned,
        } => {
            if assigned {
                api.assign_role(&user_id, &role_id).await?;
            } else {
                absent(api.unassign_role(&user_id, &role_id).await)?;
            }
        }
        Ban { user_id, banned } => {
            api.ban_user(&user_id, banned).await?;
        }
        CreateSticker { name, blob, format } => {
            let response = api.create_emoji(&name, &blob, &format).await?;
            return Ok(response
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into());
        }
        DeleteSticker { id } => absent(api.delete_emoji(&id).await)?,
    }
    Ok(String::new())
}

fn apply_demo(s: &mut Store, change: AdministrationChange) -> Result<String, String> {
    use AdministrationChange::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    let next = || format!("demo-admin-{}", SERIAL.fetch_add(1, Ordering::Relaxed));
    let (action, name, target, channel) = match &change {
        Server { name, .. } => ("server.update", Some(name.clone()), None, None),
        Icon { .. } => ("server.update", None, None, None),
        CreateChannel { name, .. } => ("channel.create", Some(name.clone()), None, None),
        UpdateChannel { id, name, .. } => {
            ("channel.update", Some(name.clone()), None, Some(id.clone()))
        }
        MoveChannel { id, .. } => ("channel.move_position", None, None, Some(id.clone())),
        DeleteChannel { id } => (
            "channel.delete",
            s.channel(id).map(|c| c.name.clone()),
            None,
            Some(id.clone()),
        ),
        SetChannelRule { channel_id, .. } | RemoveChannelRule { channel_id, .. } => (
            "channel.permissions_update",
            None,
            None,
            Some(channel_id.clone()),
        ),
        CreateRole { name, .. } => ("role.create", Some(name.clone()), None, None),
        UpdateRole { name, .. } => ("role.update", Some(name.clone()), None, None),
        DeleteRole { id } => (
            "role.delete",
            s.roles.iter().find(|r| r.id == *id).map(|r| r.name.clone()),
            None,
            None,
        ),
        AssignRole {
            user_id,
            role_id,
            assigned,
        } => (
            if *assigned {
                "user_role.assign"
            } else {
                "user_role.remove"
            },
            s.roles
                .iter()
                .find(|r| r.id == *role_id)
                .map(|r| r.name.clone()),
            Some(user_id.clone()),
            None,
        ),
        Ban { user_id, banned } => (
            if *banned { "user.ban" } else { "user.unban" },
            None,
            Some(user_id.clone()),
            None,
        ),
        CreateSticker { name, .. } => ("emoji.create", Some(name.clone()), None, None),
        DeleteSticker { id } => (
            "emoji.delete",
            s.emojis
                .iter()
                .find(|e| e.id == *id)
                .map(|e| e.name.clone()),
            None,
            None,
        ),
    };
    let mut created = String::new();
    match change {
        Server { name, public, .. } => {
            let server = s.server.as_mut().ok_or("Server no longer exists")?;
            server.name = name.trim().into();
            server.public = public;
        }
        Icon { blob, .. } => s.server.as_mut().ok_or("Server no longer exists")?.icon = Some(blob),
        CreateChannel {
            name,
            kind,
            topic,
            parent_id,
            access_roles,
        } => {
            created = next();
            let permissions = access_roles
                .into_iter()
                .map(|id| models::ChannelPermissionEntry {
                    role_name: s
                        .roles
                        .iter()
                        .find(|r| r.id == id)
                        .map(|r| r.name.clone())
                        .unwrap_or_default(),
                    role_id: id,
                    permissions: models::ChannelPermissions {
                        read_channel: true,
                        send_messages: true,
                        connect_voice: true,
                        delete_messages: false,
                    },
                })
                .collect();
            s.channels.push(papo_core::state::Channel {
                id: created.clone(),
                name: name.trim().into(),
                kind: match kind.as_str() {
                    "voice" => ChannelKind::Voice,
                    "category" => ChannelKind::Category,
                    _ => ChannelKind::Text,
                },
                topic: Some(topic),
                position: s
                    .channels
                    .iter()
                    .filter(|c| c.kind != ChannelKind::Direct)
                    .map(|c| c.position)
                    .max()
                    .unwrap_or(-1)
                    .saturating_add(1),
                parent_id,
                permissions,
                notification_settings: "only_mentions".into(),
                unread: false,
                mentions: 0,
            });
        }
        UpdateChannel { id, name, topic } => {
            let c = s
                .channels
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("Channel no longer exists")?;
            c.name = name.trim().into();
            c.topic = Some(topic);
        }
        MoveChannel {
            id,
            anchor_id,
            parent_id,
        } => {
            let old = s
                .channels
                .iter()
                .position(|c| c.id == id)
                .ok_or("Channel no longer exists")?;
            let mut moved = s.channels.remove(old);
            if let Some(parent) = parent_id {
                moved.parent_id = (!parent.is_empty()).then_some(parent);
            }
            let target = s
                .channels
                .iter()
                .position(|c| c.id == anchor_id)
                .unwrap_or(old.min(s.channels.len()));
            let target = if target >= old && !anchor_id.is_empty() && anchor_id != id {
                (target + 1).min(s.channels.len())
            } else {
                target
            };
            s.channels.insert(target, moved);
            for (position, c) in s.channels.iter_mut().enumerate() {
                c.position = position as i32;
            }
        }
        DeleteChannel { id } => {
            s.channels.retain(|c| c.id != id);
            for c in &mut s.channels {
                if c.parent_id.as_deref() == Some(&id) {
                    c.parent_id = None;
                }
            }
        }
        SetChannelRule {
            channel_id,
            role_id,
            access,
        } => {
            let name = s
                .roles
                .iter()
                .find(|r| r.id == role_id)
                .map(|r| r.name.clone())
                .ok_or("Role no longer exists")?;
            let c = s
                .channels
                .iter_mut()
                .find(|c| c.id == channel_id)
                .ok_or("Channel no longer exists")?;
            c.permissions.retain(|p| p.role_id != role_id);
            c.permissions.push(models::ChannelPermissionEntry {
                role_id,
                role_name: name,
                permissions: access.into(),
            });
        }
        RemoveChannelRule {
            channel_id,
            role_id,
        } => s
            .channels
            .iter_mut()
            .find(|c| c.id == channel_id)
            .ok_or("Channel no longer exists")?
            .permissions
            .retain(|p| p.role_id != role_id),
        CreateRole {
            name,
            color,
            permissions,
        } => {
            created = next();
            s.roles.push(models::Role {
                id: created.clone(),
                name: name.trim().into(),
                color: (!color.is_empty()).then_some(color),
                permissions: permissions.into(),
            });
        }
        UpdateRole {
            id,
            name,
            color,
            permissions,
        } => {
            let role = s
                .roles
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("Role no longer exists")?;
            role.name = name.trim().into();
            role.color = (!color.is_empty()).then_some(color);
            role.permissions = permissions.into();
            for profile in s.profiles.values_mut() {
                for r in &mut profile.roles {
                    if r.id == id {
                        r.name = role.name.clone();
                        r.color = role.color.clone();
                    }
                }
            }
        }
        DeleteRole { id } => {
            s.roles.retain(|r| r.id != id);
            for person in &mut s.members {
                person.roles.retain(|r| r != &id);
            }
            for profile in s.profiles.values_mut() {
                profile.roles.retain(|r| r.id != id);
            }
            for c in &mut s.channels {
                c.permissions.retain(|p| p.role_id != id);
            }
        }
        AssignRole {
            user_id,
            role_id,
            assigned,
        } => {
            let role = s
                .roles
                .iter()
                .find(|r| r.id == role_id)
                .ok_or("Role no longer exists")?;
            let person = s
                .members
                .iter_mut()
                .find(|p| p.id == user_id)
                .ok_or("Person no longer exists")?;
            person.roles.retain(|r| r != &role_id);
            let profile = s.profiles.entry(user_id).or_default();
            profile.roles.retain(|r| r.id != role_id);
            if assigned {
                person.roles.push(role_id.clone());
                profile.roles.push(models::RoleSummary {
                    id: role_id,
                    name: role.name.clone(),
                    color: role.color.clone(),
                    position: s.roles.iter().position(|r| r.id == role.id).unwrap_or(0) as i32,
                });
            }
        }
        Ban { user_id, banned } => {
            let p = s
                .members
                .iter_mut()
                .find(|p| p.id == user_id)
                .ok_or("Person no longer exists")?;
            p.banned = banned;
            if banned {
                p.presence = papo_core::state::Presence::Offline;
            }
        }
        CreateSticker { name, blob, .. } => {
            created = next();
            s.emojis.push(papo_core::state::CustomEmoji {
                id: created.clone(),
                name,
                blob: Some(blob),
            });
        }
        DeleteSticker { id } => s.emojis.retain(|e| e.id != id),
    }
    for person in &mut s.members {
        person.role_color = s
            .profiles
            .get(&person.id)
            .and_then(|p| {
                p.roles
                    .iter()
                    .filter(|r| r.color.is_some())
                    .max_by_key(|r| r.position)
            })
            .and_then(|r| r.color.as_deref())
            .and_then(models::parse_hex_color);
    }
    let mut metadata = serde_json::Map::new();
    if let Some(name) = name {
        metadata.insert("name".into(), json!(name));
    }
    s.audit_logs.insert(
        0,
        models::AuditLogEntry {
            id: next(),
            actor_username: s.my_name.clone(),
            action: action.into(),
            entity_type: action.split('.').next().unwrap_or("server").into(),
            created_at: Some(chrono::Utc::now()),
            actor_id: Some(s.me.clone()),
            entity_id: (!created.is_empty()).then(|| created.clone()),
            target_username: target
                .as_deref()
                .and_then(|id| s.member(id))
                .map(|p| p.username.clone()),
            target_user_id: target,
            channel_id: channel,
            metadata,
        },
    );
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn administration_saves_roles_access_membership_moderation_and_audit() {
        let client = NativeClient::demo();
        let role = client
            .administer(
                "me".into(),
                AdministrationChange::CreateRole {
                    name: "Builders".into(),
                    color: "#0088FF".into(),
                    permissions: Capabilities::default(),
                },
            )
            .unwrap()
            .created_id;
        let channel = client
            .administer(
                "me".into(),
                AdministrationChange::CreateChannel {
                    name: "work".into(),
                    kind: "text".into(),
                    topic: "A shared space".into(),
                    parent_id: None,
                    access_roles: vec![],
                },
            )
            .unwrap()
            .created_id;
        client
            .administer(
                "me".into(),
                AdministrationChange::SetChannelRule {
                    channel_id: channel.clone(),
                    role_id: role.clone(),
                    access: ChannelAccess {
                        read: true,
                        send: true,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        let rules = client.channel_rules("me".into(), channel.clone()).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].access.send);
        assert!(!rules[0].access.delete_messages);
        client
            .administer(
                "me".into(),
                AdministrationChange::AssignRole {
                    user_id: "ana".into(),
                    role_id: role.clone(),
                    assigned: true,
                },
            )
            .unwrap();
        let state = client.administration("me".into(), false).unwrap();
        assert!(
            state
                .people
                .iter()
                .find(|p| p.id == "ana")
                .unwrap()
                .roles
                .contains(&role)
        );
        client
            .administer(
                "me".into(),
                AdministrationChange::Ban {
                    user_id: "ana".into(),
                    banned: true,
                },
            )
            .unwrap();
        assert!(
            client
                .administration("me".into(), false)
                .unwrap()
                .people
                .iter()
                .find(|p| p.id == "ana")
                .unwrap()
                .banned
        );
        let page = client
            .administration_audit("me".into(), crate::admin_audit::AuditFilter::default())
            .unwrap();
        assert!(page.entries.iter().any(|e| e.action == "user.ban"));
        client
            .administer(
                "me".into(),
                AdministrationChange::DeleteRole { id: role.clone() },
            )
            .unwrap();
        assert!(
            client
                .channel_rules("me".into(), channel)
                .unwrap()
                .is_empty()
        );
        assert!(
            !client
                .administration("me".into(), false)
                .unwrap()
                .people
                .iter()
                .find(|p| p.id == "ana")
                .unwrap()
                .roles
                .contains(&role)
        );
    }

    #[test]
    fn ownership_permissions_and_backend_limits_are_enforced_before_mutation() {
        let client = NativeClient::demo();
        assert!(client.administration("someone-else".into(), false).is_err());
        assert!(
            client
                .administer(
                    "me".into(),
                    AdministrationChange::Ban {
                        user_id: "me".into(),
                        banned: true
                    }
                )
                .is_err()
        );
        assert!(
            client
                .administer(
                    "me".into(),
                    AdministrationChange::CreateChannel {
                        name: "a".repeat(33),
                        kind: "text".into(),
                        topic: "".into(),
                        parent_id: None,
                        access_roles: vec![]
                    }
                )
                .is_err()
        );
        assert!(
            client
                .administer(
                    "me".into(),
                    AdministrationChange::CreateChannel {
                        name: "room".into(),
                        kind: "text".into(),
                        topic: "x".repeat(513),
                        parent_id: None,
                        access_roles: vec![]
                    }
                )
                .is_err()
        );
        {
            let mut backend = client.backend.lock().unwrap();
            let Backend::Demo(c) = backend.as_mut().unwrap() else {
                panic!()
            };
            c.store.server.as_mut().unwrap().owner_id = Some("another-owner".into());
            c.store.roles.clear();
            c.store
                .members
                .iter_mut()
                .find(|m| m.id == "me")
                .unwrap()
                .roles
                .clear();
        }
        assert!(client.administration("me".into(), false).is_err());
        assert!(
            client
                .administer(
                    "me".into(),
                    AdministrationChange::Server {
                        name: "Unauthorized".into(),
                        public: true,
                        password: None
                    }
                )
                .is_err()
        );
        assert!(
            client
                .administration_audit("me".into(), crate::admin_audit::AuditFilter::default())
                .is_err()
        );
    }

    #[test]
    fn image_upload_budget_is_checked_without_unbounded_base64_decode() {
        assert!(validate_image("!bad", "PNG", 262144).is_err());
        let encoded = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 262145]);
        assert!(validate_image(&encoded, "GIF", 262144).is_err());
    }

    #[test]
    fn live_mutations_use_core_routes_payloads_and_server_acknowledgements() {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let script = [
                (
                    "POST",
                    "/channels",
                    200,
                    r#"{"id":"channel","name":"room","type":"text"}"#,
                ),
                (
                    "PUT",
                    "/channels/channel",
                    200,
                    r#"{"id":"channel","name":"updated","type":"text"}"#,
                ),
                ("PUT", "/channels/channel/change_position", 200, "{}"),
                ("PUT", "/channels/channel/permissions/role", 200, "{}"),
                (
                    "DELETE",
                    "/channels/channel/role/role",
                    404,
                    r#"{"status":404}"#,
                ),
                (
                    "POST",
                    "/roles",
                    200,
                    r##"{"id":"role","name":"Creators","color":"#0088FF","permissions":{}}"##,
                ),
                (
                    "PUT",
                    "/roles/role",
                    200,
                    r#"{"id":"role","name":"Renamed","permissions":{}}"#,
                ),
                ("POST", "/users/ana/roles", 200, "{}"),
                ("DELETE", "/users/ana/roles/role", 204, ""),
                ("PUT", "/users/ana/ban", 200, "{}"),
                ("POST", "/emojis", 200, r#"{"id":"sticker"}"#),
                ("DELETE", "/emojis/sticker", 204, ""),
                ("DELETE", "/roles/role", 204, ""),
                ("DELETE", "/channels/channel", 204, ""),
                (
                    "PUT",
                    "/channels/rejected",
                    403,
                    r#"{"status":403,"detail":"change rejected"}"#,
                ),
            ];
            let mut bodies = vec![];
            for (method, path, status, response) in script {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    if let Ok((s, _)) = listener.accept() {
                        break s;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "missing {method} {path}"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with(&format!("{method} {path} ")), "{line}");
                let mut length = 0;
                let mut authenticated = false;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((key, value)) = line.split_once(':') {
                        if key.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap()
                        }
                        if key.eq_ignore_ascii_case("cookie") {
                            authenticated = value.trim() == "Auth=admin-fixture"
                        }
                    }
                }
                assert!(authenticated);
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                bodies.push(serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null));
                write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
            }
            bodies
        });
        let session = Arc::new(papo_core::api::client::Session::default());
        session.set_token(Some("admin-fixture".into()));
        let api = Api::new(&url, session).unwrap();
        let changes = [
            AdministrationChange::CreateChannel {
                name: "room".into(),
                kind: "text".into(),
                topic: "topic".into(),
                parent_id: None,
                access_roles: vec![],
            },
            AdministrationChange::UpdateChannel {
                id: "channel".into(),
                name: "updated".into(),
                topic: "new topic".into(),
            },
            AdministrationChange::MoveChannel {
                id: "channel".into(),
                anchor_id: "other".into(),
                parent_id: None,
            },
            AdministrationChange::SetChannelRule {
                channel_id: "channel".into(),
                role_id: "role".into(),
                access: ChannelAccess {
                    read: true,
                    send: false,
                    delete_messages: false,
                    connect_voice: true,
                },
            },
            AdministrationChange::RemoveChannelRule {
                channel_id: "channel".into(),
                role_id: "role".into(),
            },
            AdministrationChange::CreateRole {
                name: "Creators".into(),
                color: "#0088FF".into(),
                permissions: Capabilities::default(),
            },
            AdministrationChange::UpdateRole {
                id: "role".into(),
                name: "Renamed".into(),
                color: "".into(),
                // A renamed existing role must keep the permission it loaded.
                permissions: Capabilities::from(
                    serde_json::from_value::<models::RolePermissions>(
                        json!({"manage_webhooks":true}),
                    )
                    .unwrap(),
                ),
            },
            AdministrationChange::AssignRole {
                user_id: "ana".into(),
                role_id: "role".into(),
                assigned: true,
            },
            AdministrationChange::AssignRole {
                user_id: "ana".into(),
                role_id: "role".into(),
                assigned: false,
            },
            AdministrationChange::Ban {
                user_id: "ana".into(),
                banned: true,
            },
            AdministrationChange::CreateSticker {
                name: "rocket".into(),
                blob: "encoded".into(),
                format: "GIF".into(),
            },
            AdministrationChange::DeleteSticker {
                id: "sticker".into(),
            },
            AdministrationChange::DeleteRole { id: "role".into() },
            AdministrationChange::DeleteChannel {
                id: "channel".into(),
            },
        ];
        for change in changes {
            account::run(apply(&api, change, 3, 1)).unwrap();
        }
        assert!(
            account::run(apply(
                &api,
                AdministrationChange::UpdateChannel {
                    id: "rejected".into(),
                    name: "denied".into(),
                    topic: "".into()
                },
                0,
                0
            ))
            .is_err()
        );
        let bodies = worker.join().unwrap();
        assert_eq!(bodies[0]["name"], "room");
        assert_eq!(bodies[0]["type"], "text");
        assert_eq!(bodies[2]["old_position"], 3);
        assert_eq!(bodies[2]["new_position"], 1);
        assert_eq!(
            bodies[3],
            json!({"permissions":{"read_channel":true,"send_messages":false,"delete_messages":false,"connect_voice":true}})
        );
        assert_eq!(bodies[6]["permissions"]["manage_webhooks"], true);
        assert_eq!(bodies[6]["permissions"].as_object().unwrap().len(), 8);
        assert_eq!(bodies[7], json!({"role_id":"role"}));
        assert_eq!(bodies[9], json!({"ban_state":true}));
        assert_eq!(
            bodies[10],
            json!({"name":"rocket","image_blob":"encoded","format":"GIF"})
        );
    }
}
