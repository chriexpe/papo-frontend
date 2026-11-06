//! Real-filesystem registry regressions using only isolated synthetic profiles.

use std::fs;

use super::{Change, PersistenceError, Profile};

#[test]
fn filesystem_errors_discard_raw_messages_and_paths() {
    let error = PersistenceError::from(std::io::Error::other("synthetic-private-input"));
    assert_eq!(error, PersistenceError::Io(std::io::ErrorKind::Other));
    assert!(!error.to_string().contains("synthetic-private-input"));
    assert!(!format!("{error:?}").contains("synthetic-private-input"));
}

#[test]
fn relative_root_is_rejected_without_a_working_directory_fallback() {
    assert_eq!(
        Profile::open("papo-desktop".into()).err(),
        Some(PersistenceError::InvalidPath)
    );
}

#[test]
fn fresh_profile_is_empty_without_creating_a_registry() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("papo-desktop");
    let profile = Profile::open(root.clone()).unwrap();
    assert!(profile.snapshot().servers().is_empty());
    assert_eq!(profile.snapshot().selected_server(), None);
    assert!(!root.join("servers.json").exists());
}

#[test]
fn restart_restores_order_and_selection_then_persists_removal() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("papo-desktop");
    let mut profile = Profile::open(root.clone()).unwrap();
    profile
        .apply(Change::Add {
            url: "https://first.example.invalid".into(),
        })
        .unwrap();
    profile
        .apply(Change::Add {
            url: "https://second.example.invalid".into(),
        })
        .unwrap();
    let first = profile.snapshot().servers()[0].key().to_owned();
    let second = profile.snapshot().servers()[1].key().to_owned();
    profile
        .apply(Change::Select {
            server: Some(first.clone()),
        })
        .unwrap();
    profile
        .apply(Change::Reorder {
            servers: vec![second.clone(), first.clone()],
        })
        .unwrap();
    drop(profile);

    let mut profile = Profile::open(root.clone()).unwrap();
    assert_eq!(profile.snapshot().servers()[0].key(), second);
    assert_eq!(profile.snapshot().selected_server(), Some(first.as_str()));
    profile.apply(Change::Remove { server: first }).unwrap();
    drop(profile);

    let profile = Profile::open(root).unwrap();
    assert_eq!(profile.snapshot().servers().len(), 1);
    assert_eq!(profile.snapshot().servers()[0].key(), second);
    assert_eq!(profile.snapshot().selected_server(), None);
}

#[test]
fn invalid_documents_remain_byte_for_byte_untouched() {
    for (bytes, error) in [
        (b"".as_slice(), PersistenceError::InvalidDocument),
        (b"{truncated".as_slice(), PersistenceError::InvalidDocument),
        (
            br#"{"version":42,"servers":[]}"#.as_slice(),
            PersistenceError::UnsupportedVersion,
        ),
        (
            br#"{"version":1,"servers":["file:///example"],"selected_server":null}"#.as_slice(),
            PersistenceError::InvalidDocument,
        ),
        (
            br#"{"version":1,"servers":[],"selected_server":"unknown"}"#.as_slice(),
            PersistenceError::InvalidDocument,
        ),
        (
            br#"{"version":1,"servers":[],"selected_server":null,"token":"synthetic"}"#.as_slice(),
            PersistenceError::InvalidDocument,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        fs::write(root.join("servers.json"), bytes).unwrap();
        assert_eq!(Profile::open(root.clone()).err(), Some(error));
        assert_eq!(fs::read(root.join("servers.json")).unwrap(), bytes);
        // An unsuccessful open must release its profile lock too.
        assert_eq!(Profile::open(root).err(), Some(error));
    }
}

#[test]
fn oversized_input_is_rejected_without_rewriting_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("servers.json");
    let bytes = vec![b' '; 1024 * 1024 + 1];
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        Profile::open(directory.path().into()).err(),
        Some(PersistenceError::TooLarge)
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn only_one_profile_owner_can_write_and_drop_releases_the_lock() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("papo-desktop");
    let first = Profile::open(root.clone()).unwrap();
    assert_eq!(
        Profile::open(root.clone()).err(),
        Some(PersistenceError::Locked)
    );
    drop(first);
    assert!(Profile::open(root).is_ok());
}

#[test]
fn invalid_root_or_registry_never_becomes_a_fresh_profile() {
    let directory = tempfile::tempdir().unwrap();
    let root_file = directory.path().join("not-a-directory");
    fs::write(&root_file, b"preserve").unwrap();
    assert_eq!(
        Profile::open(root_file.clone()).err(),
        Some(PersistenceError::InvalidPath)
    );
    assert_eq!(fs::read(root_file).unwrap(), b"preserve");
    fs::create_dir(directory.path().join("servers.json")).unwrap();
    assert_eq!(
        Profile::open(directory.path().into()).err(),
        Some(PersistenceError::InvalidPath)
    );
}

#[test]
fn replacement_failure_preserves_committed_snapshot_and_cleans_temp_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("papo-desktop");
    let mut profile = Profile::open(root.clone()).unwrap();
    profile
        .apply(Change::Add {
            url: "https://first.example.invalid".into(),
        })
        .unwrap();
    let before = fs::read(root.join("servers.json")).unwrap();
    // An opened directory refuses a file replacement on every supported OS.
    // Simulate an external obstruction without relaxing the profile's contract.
    fs::rename(root.join("servers.json"), root.join("preserved.json")).unwrap();
    fs::create_dir(root.join("servers.json")).unwrap();
    assert_eq!(
        profile
            .apply(Change::Add {
                url: "https://second.example.invalid".into()
            })
            .err(),
        Some(PersistenceError::InvalidPath)
    );
    assert_eq!(profile.snapshot().servers().len(), 1);
    assert_eq!(fs::read(root.join("preserved.json")).unwrap(), before);
    assert_eq!(fs::read_dir(root).unwrap().count(), 3);
}

#[cfg(unix)]
#[test]
fn unix_root_and_registry_are_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("papo-desktop");
    let mut profile = Profile::open(root.clone()).unwrap();
    profile
        .apply(Change::Add {
            url: "https://first.example.invalid".into(),
        })
        .unwrap();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for filename in [".profile.lock", "servers.json"] {
        assert_eq!(
            fs::metadata(root.join(filename))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_redirect_the_root_registry_or_lock_to_other_files() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let own = directory.path().join("papo-desktop");
    let other = directory.path().join("other-client");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("servers.json"), b"preserve").unwrap();
    symlink(&other, &own).unwrap();
    assert_eq!(
        Profile::open(own).err(),
        Some(PersistenceError::InvalidPath)
    );
    for name in ["servers.json", ".profile.lock"] {
        let root = directory.path().join(format!("profile-{name}"));
        fs::create_dir(&root).unwrap();
        symlink(other.join("servers.json"), root.join(name)).unwrap();
        assert_eq!(
            Profile::open(root).err(),
            Some(PersistenceError::InvalidPath)
        );
    }
    assert_eq!(fs::read(other.join("servers.json")).unwrap(), b"preserve");
}

#[test]
fn desktop_directory_is_not_the_legacy_application_directory() {
    let actual = Profile::data_directory().unwrap();
    let desktop = directories::ProjectDirs::from("", "", "papo-desktop").unwrap();
    let legacy = directories::ProjectDirs::from("", "", "papo").unwrap();
    assert_eq!(actual, desktop.data_dir());
    assert_ne!(actual, legacy.data_dir());
}
