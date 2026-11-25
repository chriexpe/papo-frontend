//! Someone else's profile card, drawn (`docs/design.md` §10): the facts a
//! card and a DM's profile pane share about a person, the card's header
//! and sections; its actions are `actions`'.
//!
//! Owns the drawing only; what the actions do is the card's handlers'.

use papo_sdk::Uuid;
use papo_sdk::permissions::global_permissions;
use papo_sdk_ui::{PresenceMark, display_name, person, requested_profile};
use rune::{El, SurfaceFit, col, el, row, text};

use super::super::Shell;
use crate::design::{
    AVATAR_CARD, Look, RADIUS_CONTROL, RADIUS_PILL, SPACE_LG, SPACE_MD, SPACE_SM, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM, TYPE_DISPLAY, TYPE_SMALL,
};
use crate::person_avatar::photo_avatar;
use crate::shell::Pictures;
use crate::strings;
use crate::timeline::TimelineKey;

/// What a card and a DM's profile pane say about person `user`, as
/// timeline `key`'s account knows them.
pub(in crate::shell) struct PersonFacts {
    pub(in crate::shell) name: String,
    pub(in crate::shell) username: Option<String>,
    pub(in crate::shell) presence: Option<PresenceMark>,
    pub(in crate::shell) status: Option<String>,
    pub(in crate::shell) roles: Vec<String>,
    pub(in crate::shell) about: Option<String>,
    /// "3 March 2025".
    pub(in crate::shell) since: Option<String>,
    /// The server the account is on.
    pub(in crate::shell) server: String,
    /// Their photo and banner, once fetched.
    pub(in crate::shell) pictures: Pictures,
}

impl PersonFacts {
    /// None while `key` is not shown or its resources are unknown.
    pub(in crate::shell) fn of(shell: &Shell, key: TimelineKey, user: Uuid) -> Option<PersonFacts> {
        let resources = shell.shown_timeline(key)?.resources.as_deref()?;
        let member = person(resources, user);
        let name = display_name(member.nickname.as_deref(), member.username.as_deref())
            .unwrap_or(strings::someone())
            .to_owned();
        let people = shell.people(key.account);
        let profile = requested_profile(resources, user);
        Some(PersonFacts {
            name,
            username: member.username.clone(),
            presence: people.and_then(|people| people.presence_mark(user)),
            status: people
                .and_then(|people| people.status_phrase(user))
                .map(str::to_owned),
            roles: member
                .roles
                .iter()
                .filter_map(|role| role.name.clone())
                .collect(),
            about: profile
                .as_ref()
                .and_then(|profile| profile.description.clone())
                .filter(|about| !about.trim().is_empty()),
            since: profile
                .as_ref()
                .and_then(|profile| profile.member_since.as_deref())
                .and_then(|instant| shell.local_time.full_date(instant)),
            server: shell
                .nav
                .servers()
                .get(key.account.server())
                .map(|server| server.name.clone())
                .unwrap_or_default(),
            pictures: shell
                .profile_pictures
                .get(key.account, user)
                .cloned()
                .unwrap_or_default(),
        })
    }
}

/// The open card, drawn; none while no card is open or its conversation
/// is no longer shown (`poll_card` closes it).
pub(in crate::shell) fn body(shell: &Shell, look: &Look) -> Option<El> {
    let card = shell.nav.menu.as_ref()?.card.as_ref()?;
    let facts = PersonFacts::of(shell, card.key, card.user)?;
    let resources = shell.shown_timeline(card.key)?.resources.as_deref()?;
    let me = card.key.account.user();
    let own = card.user == me;
    // The backend's ban gate is Manage server.
    let moderator = global_permissions(resources, me).is_ok_and(|granted| granted.manage_server);
    let actions = super::actions::Actions {
        key: card.key,
        user: card.user,
        block: (!own).then(|| resources.blocks.contains(card.user)),
        ban: (!own && moderator).then_some(card.banning),
    };
    Some(
        col()
            .w_full()
            .role("group")
            .label(strings::card_label(&facts.name))
            .children(header(&facts, true, look))
            .children(sections(&facts, Order::Card, look))
            .child(el().h(look.space(SPACE_MD)))
            .children(actions.view(&facts.name, &facts.server, look)),
    )
}

/// The top: the avatar with presence and, when `status` says so, the
/// status bubble; then the name and @handle.
pub(in crate::shell) fn header(facts: &PersonFacts, status: bool, look: &Look) -> [El; 3] {
    let banner = facts
        .pictures
        .banner
        .as_ref()
        .filter(|banner| banner.error().is_none());
    // The banner, 3 : 1 across the top, when one is set: its place is held
    // while it decodes, which redraws without rebuilding the card.
    let band = match banner {
        Some(banner) => el()
            .key("card-banner")
            .w_full()
            .aspect_ratio(3.0)
            // Inside the card's padding: the pill's corner, less the padding.
            .radius(look.radius(RADIUS_PILL) - look.space(SPACE_SM))
            .clip_rounded(look.radius(RADIUS_PILL) - look.space(SPACE_SM))
            .fill(look.palette.surface_hover)
            .pixels_fit(banner.pixels(), SurfaceFit::Fill)
            .a11y_hidden(),
        None => el(),
    };
    let mut top = row()
        .w_full()
        .px(look.space(SPACE_LG))
        .pt(look.space(SPACE_LG))
        .gap(look.space(SPACE_MD))
        .items_end()
        .child(photo_avatar(
            &facts.name,
            facts.pictures.avatar.as_ref(),
            AVATAR_CARD,
            facts.presence,
            look.palette.surface_pill,
            look,
        ));
    if let Some(phrase) = facts.status.as_deref().filter(|_| status) {
        top = top.child(bubble(phrase, look));
    }
    let mut who = col()
        .w_full()
        .px(look.space(SPACE_LG))
        .pt(look.space(SPACE_SM))
        .child(
            text(facts.name.clone())
                .font(look.font(TYPE_DISPLAY))
                .color(look.palette.text_primary)
                .max_lines(1),
        );
    if let Some(username) = &facts.username {
        who = who.child(
            text(format!("@{username}"))
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .max_lines(1),
        );
    }
    [band, top, who]
}

/// Where the sections go: the card leads with roles, a DM's profile pane
/// with About and names the server in its roles' heading.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::shell) enum Order {
    Card,
    Pane,
}

/// Roles, About and "Member since", each only when there is something to
/// say, in `order`'s order.
pub(in crate::shell) fn sections(facts: &PersonFacts, order: Order, look: &Look) -> Vec<El> {
    let roles = (!facts.roles.is_empty()).then(|| {
        let heading = match order {
            Order::Card => strings::card_roles().to_owned(),
            Order::Pane => strings::direct_roles_in(&facts.server),
        };
        section(
            &heading,
            row()
                .w_full()
                .gap(look.space(SPACE_SM))
                .wrap()
                .children(facts.roles.iter().map(|role| role_chip(role, look))),
            look,
        )
    });
    let about = facts.about.as_deref().map(|about| {
        section(
            strings::card_about(),
            text(about.to_owned())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_secondary)
                .w_full(),
            look,
        )
    });
    let since = facts.since.as_deref().map(|since| {
        el().w_full()
            .px(look.space(SPACE_LG))
            .pt(look.space(SPACE_MD))
            .child(
                text(strings::card_member_since(since))
                    .font(look.font(TYPE_CAPTION))
                    .color(look.palette.text_muted),
            )
    });
    let ordered = match order {
        Order::Card => [roles, about, since],
        Order::Pane => [about, roles, since],
    };
    ordered.into_iter().flatten().collect()
}

/// The status phrase in a bubble beside the avatar.
fn bubble(status: &str, look: &Look) -> El {
    el().mb(look.space(SPACE_SM))
        .px(look.space(SPACE_MD))
        .py(look.space(SPACE_SM))
        .radius(look.radius(RADIUS_CONTROL))
        .fill(look.palette.surface_content)
        .border(1.0, look.palette.line_subtle)
        .min_w(0.0)
        .child(
            text(status.to_owned())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_secondary)
                .max_lines(2),
        )
}

/// A titled section: its heading in capitals' place, then `body`.
fn section(heading: &str, body: El, look: &Look) -> El {
    col()
        .w_full()
        .px(look.space(SPACE_LG))
        .pt(look.space(SPACE_MD))
        .gap(look.space(SPACE_SM))
        .child(
            text(heading.to_uppercase())
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.text_muted),
        )
        .child(body)
}

/// A role's chip. Role colours come with their slice.
fn role_chip(role: &str, look: &Look) -> El {
    el().px(look.space(SPACE_SM))
        .py(look.space(SPACE_SM) / 2.0)
        .radius(look.radius(RADIUS_CONTROL))
        .fill(look.palette.surface_content)
        .border(1.0, look.palette.line_subtle)
        .child(
            text(role.to_owned())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_secondary),
        )
}
