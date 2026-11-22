//! "Open this link?", headless: the one confirmation shape, with the box
//! that trusts the host as its foot row's note.

use super::navigation::{click, focused, with_servers};
use super::*;
use crate::links::Prompt;

/// The app asking about a link to `example.org`.
fn asking() -> Runtime<Shell> {
    let mut rt = with_servers();
    rt.update(|shell, _tx| {
        shell.content.link = Some(Prompt {
            url: "https://example.org/page".to_owned(),
            host: "example.org".to_owned(),
            always: false,
        });
    });
    rt.frame(FRAME_S);
    rt
}

fn rect(rt: &Runtime<Shell>, key: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no {key}"))
        .rect
}

#[test]
fn the_link_question_opens_on_cancel_and_return_keeps_the_link_closed() {
    let mut rt = asking();
    assert_eq!(focused(&rt).as_deref(), Some("open-link-cancel"));
    press(&mut rt, Key::Enter);
    assert!(rt.find("open-link").is_none());
    assert!(rt.state.content.link.is_none());
    assert!(!rt.state.content.trusted.contains("example.org"));
}

#[test]
fn the_trust_box_is_the_foot_row_s_note_left_of_the_buttons() {
    let mut rt = asking();
    let always = rect(&rt, "open-link-always");
    let cancel = rect(&rt, "open-link-cancel");
    let open = rect(&rt, "open-link-confirm");
    assert!(always[0] + always[2] <= cancel[0], "left of Cancel");
    let label = rect(&rt, "open-link-always-label");
    assert!(
        label[0] + label[2] <= cancel[0],
        "its words wrap before Cancel"
    );
    assert!(cancel[0] < open[0], "Cancel, then the action");
    let middle = |rect: [f32; 4]| rect[1] + rect[3] / 2.0;
    assert!(
        (middle(always) - middle(cancel)).abs() < 1.0,
        "on one row: {always:?} {cancel:?}"
    );
    click(&mut rt, "open-link-always");
    assert!(
        rt.state
            .content
            .link
            .as_ref()
            .is_some_and(|prompt| prompt.always)
    );
}
