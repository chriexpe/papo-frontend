//! Account-scoped identity; authentication and cache ownership verification belong to sessions.

use papo_api::Uuid;

use crate::ServerId;

/// An account is a backend user UUID paired with one canonical server identity.
///
/// Constructing an ID does not authenticate a user or authorize cache access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountId {
    server: ServerId,
    user: Uuid,
}

impl AccountId {
    /// Pairs a server identity with its backend user UUID.
    #[must_use]
    pub const fn new(server: ServerId, user: Uuid) -> Self {
        Self { server, user }
    }

    /// Returns the account's canonical server identity.
    #[must_use]
    pub const fn server(&self) -> ServerId {
        self.server
    }

    /// Returns the backend user UUID, never used for chronological ordering.
    #[must_use]
    pub const fn user(&self) -> Uuid {
        self.user
    }
}

#[cfg(test)]
mod tests {
    //! Pins account scoping without authentication or storage.

    use super::*;
    use crate::ServerAddress;

    #[test]
    fn different_users_on_one_server_are_distinct_accounts() {
        let server = ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id();
        assert_ne!(
            AccountId::new(server, Uuid::from_u128(1)),
            AccountId::new(server, Uuid::from_u128(2))
        );
    }

    #[test]
    fn one_user_on_different_servers_is_two_accounts() {
        let first = ServerAddress::parse("https://example.invalid/A")
            .unwrap()
            .id();
        let second = ServerAddress::parse("https://example.invalid/a")
            .unwrap()
            .id();
        assert_ne!(
            AccountId::new(first, Uuid::from_u128(1)),
            AccountId::new(second, Uuid::from_u128(1))
        );
    }

    #[test]
    fn account_accessors_preserve_both_identity_components() {
        let server = ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id();
        let user = Uuid::from_u128(1);
        let account = AccountId::new(server, user);
        assert_eq!((account.server(), account.user()), (server, user));
    }
}
