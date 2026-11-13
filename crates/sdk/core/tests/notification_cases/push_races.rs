//! Ambiguous backend-global token transfers and signout admission races.
use super::*;
#[test]
fn ambiguous_transfer_retains_prior_possible_owner_after_new_owner_delete_404() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, _) = setup(&f).await?;
        first
            .notifications()
            .register_push(device("synthetic-token"))
            .await?;
        f.server()
            .fail_next_rest("PUT", "/users/me/push-device", 503)
            .await?;
        assert_eq!(
            second
                .notifications()
                .register_push(device("synthetic-token"))
                .await?,
            PushRegistration::Unknown
        );
        assert_eq!(
            second
                .notifications()
                .unregister_push_device(device("synthetic-token"))
                .await?,
            PushRegistration::Removed
        );
        assert_eq!(
            f.server().push_owner("synthetic-token").await,
            Some(first.account().user().to_string())
        );
        assert_eq!(
            first.notifications().unregister_push().await?,
            PushRegistration::Removed
        );
        assert!(f.server().push_owner("synthetic-token").await.is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
#[test]
fn signout_closes_registration_admission_before_remote_logout_dispatch() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        first
            .notifications()
            .register_push(device("synthetic-token"))
            .await?;
        let mut hold = f.server().hold_next_request("POST", "/auth/logout").await?;
        let copy = first.clone();
        let logout = tokio::spawn(async move { copy.auth().logout().await });
        hold.wait_started().await?;
        assert!(
            first
                .notifications()
                .register_push(device("synthetic-token"))
                .await
                .is_err()
        );
        assert!(f.server().push_owner("synthetic-token").await.is_none());
        hold.release();
        logout.await??;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
