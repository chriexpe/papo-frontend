//! Why an explicit authentication call failed, in the categories clients word differently.
//! The calls themselves are `AuthService`'s; wording and what to show belong to clients.

use crate::ClientError;

/// Why `server_login`, `register` or `login` failed, for the reasons a person can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthFailure {
    /// The server wants its shared password first (`server-access-required`): it was never
    /// given on this attempt, or the access an earlier `server_login` granted has lapsed.
    ServerAccessRequired,
    /// The server rejected the password it was given: the account's at `login`, the server's
    /// at `server_login` (401 with the `invalid-credentials` or `unauthorized` problem, or
    /// with no problem document).
    InvalidCredentials,
    /// The account is banned from the server (the `banned` problem).
    Banned,
    /// `register` asked for a username the server already has (`username-taken`).
    UsernameTaken,
    /// The server refused for too many attempts (429). Waiting helps; trying again at once
    /// does not.
    RateLimited,
    /// No usable answer: the connection failed or timed out, the reply was undecodable, or
    /// the server failed (5xx). Trying again may help.
    NoResponse,
}

impl AuthFailure {
    /// Classifies a failure of an explicit authentication call. `None` means the server's
    /// answer was not one of these: a request the server rejected for another reason
    /// (`invalid-param`, say), a request never built (an invalid address or credential
    /// snapshot), a mismatched identity, credential storage, or a closed client.
    #[must_use]
    pub fn classify(error: &ClientError) -> Option<Self> {
        use papo_api::{Error as Api, ProblemKind};
        let api = match error {
            ClientError::Timeout => return Some(Self::NoResponse),
            ClientError::Api(api) => api,
            _ => return None,
        };
        match api {
            Api::Transport | Api::NotDispatched | Api::Decode => Some(Self::NoResponse),
            Api::Http { status: 429 } | Api::Problem { status: 429, .. } => Some(Self::RateLimited),
            Api::Http { status } | Api::Problem { status, .. } if (500..=599).contains(status) => {
                Some(Self::NoResponse)
            }
            // By problem type, not status: the backend sends a missing server password as 401,
            // as it does wrong credentials.
            Api::Problem { problem, .. } => match problem.kind() {
                ProblemKind::ServerAccessRequired => Some(Self::ServerAccessRequired),
                ProblemKind::InvalidCredentials | ProblemKind::Unauthorized => {
                    Some(Self::InvalidCredentials)
                }
                ProblemKind::Banned => Some(Self::Banned),
                ProblemKind::UsernameTaken => Some(Self::UsernameTaken),
                _ => None,
            },
            Api::Http { status: 401 } => Some(Self::InvalidCredentials),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod tests;
