//! The General page and its Trusted domains sub-page (`docs/design.md`
//! §8): Language; Links (Confirm external links, Trusted domains), and the
//! domains themselves, with a field that adds one. Save media to comes
//! with D7.
//!
//! Owns these pages' rows; the settings are `general_settings`' and
//! `trusted_domains`'.

use super::{Context, Group, Kind, Row};
use crate::general_settings::Language;
use crate::settings::{PageId, RowId};
use crate::strings;
use crate::trusted_domains::host_id;

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
            ],
        },
    ]
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
