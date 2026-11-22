//! Public account media isolation, conditional recovery and provider credential separation over loopback.

#![cfg(feature = "test-support")]
mod fixture;
#[path = "media_cases/lifecycle.rs"]
mod lifecycle;
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
        .await?
        .id;
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

#[derive(Clone)]
struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn streaming_writer_export_verifies_digest_handles_304_and_writes_zero_on_tamper() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let user = session.account().user();
        let avatar = MediaSource::UserAvatar {
            user,
            revision: "stream-v1".into(),
        };
        let path = format!("/users/{user}/avatar");
        let payload: Vec<u8> = (0..130_000)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        f.server()
            .set_encoded_media(EncodedMedia {
                path: path.clone(),
                bytes: payload.clone(),
                users: BTreeSet::new(),
                etag: Some("etag-1".into()),
                status: 200,
                content_range: None,
            })
            .await?;

        // Miss -> download stream -> cache -> writer receives exact bytes.
        let miss_buf = SharedBuf(std::sync::Arc::default());
        let miss_meta = session
            .media()
            .fetch_to_writer(avatar.clone(), false, miss_buf.clone())
            .await?;
        assert_eq!(miss_meta.value.size_bytes, payload.len() as u64);
        assert_eq!(*miss_buf.0.lock().unwrap(), payload);

        // Refresh returning 304 writes cached bytes ONCE.
        let revalidated_buf = SharedBuf(std::sync::Arc::default());
        let revalidated_meta = session
            .media()
            .fetch_to_writer(avatar.clone(), true, revalidated_buf.clone())
            .await?;
        assert_eq!(revalidated_meta.value.size_bytes, payload.len() as u64);
        assert_eq!(*revalidated_buf.0.lock().unwrap(), payload);

        // Refresh returning updated payload must write ONLY updated bytes.
        let updated: Vec<u8> = vec![42; 70_000];
        f.server()
            .set_encoded_media(EncodedMedia {
                path: path.clone(),
                bytes: updated.clone(),
                users: BTreeSet::new(),
                etag: Some("etag-2".into()),
                status: 200,
                content_range: None,
            })
            .await?;
        let updated_buf = SharedBuf(std::sync::Arc::default());
        let updated_meta = session
            .media()
            .fetch_to_writer(avatar.clone(), true, updated_buf.clone())
            .await?;
        assert_eq!(updated_meta.value.size_bytes, updated.len() as u64);
        assert_eq!(*updated_buf.0.lock().unwrap(), updated);

        // Cache hit via cached_to_writer works offline / without network.
        let cached_buf = SharedBuf(std::sync::Arc::default());
        session
            .media()
            .cached_to_writer(avatar.clone(), cached_buf.clone())
            .await?;
        assert_eq!(*cached_buf.0.lock().unwrap(), updated);

        // Corrupt cached file on disk -> cached_to_writer fails and writes ZERO bytes.
        let media_dir = f.profile.path().join("media");
        for prefix in std::fs::read_dir(&media_dir)? {
            let prefix = prefix?;
            if prefix.file_type()?.is_dir() {
                for file in std::fs::read_dir(prefix.path())? {
                    std::fs::write(file?.path(), b"tampered-bytes")?;
                }
            }
        }
        let tamper_buf = SharedBuf(std::sync::Arc::default());
        let tamper_err = session
            .media()
            .cached_to_writer(avatar.clone(), tamper_buf.clone())
            .await;
        assert!(matches!(tamper_err, Err(MediaError::InvalidData)));
        assert!(tamper_buf.0.lock().unwrap().is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn streaming_writer_range_slice_exports_exact_subrange() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let user = session.account().user();
        // Range 200 slicing streams exact subrange without holding full body in writer.
        let range_src = MediaSource::Range {
            source: Box::new(MediaSource::Attachment(Uuid::from_u128(9911))),
            start_bytes: 100,
            end_bytes: 299,
        };
        let attach_path = format!("/attachments/{}", Uuid::from_u128(9911));
        f.server()
            .set_encoded_media(EncodedMedia {
                path: attach_path,
                bytes: (0..500_u16)
                    .map(|i| u8::try_from(i % 256).unwrap())
                    .collect(),
                users: BTreeSet::from([user.to_string()]),
                etag: None,
                status: 200,
                content_range: None,
            })
            .await?;
        let range_buf = SharedBuf(std::sync::Arc::default());
        let range_meta = session
            .media()
            .fetch_to_writer(range_src, false, range_buf.clone())
            .await?;
        assert_eq!(range_meta.value.size_bytes, 200);
        let written = range_buf.0.lock().unwrap();
        assert_eq!(written.len(), 200);
        assert_eq!(written[0], 100);
        assert_eq!(written[199], u8::try_from(299_u16 % 256).unwrap());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
