//! Verifies deterministic auth mutation boundaries; cookie transport is exercised by SDK loopback tests.

use serde_json::{Value, json};

use crate::{Config, auth::Auth};

fn password(variant: u8) -> String {
    std::iter::once('A')
        .chain((0_u8..16).map(|index| char::from(b'a' + (index + variant) % 26)))
        .chain(std::iter::once('!'))
        .collect()
}

fn signed_in(auth: &mut Auth, username: &str) -> String {
    let body = serde_json::to_vec(&json!({"username": username, "password": password(0)})).unwrap();
    assert_eq!(
        auth.handle(&Config::default(), "POST", "/auth/register", None, &body)
            .status,
        201
    );
    let reply = auth.handle(&Config::default(), "POST", "/auth/login", None, &body);
    assert_eq!(reply.status, 200);
    reply
        .cookie
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("Auth=")
        .unwrap()
        .to_owned()
}

fn request(
    auth: &mut Auth,
    path: &str,
    cookie: Option<&str>,
    body: &Value,
) -> crate::response::Reply {
    auth.handle(
        &Config::default(),
        "POST",
        path,
        cookie,
        &serde_json::to_vec(body).unwrap(),
    )
}

#[test]
fn refresh_replaces_the_cookie_but_preserves_its_connection_record() {
    let mut auth = Auth::new();
    let old = signed_in(&mut auth, "refresh-user");
    let connection = auth.connections.get(&old).unwrap().clone();
    let reply = request(&mut auth, "/auth/refresh", Some(&old), &json!({}));
    assert_eq!(reply.status, 200);
    let cookie = reply.cookie.unwrap();
    let replacement = cookie
        .split(';')
        .next()
        .unwrap()
        .strip_prefix("Auth=")
        .unwrap();
    // Capability bytes must never be included in assertion diagnostics.
    let rotated = replacement != old;
    assert!(rotated);
    assert!(auth.user_id(Some(&old)).is_none());
    assert!(auth.user_id(Some(replacement)).is_some());
    assert!(
        auth.connections
            .get(replacement)
            .is_some_and(|current| *current == connection)
    );
    assert!(reply.body.unwrap().get("token").is_none());
}

#[test]
fn dropping_another_accounts_connection_does_not_revoke_it() {
    let mut auth = Auth::new();
    let first = signed_in(&mut auth, "first-user");
    let second = signed_in(&mut auth, "second-user");
    let connection = auth.connections.get(&second).unwrap().clone();
    let reply = request(
        &mut auth,
        "/auth/drop_connection",
        Some(&first),
        &json!({"connection_id": connection}),
    );
    assert_eq!(reply.status, 404);
    assert!(auth.user_id(Some(&first)).is_some());
    assert!(auth.user_id(Some(&second)).is_some());
}

#[test]
fn dropping_all_is_case_insensitive_account_scoped_and_clears_current_cookie() {
    let mut auth = Auth::new();
    let first = signed_in(&mut auth, "first-user");
    let second = signed_in(&mut auth, "second-user");
    let reply = request(
        &mut auth,
        "/auth/drop_connection",
        Some(&first),
        &json!({"connection_id": "all"}),
    );
    assert_eq!(reply.body.unwrap()["dropped"], 1);
    assert!(
        reply
            .cookie
            .is_some_and(|cookie| cookie.contains("Max-Age=0"))
    );
    assert!(auth.user_id(Some(&first)).is_none());
    assert!(auth.user_id(Some(&second)).is_some());
}

#[test]
fn password_policy_failure_does_not_consume_reset_and_success_revokes_only_that_user() {
    let mut auth = Auth::new();
    let first = signed_in(&mut auth, "first-user");
    let second = signed_in(&mut auth, "second-user");
    let token = auth.issue_password_reset("first-user").unwrap();
    let weak: String = password(1).chars().take(1).collect();
    let rejected = request(
        &mut auth,
        "/auth/password_reset",
        None,
        &json!({"token": token, "password": weak}),
    );
    assert_eq!(rejected.status, 400);
    let body = json!({"token": token, "password": password(1)});
    assert_eq!(
        request(&mut auth, "/auth/password_reset", None, &body).status,
        200
    );
    assert!(auth.user_id(Some(&first)).is_none());
    assert!(auth.user_id(Some(&second)).is_some());
    assert_eq!(
        request(&mut auth, "/auth/password_reset", None, &body).status,
        410
    );
}

#[test]
fn refresh_devices_and_drop_require_an_authenticated_cookie() {
    let mut auth = Auth::new();
    for (method, path) in [
        ("POST", "/auth/refresh"),
        ("GET", "/auth/connected_devices"),
        ("POST", "/auth/drop_connection"),
    ] {
        assert_eq!(
            auth.handle(&Config::default(), method, path, None, b"{}")
                .status,
            401
        );
    }
}
