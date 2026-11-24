//! The Conversations page (`docs/design.md` §8): its rows save at once,
//! what they change applies at once, a failed save goes back with its
//! danger line, and Show avatars brings Stack messages to every density.

use rune::Runtime;
use rune::prefs::Prefs;

use super::navigation::click;
use super::settings::{open, saved};
use super::*;
use crate::conversation_settings::{ConversationSettings, ConversationStore};
use crate::design::Density;
use crate::settings::{PageId, RowId};

fn conversations(rt: &Runtime<Shell>) -> ConversationSettings {
    *rt.state.conversations.settings()
}

fn open_conversations(rt: &mut Runtime<Shell>) {
    open(rt);
    click(rt, &format!("settings-nav-{:?}", PageId::Conversations));
    assert_eq!(
        rt.state
            .settings()
            .map(crate::settings::SettingsPanel::page),
        Some(PageId::Conversations)
    );
}

#[test]
fn show_times_and_show_avatars_flip_at_once_and_reach_the_rows() {
    let mut rt = app();
    open_conversations(&mut rt);
    click(&mut rt, &RowId::ShowTimes.key());
    click(&mut rt, &RowId::ShowAvatars.key());
    let settings = conversations(&rt);
    assert!(!settings.show_times && !settings.show_avatars);
    let layout = rt.state.message_layout();
    assert!(!layout.times && !layout.avatars);
    assert!(!layout.has_avatars());
}

#[test]
fn without_avatars_stack_messages_shows_at_any_spacing() {
    let mut rt = app();
    open_conversations(&mut rt);
    click(&mut rt, &RowId::ShowAvatars.key());
    click(&mut rt, &format!("settings-nav-{:?}", PageId::Appearance));
    assert_eq!(saved(&rt).messages.density, Density::Normal);
    assert!(rt.find(&RowId::Stacked.key()).is_some());
}

#[test]
fn show_avatars_is_left_out_at_extra_compact() {
    let mut rt = app();
    open(&mut rt);
    rt.update(|shell, tx| {
        shell.settings_chosen(
            crate::shell::SettingChoice::Density(Density::ExtraCompact),
            tx,
        );
    });
    rt.frame(FRAME_S);
    click(
        &mut rt,
        &format!("settings-nav-{:?}", PageId::Conversations),
    );
    assert!(rt.find(&RowId::ShowAvatars.key()).is_none());
    assert!(rt.find(&RowId::ShowTimes.key()).is_some());
}

#[test]
fn a_hue_swatch_sets_the_mention_highlight() {
    let mut rt = app();
    open_conversations(&mut rt);
    let before = rt.state.look(true).palette.highlight_mention;
    click(&mut rt, &RowId::Highlights.key());
    click(&mut rt, &format!("{}-196", RowId::MentionHue.key()));
    assert!((conversations(&rt).mention_hue_degrees - 196.0).abs() < f32::EPSILON);
    assert_ne!(rt.state.look(true).palette.highlight_mention, before);
}

#[test]
fn a_failed_save_goes_back_and_try_again_saves_the_same_settings() {
    let mut rt = app();
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("file");
    std::fs::write(&blocker, b"").unwrap();
    let failing = blocker.join("prefs.json");
    rt.update(move |shell, _tx| {
        shell.conversations = ConversationStore::read(&Prefs::at(failing));
    });
    open_conversations(&mut rt);
    click(&mut rt, &RowId::Hourglass.key());
    assert!(conversations(&rt).unread_hourglass, "the old value stays");
    let shown = tree(&rt);
    assert_eq!(shown.matches("\"role\":\"alert\"").count(), 1, "{shown}");
    // The file can be written now: Try again saves the hourglass off.
    let working = directory.path().join("prefs.json");
    let path = working.clone();
    rt.update(move |shell, _tx| {
        shell.conversations = ConversationStore::read(&Prefs::at(path));
    });
    click(&mut rt, "settings-retry");
    assert!(!conversations(&rt).unread_hourglass);
    assert_eq!(
        Prefs::at(&working).get::<bool>("unread_hourglass"),
        Some(false)
    );
    assert!(!tree(&rt).contains("\"role\":\"alert\""));
}
