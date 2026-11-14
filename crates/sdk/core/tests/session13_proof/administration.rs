//! Production administration, complete role replacement, channel overrides and owner protection.

use super::{Result, id, is_problem, is_status};
use papo_api::wire::{
    ChangeChannelPositionRequest, ChannelPermissions, CreateChannelRequest, CreateRoleRequest,
    PatchServerRequest, UpdateChannelPermissionsRequest,
};
use papo_sdk::{
    Session, Uuid,
    operations::roles::{RoleGrants, RoleUpdate},
};

async fn prepare_role(owner: &Session, member: &Session) -> Result<Uuid> {
    let operations = owner.operations();
    let server = operations
        .patch_server(PatchServerRequest {
            public: Some(true),
            ..Default::default()
        })
        .await?;
    server.validate(owner)?;
    assert_eq!(server.value.public, Some(true));
    assert!(is_problem(
        &member
            .operations()
            .patch_server(PatchServerRequest {
                public: Some(false),
                ..Default::default()
            })
            .await,
        403,
        &papo_api::ProblemKind::Forbidden,
    ));
    let grants = RoleGrants {
        manage_webhooks: true,
        ..Default::default()
    };
    let role = operations
        .create_role(CreateRoleRequest {
            name: "sdk-operations-role".into(),
            color: Some("#123456".into()),
            permissions: Some(grants.into()),
            users: Some(vec![member.account().user().to_string()]),
        })
        .await?
        .value;
    let role_id = id(role.id.as_deref())?;
    let renamed = operations
        .update_role(
            role_id,
            RoleUpdate::rename(&role, "sdk-renamed-role".into())?,
        )
        .await?
        .value;
    assert_eq!(
        RoleGrants::try_from(renamed.permissions.as_ref().ok_or("permissions")?)?,
        grants
    );
    let assignments = operations
        .assign_roles_batch(vec![
            (member.account().user(), role_id),
            (Uuid::nil(), role_id),
        ])
        .await?;
    assert!(
        assignments[0].is_ok()
            && is_problem(&assignments[1], 404, &papo_api::ProblemKind::NotFound)
    );
    Ok(role_id)
}

pub async fn prove(owner: &Session, member: &Session) -> Result<Uuid> {
    let operations = owner.operations();
    prove_voice_channel(owner).await?;
    let role_id = prepare_role(owner, member).await?;
    let channel_id = prove_channels(owner, role_id).await?;
    operations
        .remove_role(member.account().user(), role_id)
        .await?;
    operations.delete_role(role_id).await?;
    assert!(is_problem(
        &member
            .operations()
            .create_role(CreateRoleRequest {
                name: "denied".into(),
                ..Default::default()
            })
            .await,
        403,
        &papo_api::ProblemKind::Forbidden,
    ));
    assert!(is_status(
        &operations.set_banned(owner.account().user(), true).await,
        409,
    ));
    let ban_results = operations
        .set_banned_batch(vec![
            (member.account().user(), false),
            (owner.account().user(), true),
        ])
        .await?;
    assert!(ban_results[0].is_ok() && is_status(&ban_results[1], 409));
    Ok(channel_id)
}

async fn prove_channels(owner: &Session, role_id: Uuid) -> Result<Uuid> {
    let operations = owner.operations();
    let category = operations
        .create_channel(CreateChannelRequest {
            name: "sdk-operation-category".into(),
            r#type: Some("category".into()),
            ..Default::default()
        })
        .await?
        .value;
    let category_id = id(category.id.as_deref())?;
    let channel = operations
        .create_channel(CreateChannelRequest {
            name: "sdk-operation-channel".into(),
            r#type: Some("text".into()),
            parent_id: Some(category_id.to_string()),
            access_roles: Some(vec![role_id.to_string()]),
            ..Default::default()
        })
        .await?
        .value;
    let channel_id = id(channel.id.as_deref())?;
    assert_eq!(
        channel.parent_id.as_deref(),
        Some(category_id.to_string().as_str())
    );
    let overrides = operations.channel_permissions(channel_id).await?.value;
    assert!(overrides.permissions.as_ref().is_some_and(|entries| {
        entries
            .iter()
            .any(|entry| entry.role_id.as_deref() == Some(role_id.to_string().as_str()))
    }));
    let changed = operations
        .update_channel_permissions(
            channel_id,
            role_id,
            UpdateChannelPermissionsRequest {
                permissions: ChannelPermissions {
                    read_channel: Some(true),
                    send_messages: Some(false),
                    delete_messages: Some(true),
                    connect_voice: Some(false),
                },
            },
        )
        .await?
        .value;
    assert_eq!(
        changed.permissions.ok_or("override")?.send_messages,
        Some(false)
    );
    let position = channel.position.ok_or("channel position")?;
    let moved = operations
        .move_channel(
            channel_id,
            ChangeChannelPositionRequest {
                old_position: position,
                new_position: position,
                parent_id: Some(String::new()),
            },
        )
        .await?
        .value;
    assert!(moved.parent_id.is_none());
    operations.remove_channel_role(channel_id, role_id).await?;
    Ok(channel_id)
}

async fn prove_voice_channel(owner: &Session) -> Result {
    let operations = owner.operations();
    let channel = operations
        .create_channel(CreateChannelRequest {
            name: "sdk-operation-voice".into(),
            r#type: Some("voice".into()),
            topic: Some("synthetic voice topic".into()),
            ..Default::default()
        })
        .await?
        .value;
    let channel_id = id(channel.id.as_deref())?;
    assert_eq!(channel.r#type.as_deref(), Some("voice"));
    let updated = operations
        .update_channel(
            channel_id,
            papo_api::wire::UpdateChannelRequest {
                name: "sdk-operation-voice-renamed".into(),
                topic: Some(String::new()),
            },
        )
        .await?
        .value;
    assert_eq!(updated.name.as_deref(), Some("sdk-operation-voice-renamed"));
    operations.delete_channel(channel_id).await?;
    Ok(())
}
