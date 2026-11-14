//! Role definition changes across retained identity contexts; membership fetching stays in refresh.

use crate::ResourceSnapshot;
use papo_api::wire::{Role, RoleSummary};
use std::{collections::HashMap, sync::Arc};

/// Applies authoritative role names/colors and removes deleted role references.
/// Existing assignments are preserved; this never guesses new memberships.
pub(super) fn apply(snapshot: &mut ResourceSnapshot) {
    let roles: HashMap<_, _> = snapshot
        .roles
        .iter()
        .filter_map(|role| role.id.as_ref().map(|id| (id.as_str(), role)))
        .collect();
    for member in Arc::make_mut(&mut snapshot.members) {
        labels(&mut member.roles, &roles);
    }
    for profile in Arc::make_mut(&mut snapshot.profiles) {
        labels(&mut profile.roles, &roles);
    }
    for conversation in Arc::make_mut(&mut snapshot.dms) {
        labels(&mut conversation.user.roles, &roles);
    }
    for blocked in Arc::make_mut(&mut snapshot.blocks) {
        labels(&mut blocked.roles, &roles);
    }
    for channel in Arc::make_mut(&mut snapshot.channels) {
        if let Some(overrides) = &mut channel.permissions {
            overrides.retain_mut(|entry| {
                let Some(role) = entry.role_id.as_deref().and_then(|id| roles.get(id)) else {
                    return false;
                };
                entry.role_name.clone_from(&role.name);
                true
            });
        }
    }
}

fn labels(current: &mut Option<Vec<RoleSummary>>, roles: &HashMap<&str, &Role>) {
    if let Some(current) = current {
        current.retain_mut(|label| {
            let Some(role) = label.id.as_deref().and_then(|id| roles.get(id)) else {
                return false;
            };
            label.name.clone_from(&role.name);
            label.color.clone_from(&role.color);
            true
        });
    }
}

#[cfg(test)]
mod tests {
    //! Pure complete-context regressions without fetching profiles or member rosters.

    use super::*;
    use papo_api::wire::{
        Channel, ChannelPermissionEntry, DirectConversation, UserProfile, UserSummary,
    };

    fn summary() -> UserSummary {
        UserSummary {
            id: Some("member".into()),
            roles: Some(vec![
                RoleSummary {
                    id: Some("retained".into()),
                    name: Some("old".into()),
                    color: Some("old-color".into()),
                },
                RoleSummary {
                    id: Some("deleted".into()),
                    ..RoleSummary::default()
                },
            ]),
            ..UserSummary::default()
        }
    }

    fn snapshot() -> ResourceSnapshot {
        let mut snapshot = ResourceSnapshot::empty(crate::Generation(1));
        snapshot.members = vec![summary()].into();
        snapshot.profiles = vec![UserProfile {
            id: summary().id,
            roles: summary().roles,
            ..UserProfile::default()
        }]
        .into();
        snapshot.blocks = vec![summary()].into();
        snapshot.dms = vec![DirectConversation {
            user: summary(),
            ..DirectConversation::default()
        }]
        .into();
        snapshot.channels = vec![Channel {
            permissions: Some(vec![
                ChannelPermissionEntry {
                    role_id: Some("retained".into()),
                    role_name: Some("old".into()),
                    ..ChannelPermissionEntry::default()
                },
                ChannelPermissionEntry {
                    role_id: Some("deleted".into()),
                    ..ChannelPermissionEntry::default()
                },
            ]),
            ..Channel::default()
        }]
        .into();
        snapshot.roles = vec![Role {
            id: Some("retained".into()),
            name: Some("new".into()),
            color: Some("new-color".into()),
            ..Role::default()
        }]
        .into();
        snapshot
    }

    #[test]
    fn renamed_and_deleted_roles_reconcile_every_retained_identity_context() {
        let mut snapshot = snapshot();
        apply(&mut snapshot);
        for labels in [
            &snapshot.members[0].roles,
            &snapshot.profiles[0].roles,
            &snapshot.blocks[0].roles,
            &snapshot.dms[0].user.roles,
        ] {
            let labels = labels.as_ref().unwrap();
            assert_eq!(labels.len(), 1);
            assert_eq!(labels[0].id.as_deref(), Some("retained"));
            assert_eq!(labels[0].name.as_deref(), Some("new"));
            assert_eq!(labels[0].color.as_deref(), Some("new-color"));
        }
        let overrides = snapshot.channels[0].permissions.as_ref().unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].role_name.as_deref(), Some("new"));
    }

    #[test]
    fn empty_authoritative_role_list_removes_labels_but_preserves_snapshot_identity() {
        let mut snapshot = snapshot();
        snapshot.roles = Arc::default();
        apply(&mut snapshot);
        assert!(snapshot.members[0].roles.as_ref().unwrap().is_empty());
        assert!(snapshot.profiles[0].roles.as_ref().unwrap().is_empty());
        assert!(snapshot.blocks[0].roles.as_ref().unwrap().is_empty());
        assert!(snapshot.dms[0].user.roles.as_ref().unwrap().is_empty());
        assert!(
            snapshot.channels[0]
                .permissions
                .as_ref()
                .unwrap()
                .is_empty()
        );
        assert_eq!(snapshot.members[0].id.as_deref(), Some("member"));
    }
}
