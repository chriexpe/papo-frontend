//! View affordances from current SDK permission inputs; never substitutes for server authorization.

use papo_sdk::{
    ConnectionState, Generated, Message, ResourceSnapshot, Session, Uuid,
    permissions::{
        ChannelGrants, GlobalPermissions, PermissionDataError, channel_grants, direct_grants,
        global_permissions,
    },
};

/// Operation-specific affordances matching the pinned backend, without executing mutations.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent backend permission bits are not mutually exclusive states"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelCapabilities {
    /// Read history/list pins/remove own reactions.
    pub read: bool,
    /// Send messages; independent of read permission.
    pub send: bool,
    /// Send uploaded attachments, including DM's role exemption.
    pub attach: bool,
    /// Pin a known message; ordinary-channel pin does not require read.
    pub pin: bool,
    /// Unpin a known message; always requires read.
    pub unpin: bool,
    /// Add a reaction; backend checks send permission.
    pub add_reaction: bool,
    /// Remove one's own reaction; backend checks read permission.
    pub remove_reaction: bool,
    /// Report typing; backend checks read rather than send.
    pub type_here: bool,
    /// Join a voice channel; no extra read requirement.
    pub connect_voice: bool,
    /// Delete someone else's message, independent of channel read.
    pub delete_others: bool,
    /// Whether this is a backend direct conversation.
    pub direct: bool,
}
impl ChannelCapabilities {
    /// Projects backend predicates through each operation's type-specific gate.
    #[must_use]
    pub fn from_grants(kind: &str, grants: ChannelGrants, global: GlobalPermissions) -> Self {
        let direct = kind == "dm";
        let send = grants.send && matches!(kind, "text" | "voice" | "dm");
        let pin = if direct {
            grants.read
        } else {
            global.pin_message
        };
        Self {
            read: grants.read,
            send,
            attach: send && (direct || global.send_attachment),
            pin,
            unpin: grants.read && pin,
            add_reaction: grants.send,
            remove_reaction: grants.read,
            type_here: grants.read,
            connect_voice: kind == "voice" && grants.voice,
            delete_others: grants.delete_others,
            direct,
        }
    }

    /// Existing human messages can be edited by their author; DM edits also require access.
    #[must_use]
    pub fn can_edit(self, own_message: bool) -> bool {
        own_message && (!self.direct || self.read)
    }
    /// Own-message deletion survives access loss, including a later DM block.
    #[must_use]
    pub fn can_delete(self, own_message: bool) -> bool {
        own_message || self.delete_others
    }
}

/// Computes ordinary-channel affordances from explicit inputs. DMs require the owner-bound `CapabilityService`.
/// # Errors
/// Returns `Unavailable` if required server, channel, role or membership data is absent.
pub fn capabilities(
    snapshot: &ResourceSnapshot,
    user: Uuid,
    channel: Uuid,
) -> Result<ChannelCapabilities, PermissionDataError> {
    let id = channel.to_string();
    if snapshot.dms.iter().any(|dm| dm.id == id) {
        return Err(PermissionDataError::Unavailable);
    }
    let channel = snapshot
        .channels
        .iter()
        .find(|channel| channel.id.as_deref() == Some(id.as_str()))
        .ok_or(PermissionDataError::Unavailable)?;
    let kind = channel
        .r#type
        .as_deref()
        .ok_or(PermissionDataError::Unavailable)?;
    Ok(ChannelCapabilities::from_grants(
        kind,
        channel_grants(snapshot, user, channel)?,
        global_permissions(snapshot, user)?,
    ))
}

// Only a Session-bound service may interpret the account-owned DM list as access evidence.
fn direct_capabilities(
    snapshot: &ResourceSnapshot,
    channel: Uuid,
) -> Result<ChannelCapabilities, PermissionDataError> {
    let id = channel.to_string();
    let dm = snapshot
        .dms
        .iter()
        .find(|dm| dm.id == id)
        .ok_or(PermissionDataError::Unavailable)?;
    let blocked = snapshot
        .blocks
        .iter()
        .any(|blocked| blocked.id == dm.user.id);
    Ok(ChannelCapabilities::from_grants(
        "dm",
        direct_grants(true, blocked),
        GlobalPermissions::default(),
    ))
}

/// Reads affordances for one account. Offline/cached conclusions never masquerade as fresh grants.
#[derive(Clone)]
pub struct CapabilityService {
    session: Session,
}
impl CapabilityService {
    /// Binds to the existing account lifecycle; owns no transport or cache.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self { session }
    }
    /// Evaluates the latest reconciled online image. Resource/connection watches notify changes.
    /// # Errors
    /// Returns `Unavailable` before reconcile, offline, after replacement or with missing inputs.
    pub fn channel(
        &self,
        channel: Uuid,
    ) -> Result<Generated<ChannelCapabilities>, PermissionDataError> {
        let source = self.session.subscribe_resources();
        let snapshot = std::sync::Arc::clone(&source.borrow());
        self.fresh(&snapshot)?;
        Ok(Generated {
            generation: snapshot.generation,
            value: if snapshot.dms.iter().any(|dm| dm.id == channel.to_string()) {
                direct_capabilities(&snapshot, channel)?
            } else {
                capabilities(&snapshot, self.session.account().user(), channel)?
            },
        })
    }
    /// Evaluates current global roles, including the backend's actual `manage_server` ban gate.
    /// # Errors
    /// Returns `Unavailable` unless the resource image belongs to a current online connection.
    pub fn global(&self) -> Result<Generated<GlobalPermissions>, PermissionDataError> {
        let source = self.session.subscribe_resources();
        let snapshot = std::sync::Arc::clone(&source.borrow());
        self.fresh(&snapshot)?;
        Ok(Generated {
            generation: snapshot.generation,
            value: global_permissions(&snapshot, self.session.account().user())?,
        })
    }
    fn fresh(&self, snapshot: &ResourceSnapshot) -> Result<(), PermissionDataError> {
        let status = *self.session.subscribe_connection().borrow();
        if self.session.is_closed()
            || status.state != ConnectionState::Online
            || status.generation != snapshot.generation
            || status.generation != self.session.generation()
        {
            return Err(PermissionDataError::Unavailable);
        }
        Ok(())
    }
}

/// Display identity keeps external webhook authors separate from the bridge account.
#[derive(Clone)]
#[non_exhaustive]
pub enum AuthorIdentity {
    /// A backend person, eligible for profile, role, mention and DM projections.
    Person(Uuid),
    /// External app identity. No bridge-user roles, mentions or direct-message target.
    External(papo_sdk::ExternalAuthor),
    /// Webhook without a complete external identity.
    App,
    /// Missing or malformed backend person identity.
    Unavailable,
}
impl AuthorIdentity {
    /// Chooses external identity before inspecting a possible bridge author ID.
    #[must_use]
    pub fn for_message(message: &Message) -> Self {
        if let Some(author) = &message.external_author {
            return Self::External(author.clone());
        }
        if message.webhook_id.is_some() {
            return Self::App;
        }
        message
            .author_id
            .as_deref()
            .and_then(|id| id.parse().ok())
            .map_or(Self::Unavailable, Self::Person)
    }
}

/// A coherent affordance read; missing inputs remain typed rather than becoming default grants.
#[derive(Clone, Debug)]
pub struct CapabilityView {
    /// Connection epoch to validate before presenting or acting on these affordances.
    pub generation: papo_sdk::Generation,
    /// Effective global roles, or unavailable while stale/offline/incomplete.
    pub global: Result<GlobalPermissions, PermissionDataError>,
    /// Channel affordances, or unavailable while stale/offline/incomplete.
    pub channel: Result<ChannelCapabilities, PermissionDataError>,
}
/// Capacity-one source watches keep slow capability consumers bounded.
pub struct CapabilitySubscription {
    /// View captured after both source watches were subscribed.
    pub snapshot: CapabilityView,
    /// Subsequent views after resource or connection changes, closing on revocation.
    pub stream: std::pin::Pin<Box<dyn futures_util::Stream<Item = CapabilityView> + Send>>,
}
impl CapabilityService {
    /// Observes role, overwrite, DM/block and connection changes without starting network work.
    #[must_use]
    pub fn subscribe(&self, channel: Uuid) -> CapabilitySubscription {
        use futures_util::{StreamExt, stream};
        let mut resources = self.session.subscribe_resources();
        let mut connection = self.session.subscribe_connection();
        resources.borrow_and_update();
        connection.borrow_and_update();
        let snapshot = self.view(channel);
        let service = self.clone();
        let changes = stream::unfold(
            (resources, connection, service),
            move |(mut resources, mut connection, service)| async move {
                tokio::select! {
                    result = resources.changed() => result.ok()?,
                    result = connection.changed() => result.ok()?,
                }
                if service.session.is_closed() {
                    return None;
                }
                resources.borrow_and_update();
                connection.borrow_and_update();
                let view = service.view(channel);
                Some((view, (resources, connection, service)))
            },
        );
        CapabilitySubscription {
            snapshot,
            stream: Box::pin(changes.fuse()),
        }
    }
    fn view(&self, channel: Uuid) -> CapabilityView {
        let source = self.session.subscribe_resources();
        let snapshot = std::sync::Arc::clone(&source.borrow());
        let generation = self.session.generation();
        if self.fresh(&snapshot).is_err() {
            return CapabilityView {
                generation,
                global: Err(PermissionDataError::Unavailable),
                channel: Err(PermissionDataError::Unavailable),
            };
        }
        let global = global_permissions(&snapshot, self.session.account().user());
        let channel = if snapshot.dms.iter().any(|dm| dm.id == channel.to_string()) {
            direct_capabilities(&snapshot, channel)
        } else {
            capabilities(&snapshot, self.session.account().user(), channel)
        };
        CapabilityView {
            generation,
            global,
            channel,
        }
    }
}

#[cfg(test)]
mod tests;

/// Server configuration affordances; administrative requests themselves belong to the operations service.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent backend operation gates"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerCapabilities {
    /// Server configuration and moderation settings.
    pub manage_server: bool,
    /// Create/edit/delete channels and their overwrites.
    pub manage_channels: bool,
    /// Create/edit/delete roles and assign membership.
    pub manage_roles: bool,
    /// Create/manage webhook credentials.
    pub manage_webhooks: bool,
    /// Ban/unban nonowner targets. Backend currently gates this with `manage_server`, not `ban_members`.
    pub ban_nonowner: bool,
    /// Reset another person's password; `manage_server` alone does not suffice.
    pub reset_other_password: bool,
}
impl ServerCapabilities {
    /// Uses the actual pinned route/middleware gates rather than similarly named stored flags.
    #[must_use]
    pub const fn from_grants(global: GlobalPermissions) -> Self {
        Self {
            manage_server: global.manage_server,
            manage_channels: global.manage_channels,
            manage_roles: global.manage_roles,
            manage_webhooks: global.manage_webhooks,
            ban_nonowner: global.manage_server,
            reset_other_password: global.owner,
        }
    }
    /// Owner targets are protected by storage even when unbanning.
    #[must_use]
    pub const fn can_ban(self, target_is_owner: bool) -> bool {
        self.ban_nonowner && !target_is_owner
    }
}
