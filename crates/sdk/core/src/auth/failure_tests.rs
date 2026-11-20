//! Pins how authentication failures are sorted, without a server; the loopback tests in
//! `tests/loopback/auth.rs` show the test backend's real replies land in these categories.

use papo_api::wire::Problem;

use super::*;

fn problem(status: u16, slug: &str) -> ClientError {
    ClientError::Api(papo_api::Error::Problem {
        status,
        problem: Box::new(Problem {
            r#type: format!("/errors/{slug}"),
            title: String::new(),
            status: i64::from(status),
            detail: None,
            instance: None,
        }),
    })
}

fn http(status: u16) -> ClientError {
    ClientError::Api(papo_api::Error::Http { status })
}

#[test]
fn a_missing_server_password_is_told_apart_from_wrong_credentials() {
    assert_eq!(
        AuthFailure::classify(&problem(401, "server-access-required")),
        Some(AuthFailure::ServerAccessRequired)
    );
    assert_eq!(
        AuthFailure::classify(&problem(401, "invalid-credentials")),
        Some(AuthFailure::InvalidCredentials)
    );
}

#[test]
fn a_rejection_without_a_problem_document_is_wrong_credentials() {
    assert_eq!(
        AuthFailure::classify(&http(401)),
        Some(AuthFailure::InvalidCredentials)
    );
    assert_eq!(
        AuthFailure::classify(&problem(401, "unauthorized")),
        Some(AuthFailure::InvalidCredentials)
    );
}

#[test]
fn a_banned_account_and_a_taken_name_are_their_own_reasons() {
    assert_eq!(
        AuthFailure::classify(&problem(403, "banned")),
        Some(AuthFailure::Banned)
    );
    assert_eq!(
        AuthFailure::classify(&problem(409, "username-taken")),
        Some(AuthFailure::UsernameTaken)
    );
}

#[test]
fn too_many_attempts_is_not_a_server_that_failed_to_answer() {
    assert_eq!(
        AuthFailure::classify(&http(429)),
        Some(AuthFailure::RateLimited)
    );
    assert_eq!(
        AuthFailure::classify(&problem(429, "rate-limited")),
        Some(AuthFailure::RateLimited)
    );
    for error in [
        http(500),
        problem(503, "internal"),
        ClientError::Timeout,
        ClientError::Api(papo_api::Error::Transport),
        ClientError::Api(papo_api::Error::NotDispatched),
        ClientError::Api(papo_api::Error::Decode),
    ] {
        assert_eq!(
            AuthFailure::classify(&error),
            Some(AuthFailure::NoResponse),
            "{error:?}"
        );
    }
}

#[test]
fn failures_that_are_not_the_servers_answer_are_left_to_the_caller() {
    for error in [
        problem(400, "invalid-param"),
        problem(401, "some-future-problem"),
        http(404),
        ClientError::Api(papo_api::Error::Credentials),
        ClientError::Api(papo_api::Error::Address),
        ClientError::Identity,
        ClientError::InvalidAuth,
        ClientError::Closed,
    ] {
        assert_eq!(AuthFailure::classify(&error), None, "{error:?}");
    }
}
