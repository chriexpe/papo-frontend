//! Someone else's profile card, drawn (`docs/design.md` §10): the facts a
//! card and a DM's profile pane share about a person, the card's header
//! and sections, and its actions as the account may use them.
//!
//! Owns the drawing only; what the actions do is the card's handlers'.

use papo_sdk::Uuid;
use papo_sdk::permissions::global_permissions;
use papo_sdk_ui::{PresenceMark, display_name, person, requested_profile};
use rune::{El, col, el, icon, row, text};

use super::super::Shell;
use crate::design::{
    AVATAR_CARD, FOCUS_RING_PT, Look, MENU_ROW_HEIGHT_PT, RADIUS_CONTROL, ROW_ICON_PT, SPACE_LG,
    SPACE_MD, SPACE_SM, TYPE_BODY, TYPE_BODY_STRONG, TYPE_CAPTION, TYPE_CAPTION_MEDIUM,
    TYPE_DISPLAY, TYPE_SMALL,
};
use crate::icons::{BAN, BLOCK, DIRECT, ID};
use crate::menu::{MenuAction, MenuItem, PersonAction, item_row};
use crate::person_avatar::person_avatar;
use crate::strings;
use crate::timeline::TimelineKey;

/// The element key of the card's Message.
pub(super) const MESSAGE_KEY: &str = "card-message";
/// The element key of the card's Copy ID.
pub(super) const COPY_ID_KEY: &str = "card-copy-id";
/// The element key of the card's Ban.
pub(super) const BAN_KEY: &str = "card-ban";
/// The element key of the Ban question's Cancel.
pub(super) const BAN_CANCEL_KEY: &str = "card-ban-cancel";

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
    let actions = Actions {
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
pub(in crate::shell) fn header(facts: &PersonFacts, status: bool, look: &Look) -> [El; 2] {
    let mut top = row()
        .w_full()
        .px(look.space(SPACE_LG))
        .pt(look.space(SPACE_LG))
        .gap(look.space(SPACE_MD))
        .items_end()
        .child(person_avatar(
            &facts.name,
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
    [top, who]
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

/// The card's actions, as its person and the account allow. Your own card
/// has no Message, Block or Ban.
struct Actions {
    key: TimelineKey,
    user: Uuid,
    /// Whether this account blocks them; none on your own card.
    block: Option<bool>,
    /// Whether Ban is asking; none unless the account may ban them.
    ban: Option<bool>,
}

impl Actions {
    fn view(&self, name: &str, server: &str, look: &Look) -> Vec<El> {
        let mut rows = Vec::new();
        if self.block.is_some() {
            rows.push(self.message_button(look));
        }
        let copy_id = PersonAction::CopyId;
        let mut items = vec![
            self.item(COPY_ID_KEY, strings::card_copy_id(), copy_id, false)
                .with_icon(&ID),
        ];
        if let Some(blocked) = self.block {
            let (label, action) = if blocked {
                (strings::card_unblock(), PersonAction::Unblock)
            } else {
                (strings::card_block(), PersonAction::Block)
            };
            items.push(
                self.item("card-block", label, action, false)
                    .with_icon(&BLOCK),
            );
        }
        if self.ban == Some(false) {
            let ask = PersonAction::AskBan;
            items.push(
                self.item(BAN_KEY, strings::card_ban(), ask, true)
                    .with_icon(&BAN),
            );
        }
        rows.extend(items.iter().map(|item| item_row(item, false, look)));
        if self.ban == Some(true) {
            rows.push(self.ban_question(name, server, look));
        }
        rows
    }

    fn item(&self, key: &str, label: &str, action: PersonAction, danger: bool) -> MenuItem {
        MenuItem::plain(
            key,
            label,
            MenuAction::Person {
                key: self.key,
                user: self.user,
                action,
            },
            danger,
        )
    }

    /// Message, the accent button: it starts something.
    fn message_button(&self, look: &Look) -> El {
        let action = MenuAction::Person {
            key: self.key,
            user: self.user,
            action: PersonAction::Message,
        };
        row()
            .key(MESSAGE_KEY)
            .w_full()
            .h(MENU_ROW_HEIGHT_PT)
            .mb(look.space(SPACE_SM))
            .gap(look.space(SPACE_SM))
            .center()
            .radius(look.radius(RADIUS_CONTROL))
            .fill(look.palette.accent)
            .role("button")
            .label(strings::card_message())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |shell: &mut Shell, tx| shell.menu_picked(action.clone(), tx))
            .child(icon(&DIRECT, ROW_ICON_PT).color(look.palette.accent_on))
            .child(
                text(strings::card_message())
                    .font(look.font(TYPE_BODY_STRONG))
                    .color(look.palette.accent_on)
                    .a11y_hidden(),
            )
    }

    /// "Ban Lia from Cyber Asilo?", unframed, with Cancel and Ban on the
    /// right.
    fn ban_question(&self, name: &str, server: &str, look: &Look) -> El {
        let button = |key: &str, label: &str, action: PersonAction, danger: bool| {
            let action = MenuAction::Person {
                key: self.key,
                user: self.user,
                action,
            };
            let (fill, ink) = if danger {
                (look.palette.danger, look.palette.accent_on)
            } else {
                (look.palette.surface_hover, look.palette.text_primary)
            };
            row()
                .key(key.to_owned())
                .px(look.space(SPACE_MD))
                .py(look.space(SPACE_SM))
                .radius(look.radius(RADIUS_CONTROL))
                .fill(fill)
                .role("button")
                .label(label.to_owned())
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press_tx(move |shell: &mut Shell, tx| shell.menu_picked(action.clone(), tx))
                .child(
                    text(label.to_owned())
                        .font(look.font(TYPE_BODY))
                        .color(ink)
                        .a11y_hidden(),
                )
        };
        col()
            .w_full()
            .px(look.space(SPACE_LG))
            .py(look.space(SPACE_SM))
            .gap(look.space(SPACE_SM))
            .child(
                text(strings::card_ban_ask(name, server))
                    .font(look.font(TYPE_BODY))
                    .color(look.palette.text_primary)
                    .w_full(),
            )
            .child(
                row()
                    .w_full()
                    .gap(look.space(SPACE_SM))
                    .justify_end()
                    .child(button(
                        BAN_CANCEL_KEY,
                        strings::card_cancel(),
                        PersonAction::CancelBan,
                        false,
                    ))
                    .child(button(
                        "card-ban-confirm",
                        strings::card_ban(),
                        PersonAction::Ban,
                        true,
                    )),
            )
    }
}
