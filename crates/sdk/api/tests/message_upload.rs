//! Verifies multipart message transport against the bounded loopback fixture.

use papo_api::{
    MessageAttachment, MessageUpload, RestClient,
    wire::{EmbedInput, EmbedMediaInput, LoginRequest, RegisterRequest},
};
use papo_test_server::{Config, TestServer};

#[tokio::test]
async fn multipart_upload_preserves_each_filename_and_payload() {
    let server = TestServer::start(Config::default()).await.unwrap();
    let client = RestClient::loopback(&server.base_url()).unwrap();
    let password: String = (0_u8..16)
        .map(|i| char::from(b'A' + i))
        .chain(['!'])
        .collect();
    client
        .register(&RegisterRequest {
            username: "tester".into(),
            password: password.clone(),
        })
        .await
        .unwrap();
    client
        .login(&LoginRequest {
            username: "tester".into(),
            password,
        })
        .await
        .unwrap();
    let files = [
        MessageAttachment {
            filename: "renamed.png",
            bytes: b"first-bytes",
        },
        MessageAttachment {
            filename: "SPOILER_photo.gif",
            bytes: b"second-bytes",
        },
    ];
    let embeds = serde_json::to_string(&[EmbedInput {
        image: Some(EmbedMediaInput {
            attachment_name: Some("renamed.png".into()),
            ..Default::default()
        }),
        ..Default::default()
    }])
    .unwrap();
    client
        .create_message(&MessageUpload {
            channel_id: "channel-id",
            content: Some("caption"),
            reply_to: Some("reply-id"),
            embeds: Some(&embeds),
            attachments: &files,
        })
        .await
        .unwrap();
    let body = String::from_utf8(server.take_message_body().await.unwrap()).unwrap();
    let first = body
        .find("name=\"attachments\"; filename=\"renamed.png\"")
        .unwrap();
    let second = body
        .find("name=\"attachments\"; filename=\"SPOILER_photo.gif\"")
        .unwrap();
    assert!(first < second);
    assert!(body[first..second].contains("first-bytes"));
    assert!(body[second..].contains("second-bytes"));
    assert!(body.contains("caption"));
    assert!(body.contains("reply-id"));
    assert!(body.contains(&embeds));
    assert!(!body.contains("\"url\""));
    assert!(server.take_message_body().await.is_none());
    server.shutdown().await.unwrap();
}
