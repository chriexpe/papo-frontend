//! The attachment tray in the shell, headless (`docs/design.md` §14.2): it
//! shows on the text pill, is one tab stop reached by ⇧Tab from the field,
//! moves between tiles with ←/→, moves a tile with Alt+←/→, removes with
//! Delete and gives the field back on Escape; Send takes attachments only;
//! the drop card names the conversation; the window rests with tiles in it.

use rune::{Key, KeyEvent, Mods, Runtime};

use super::composer::{shown, writing};
use super::navigation::focused;
use super::{FRAME_S, Shell, press, settles, tree};
use crate::composer::COMPOSER_KEY;
use crate::composer::tray::{Tile, TileKind, TileSource};
use crate::composer::tray_view::TRAY_KEY;

/// A tile of `kind` named `name`, as a headless test makes it.
pub(super) fn tile(shell: &mut Shell, name: &str, kind: TileKind) -> Tile {
    Tile {
        id: shell.composer.next_tile_id(),
        name: name.to_owned(),
        size_bytes: Some(2_516_582),
        kind,
        source: TileSource::Fixture,
        thumbnail: None,
        video: None,
        inspection: crate::composer::tray::Inspection::Pending,
        edit: papo_sdk::AttachmentEdit::default(),
        typed_name: None,
    }
}

/// #general with an empty field and three files in its tray.
pub(super) fn with_three() -> Runtime<Shell> {
    let mut rt = writing("");
    rt.update(|shell, _tx| {
        let key = shown(shell);
        let tiles = vec![
            tile(shell, "praia.jpg", TileKind::Image),
            tile(shell, "party.gif", TileKind::Gif),
            tile(shell, "relatório.pdf", TileKind::File),
        ];
        // Not saved: the tests have no SDK.
        let _ = shell.composer.attach(key, tiles);
    });
    rt.frame(FRAME_S);
    rt
}

pub(super) fn names(rt: &Runtime<Shell>) -> Vec<String> {
    rt.state
        .composer
        .shown()
        .map(|(_, draft)| {
            draft
                .tray
                .tiles()
                .iter()
                .map(|tile| tile.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn current(rt: &Runtime<Shell>) -> Option<String> {
    let (_, draft) = rt.state.composer.shown()?;
    let id = rt.state.tray_ui.current_in(&draft.tray.ids())?;
    draft.tray.get(id).map(|tile| tile.name.clone())
}

pub(super) fn key_with(rt: &mut Runtime<Shell>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(FRAME_S);
}

pub(super) fn shift_tab(rt: &mut Runtime<Shell>) {
    let mods = Mods {
        shift: true,
        ..Mods::default()
    };
    key_with(rt, Key::Tab, mods);
}

fn into_tray() -> Runtime<Shell> {
    let mut rt = with_three();
    focus_field(&mut rt);
    shift_tab(&mut rt);
    rt
}

#[test]
fn the_tray_says_how_many_files_and_names_each() {
    let rt = with_three();
    let tree = tree(&rt);
    assert!(tree.contains("Attachments, 3"), "{tree}");
    assert!(tree.contains("Image, praia.jpg, 2.4 MB"), "{tree}");
    assert!(tree.contains("GIF, party.gif, 2.4 MB"), "{tree}");
    assert!(tree.contains("File, relatório.pdf, 2.4 MB"), "{tree}");
    assert!(tree.contains("Remove praia.jpg"), "{tree}");
}

#[test]
fn shift_tab_from_the_field_lands_on_the_last_tile() {
    let rt = into_tray();
    assert_eq!(focused(&rt).as_deref(), Some(TRAY_KEY));
    assert_eq!(current(&rt).as_deref(), Some("relatório.pdf"));
}

#[test]
fn arrows_move_between_tiles_and_alt_arrows_move_the_tile() {
    let mut rt = into_tray();
    press(&mut rt, Key::Left);
    press(&mut rt, Key::Left);
    assert_eq!(current(&rt).as_deref(), Some("praia.jpg"));
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    key_with(&mut rt, Key::Right, alt);
    assert_eq!(names(&rt), ["party.gif", "praia.jpg", "relatório.pdf"]);
    assert_eq!(
        current(&rt).as_deref(),
        Some("praia.jpg"),
        "the focus follows the tile"
    );
    assert_eq!(
        focused(&rt).as_deref(),
        Some(TRAY_KEY),
        "Alt+→ is not back/forward here"
    );
}

#[test]
fn delete_removes_the_tile_and_the_focus_goes_to_the_next() {
    let mut rt = into_tray();
    press(&mut rt, Key::Left);
    press(&mut rt, Key::Delete);
    assert_eq!(names(&rt), ["praia.jpg", "relatório.pdf"]);
    assert_eq!(current(&rt).as_deref(), Some("relatório.pdf"));
    press(&mut rt, Key::Backspace);
    press(&mut rt, Key::Backspace);
    assert_eq!(names(&rt), Vec::<String>::new());
    assert_eq!(
        focused(&rt).as_deref(),
        Some(COMPOSER_KEY),
        "the field takes the keyboard back"
    );
}

#[test]
fn escape_gives_the_field_back() {
    let mut rt = into_tray();
    press(&mut rt, Key::Escape);
    assert_eq!(focused(&rt).as_deref(), Some(COMPOSER_KEY));
    assert_eq!(names(&rt).len(), 3, "nothing is removed");
}

#[test]
fn the_menu_key_opens_remove_beside_the_tile() {
    let mut rt = into_tray();
    press(&mut rt, Key::ContextMenu);
    let tree = tree(&rt);
    assert!(tree.contains("menu-tray-remove"), "{tree}");
}

#[test]
fn attachments_alone_can_be_sent() {
    let rt = with_three();
    assert!(rt.state.composer.can_send());
}

#[test]
fn up_in_an_empty_field_does_not_edit_while_files_wait() {
    let mut rt = with_three();
    focus_field(&mut rt);
    press(&mut rt, Key::Up);
    let (_, draft) = rt.state.composer.shown().unwrap();
    assert!(draft.mode.editing().is_none());
}

#[test]
fn files_dragged_over_the_window_show_the_card_for_the_conversation() {
    let mut rt = with_three();
    rt.update(|shell, _tx| shell.drop_hovered(true));
    rt.frame(FRAME_S);
    let tree = tree(&rt);
    assert!(tree.contains("Drop files to attach"), "{tree}");
    rt.update(|shell, _tx| shell.drop_hovered(false));
    rt.frame(FRAME_S);
    assert!(!tree_has(&rt, "Drop files to attach"));
}

#[test]
fn the_window_rests_with_files_in_the_tray() {
    let mut rt = with_three();
    assert!(settles(&mut rt));
}

fn tree_has(rt: &Runtime<Shell>, words: &str) -> bool {
    tree(rt).contains(words)
}

/// The field takes the keyboard, as the app starts with it.
fn focus_field(rt: &mut Runtime<Shell>) {
    rt.update(|_shell, tx| tx.focus(COMPOSER_KEY, true));
    rt.frame(FRAME_S);
}

/// #general with ten file tiles, wider together than the window.
fn with_ten_files() -> Runtime<Shell> {
    let mut rt = writing("");
    rt.update(|shell, _tx| {
        let key = shown(shell);
        let tiles = (0..10)
            .map(|n| tile(shell, &format!("report-{n}.pdf"), TileKind::File))
            .collect();
        let _ = shell.composer.attach(key, tiles);
    });
    rt.frame(FRAME_S);
    rt
}

fn left_of(rt: &Runtime<Shell>, key: &str) -> f32 {
    rt.presented(key).map_or(f32::NAN, |(rect, _)| rect[0])
}

#[test]
fn the_mouse_wheel_scrolls_a_full_tray_sideways() {
    let mut rt = with_ten_files();
    let first = "tray-tile-1";
    let before = left_of(&rt, first);
    let (tray, _) = rt.presented(TRAY_KEY).unwrap();
    rt.wheel(tray[0] + tray[2] / 2.0, tray[1] + tray[3] / 2.0, 120.0);
    for _ in 0..30 {
        rt.frame(FRAME_S);
    }
    assert!(
        left_of(&rt, first) < before,
        "{before} → {}",
        left_of(&rt, first)
    );
}

#[test]
fn a_precise_vertical_wheel_scrolls_a_full_tray_sideways_too() {
    let mut rt = with_ten_files();
    let first = "tray-tile-1";
    let before = left_of(&rt, first);
    let (tray, _) = rt.presented(TRAY_KEY).unwrap();
    // A smooth wheel or a trackpad: point deltas, moving the content up.
    let mut wheel = rune::WheelEvent::default();
    wheel.dy = -120.0;
    wheel.x = tray[0] + tray[2] / 2.0;
    wheel.y = tray[1] + tray[3] / 2.0;
    wheel.precise = true;
    rt.wheel_event(wheel);
    for _ in 0..30 {
        rt.frame(FRAME_S);
    }
    assert!(
        left_of(&rt, first) < before,
        "{before} → {}",
        left_of(&rt, first)
    );
}

#[test]
fn the_tile_the_keyboard_lands_on_scrolls_into_view() {
    let mut rt = with_ten_files();
    focus_field(&mut rt);
    shift_tab(&mut rt);
    for _ in 0..30 {
        rt.frame(FRAME_S);
    }
    let (tray, _) = rt.presented(TRAY_KEY).unwrap();
    let last = left_of(&rt, "tray-tile-10");
    assert!(
        last < tray[0] + tray[2],
        "the last tile shows: {last} in {tray:?}"
    );
}

#[test]
fn a_file_still_copying_counts_toward_the_message_total() {
    let mut rt = writing("");
    rt.update(|shell, _tx| {
        let key = shown(shell);
        let mut copying = tile(shell, "big.mkv", TileKind::File);
        copying.size_bytes = Some(90 * 1024 * 1024);
        let _ = shell.composer.attach(key, vec![copying]);
    });
    let key = shown(&rt.state);
    let (_, bytes) = rt.state.composer.tray_load(key);
    let plan = crate::attaching::plan(
        &[crate::attaching::Offered::File {
            name: "more.mkv".to_owned(),
            size_bytes: 20 * 1024 * 1024,
        }],
        1,
        bytes,
    );
    assert_eq!(
        plan.refused,
        Some(crate::attaching::Refused::TooMuchTogether)
    );
}

#[test]
fn the_keys_keep_working_after_a_tile_goes_by_the_pointer() {
    let mut rt = into_tray();
    // ✕ on the current (last) tile, by the pointer.
    let id = rt.state.tray_ui.current.unwrap();
    rt.accessibility_action(
        &format!("tray-remove-{}", id.0),
        rune::accesskit::Action::Click,
        None,
    );
    rt.frame(FRAME_S);
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    key_with(&mut rt, Key::Left, alt);
    assert_eq!(
        names(&rt),
        ["party.gif", "praia.jpg"],
        "Alt+← moved a tile, not the channel"
    );
}
