//! Public account media isolation, conditional recovery and provider credential separation over loopback.

#![cfg(feature = "test-support")]
mod fixture;
use fixture::{Fixture, Result, login, register};
use papo_sdk::{MediaError, MediaSource, Session, UnixMicros, Uuid};
use papo_test_server::{Config, EncodedMedia};
use std::collections::BTreeSet;

#[test]
fn giphy_transport_is_cookie_free_paginated_and_generation_scoped() -> Result {
    let mut f = Fixture::new(Config::default())?;
    let account =
        f.run(async { Ok::<_, Box<dyn std::error::Error>>(accounts(&f).await?.0.account()) })??;
    let endpoint = format!("{}/catalog/trending", f.server().base_url());
    f.config.giphy = Some(papo_sdk::GiphyConfig::loopback(
        (0..32).map(|_| 'x').collect(),
        endpoint,
    )?);
    f.reopen()?;
    f.run(async {
        let session = f.client().handle().session(account)?;
        fixture::online(&session).await?;
        let body = serde_json::json!({
            "meta": {"status": 200},
            "pagination": {"offset": 0, "count": 1, "total_count": 2},
            "data": [{
                "id": "fixture", "url": "https://example.invalid/attribution", "title": "",
                "images": {"original": {"url": "https://example.invalid/encoded.gif"}}
            }]
        });
        f.server()
            .set_encoded_media(EncodedMedia {
                path: "/catalog/trending".into(),
                bytes: serde_json::to_vec(&body)?,
                users: BTreeSet::new(),
                etag: None,
                status: 200,
                content_range: None,
            })
            .await?;
        let page = session.expressions().trending(0).await?;
        assert_eq!(page.value.gifs.len(), 1);
        assert_eq!(page.value.next_offset, Some(1));
        let observations = f.server().take_media_observations().await;
        assert_eq!(observations.len(), 1);
        assert!(
            observations
                .iter()
                .all(|request| !request.cookie && !request.authorization)
        );
        let mut hold = f
            .server()
            .hold_next_rest("GET", "/catalog/trending")
            .await?;
        let library = session.expressions();
        let pending = tokio::spawn(async move { library.search("exact query".into(), 0).await });
        hold.wait_started().await?;
        session.reconnect()?;
        assert!(matches!(
            pending.await?,
            Err(papo_sdk::ExpressionError::Session(
                papo_sdk::ClientError::Superseded
            ))
        ));
        hold.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

async fn accounts(f: &Fixture) -> Result<(Session, Session)> {
    let client = f.client().handle();
    let server = client
        .add_server(&f.server().base_url(), UnixMicros(0))
        .await?;
    let auth = client.auth(server)?;
    register(&auth, "media-first").await?;
    let first = login(&auth, "media-first").await?;
    fixture::online(&first).await?;
    let auth = client.auth(server)?;
    register(&auth, "media-second").await?;
    let second = login(&auth, "media-second").await?;
    fixture::online(&second).await?;
    Ok((first, second))
}
fn encoded(path: String, users: BTreeSet<String>) -> EncodedMedia {
    EncodedMedia {
        path,
        bytes: vec![5; 8193],
        users,
        etag: None,
        status: 200,
        content_range: None,
    }
}

#[test]
fn partial_and_ignored_ranges_never_create_full_object_cache_grants() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2500);
        let whole = MediaSource::Attachment(id);
        let range = MediaSource::Range {
            source: Box::new(whole.clone()),
            start_bytes: 2,
            end_bytes: 4,
        };
        let path = format!("/attachments/{id}");
        let mut entry = encoded(
            path.clone(),
            BTreeSet::from([session.account().user().to_string()]),
        );
        entry.bytes = vec![7; 3];
        entry.status = 206;
        entry.content_range = Some("bytes 2-4/10".into());
        f.server().set_encoded_media(entry).await?;
        let selected = session.media().fetch(range.clone(), false).await?.value;
        assert_eq!(selected.bytes.len(), 3);
        assert_eq!(selected.range.unwrap().total_bytes, Some(10));
        assert!(matches!(
            session.media().cached(whole.clone()).await,
            Err(MediaError::Unavailable)
        ));
        assert!(session.media().fetch(whole.clone(), false).await.is_err());
        f.server()
            .set_encoded_media(encoded(
                path,
                BTreeSet::from([session.account().user().to_string()]),
            ))
            .await?;
        assert_eq!(
            session.media().fetch(range, false).await?.value.bytes.len(),
            3
        );
        let other = MediaSource::Range {
            source: Box::new(whole.clone()),
            start_bytes: 100,
            end_bytes: 199,
        };
        let fallback = session.media().fetch(other, false).await?.value;
        assert_eq!(fallback.bytes.len(), 100);
        assert_eq!(fallback.range.unwrap().start_bytes, 100);
        assert!(matches!(
            session.media().cached(whole).await,
            Err(MediaError::Unavailable)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn authorization_denial_retires_full_and_range_siblings() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2502);
        let whole = MediaSource::Attachment(id);
        let range = MediaSource::Range {
            source: Box::new(whole.clone()),
            start_bytes: 2,
            end_bytes: 4,
        };
        let path = format!("/attachments/{id}");
        for denied_source in [whole.clone(), range.clone()] {
            let authorized = BTreeSet::from([session.account().user().to_string()]);
            f.server()
                .set_encoded_media(encoded(path.clone(), authorized))
                .await?;
            session.media().fetch(whole.clone(), false).await?;
            session.media().fetch(range.clone(), false).await?;
            let mut denied = encoded(path.clone(), BTreeSet::new());
            denied.status = 403;
            f.server().set_encoded_media(denied).await?;
            assert!(session.media().fetch(denied_source, true).await.is_err());
            for sibling in [whole.clone(), range.clone()] {
                assert!(matches!(
                    session.media().cached(sibling).await,
                    Err(MediaError::Unavailable)
                ));
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn account_removal_finishes_while_its_media_response_is_held_and_late_bytes_cannot_restore_refs()
-> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, other) = accounts(&f).await?;
        let id = Uuid::from_u128(2501);
        let source = MediaSource::Attachment(id);
        let path = format!("/attachments/{id}");
        f.server()
            .set_encoded_media(encoded(
                path.clone(),
                BTreeSet::from([session.account().user().to_string()]),
            ))
            .await?;
        let mut hold = f.server().hold_next_rest("GET", &path).await?;
        let media = session.media();
        let pending = tokio::spawn(async move { media.fetch(source, false).await });
        hold.wait_started().await?;
        f.client()
            .handle()
            .remove_account(session.account())
            .await?;
        assert!(pending.await?.is_err());
        hold.release();
        assert!(matches!(
            other.media().cached(MediaSource::Attachment(id)).await,
            Err(MediaError::Unavailable)
        ));
        assert_eq!(f.client().handle().disk_usage().await?.media_bytes, 0);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn scoped_refs_deduplicate_but_cannot_be_borrowed_and_removal_preserves_the_other_owner() -> Result
{
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (first, second) = accounts(&f).await?;
        let id = Uuid::from_u128(2001);
        let source = MediaSource::Attachment(id);
        let path = format!("/attachments/{id}");
        f.server()
            .set_encoded_media(encoded(
                path.clone(),
                BTreeSet::from([first.account().user().to_string()]),
            ))
            .await?;
        let downloaded = first.media().fetch(source.clone(), false).await?;
        let before = f.client().handle().disk_usage().await?;
        assert!(matches!(
            second.media().cached(source.clone()).await,
            Err(MediaError::Unavailable)
        ));
        assert!(second.media().fetch(source.clone(), false).await.is_err());
        f.server()
            .set_encoded_media(encoded(
                path,
                BTreeSet::from([
                    first.account().user().to_string(),
                    second.account().user().to_string(),
                ]),
            ))
            .await?;
        assert_eq!(
            second
                .media()
                .fetch(source.clone(), false)
                .await?
                .value
                .bytes,
            downloaded.value.bytes
        );
        assert_eq!(
            f.client().handle().disk_usage().await?.media_bytes,
            before.media_bytes
        );
        f.client().handle().remove_account(first.account()).await?;
        assert!(first.media().cached(source.clone()).await.is_err());
        assert_eq!(
            second.media().cached(source.clone()).await?.value.bytes,
            downloaded.value.bytes
        );
        f.client().handle().remove_account(second.account()).await?;
        assert_eq!(f.client().handle().disk_usage().await?.media_bytes, 0);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn lost_or_corrupt_index_requires_a_new_authenticated_fetch_before_content_can_be_read() -> Result {
    for corrupt in [false, true] {
        let mut f = Fixture::new(Config::default())?;
        let (account, source) = f.run(async {
            let (session, _) = accounts(&f).await?;
            let id = Uuid::from_u128(2002);
            let source = MediaSource::Attachment(id);
            f.server()
                .set_encoded_media(encoded(
                    format!("/attachments/{id}"),
                    BTreeSet::from([session.account().user().to_string()]),
                ))
                .await?;
            session.media().fetch(source.clone(), false).await?;
            Ok::<_, Box<dyn std::error::Error>>((session.account(), source))
        })??;
        f.close_client()?;
        let index = f.profile.path().join("media/index.sqlite");
        if corrupt {
            std::fs::write(&index, b"corrupt index")?;
        } else {
            std::fs::remove_file(&index)?;
        }
        f.reopen()?;
        f.run(async {
            let session = f.client().handle().session(account)?;
            fixture::online(&session).await?;
            assert!(matches!(
                session.media().cached(source.clone()).await,
                Err(MediaError::Unavailable)
            ));
            session.media().collect_orphans().await?;
            assert!(f.client().handle().disk_usage().await?.media_bytes > 0);
            assert_eq!(
                session
                    .media()
                    .fetch(source, false)
                    .await?
                    .value
                    .bytes
                    .len(),
                8193
            );
            Ok::<_, Box<dyn std::error::Error>>(())
        })??;
        f.finish()?;
    }
    Ok(())
}

#[test]
fn conditional_avatar_refresh_repairs_missing_content_and_denial_retires_the_reference() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let user = session.account().user();
        let avatar = MediaSource::UserAvatar {
            user,
            revision: "fixture-v1".into(),
        };
        let path = format!("/users/{user}/avatar");
        let mut entry = encoded(path.clone(), BTreeSet::new());
        entry.etag = Some("fixture-etag".into());
        f.server().set_encoded_media(entry).await?;
        session.media().fetch(avatar.clone(), false).await?;
        session.media().fetch(avatar.clone(), true).await?;
        let observations = f.server().take_media_observations().await;
        assert!(
            observations
                .iter()
                .all(|request| !request.cookie && !request.authorization)
        );
        assert!(observations.last().unwrap().conditional);
        let media = f.profile.path().join("media");
        for prefix in std::fs::read_dir(&media)? {
            let prefix = prefix?;
            if prefix.file_type()?.is_dir() {
                for file in std::fs::read_dir(prefix.path())? {
                    std::fs::write(file?.path(), b"damaged")?;
                }
            }
        }
        assert_eq!(
            session
                .media()
                .fetch(avatar.clone(), true)
                .await?
                .value
                .bytes
                .len(),
            8193
        );
        assert!(
            !f.server()
                .take_media_observations()
                .await
                .last()
                .unwrap()
                .conditional
        );
        let mut entry = encoded(path, BTreeSet::new());
        entry.status = 404;
        f.server().set_encoded_media(entry).await?;
        assert!(session.media().fetch(avatar.clone(), true).await.is_err());
        assert!(matches!(
            session.media().cached(avatar).await,
            Err(MediaError::Unavailable)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
