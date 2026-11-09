//! Owns role and membership REST calls; authorization remains on the backend.

use crate::{
    Error, RestClient, Uuid, routes,
    wire::{
        AssignRoleRequest, CreateRoleRequest, Role, RoleListResponse, UpdateRoleRequest, UserRole,
    },
};
impl RestClient {
    /// Lists server roles.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn roles(&self) -> Result<RoleListResponse, Error> {
        self.json(self.route_request(&routes::GET_ROLES, &[])?)
            .await
    }

    /// Creates a server role.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn create_role(&self, request: &CreateRoleRequest) -> Result<Role, Error> {
        self.post(&routes::POST_ROLES, &[], request).await
    }

    /// Replaces a server role's name, color and permissions.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn update_role(
        &self,
        role: Uuid,
        request: &UpdateRoleRequest,
    ) -> Result<Role, Error> {
        self.json(
            self.route_request(&routes::PUT_ROLES_ROLE_ID, &[role.to_string()])?
                .json(request),
        )
        .await
    }

    /// Deletes a server role.
    /// # Errors
    /// Returns transport or backend failures.
    pub async fn delete_role(&self, role: Uuid) -> Result<(), Error> {
        self.response(self.route_request(&routes::DELETE_ROLES_ROLE_ID, &[role.to_string()])?)
            .await
            .map(|_| ())
    }

    /// Assigns a role to a member.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn assign_role(
        &self,
        user: Uuid,
        request: &AssignRoleRequest,
    ) -> Result<UserRole, Error> {
        self.post(
            &routes::POST_USERS_USER_ID_ROLES,
            &[user.to_string()],
            request,
        )
        .await
    }

    /// Removes a member's role assignment.
    /// # Errors
    /// Returns transport or backend failures.
    pub async fn remove_role(&self, user: Uuid, role: Uuid) -> Result<(), Error> {
        self.response(self.route_request(
            &routes::DELETE_USERS_USER_ID_ROLES_ROLE_ID,
            &[user.to_string(), role.to_string()],
        )?)
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    //! Verifies role-list response decoding.

    use super::*;

    #[test]
    fn empty_roles_response_decodes() {
        let list: RoleListResponse = serde_json::from_str(r#"{"roles":[]}"#).unwrap();
        assert!(list.roles.unwrap().is_empty());
    }
}
