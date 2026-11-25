//! Saving an attachment's original to a folder or a chosen file over loopback: the exact bytes,
//! never over an existing file, from the cache without a request, and nothing left behind when
//! a save is denied or cancelled.

use super::*;
use papo_sdk::{SaveFailure, SaveState, SaveTarget};
use std::path::Path;

/// Bytes that differ along their length, so a shifted or partial copy shows.
fn original() -> Vec<u8> {
    (0..20_000_u32)
        .map(|n| u8::try_from(n % 251).unwrap_or_default())
        .collect()
}

async fn serve(f: &Fixture, session: &Session, id: Uuid, status: u16) -> Result {
    let mut entry = encoded(
        format!("/attachments/{id}"),
        BTreeSet::from([session.account().user().to_string()]),
    );
    entry.bytes = original();
    entry.status = status;
    f.server().set_encoded_media(entry).await?;
    Ok(())
}

fn folder_names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(std::result::Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn into(folder: &Path, name: &str) -> SaveTarget {
    SaveTarget::Folder {
        folder: folder.to_path_buf(),
        name: name.to_owned(),
    }
}

#[test]
fn a_save_writes_the_original_and_never_replaces_a_file() -> Result {
    let f = Fixture::new(Config::default())?;
    let folder = tempfile::tempdir()?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2800);
        serve(&f, &session, id, 200).await?;
        let first = session
            .media()
            .save_attachment(id, into(folder.path(), "praia.jpg"))
            .finished()
            .await;
        assert_eq!(first, Ok(folder.path().join("praia.jpg")));
        assert_eq!(std::fs::read(folder.path().join("praia.jpg"))?, original());
        let save = session
            .media()
            .save_attachment(id, into(folder.path(), "praia.jpg"));
        let second = save.finished().await;
        assert_eq!(second, Ok(folder.path().join("praia (1).jpg")));
        assert_eq!(
            save.state(),
            SaveState::Saved(folder.path().join("praia (1).jpg"))
        );
        // A name from the server cannot leave the folder.
        let escaped = session
            .media()
            .save_attachment(id, into(folder.path(), "../../escape.bin"))
            .finished()
            .await;
        assert_eq!(escaped, Ok(folder.path().join("escape.bin")));
        assert_eq!(
            folder_names(folder.path()),
            ["escape.bin", "praia (1).jpg", "praia.jpg"]
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_held_original_is_copied_from_the_cache_without_a_request() -> Result {
    let f = Fixture::new(Config::default())?;
    let folder = tempfile::tempdir()?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2801);
        serve(&f, &session, id, 200).await?;
        session
            .media()
            .fetch(MediaSource::Attachment(id), false)
            .await?;
        f.server().take_media_observations().await;
        let saved = session
            .media()
            .save_attachment(id, into(folder.path(), "held.png"))
            .finished()
            .await;
        assert_eq!(saved, Ok(folder.path().join("held.png")));
        assert_eq!(std::fs::read(folder.path().join("held.png"))?, original());
        assert!(f.server().take_media_observations().await.is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_denied_original_is_gone_and_leaves_nothing() -> Result {
    let f = Fixture::new(Config::default())?;
    let folder = tempfile::tempdir()?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2802);
        serve(&f, &session, id, 403).await?;
        let saved = session
            .media()
            .save_attachment(id, into(folder.path(), "secret.pdf"))
            .finished()
            .await;
        assert_eq!(saved, Err(SaveFailure::Gone));
        assert_eq!(folder_names(folder.path()), Vec::<String>::new());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_cancelled_save_leaves_nothing() -> Result {
    let f = Fixture::new(Config::default())?;
    let folder = tempfile::tempdir()?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2803);
        serve(&f, &session, id, 200).await?;
        let mut held = f
            .server()
            .hold_next_rest("GET", &format!("/attachments/{id}"))
            .await?;
        let save = session
            .media()
            .save_attachment(id, into(folder.path(), "big.zip"));
        held.wait_started().await?;
        save.cancel();
        assert_eq!(save.finished().await, Err(SaveFailure::Cancelled));
        held.release();
        assert_eq!(folder_names(folder.path()), Vec::<String>::new());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_chosen_file_is_written_in_place() -> Result {
    let f = Fixture::new(Config::default())?;
    let folder = tempfile::tempdir()?;
    let chosen = folder.path().join("chosen.bin");
    std::fs::write(&chosen, b"what was there")?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2804);
        serve(&f, &session, id, 200).await?;
        let saved = session
            .media()
            .save_attachment(id, SaveTarget::File(chosen.clone()))
            .finished()
            .await;
        assert_eq!(saved, Ok(chosen.clone()));
        assert_eq!(std::fs::read(&chosen)?, original());
        assert_eq!(folder_names(folder.path()), ["chosen.bin"]);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_refused_save_keeps_the_file_it_would_have_replaced() -> Result {
    let f = Fixture::new(Config::default())?;
    let folder = tempfile::tempdir()?;
    let chosen = folder.path().join("keep.bin");
    std::fs::write(&chosen, b"what was there")?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2805);
        serve(&f, &session, id, 403).await?;
        let saved = session
            .media()
            .save_attachment(id, SaveTarget::File(chosen.clone()))
            .finished()
            .await;
        assert_eq!(saved, Err(SaveFailure::Gone));
        assert_eq!(std::fs::read(&chosen)?, b"what was there");
        assert_eq!(folder_names(folder.path()), ["keep.bin"]);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_save_that_ended_first_still_reports_its_end_once() -> Result {
    let f = Fixture::new(Config::default())?;
    let folder = tempfile::tempdir()?;
    f.run(async {
        let (session, _) = accounts(&f).await?;
        let id = Uuid::from_u128(2806);
        serve(&f, &session, id, 200).await?;
        let mut save = session
            .media()
            .save_attachment(id, into(folder.path(), "quick.txt"));
        let path = save.finished().await;
        assert_eq!(path, Ok(folder.path().join("quick.txt")));
        assert_eq!(
            save.changed().await,
            Some(SaveState::Saved(folder.path().join("quick.txt")))
        );
        assert_eq!(save.changed().await, None);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
