//! Verifies history tuples and multipart embed uploads against the pinned backend and PostgreSQL.
//! Run only through the isolated loopback proof harness, never against a user server.

use papo_api::{
    HistoryOrder, HistoryQuery, RestClient, Uuid,
    wire::{CreateChannelRequest, CreateServerRequest, LoginRequest, RegisterRequest},
};

#[path = "postgres_upload/mod.rs"]
mod postgres_upload;

#[tokio::test]
#[ignore = "requires the pinned backend with a fresh PostgreSQL database"]
async fn pinned_backend_preserves_history_tuples_and_multipart_embed_references() {
    let (client, channel) = seed_backend().await;
    let expected: Vec<_> = (1..=7)
        .map(|n| format!("00000000-0000-4000-8000-{n:012}"))
        .collect();
    for limit in [1, 2, 3, 7, 8] {
        for order in [HistoryOrder::Asc, HistoryOrder::Desc] {
            let mut cursor = None;
            let mut actual = Vec::new();
            let mut page_count = 0;
            loop {
                let page = client
                    .messages(
                        channel,
                        &HistoryQuery {
                            cursor: cursor.as_ref(),
                            order,
                            limit: Some(limit),
                        },
                    )
                    .await
                    .unwrap();
                let messages = page.messages.unwrap();
                assert!(messages.len() <= usize::from(limit));
                if let Some(previous) = &cursor {
                    assert!(
                        messages
                            .iter()
                            .all(|message| message.id.as_ref() != Some(&previous.id))
                    );
                }
                for message in &messages {
                    let next = message.cursor().unwrap();
                    // Decode/re-encode preserves exactly the timestamp supplied by PostgreSQL/Go.
                    let value = serde_json::to_value(message).unwrap();
                    assert_eq!(value["created_at"].as_str(), Some(next.created_at.as_str()));
                    if expected[1..6].contains(&next.id) {
                        assert!(next.created_at.contains(".123456"));
                    }
                    actual.push(next.id.clone());
                    cursor = Some(next);
                }
                page_count += 1;
                assert!(page_count <= 8);
                let has_more = page.has_more.unwrap();
                assert_eq!(has_more, actual.len() < expected.len());
                if !has_more {
                    break;
                }
                assert!(!messages.is_empty());
            }
            let wanted: Vec<_> = match order {
                HistoryOrder::Asc => expected.clone(),
                HistoryOrder::Desc => expected.iter().rev().cloned().collect(),
            };
            assert_eq!(actual, wanted);
            let empty = client
                .messages(
                    channel,
                    &HistoryQuery {
                        cursor: cursor.as_ref(),
                        order,
                        limit: Some(limit),
                    },
                )
                .await
                .unwrap();
            assert!(empty.messages.unwrap().is_empty());
            assert_eq!(empty.has_more, Some(false));
        }
    }
    reverse_at_interior_boundary(&client, channel, &expected).await;
    postgres_upload::verify(&client).await;
}

async fn reverse_at_interior_boundary(client: &RestClient, channel: Uuid, expected: &[String]) {
    // Reverse from an interior equal-timestamp boundary, not just from the channel edges.
    let page = client
        .messages(
            channel,
            &HistoryQuery {
                order: HistoryOrder::Asc,
                limit: Some(4),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let cursor = page.messages.unwrap()[3].cursor().unwrap();
    for (order, wanted) in [
        (HistoryOrder::Asc, expected[4..].to_vec()),
        (
            HistoryOrder::Desc,
            expected[..3].iter().rev().cloned().collect(),
        ),
    ] {
        let page = client
            .messages(
                channel,
                &HistoryQuery {
                    cursor: Some(&cursor),
                    order,
                    limit: Some(100),
                },
            )
            .await
            .unwrap();
        let actual: Vec<_> = page
            .messages
            .unwrap()
            .into_iter()
            .map(|message| message.id.unwrap())
            .collect();
        assert_eq!(actual, wanted);
    }
}

async fn seed_backend() -> (RestClient, Uuid) {
    let address = std::env::var("PAPO_BACKEND_PROOF_ADDRESS").unwrap();
    let password = std::env::var("PAPO_PROOF_PASSWORD").unwrap();
    let client = RestClient::loopback(&address).unwrap();
    client
        .register(&RegisterRequest {
            username: "cursorproof".into(),
            password: password.clone(),
        })
        .await
        .unwrap();
    client
        .login(&LoginRequest {
            username: "cursorproof".into(),
            password,
        })
        .await
        .unwrap();
    client
        .create_server(&CreateServerRequest {
            name: "Cursor proof".into(),
            public: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    let channel = client
        .create_channel(&CreateChannelRequest {
            name: "cursor-proof".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let channel: Uuid = channel.id.unwrap().parse().unwrap();
    let seed = reqwest::Client::new()
        .post(format!("{address}/__proof/seed"))
        .send()
        .await
        .unwrap();
    assert_eq!(seed.status(), 204);
    (client, channel)
}
