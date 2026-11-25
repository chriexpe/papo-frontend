//! The image editor on a tray tile, headless (`docs/design.md` §14.10):
//! Enter on a tile opens it, the keyboard alone renames and saves
//! (Ctrl+Enter) or cancels (Escape); a tile shows and says the marks of
//! what it goes as; an image the SDK can't clean is refused while
//! Anonymize is on; and "Anonymize new attachments" sets what a new file
//! starts with.

use papo_sdk::{Anonymize, AttachmentEdit};
use rune::{Key, Mods, Runtime};

use super::composer::shown;
use super::tray::{key_with, names, shift_tab, tile, with_three};
use super::{FRAME_S, Shell, tree};
use crate::composer::tray::{Inspection, TileKind};
use crate::image_editor::view::EDITOR_KEY;

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

/// #general's tray with the keyboard on its last tile (the document),
/// the editor open on it.
fn editing_last() -> Runtime<Shell> {
    let mut rt = with_three();
    shift_tab(&mut rt);
    key_with(&mut rt, Key::Enter, Mods::default());
    rt
}

#[test]
fn enter_on_a_tile_opens_the_editor_and_ctrl_enter_saves_the_name() {
    let mut rt = editing_last();
    assert!(rt.state.editor.is_some(), "{}", tree(&rt));
    assert!(tree(&rt).contains(EDITOR_KEY));
    rt.update(|shell, _tx| {
        if let Some(open) = shell.editor.as_mut() {
            open.editor.name.replace_text("beach: day");
        }
        shell.editor_name_changed();
    });
    rt.frame(FRAME_S);
    key_with(&mut rt, Key::Enter, ctrl());
    assert!(rt.state.editor.is_none());
    assert_eq!(names(&rt)[2], "beach day.pdf");
}

#[test]
fn escape_closes_the_editor_and_changes_nothing() {
    let mut rt = editing_last();
    rt.update(|shell, _tx| {
        if let Some(open) = shell.editor.as_mut() {
            open.editor.name.replace_text("other");
        }
    });
    key_with(&mut rt, Key::Escape, Mods::default());
    assert!(rt.state.editor.is_none());
    assert_eq!(names(&rt)[2], "relatório.pdf");
}

#[test]
fn a_tile_going_anonymized_shows_and_says_both_marks() {
    let mut rt = with_three();
    rt.update(|shell, _tx| {
        let key = shown(shell);
        let id = shell.composer.shown().map(|(_, draft)| draft.tray.ids()[0]);
        let Some(id) = id else {
            return;
        };
        let inspection = Inspection::Cleanable {
            width: 3000,
            height: 2000,
            animated: false,
        };
        shell.composer.tile_inspected(key, id, inspection);
        let edit = AttachmentEdit {
            anonymize: Anonymize::Required,
            random_name: true,
            ..AttachmentEdit::default()
        };
        let _ = shell
            .composer
            .tile_edited(key, id, "praia.jpg".to_owned(), edit);
    });
    rt.frame(FRAME_S);
    let tree = tree(&rt);
    assert!(tree.contains("anonymized, random name"), "{tree}");
}

#[test]
fn a_device_default_marks_only_what_the_sdk_cleans() {
    let mut rt = with_three();
    rt.update(|shell, _tx| {
        let key = shown(shell);
        let ids = shell
            .composer
            .shown()
            .map(|(_, draft)| draft.tray.ids())
            .unwrap_or_default();
        let default = AttachmentEdit {
            anonymize: Anonymize::WhereSupported,
            ..AttachmentEdit::default()
        };
        for id in &ids {
            let _ = shell.composer.tile_edited(key, *id, String::new(), default);
        }
        let _ = shell
            .composer
            .tile_inspected(key, ids[2], Inspection::NotAnImage);
    });
    rt.frame(FRAME_S);
    let tile = rt
        .state
        .composer
        .shown()
        .and_then(|(_, draft)| draft.tray.tiles().get(2).cloned())
        .expect("the document");
    assert!(!tile.anonymized() && !tile.renamed());
}

#[test]
fn an_image_the_sdk_cannot_clean_is_refused_while_anonymize_is_on() {
    let mut rt = super::composer::writing("");
    rt.update(|shell, tx| {
        let key = shown(shell);
        let mut heic = tile(shell, "IMG_2041.HEIC", TileKind::File);
        heic.edit.anonymize = Anonymize::WhereSupported;
        let id = heic.id;
        let _ = shell.composer.attach(key, vec![heic]);
        shell.tile_read(key, id, Inspection::Uncleanable, None, tx);
    });
    rt.frame(FRAME_S);
    assert_eq!(names(&rt), Vec::<String>::new());
    let tree = tree(&rt);
    assert!(
        tree.contains("IMG_2041.HEIC can't be anonymized."),
        "{tree}"
    );
}

#[test]
fn an_image_the_sdk_cannot_clean_stays_while_anonymize_is_off() {
    let mut rt = super::composer::writing("");
    rt.update(|shell, tx| {
        let key = shown(shell);
        let heic = tile(shell, "IMG_2041.HEIC", TileKind::File);
        let id = heic.id;
        let _ = shell.composer.attach(key, vec![heic]);
        shell.tile_read(key, id, Inspection::Uncleanable, None, tx);
    });
    rt.frame(FRAME_S);
    assert_eq!(names(&rt), ["IMG_2041.HEIC"]);
}

#[test]
fn the_device_setting_sets_what_a_new_file_starts_with() {
    let mut rt = super::composer::writing("");
    rt.update(|shell, tx| {
        shell.toggle_settings(tx);
        shell.settings_page(crate::settings::PageId::Privacy, tx);
    });
    rt.frame(FRAME_S);
    let tree = tree(&rt);
    assert!(tree.contains("Anonymize new attachments"), "{tree}");
    let mut starts = Vec::new();
    rt.update(|shell, tx| {
        starts.push(shell.new_attachment_edit().anonymize);
        shell.settings_change(
            crate::settings::Change::Switch(crate::settings::RowId::AnonymizeNew, true),
            tx,
        );
        starts.push(shell.new_attachment_edit().anonymize);
    });
    assert_eq!(starts, [Anonymize::Off, Anonymize::WhereSupported]);
}
