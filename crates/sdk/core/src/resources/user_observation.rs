//! Admitted fields supplied by one user readback. Resolving observations produces the
//! canonical `UserRecord`. Relationships and profile-only details are mutated separately.

use super::{ResourceSnapshot, UserRecord};
use crate::Uuid;
use papo_api::NullableText;
use std::collections::HashMap;

/// Partial observation from a roster, targeted member, profile, DM or block.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UserObservation {
    pub id: Uuid,
    pub username: Option<String>,
    pub nickname: NullableText,
    pub typing: NullableText,
    pub status_message: NullableText,
    pub banned: Option<bool>,
    pub created_at: Option<String>,
    pub roles: Option<Vec<Uuid>>,
}

impl UserObservation {
    /// Social and profile results cannot confer own assignments or change an
    /// authoritative membership ban. A peer's supplied assignments are display evidence.
    /// The commit fence rejects observations started before a newer accepted readback.
    pub fn sparse(mut self, base: &ResourceSnapshot, own: Uuid) -> Self {
        if self.id == own {
            self.roles = None;
            self.banned = None;
        } else if base.members.contains(self.id)
            && base
                .users
                .get(self.id)
                .is_some_and(|user| user.banned.is_some())
        {
            self.banned = None;
        }
        self
    }

    /// Overlay supported fields in increasing source precedence. Each identity is
    /// compared with its published value only after the final observation has won.
    pub fn overlay(self, pending: &mut HashMap<Uuid, Self>) {
        let Some(old) = pending.get_mut(&self.id) else {
            pending.insert(self.id, self);
            return;
        };
        if self.username.is_some() {
            old.username = self.username;
        }
        if !self.nickname.is_missing() {
            old.nickname = self.nickname;
        }
        if !self.typing.is_missing() {
            old.typing = self.typing;
        }
        if !self.status_message.is_missing() {
            old.status_message = self.status_message;
        }
        if self.banned.is_some() {
            old.banned = self.banned;
        }
        if self.created_at.is_some() {
            old.created_at = self.created_at;
        }
        if self.roles.is_some() {
            old.roles = self.roles;
        }
    }

    pub fn resolve(self, old: Option<&UserRecord>) -> UserRecord {
        UserRecord {
            id: self.id,
            username: self
                .username
                .or_else(|| old.and_then(|user| user.username.clone())),
            nickname: text(self.nickname, old.and_then(|user| user.nickname.as_ref())),
            typing: text(self.typing, old.and_then(|user| user.typing.as_ref())),
            status_message: text(
                self.status_message,
                old.and_then(|user| user.status_message.as_ref()),
            ),
            banned: self.banned.or_else(|| old.and_then(|user| user.banned)),
            created_at: self
                .created_at
                .or_else(|| old.and_then(|user| user.created_at.clone())),
            roles: self
                .roles
                .or_else(|| old.and_then(|user| user.roles.clone())),
        }
    }
}

fn text(field: NullableText, previous: Option<&String>) -> Option<String> {
    match field {
        NullableText::Missing => previous.cloned(),
        NullableText::Null => None,
        NullableText::Value(text) => Some(text),
    }
}
