//! The General page (`docs/design.md` §8): Language takes effect at once
//! and goes back if its save fails, Confirm external links saves, and
//! Trusted domains adds what is typed, refuses what names no site, and
//! removes at once.

use rune::Runtime;
use rune::prefs::Prefs;

use super::navigation::click;
use super::settings::open;
use super::*;
use crate::general_settings::{GeneralStore, Language};
use crate::settings::domain_field::DOMAIN_FIELD_KEY;
use crate::settings::{PageId, RowId};

fn open_page(rt: &mut Runtime<Shell>, page: PageId) {
    open(rt);
    click(rt, &format!("settings-nav-{:?}", page.nav()));
    if page.parent().is_some() {
        rt.update(move |shell, tx| shell.settings_page(page, tx));
        rt.frame(FRAME_S);
    }
}

fn choose_portuguese(rt: &mut Runtime<Shell>) {
    click(rt, &RowId::Language.key());
    click(rt, "settings-language-pt-BR");
}

#[test]
fn a_language_takes_effect_at_once_and_redraws_every_view() {
    let mut rt = app();
    open_page(&mut rt, PageId::General);
    let before = rt.state.look(true);
    choose_portuguese(&mut rt);
    assert_eq!(
        rt.state.general.settings().language,
        Language::Locale("pt-BR")
    );
    assert_eq!(
        crate::strings::locale_index(),
        1,
        "the catalog's pt-BR is in effect"
    );
    assert_ne!(
        rt.state.look(true),
        before,
        "the look's key moved, so memoised rows redraw"
    );
    click(&mut rt, &RowId::Language.key());
    click(&mut rt, "settings-language-system");
    assert_eq!(rt.state.general.settings().language, Language::System);
}

#[test]
fn a_language_whose_save_fails_goes_back() {
    let mut rt = app();
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("file");
    std::fs::write(&blocker, b"").unwrap();
    let failing = blocker.join("prefs.json");
    rt.update(move |shell, _tx| shell.general = GeneralStore::read(&Prefs::at(failing)));
    open_page(&mut rt, PageId::General);
    let before = crate::strings::locale_index();
    choose_portuguese(&mut rt);
    assert_eq!(rt.state.general.settings().language, Language::System);
    assert_eq!(
        crate::strings::locale_index(),
        before,
        "the words go back too"
    );
    assert!(tree(&rt).contains("\"role\":\"alert\""));
}

#[test]
fn confirm_external_links_saves() {
    let mut rt = app();
    open_page(&mut rt, PageId::General);
    click(&mut rt, &RowId::ConfirmLinks.key());
    assert!(!rt.state.general.settings().confirm_links);
}

fn type_domain(rt: &mut Runtime<Shell>, text: &str) {
    let text = text.to_owned();
    rt.update(move |shell, _tx| *shell.settings_domain_mut() = rune::TextEdit::new(&text));
    rt.frame(FRAME_S);
    click(rt, &format!("{}-add", RowId::TrustedAdd.key()));
}

#[test]
fn a_typed_domain_is_trusted_and_its_remove_takes_it_off() {
    let mut rt = app();
    open_page(&mut rt, PageId::Trusted);
    assert!(rt.find(DOMAIN_FIELD_KEY).is_some());
    assert!(tree(&rt).contains("No trusted domains."));
    type_domain(&mut rt, "Docs.Example.org");
    assert!(rt.state.content.trusted.contains("docs.example.org"));
    let panel = rt.state.settings().expect("open");
    assert!(panel.domain.text.is_empty(), "the field clears");
    click(&mut rt, &format!("{}-remove", RowId::TrustedHost(0).key()));
    assert!(!rt.state.content.trusted.contains("docs.example.org"));
}

#[test]
fn text_that_names_no_site_stays_with_its_line() {
    let mut rt = app();
    open_page(&mut rt, PageId::Trusted);
    type_domain(&mut rt, "not a site");
    assert!(rt.state.content.trusted.hosts().is_empty());
    let shown = tree(&rt);
    assert!(
        shown.contains("Type a domain, like example.org."),
        "{shown}"
    );
    assert_eq!(rt.state.settings().expect("open").domain.text, "not a site");
}
