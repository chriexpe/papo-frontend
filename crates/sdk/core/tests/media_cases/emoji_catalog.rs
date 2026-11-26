//! The custom emoji catalog over loopback: every page listed, kept across a relaunch, images in
//! the media store, a missing image listed again, a failed listing keeping what it had.

use super::{accounts, encoded};
use crate::fixture::{self, Fixture, Result};
use papo_sdk::{EmojiCatalog, EmojiCatalogUpdate, MediaSource, Session, Uuid};
use papo_test_server::{Config, ServerEmoji};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::watch;

fn roster(count: u128) -> Vec<ServerEmoji> {
    (0..count)
        .map(|index| ServerEmoji {
            id: Uuid::from_u128(0xE000 + index).to_string(),
            name: format!("emoji_{index}"),
            format: "png".into(),
            created_at: format!("2026-01-01T00:00:{:02}Z", index % 60),
            image: image(index),
        })
        .collect()
}

fn image(index: u128) -> Vec<u8> {
    format!("image {index}").into_bytes()
}

/// The next catalog the watch publishes that `ready` accepts.
async fn catalog_where(
    updates: &mut watch::Receiver<EmojiCatalogUpdate>,
    ready: impl Fn(&EmojiCatalog) -> bool,
) -> Result<Arc<EmojiCatalog>> {
    let wait = async {
        loop {
            if let EmojiCatalogUpdate::Ready(catalog) = &*updates.borrow_and_update()
                && ready(catalog)
            {
                return Ok(Arc::clone(catalog));
            }
            updates.changed().await?;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), wait).await?
}

async fn signed_in(f: &Fixture) -> Result<Session> {
    Ok(accounts(f).await?.0)
}

#[test]
fn every_page_is_listed_on_connect_with_its_images_in_the_store() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        f.server().set_emojis(roster(30)).await;
        let session = signed_in(&f).await?;
        let mut updates = session.emoji_catalog().subscribe().await?;
        let catalog = catalog_where(&mut updates, |catalog| catalog.emojis.len() == 30).await?;
        let names: Vec<_> = catalog
            .emojis
            .iter()
            .map(|emoji| emoji.name.as_str())
            .collect();
        assert_eq!(names.first(), Some(&"emoji_0"));
        assert_eq!(
            names.last(),
            Some(&"emoji_29"),
            "the second page follows the first"
        );
        let last = &catalog.emojis[29];
        let cached = session.media().cached(last.source()).await?;
        assert_eq!(&*cached.value.bytes, image(29).as_slice());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_relaunch_reads_the_kept_catalog_before_listing_again() -> Result {
    let mut f = Fixture::new(Config::default())?;
    let account = f.run(async {
        f.server().set_emojis(roster(3)).await;
        let session = signed_in(&f).await?;
        let mut updates = session.emoji_catalog().subscribe().await?;
        catalog_where(&mut updates, |catalog| catalog.emojis.len() == 3).await?;
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    f.reopen()?;
    f.run(async {
        // The server's emoji are gone; the kept catalog still answers first.
        f.server().set_emojis(Vec::new()).await;
        let session = f.client().handle().session(account)?;
        let mut updates = session.emoji_catalog().subscribe().await?;
        let kept = catalog_where(&mut updates, |_| true).await?;
        assert_eq!(kept.emojis.len(), 3);
        fixture::online(&session).await?;
        let listed = catalog_where(&mut updates, |catalog| catalog.emojis.is_empty()).await?;
        assert_ne!(
            listed.revision, kept.revision,
            "a change has a new revision"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_missing_image_lists_again_and_then_draws() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        f.server().set_emojis(roster(2)).await;
        let session = signed_in(&f).await?;
        let mut updates = session.emoji_catalog().subscribe().await?;
        let catalog = catalog_where(&mut updates, |catalog| catalog.emojis.len() == 2).await?;
        session.media().clear_cache().await?;
        let source = catalog.emojis[1].source();
        assert!(session.media().cached(source.clone()).await.is_err());
        let fetched = session.media().fetch(source, false).await?;
        assert_eq!(&*fetched.value.bytes, image(1).as_slice());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_missing_image_lists_at_most_once_in_the_interval_and_an_unknown_one_never() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        f.server().set_emojis(roster(1)).await;
        let session = signed_in(&f).await?;
        let mut updates = session.emoji_catalog().subscribe().await?;
        let catalog = catalog_where(&mut updates, |catalog| catalog.emojis.len() == 1).await?;
        let known = catalog.emojis[0].source();
        let pages = f.server().emoji_pages_served().await;
        // The catalog doesn't name it: nothing to list.
        let gone = MediaSource::Emoji {
            id: Uuid::from_u128(0xDEAD),
            revision: "2026-01-01T00:00:00Z".into(),
        };
        assert!(session.media().fetch(gone, false).await.is_err());
        assert_eq!(f.server().emoji_pages_served().await, pages);
        session.media().clear_cache().await?;
        assert!(session.media().fetch(known.clone(), false).await.is_ok());
        assert_eq!(
            f.server().emoji_pages_served().await,
            pages + 1,
            "one listing"
        );
        // Within the interval a second miss doesn't list again.
        session.media().clear_cache().await?;
        assert!(session.media().fetch(known, false).await.is_err());
        assert_eq!(f.server().emoji_pages_served().await, pages + 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_failed_listing_keeps_the_catalog_it_had() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        f.server().set_emojis(roster(2)).await;
        let session = signed_in(&f).await?;
        let mut updates = session.emoji_catalog().subscribe().await?;
        let before = catalog_where(&mut updates, |catalog| catalog.emojis.len() == 2).await?;
        let mut failing = encoded("/emojis".into(), BTreeSet::new());
        failing.status = 500;
        f.server().set_encoded_media(failing).await?;
        assert!(session.emoji_catalog().refresh().await.is_err());
        let after = catalog_where(&mut updates, |_| true).await?;
        assert_eq!(after, before);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_new_connection_lists_again_and_removing_the_account_closes_the_catalog() -> Result {
    let f = Fixture::new(Config::default())?;
    f.run(async {
        f.server().set_emojis(roster(1)).await;
        let session = signed_in(&f).await?;
        let mut updates = session.emoji_catalog().subscribe().await?;
        catalog_where(&mut updates, |catalog| catalog.emojis.len() == 1).await?;
        // Added elsewhere: no event says so, the next connection lists it.
        f.server().set_emojis(roster(2)).await;
        let before = fixture::online(&session).await?;
        session.reconnect()?;
        let mut status = session.subscribe_connection();
        fixture::wait_for(&mut status, |status| {
            status.state == papo_sdk::ConnectionState::Online && status.generation > before
        })
        .await?;
        catalog_where(&mut updates, |catalog| catalog.emojis.len() == 2).await?;
        f.client()
            .handle()
            .remove_account(session.account())
            .await?;
        let closed = async {
            loop {
                if matches!(*updates.borrow_and_update(), EmojiCatalogUpdate::Closed) {
                    return Ok::<_, Box<dyn std::error::Error>>(());
                }
                updates.changed().await?;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), closed).await??;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
