//! Writing this device's settings file (`rune::prefs`) without undoing
//! another part of the app: each part keeps its own handle from launch, and
//! a handle writes its whole copy back, so before a write it takes what is
//! on disk now.
//!
//! Owns only that reload; what each setting means is its owner's (`links`,
//! `layout_prefs`).

use rune::prefs::Prefs;

/// Makes `prefs` what its file holds now, before a write; a store with no
/// file stays as it is.
pub(crate) fn reload(prefs: &mut Prefs) {
    if let Some(path) = prefs.path().map(std::path::Path::to_path_buf) {
        *prefs = Prefs::at(path);
    }
}

#[cfg(test)]
mod tests {
    use rune::prefs::Prefs;

    #[test]
    fn a_write_after_a_reload_keeps_what_another_handle_saved_since() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("prefs.json");
        let mut links = Prefs::at(&path);
        let mut layout = Prefs::at(&path);
        super::reload(&mut links);
        links.set("trusted", &["example.org"]).unwrap();
        super::reload(&mut layout);
        layout.set("pane", &true).unwrap();
        let on_disk = Prefs::at(&path);
        assert_eq!(
            on_disk.get::<Vec<String>>("trusted"),
            Some(vec!["example.org".to_owned()])
        );
        assert_eq!(on_disk.get::<bool>("pane"), Some(true));
    }
}
