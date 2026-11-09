//! Owns sanitized transport failures and typed backend problem classification; leaves recovery to callers.

/// Machine-readable classification of a backend RFC 7807 problem URI.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProblemKind {
    /// The session is absent or invalid.
    Unauthorized,
    /// Credentials were rejected.
    InvalidCredentials,
    /// Permission was denied.
    Forbidden,
    /// The account is banned.
    Banned,
    /// A resource is missing.
    NotFound,
    /// A request field is invalid.
    InvalidParam,
    /// The username is already registered.
    UsernameTaken,
    /// Temporary private-server authorization is required.
    ServerAccessRequired,
    /// A reset link is invalid, expired or consumed.
    ResetLinkInvalid,
    /// The session token was reused and revoked.
    ConnectionReused,
    /// A server already exists.
    ServerAlreadyExists,
    /// The channel name is already used.
    ChannelNameTaken,
    /// The backend channel count limit was reached.
    ChannelLimitReached,
    /// The direct-message participant blocked delivery.
    DirectMessageBlocked,
    /// The request was rate limited.
    RateLimited,
    /// An internal backend failure occurred.
    Internal,
    /// A future or route-specific problem slug. It is intentionally not logged.
    Unknown,
}

impl crate::wire::Problem {
    /// Classifies only the URI path, without exposing backend text or addresses.
    #[must_use]
    pub fn kind(&self) -> ProblemKind {
        let path = url::Url::parse(&self.r#type)
            .map_or_else(|_| self.r#type.clone(), |url| url.path().to_owned());
        match path.as_str() {
            "/errors/unauthorized" => ProblemKind::Unauthorized,
            "/errors/invalid-credentials" => ProblemKind::InvalidCredentials,
            "/errors/forbidden" => ProblemKind::Forbidden,
            "/errors/banned" => ProblemKind::Banned,
            "/errors/not-found" => ProblemKind::NotFound,
            "/errors/invalid-param" => ProblemKind::InvalidParam,
            "/errors/username-taken" => ProblemKind::UsernameTaken,
            "/errors/server-access-required" => ProblemKind::ServerAccessRequired,
            "/errors/reset-link-invalid" => ProblemKind::ResetLinkInvalid,
            "/errors/connection-reused" => ProblemKind::ConnectionReused,
            "/errors/server-already-exists" => ProblemKind::ServerAlreadyExists,
            "/errors/channel-name-taken" => ProblemKind::ChannelNameTaken,
            "/errors/channel-limit-reached" => ProblemKind::ChannelLimitReached,
            "/errors/dm-blocked" => ProblemKind::DirectMessageBlocked,
            "/errors/rate-limit" => ProblemKind::RateLimited,
            "/errors/internal" => ProblemKind::Internal,
            _ => ProblemKind::Unknown,
        }
    }
}

/// Failures retain typed status/problem data, but never format server-controlled data.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The supplied base URL violates the HTTP transport contract.
    #[error("invalid API address")]
    Address,
    /// Credential snapshot encoding, bounds or scope validation failed.
    #[error("invalid API credential snapshot")]
    Credentials,
    /// A request failed; dispatch may have occurred. No automatic retry is performed.
    #[error("API transport failed")]
    Transport,
    /// A successful response could not be decoded.
    #[error("invalid API response")]
    Decode,
    /// A backend problem. The HTTP status is authoritative, independent of its body.
    #[error("backend rejected request ({status})")]
    Problem {
        /// Actual HTTP response status.
        status: u16,
        /// Decoded problem, including unknown future problem types.
        problem: Box<crate::wire::Problem>,
    },
    /// An error response did not contain a valid problem document.
    #[error("backend HTTP failure ({status})")]
    Http {
        /// Actual HTTP response status.
        status: u16,
    },
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    //! Owns problem classification tests; wire payloads are never formatted.
    use super::ProblemKind;

    #[test]
    fn implemented_auth_problems_have_machine_readable_kinds() {
        for (slug, expected) in [
            ("username-taken", ProblemKind::UsernameTaken),
            ("server-access-required", ProblemKind::ServerAccessRequired),
            ("reset-link-invalid", ProblemKind::ResetLinkInvalid),
            ("connection-reused", ProblemKind::ConnectionReused),
        ] {
            let problem = crate::wire::Problem {
                r#type: format!("/errors/{slug}"),
                ..Default::default()
            };
            assert_eq!(problem.kind(), expected);
        }
    }
}
