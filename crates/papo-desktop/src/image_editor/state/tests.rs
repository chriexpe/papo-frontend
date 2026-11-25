//! The switches' rule, the name field and what Save keeps.
use super::*;

fn photo() -> Editor {
    let mut editor = Editor::attachment("IMG_2041.JPG", AttachmentEdit::default(), true);
    editor.inspected(
        Facts {
            width: 3000,
            height: 2000,
            format: "JPEG",
            size_bytes: 2_400_000,
            animated: false,
        },
        &AttachmentEdit::default(),
    );
    editor
}

#[test]
fn the_extension_is_kept_apart_and_forbidden_characters_go() {
    let mut editor = photo();
    assert_eq!(editor.name.text, "IMG_2041");
    assert_eq!(editor.extension, ".JPG");
    editor.name.replace_text("beach: day/1?");
    editor.name_edited();
    assert_eq!(editor.saved().name, "beach day1.JPG");
}

#[test]
fn anonymize_holds_random_name_on() {
    let mut editor = photo();
    editor.toggle_anonymize();
    assert!(editor.anonymized() && editor.random_name);
    editor.toggle_random_name();
    assert!(editor.random_name, "held on while anonymized");
    editor.toggle_anonymize();
    editor.toggle_random_name();
    assert!(!editor.random_name);
}

#[test]
fn anonymize_is_not_offered_where_the_sdk_cannot_clean() {
    let mut editor = Editor::attachment("notes.pdf", AttachmentEdit::default(), false);
    editor.toggle_anonymize();
    assert!(!editor.anonymized());
    assert_eq!(editor.saved().edit.anonymize, Anonymize::Off);
}

#[test]
fn an_untouched_picture_saves_no_crop_and_r_turns_it() {
    let mut editor = photo();
    assert_eq!(editor.saved().edit, AttachmentEdit::default());
    let r = KeyEvent {
        key: Key::Text("r".to_owned()),
        mods: rune::Mods::default(),
    };
    assert!(editor.stage_key(&r));
    assert_eq!(editor.saved().edit.quarter_turns, 1);
}

#[test]
fn a_device_default_on_a_document_neither_renames_nor_is_lost() {
    let default = AttachmentEdit {
        anonymize: Anonymize::WhereSupported,
        ..AttachmentEdit::default()
    };
    let editor = Editor::attachment("notes.pdf", default, false);
    assert!(!editor.random_name);
    assert_eq!(editor.saved().edit, default);
}

#[test]
fn an_untouched_or_fully_turned_picture_goes_byte_for_byte() {
    for (width, height) in [(3151, 2102), (2102, 3151), (1, 1), (4000, 3)] {
        let mut editor = Editor::attachment("p.jpg", AttachmentEdit::default(), true);
        editor.inspected(
            Facts {
                width,
                height,
                format: "JPEG",
                size_bytes: 1,
                animated: false,
            },
            &AttachmentEdit::default(),
        );
        assert_eq!(
            editor.saved().edit,
            AttachmentEdit::default(),
            "{width} × {height}"
        );
        if let Some(crop) = editor.crop.as_mut() {
            for _ in 0..4 {
                crop.rotate();
            }
        }
        assert_eq!(
            editor.saved().edit,
            AttachmentEdit::default(),
            "turned {width} × {height}"
        );
    }
}

#[test]
fn an_animation_has_no_stage() {
    let mut editor = Editor::attachment("party.gif", AttachmentEdit::default(), true);
    editor.inspected(
        Facts {
            width: 480,
            height: 360,
            format: "GIF",
            size_bytes: 1,
            animated: true,
        },
        &AttachmentEdit::default(),
    );
    assert!(editor.crop.is_none());
    assert!(editor.can_save());
}

#[test]
fn a_profile_banner_needs_its_picture_before_save() {
    let editor = Editor::profile(Use::Banner);
    assert!(!editor.can_save());
}

#[test]
fn the_random_name_shown_is_the_one_saved_and_off_brings_the_typed_one_back() {
    let mut editor = photo();
    editor.toggle_random_name();
    assert!(editor.needs_draw());
    editor.drew("k3f9q2mzx81bd0a.jpg");
    assert!(!editor.needs_draw());
    let saved = editor.saved();
    assert_eq!(saved.name, "k3f9q2mzx81bd0a.jpg");
    assert!(saved.edit.random_name && saved.edit.random_name_chosen);
    // Reroll: another draw replaces it, the typed name still kept.
    editor.drew("a0b1c2d3e4f5g6h.jpg");
    assert_eq!(editor.saved().name, "a0b1c2d3e4f5g6h.jpg");
    editor.toggle_random_name();
    let saved = editor.saved();
    assert_eq!(saved.name, "IMG_2041.JPG");
    assert!(!saved.edit.random_name && !saved.edit.random_name_chosen);
}

#[test]
fn anonymize_asks_for_a_random_name_to_show() {
    let mut editor = photo();
    editor.toggle_anonymize();
    assert!(editor.needs_draw());
    editor.drew("k3f9q2mzx81bd0a.jpg");
    assert!(editor.saved().edit.random_name_chosen);
}

#[test]
fn a_name_the_sdk_would_pick_at_send_is_drawn_when_the_editor_opens() {
    let default = AttachmentEdit {
        anonymize: Anonymize::WhereSupported,
        ..AttachmentEdit::default()
    };
    let editor = Editor::attachment("IMG_2041.JPG", default, true);
    assert!(editor.random_name && editor.needs_draw());
}

#[test]
fn a_reopened_random_name_still_brings_the_typed_one_back() {
    let edit = AttachmentEdit {
        random_name: true,
        random_name_chosen: true,
        ..AttachmentEdit::default()
    };
    let mut editor = Editor::attachment("k3f9q2mzx81bd0a.jpg", edit, true);
    editor.remember_typed(Some("IMG_2041.JPG"));
    assert_eq!(editor.saved().typed.as_deref(), Some("IMG_2041.JPG"));
    editor.toggle_random_name();
    assert_eq!(editor.saved().name, "IMG_2041.JPG");
}

#[test]
fn the_wheel_zooms_by_how_far_it_moved_and_not_sideways() {
    let mut editor = photo();
    editor.wheel(0.0, true);
    let zoom = |editor: &Editor| editor.crop.as_ref().map(|crop| crop.zoom);
    assert_eq!(zoom(&editor), Some(1.0), "a sideways scroll doesn't zoom");
    editor.wheel(4.0, true);
    let small = zoom(&editor).expect("a stage");
    editor.wheel(1.0, false);
    let notch = zoom(&editor).expect("a stage");
    assert!(small > 1.0 && small < 1.1 && notch > small * 1.09);
}

#[test]
fn shortcuts_on_the_stage_are_left_to_the_app() {
    let mut editor = photo();
    let ctrl_r = KeyEvent {
        key: Key::Text("r".to_owned()),
        mods: rune::Mods {
            ctrl: true,
            ..rune::Mods::default()
        },
    };
    assert!(!editor.stage_key(&ctrl_r));
}
