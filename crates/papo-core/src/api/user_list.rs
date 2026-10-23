//! Complete member snapshots over the backend's timestamp/ID pagination.
use super::{Api, ApiError, ApiResult};
use crate::api::models::UserSummary;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
struct ListedUser {
    #[serde(flatten)]
    summary: UserSummary,
    created_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct UserPage {
    #[serde(default, deserialize_with = "listed_users")]
    users: Vec<ListedUser>,
    #[serde(default)]
    has_more: bool,
}

fn listed_users<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<ListedUser>, D::Error> {
    Option::<Vec<ListedUser>>::deserialize(d).map(Option::unwrap_or_default)
}

impl Api {
    pub(super) async fn all_users(&self) -> ApiResult<Vec<UserSummary>> {
        let mut path = "/users".to_owned();
        let mut users = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor: Option<(DateTime<Utc>, String)> = None;
        loop {
            let page: UserPage = self.get(&path).await?;
            let next = if page.has_more {
                let last = page.users.last().ok_or_else(invalid_cursor)?;
                let next = (
                    last.created_at.ok_or_else(invalid_cursor)?,
                    last.summary.id.clone(),
                );
                // A non-advancing response must not turn a partial member list
                // into an apparently complete snapshot (or an infinite loop).
                if next.1.is_empty() || cursor.as_ref().is_some_and(|old| &next <= old) {
                    return Err(invalid_cursor());
                }
                Some(next)
            } else {
                None
            };
            users.extend(
                page.users
                    .into_iter()
                    .map(|user| user.summary)
                    .filter(|user| seen.insert(user.id.clone())),
            );
            let Some(next) = next else { return Ok(users) };
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("order", "asc")
                .append_pair("since", &next.0.to_rfc3339_opts(SecondsFormat::Nanos, true))
                .append_pair("last_id", &next.1)
                .finish();
            path = format!("/users?{query}");
            cursor = Some(next);
        }
    }
}

fn invalid_cursor() -> ApiError {
    ApiError::Decode("Member list cursor did not advance".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn fixture(pages: Vec<Value>) -> (Api, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = Api::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            Arc::new(super::super::Session::default()),
        )
        .unwrap();
        let task = tokio::spawn(async move {
            let mut paths = Vec::new();
            for page in pages {
                let (stream, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                        .await
                        .unwrap()
                        .unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                paths.push(line.split_whitespace().nth(1).unwrap().to_owned());
                loop {
                    line.clear();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                let body = page.to_string();
                reader.get_mut().write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
                ).as_bytes()).await.unwrap();
            }
            paths
        });
        (api, task)
    }

    fn user(id: &str) -> Value {
        json!({"id":id,"username":id,"created_at":"2026-10-23T10:00:00.123456789Z","banned":id=="b"})
    }

    #[tokio::test]
    async fn walks_all_pages_with_exact_cursors_and_keeps_banned_members() {
        let (api, server) = fixture(vec![
            json!({"users":[user("a"),user("b")],"has_more":true}),
            json!({"users":[user("b"),user("c")],"has_more":true}),
            json!({"users":[user("d")],"has_more":false}),
        ])
        .await;
        let users = api.users().await.unwrap();
        assert_eq!(
            users.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c", "d"]
        );
        assert!(users[1].banned);
        let paths = server.await.unwrap();
        assert_eq!(paths[0], "/users");
        for (path, id) in paths[1..].iter().zip(["b", "c"]) {
            let url = url::Url::parse(&format!("http://fixture{path}")).unwrap();
            let query = url
                .query_pairs()
                .collect::<std::collections::HashMap<_, _>>();
            assert_eq!(query["last_id"], id);
            assert_eq!(query["since"], "2026-10-23T10:00:00.123456789Z");
            assert_eq!(query["order"], "asc");
        }
    }

    #[tokio::test]
    async fn malformed_or_repeated_cursors_fail_instead_of_publishing_a_partial_snapshot() {
        for pages in [
            vec![json!({"users":[],"has_more":true})],
            vec![json!({"users":[{"id":"a","username":"a"}],"has_more":true})],
            vec![json!({"users":[user("a")],"has_more":true}); 2],
        ] {
            let (api, server) = fixture(pages).await;
            assert!(matches!(api.users().await, Err(ApiError::Decode(_))));
            server.await.unwrap();
        }
    }
}
