//! Exercises media transport through the deterministic loopback test backend.

use papo_api::{
    Error, RestClient, Uuid,
    media::{ByteRange, MediaError},
    wire::LoginRequest,
};
use papo_test_server::MediaTestServer;

#[tokio::test]
async fn media_ranges_redirects_and_public_avatars_preserve_cookie_isolation() {
    let server = MediaTestServer::start().await.unwrap();
    let client = RestClient::loopback(&server.base_url()).unwrap();
    client
        .login(&LoginRequest {
            username: "tester".into(),
            password: "not-a-secret".into(),
        })
        .await
        .unwrap();
    let id = Uuid::nil();
    let range = Some(ByteRange::Bounded {
        start_bytes: 2,
        end_bytes: 4,
    });
    let partial = client.media_bytes(&"a".repeat(64), range).await.unwrap();
    assert_eq!(partial.status, 206);
    assert_eq!(partial.bytes, b"cde");
    assert_eq!(partial.headers["content-range"], "bytes 2-4/10");
    assert_eq!(partial.headers["accept-ranges"], "bytes");
    assert_eq!(partial.headers["etag"], "\"v1\"");
    let ignored = client.attachment_bytes(id, range).await.unwrap();
    assert_eq!(ignored.status, 200);
    assert_eq!(ignored.bytes, b"full");
    assert_eq!(
        client.attachment_thumbnail_bytes(id).await.unwrap().bytes,
        b"thumb"
    );
    assert!(matches!(
        client.attachment_bytes(id, range).await,
        Err(MediaError::Api(Error::Http { status: 416 }))
    ));
    assert!(matches!(
        client.embed_video_bytes(id, range).await,
        Err(MediaError::Api(Error::Http { status: 302 }))
    ));
    assert_eq!(
        client.public_user_avatar_bytes(id).await.unwrap().bytes,
        b"u"
    );
    assert_eq!(
        client.public_webhook_avatar_bytes(id).await.unwrap().bytes,
        b"w"
    );
    client.media_bytes(&"a".repeat(64), None).await.unwrap();
    let requests = server.finish().await;
    assert_eq!(requests.len(), 9);
    for index in [1, 2, 3, 4, 5, 8] {
        assert_eq!(requests[index].cookie.as_deref(), Some("fixture=original"));
    }
    for index in [6, 7] {
        assert!(requests[index].cookie.is_none());
    }
    for index in [1, 2, 4, 5] {
        assert_eq!(requests[index].range.as_deref(), Some("bytes=2-4"));
    }
    assert!(requests[3].range.is_none());
    assert_eq!(requests[5].path, format!("/embeds/{id}/video"));
    assert_eq!(requests[6].path, format!("/users/{id}/avatar"));
    assert_eq!(requests[7].path, format!("/webhooks/{id}/avatar"));
}
