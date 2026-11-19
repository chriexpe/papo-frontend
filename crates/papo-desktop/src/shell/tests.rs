//! Headless tests of the shell view: the production view in Rune's
//! `Runtime`, with no window, display or GPU. They prove behaviour, not
//! pixels or the platform's input method.

use rune::{Env, Ime, Key, KeyEvent, Mods, Runtime};

use super::{COMPOSER_KEY, Shell, view};

const SIZE_PT: (f32, f32) = (1000.0, 700.0);
const FRAME_S: f64 = 1.0 / 60.0;

fn runtime() -> Runtime<Shell> {
    let env = Env {
        system_dark: true,
        scale: 1.0,
    };
    let mut rt = Runtime::new(Shell::default(), view, SIZE_PT, env);
    rt.frame(0.0);
    rt
}

fn type_text(rt: &mut Runtime<Shell>, text: &str) {
    rt.key(KeyEvent {
        key: Key::Text(text.to_owned()),
        mods: Mods::default(),
    });
    rt.frame(FRAME_S);
}

#[test]
fn the_composer_has_focus_at_start_and_takes_typing() {
    let mut rt = runtime();
    assert!(rt.has_focus());
    type_text(&mut rt, "olá");
    assert_eq!(rt.state.composer.text, "olá");
}

#[test]
fn an_input_method_composition_commits_into_the_composer() {
    let mut rt = runtime();
    rt.ime(Ime::Preedit("にほん".to_owned(), Some((9, 9)), Vec::new()));
    rt.frame(FRAME_S);
    assert_eq!(
        rt.state.composer.text, "",
        "a preedit is drawn, not inserted"
    );
    rt.ime(Ime::Commit("日本".to_owned()));
    rt.frame(FRAME_S);
    assert_eq!(rt.state.composer.text, "日本");
}

#[test]
fn a_cancelled_composition_leaves_the_text_as_it_was() {
    let mut rt = runtime();
    type_text(&mut rt, "a");
    rt.ime(Ime::Preedit("か".to_owned(), Some((3, 3)), Vec::new()));
    rt.frame(FRAME_S);
    rt.ime(Ime::Preedit(String::new(), None, Vec::new()));
    rt.frame(FRAME_S);
    assert_eq!(rt.state.composer.text, "a");
}

#[test]
fn assistive_technology_finds_a_labelled_text_field() {
    let rt = runtime();
    let tree = rt.accessibility_json().to_string();
    assert!(
        tree.contains(&format!("\"{COMPOSER_KEY}\"")),
        "the composer is in the tree: {tree}"
    );
    assert!(
        tree.contains("\"Message\""),
        "the composer is named: {tree}"
    );
}

#[test]
fn a_settled_window_asks_for_no_more_frames() {
    let mut rt = runtime();
    for _ in 0..120 {
        rt.frame(FRAME_S);
    }
    assert!(
        !rt.frame(FRAME_S),
        "an idle window with a focused field keeps animating"
    );
    assert_eq!(
        rt.wake_deadline(),
        None,
        "an idle window with a focused field keeps a timer"
    );
}
