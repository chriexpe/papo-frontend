//! Embed pictures come only through the server's relay (`GET /embeds/{id}`), keyed by the
//! embed's revision, and save the same way; the addresses an embed carries are never contacted.

use super::*;
use base64::Engine;
use papo_test_server::MediaTestServer;

/// An embed answer as the backend sends it: every address in it points at `site`, and its
/// picture is `picture` in base64.
fn relayed(site: &str, picture: &[u8]) -> serde_json::Value {
    let page = format!("{site}/article");
    serde_json::json!({
        "id": "00000000-0000-4000-8000-000000002800",
        "source_type": "link",
        "fetch_method": "opengraph",
        "url": page,
        "title": "A title",
        "embed_url": format!("{site}/embed"),
        "author": {"name": "Someone", "url": format!("{site}/author"),
                   "media": {"url": format!("{site}/author.png")}},
        "thumbnail": {"url": format!("{site}/picture.webp"), "mime_type": "image/webp",
                      "width": 640, "height": 360},
        "image": {"url": format!("{site}/image.png")},
        "video": {"url": format!("{site}/video.mp4"), "mime_type": "video/mp4"},
        "footer": {"text": "Footer", "icon": {"url": format!("{site}/icon.png")}},
        "image_data": base64::engine::general_purpose::STANDARD.encode(picture),
        "author_image_data": base64::engine::general_purpose::STANDARD.encode(b"author"),
    })
}

fn answer(path: &str, users: BTreeSet<String>, body: &serde_json::Value) -> Result<EncodedMedia> {
    Ok(EncodedMedia {
        bytes: serde_json::to_vec(body)?,
        ..encoded(path.to_owned(), users)
    })
}

#[test]
fn an_embed_picture_comes_from_the_relay_and_never_from_the_site() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let site = MediaTestServer::start().await?;
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2800);
        let path = format!("/embeds/{id}");
        let users = BTreeSet::from([session.account().user().to_string()]);
        let body = relayed(&site.base_url(), b"relayed picture");
        f.server()
            .set_encoded_media(answer(&path, users, &body)?)
            .await?;
        let source = MediaSource::EmbedImage {
            embed: id,
            revision: "2026-11-25T10:00:00Z".into(),
        };
        let picture = session.media().fetch(source.clone(), false).await?.value;
        assert_eq!(&*picture.bytes, b"relayed picture");
        assert_eq!(picture.content_type.as_deref(), Some("image/webp"));
        let cached = session.media().cached(source).await?.value;
        assert_eq!(&*cached.bytes, b"relayed picture");
        let observations = f.server().take_media_observations().await;
        assert_eq!(observations.len(), 1, "one relay request, then the cache");
        assert!(site.finish().await.is_empty(), "the site saw no request");
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_new_revision_fetches_again_and_an_embed_without_a_picture_is_unavailable() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2801);
        let path = format!("/embeds/{id}");
        let users = BTreeSet::from([session.account().user().to_string()]);
        let site = "https://site.invalid";
        f.server()
            .set_encoded_media(answer(&path, users.clone(), &relayed(site, b"first"))?)
            .await?;
        let first = MediaSource::EmbedImage {
            embed: id,
            revision: "first".into(),
        };
        session.media().fetch(first.clone(), false).await?;
        f.server()
            .set_encoded_media(answer(&path, users.clone(), &relayed(site, b"second"))?)
            .await?;
        let second = MediaSource::EmbedImage {
            embed: id,
            revision: "second".into(),
        };
        let refetched = session.media().fetch(second, false).await?.value;
        assert_eq!(&*refetched.bytes, b"second");
        assert_eq!(
            &*session.media().fetch(first, false).await?.value.bytes,
            b"first"
        );
        let mut bare = relayed(site, b"");
        bare.as_object_mut().map(|embed| embed.remove("image_data"));
        f.server()
            .set_encoded_media(answer(&path, users, &bare)?)
            .await?;
        let third = MediaSource::EmbedImage {
            embed: id,
            revision: "third".into(),
        };
        assert!(matches!(
            session.media().fetch(third, false).await,
            Err(MediaError::Unavailable)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_denied_embed_retires_its_cached_picture() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        let (session, other) = accounts(&f).await?;
        let id = Uuid::from_u128(2802);
        let path = format!("/embeds/{id}");
        let body = relayed("https://site.invalid", b"picture");
        let mine = BTreeSet::from([session.account().user().to_string()]);
        f.server()
            .set_encoded_media(answer(&path, mine, &body)?)
            .await?;
        let source = MediaSource::EmbedImage {
            embed: id,
            revision: "only".into(),
        };
        session.media().fetch(source.clone(), false).await?;
        let theirs = BTreeSet::from([other.account().user().to_string()]);
        f.server()
            .set_encoded_media(answer(&path, theirs, &body)?)
            .await?;
        assert!(session.media().fetch(source.clone(), true).await.is_err());
        assert!(matches!(
            session.media().cached(source).await,
            Err(MediaError::Unavailable)
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn an_embed_picture_saves_through_the_relay_then_from_the_cache() -> Result {
    let f = Fixture::new(Config::default())?;
    let folder = tempfile::tempdir()?;
    f.run(async {
        let site = MediaTestServer::start().await?;
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2803);
        let path = format!("/embeds/{id}");
        let users = BTreeSet::from([session.account().user().to_string()]);
        let body = relayed(&site.base_url(), b"relayed picture");
        f.server()
            .set_encoded_media(answer(&path, users, &body)?)
            .await?;
        let target = papo_sdk::SaveTarget::Folder {
            folder: folder.path().to_path_buf(),
            name: "picture.webp".into(),
        };
        let revision = "2026-11-25T10:00:00Z".to_owned();
        let saved = session
            .media()
            .save_embed_image(id, revision.clone(), target.clone())
            .finished()
            .await;
        assert_eq!(saved, Ok(folder.path().join("picture.webp")));
        assert_eq!(
            std::fs::read(folder.path().join("picture.webp"))?,
            b"relayed picture"
        );
        assert_eq!(f.server().take_media_observations().await.len(), 1);
        session
            .media()
            .fetch(
                MediaSource::EmbedImage {
                    embed: id,
                    revision: revision.clone(),
                },
                false,
            )
            .await?;
        assert_eq!(
            f.server().take_media_observations().await.len(),
            1,
            "not cached by a save"
        );
        let again = session
            .media()
            .save_embed_image(id, revision, target)
            .finished()
            .await;
        assert_eq!(again, Ok(folder.path().join("picture (1).webp")));
        assert!(
            f.server().take_media_observations().await.is_empty(),
            "the second save copies the cached picture"
        );
        assert!(site.finish().await.is_empty(), "the site saw no request");
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_denied_embed_picture_saves_nothing() -> Result {
    let f = Fixture::new(Config::default())?;
    let folder = tempfile::tempdir()?;
    f.run(async {
        let (session, other) = accounts(&f).await?;
        let id = Uuid::from_u128(2804);
        let theirs = BTreeSet::from([other.account().user().to_string()]);
        let body = relayed("https://site.invalid", b"picture");
        f.server()
            .set_encoded_media(answer(&format!("/embeds/{id}"), theirs, &body)?)
            .await?;
        let target = papo_sdk::SaveTarget::Folder {
            folder: folder.path().to_path_buf(),
            name: "picture.webp".into(),
        };
        let saved = session
            .media()
            .save_embed_image(id, "only".into(), target)
            .finished()
            .await;
        assert_eq!(saved, Err(papo_sdk::SaveFailure::Gone));
        assert_eq!(
            std::fs::read_dir(folder.path())?.count(),
            0,
            "nothing left behind"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
