//! The General page and its Trusted domains and Embedded players sub-pages
//! (`docs/design.md` §8, §15.7): Language; Links (Confirm external links,
//! Trusted domains, Embedded players), the domains themselves with a field
//! that adds one, and the players' switch and sites; Files (Save media to).
//!
//! Owns these pages' rows; the settings are `general_settings`',
//! `trusted_domains`' and `embedded_players`'.

use super::{Context, Group, Kind, Row, SiteRow};
use crate::general_settings::Language;
use crate::settings::{PageId, RowId};
use crate::strings;
use crate::trusted_domains::host_id;
use papo_sdk_ui::PlayerProvider;

pub(super) fn general(cx: &Context<'_>) -> Vec<Group> {
    let settings = cx.general;
    vec![
        Group {
            title: None,
            rows: vec![Row {
                id: RowId::Language,
                title: strings::settings_language().to_owned(),
                sub: None,
                kind: Kind::Choice(language_name(settings.language).to_owned()),
            }],
        },
        Group {
            title: Some(strings::settings_group_links().to_owned()),
            rows: vec![
                Row {
                    id: RowId::ConfirmLinks,
                    title: strings::settings_confirm_links().to_owned(),
                    sub: Some(strings::settings_confirm_links_sub().to_owned()),
                    kind: Kind::Switch(settings.confirm_links),
                },
                Row {
                    id: RowId::Trusted,
                    title: strings::settings_trusted().to_owned(),
                    sub: Some(strings::settings_trusted_sub(cx.trusted.len())),
                    kind: Kind::SubPage(PageId::Trusted, String::new()),
                },
                Row {
                    id: RowId::Players,
                    title: strings::settings_players().to_owned(),
                    sub: Some(strings::settings_players_sub(cx.players.allowed.len())),
                    kind: Kind::SubPage(PageId::Players, String::new()),
                },
            ],
        },
        Group {
            title: Some(strings::settings_group_files().to_owned()),
            rows: vec![Row {
                id: RowId::SaveMediaTo,
                title: strings::settings_save_media_to().to_owned(),
                sub: settings.media_folder().as_deref().map(folder_shown),
                kind: Kind::Folder,
            }],
        },
    ]
}

/// A folder as the row shows it: the home folder as "~".
fn folder_shown(folder: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| folder.strip_prefix(home).ok())
    {
        Some(inside) if inside.as_os_str().is_empty() => "~".to_owned(),
        Some(inside) => format!("~/{}", inside.display()),
        None => folder.display().to_string(),
    }
}

pub(super) fn trusted(cx: &Context<'_>) -> Vec<Group> {
    let hosts: Vec<Row> = cx
        .trusted
        .iter()
        .map(|host| Row {
            id: RowId::TrustedHost(host_id(host)),
            title: host.clone(),
            sub: None,
            kind: Kind::Remove(host.clone()),
        })
        .collect();
    let list = if hosts.is_empty() {
        vec![Row {
            id: RowId::TrustedHost(0),
            title: strings::settings_trusted_empty().to_owned(),
            sub: None,
            kind: Kind::Note,
        }]
    } else {
        hosts
    };
    vec![
        Group {
            title: None,
            rows: vec![Row {
                id: RowId::TrustedAdd,
                title: strings::settings_trusted().to_owned(),
                sub: None,
                kind: Kind::DomainField,
            }],
        },
        Group {
            title: None,
            rows: list,
        },
    ]
}

/// Embedded players (`docs/design.md` §15.7): Play videos and music from other sites, then a
/// row for every site that is allowed or still has data, with Clear data while it has data
/// and Remove while it is allowed, then the notes.
pub(super) fn players(cx: &Context<'_>) -> Vec<Group> {
    let sites: Vec<Row> = PlayerProvider::ALL
        .into_iter()
        .filter(|provider| {
            cx.players.allowed.contains(provider) || cx.sites_with_data.contains(provider)
        })
        .map(|provider| Row {
            id: RowId::PlayersSite(provider),
            title: provider.name().to_owned(),
            sub: Some(provider.host().to_owned()),
            kind: Kind::Site(SiteRow {
                provider,
                clear: cx.sites_with_data.contains(&provider),
                remove: cx.players.allowed.contains(&provider),
            }),
        })
        .collect();
    let mut list = if sites.is_empty() {
        vec![Row {
            id: RowId::PlayersNote,
            title: strings::settings_players_empty().to_owned(),
            sub: None,
            kind: Kind::Note,
        }]
    } else {
        sites
    };
    list.push(Row {
        id: RowId::PlayersNote,
        title: strings::settings_players_list_note().to_owned(),
        sub: Some(strings::settings_players_clear_note().to_owned()),
        kind: Kind::Note,
    });
    vec![
        Group {
            title: None,
            rows: vec![Row {
                id: RowId::PlayersOn,
                title: strings::settings_players_on().to_owned(),
                sub: Some(strings::settings_players_on_sub().to_owned()),
                kind: Kind::Switch(cx.players.on),
            }],
        },
        Group {
            title: Some(strings::settings_players_sites().to_owned()),
            rows: list,
        },
    ]
}

/// A language as the Language row and its menu name it: the system's, or
/// the language in its own words.
pub(crate) fn language_name(language: Language) -> &'static str {
    match language {
        Language::System => strings::settings_language_system(),
        Language::Locale(id) => strings::languages()
            .find(|(locale, _)| *locale == id)
            .map_or(id, |(_, label)| label),
    }
}

/// The General nav row's one line: "System · links".
pub(crate) fn general_summary(cx: &Context<'_>) -> String {
    strings::settings_general_summary(language_name(cx.general.language))
}
