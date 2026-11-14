//! Production reset-link ownership and self-only status/avatar/banner acknowledgments.

use super::{PNG, Result, is_problem};
use papo_api::wire::{UpdateAvatarRequest, UpdateBannerRequest, UpdateStatusRequest};
use papo_sdk::{Session, operations::profile::PasswordResetPreparation};

pub async fn prove(owner: &Session, member: &Session, other: &Session) -> Result {
    let operations = member.operations();
    operations
        .update_status(UpdateStatusRequest {
            status: Some("busy".into()),
        })
        .await?;
    operations
        .update_status(UpdateStatusRequest::default())
        .await?;
    operations
        .update_avatar(UpdateAvatarRequest {
            avatar: PNG.into(),
            avatar_format: "PNG".into(),
        })
        .await?;
    operations
        .update_banner(UpdateBannerRequest {
            banner: PNG.into(),
            banner_format: "PNG".into(),
        })
        .await?;
    operations
        .update_banner(UpdateBannerRequest {
            banner: String::new(),
            banner_format: String::new(),
        })
        .await?;
    assert!(is_problem(
        &operations
            .prepare_password_reset(other.account().user())
            .await,
        403,
        &papo_api::ProblemKind::Forbidden,
    ));
    let receipt = owner
        .operations()
        .prepare_password_reset(other.account().user())
        .await?;
    receipt.validate(owner)?;
    let PasswordResetPreparation::Link(link) = receipt.value else {
        return Err("owner reset omitted the scoped link".into());
    };
    assert_eq!(link.account(), owner.account());
    assert_eq!(link.target(), other.account().user());
    assert_ne!(link.expires_at(), "");
    chrono::DateTime::parse_from_rfc3339(link.expires_at())?;
    let parsed = url::Url::parse(link.expose_url())?;
    assert!(
        parsed
            .path()
            .rsplit('/')
            .next()
            .is_some_and(|token| token.len() == 43)
    );
    Ok(())
}
