//! Acknowledged role membership refresh updates requested profiles without roster refetch.
use super::{Fixture, Result, setup};
use papo_sdk::{RefreshStatus, operations::roles::CreateRoleRequest};

#[test]
fn assignment_and_removal_update_already_requested_profile_roles() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, person, _) = setup(&f).await?;
        let user = person.account().user();
        session.profiles(&[user]).await?;
        let role = session
            .operations()
            .create_role(CreateRoleRequest {
                name: "fixture-role".into(),
                ..Default::default()
            })
            .await?;
        let role = role.value.id.ok_or("missing role")?.parse()?;
        let receipt = session.operations().assign_role(user, role).await?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        let resources = session.subscribe_resources().borrow().clone();
        assert!(resources.profiles.get(user).is_some());
        let assignments = resources
            .users
            .get(user)
            .and_then(|summary| summary.roles.as_ref())
            .ok_or("missing assignments")?;
        assert!(assignments.contains(&role));
        let receipt = session.operations().remove_role(user, role).await?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        let resources = session.subscribe_resources().borrow().clone();
        assert!(resources.profiles.get(user).is_some());
        let assignments = resources
            .users
            .get(user)
            .and_then(|summary| summary.roles.as_ref())
            .ok_or("missing assignments")?;
        assert!(!assignments.contains(&role));
        let requests = f.server().take_operation_requests().await;
        assert!(
            !requests
                .iter()
                .any(|(method, path, _, _)| method == "GET" && path == "/users")
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
