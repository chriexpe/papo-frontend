//! Someone else's profile card (`docs/design.md` §10), from the shell's
//! side: opening it beside what was clicked (an author's name, a reactor's
//! row) as an open menu's body, drawing it, and what its actions do:
//! Message opens or reopens a direct conversation, Copy ID, Block and
//! Unblock at once, and for moderators Ban, asked inside its row.
//!
//! Owns the card's content and actions; the overlay (its anchor, Escape, a
//! click outside, focus going back) is the menu's, the person and their
//! profile are the SDK's, and no picture is drawn yet (a later slice).

use papo_sdk::Uuid;
use papo_sdk::operations::social::DirectAccess;
use papo_sdk::permissions::global_permissions;
use papo_sdk_ui::{DirectMessageId, PresenceMark, display_name, person, requested_profile};
use rune::{El, Tx, col, el, icon, row, text};

use super::Shell;
use crate::design::{
    AVATAR_CARD, FOCUS_RING_PT, MENU_ROW_HEIGHT_PT, Palette, RADIUS_CONTROL_PT, ROW_ICON_PT,
    SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_BODY_STRONG, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM, TYPE_DISPLAY, TYPE_SMALL,
};
use crate::icons::{BAN, BLOCK, DIRECT, ID};
use crate::menu::{CardView, MenuAction, MenuItem, OpenMenu, PersonAction, Placement, item_row};
use crate::notice::Refusal;
use crate::person_avatar::person_avatar;
use crate::strings;
use crate::timeline::TimelineKey;

/// The element key of the card's Message.
const MESSAGE_KEY: &str = "card-message";

impl Shell {
    /// Opens person `user`'s card, seen from timeline `key`, at
    /// `placement`; `trigger` gets the keyboard back when it closes. Asks
    /// for their profile when the account does not have it yet.
    pub(crate) fn open_card(
        &mut self,
        key: TimelineKey,
        user: Uuid,
        placement: Placement,
        trigger: String,
        tx: &mut Tx,
    ) {
        self.worked_outside();
        let mut menu = OpenMenu::with_entries(
            placement,
            trigger,
            Vec::new(),
            crate::menu::PROFILE_CARD_WIDTH_PT,
        );
        menu.card = Some(CardView {
            key,
            user,
            banning: false,
        });
        self.nav.menu = Some(menu);
        tx.focus(MESSAGE_KEY, true);
        self.load_profile(key, user);
    }

    /// A reactor's row was pressed: the reactions list turns into their
    /// card, which Escape turns back.
    pub(crate) fn reactor_pressed(&mut self, user: Uuid, tx: &mut Tx) {
        let Some(menu) = self.nav.menu.as_mut() else {
            return;
        };
        let Some(view) = &menu.reactions else {
            return;
        };
        let key = view.key;
        menu.card = Some(CardView {
            key,
            user,
            banning: false,
        });
        tx.focus(MESSAGE_KEY, true);
        self.load_profile(key, user);
    }

    /// Escape on a card the reactions list turned into: back to the list.
    /// Returns whether it was one.
    pub(super) fn card_back(&mut self, tx: &mut Tx) -> bool {
        let Some(menu) = self.nav.menu.as_mut() else {
            return false;
        };
        if menu.reactions.is_none() || menu.card.take().is_none() {
            return false;
        }
        tx.focus(crate::menu::MENU_KEY, true);
        true
    }

    /// Asks for person `user`'s profile when timeline `key`'s account does
    /// not have it yet; the SDK publishes it with the account's resources.
    pub(super) fn load_profile(&self, key: TimelineKey, user: Uuid) {
        let known = self
            .shown_timeline(key)
            .and_then(|open| open.resources.as_deref())
            .is_some_and(|resources| requested_profile(resources, user).is_some());
        if !known && let Some(sdk) = &self.sdk {
            sdk.run(move |client| async move {
                let Ok(session) = client.session(key.account) else {
                    return;
                };
                if let Err(error) = session.profiles(&[user]).await {
                    log::debug!("a profile was not read: {error}");
                }
            });
        }
    }

    /// Ban asks inside its row, or stops asking.
    pub(super) fn card_ban_asked(&mut self, asking: bool) {
        if let Some(card) = self.nav.menu.as_mut().and_then(|menu| menu.card.as_mut()) {
            card.banning = asking;
        }
    }

    /// A card's action on person `user` of timeline `key`'s account; the
    /// card has closed.
    pub(super) fn person_action(
        &mut self,
        key: TimelineKey,
        user: Uuid,
        action: PersonAction,
        tx: &mut Tx,
    ) {
        match action {
            PersonAction::Message => self.message_person(key, user),
            PersonAction::CopyId => tx.copy(user.to_string()),
            PersonAction::Block => self.set_blocked(key, user, true),
            PersonAction::Unblock => self.set_blocked(key, user, false),
            PersonAction::Ban => self.ban(key, user),
            // Kept by `menu_picked`: the card stays for them.
            PersonAction::AskBan | PersonAction::CancelBan => {}
        }
    }

    /// Opens or reopens the direct conversation with `user`, and picks it
    /// once the SDK's list holds it.
    fn message_person(&mut self, key: TimelineKey, user: Uuid) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(key.account) else {
                return;
            };
            let operations = session.operations();
            let opened = operations
                .open_direct_conversation(user)
                .await
                .map(|receipt| Uuid::parse_str(&receipt.value.id).ok());
            let refusal = match opened {
                Ok(Some(channel)) => {
                    let id = DirectMessageId {
                        account: key.account,
                        channel,
                    };
                    updates.send_tx(move |shell, tx| {
                        shell.await_dm(id, tx);
                        true
                    });
                    return;
                }
                Ok(None) => Refusal::OpenDm,
                // A refusal reads as one only when the backend says so.
                Err(error) => {
                    log::warn!("a direct conversation was not opened: {error}");
                    match DirectAccess::proven_by(&error) {
                        Some(DirectAccess::Refused) => Refusal::DmRefused,
                        _ => Refusal::OpenDm,
                    }
                }
            };
            updates.send_tx(move |shell, tx| {
                shell.refuse(refusal, tx);
                true
            });
        });
    }

    pub(super) fn set_blocked(&mut self, key: TimelineKey, user: Uuid, blocked: bool) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let done = match client.session(key.account) {
                Ok(session) => {
                    let operations = session.operations();
                    let sent = if blocked {
                        operations.block_user(user).await
                    } else {
                        operations.unblock_user(user).await
                    };
                    sent.map(|_| ()).map_err(|error| error.to_string())
                }
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = done {
                log::warn!("a block was not changed: {error}");
                let refusal = if blocked {
                    Refusal::Block
                } else {
                    Refusal::Unblock
                };
                updates.send_tx(move |shell, tx| {
                    shell.refuse(refusal, tx);
                    true
                });
            }
        });
    }

    fn ban(&mut self, key: TimelineKey, user: Uuid) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let done = match client.session(key.account) {
                Ok(session) => session
                    .operations()
                    .set_banned(user, true)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = done {
                log::warn!("a ban did not happen: {error}");
                updates.send_tx(|shell, tx| {
                    shell.refuse(Refusal::Ban, tx);
                    true
                });
            }
        });
    }
}

/// The open card, drawn; none while no card is open or its conversation
/// is no longer shown (the next poll closes it).
pub(super) fn body(shell: &Shell, palette: &Palette) -> Option<El> {
    let card = shell.nav.menu.as_ref()?.card.as_ref()?;
    let open = shell.shown_timeline(card.key)?;
    let resources = open.resources.as_deref()?;
    let me = card.key.account.user();
    let member = person(resources, card.user);
    let name = display_name(member.nickname.as_deref(), member.username.as_deref())
        .unwrap_or(strings::someone())
        .to_owned();
    let people = shell.people(card.key.account);
    let profile = requested_profile(resources, card.user);
    let since = profile
        .as_ref()
        .and_then(|profile| profile.member_since.as_deref())
        .and_then(|instant| shell.local_time.full_date(instant));
    let header = Header {
        name: &name,
        username: member.username.as_deref(),
        presence: people.and_then(|people| people.presence_mark(card.user)),
        status: people.and_then(|people| people.status_phrase(card.user)),
    };
    let about = profile
        .as_ref()
        .and_then(|profile| profile.description.as_deref())
        .filter(|about| !about.trim().is_empty());
    let roles: Vec<&str> = member
        .roles
        .iter()
        .filter_map(|role| role.name.as_deref())
        .collect();
    let own = card.user == me;
    // The backend's ban gate is Manage server.
    let moderator = global_permissions(resources, me).is_ok_and(|granted| granted.manage_server);
    let actions = Actions {
        key: card.key,
        user: card.user,
        block: (!own).then(|| resources.blocks.contains(card.user)),
        ban: (!own && moderator).then_some(card.banning),
    };
    let server = shell
        .nav
        .servers()
        .get(card.key.account.server())
        .map(|server| server.name.clone())
        .unwrap_or_default();
    Some(
        col()
            .w_full()
            .role("group")
            .label(strings::card_label(&name))
            .children(header.view(palette))
            .children(sections(
                strings::card_roles(),
                &roles,
                about,
                since.as_deref(),
                palette,
            ))
            .child(el().h(SPACE_MD_PT))
            .children(actions.view(&name, &server, palette)),
    )
}

/// The card's top: the avatar with presence and the status bubble, then
/// the name and @handle.
pub(super) struct Header<'a> {
    pub(super) name: &'a str,
    pub(super) username: Option<&'a str>,
    pub(super) presence: Option<PresenceMark>,
    pub(super) status: Option<&'a str>,
}

impl Header<'_> {
    pub(super) fn view(&self, palette: &Palette) -> [El; 2] {
        let mut top = row()
            .w_full()
            .px(SPACE_LG_PT)
            .pt(SPACE_LG_PT)
            .gap(SPACE_MD_PT)
            .items_end()
            .child(person_avatar(
                self.name,
                AVATAR_CARD,
                self.presence,
                palette.surface_pill,
                palette,
            ));
        if let Some(status) = self.status {
            top = top.child(bubble(status, palette));
        }
        let mut who = col().w_full().px(SPACE_LG_PT).pt(SPACE_SM_PT).child(
            text(self.name.to_owned())
                .font(TYPE_DISPLAY)
                .color(palette.text_primary)
                .max_lines(1),
        );
        if let Some(username) = self.username {
            who = who.child(
                text(format!("@{username}"))
                    .font(TYPE_SMALL)
                    .color(palette.text_muted)
                    .max_lines(1),
            );
        }
        [top, who]
    }
}

/// Roles (under `roles_heading`), About and "Member since", each only when
/// there is something to say.
pub(super) fn sections(
    roles_heading: &str,
    roles: &[&str],
    about: Option<&str>,
    since: Option<&str>,
    palette: &Palette,
) -> Vec<El> {
    let mut sections = Vec::new();
    if !roles.is_empty() {
        sections.push(section(
            roles_heading,
            row()
                .w_full()
                .gap(SPACE_SM_PT)
                .wrap()
                .children(roles.iter().map(|role| role_chip(role, palette))),
            palette,
        ));
    }
    if let Some(about) = about {
        sections.push(section(
            strings::card_about(),
            text(about.to_owned())
                .font(TYPE_SMALL)
                .color(palette.text_secondary)
                .w_full(),
            palette,
        ));
    }
    if let Some(since) = since {
        sections.push(
            el().w_full().px(SPACE_LG_PT).pt(SPACE_MD_PT).child(
                text(strings::card_member_since(since))
                    .font(TYPE_CAPTION)
                    .color(palette.text_muted),
            ),
        );
    }
    sections
}

/// The status phrase in a bubble beside the avatar.
fn bubble(status: &str, palette: &Palette) -> El {
    el().mb(SPACE_SM_PT)
        .px(SPACE_MD_PT)
        .py(SPACE_SM_PT)
        .radius(RADIUS_CONTROL_PT)
        .fill(palette.surface_content)
        .border(1.0, palette.line_subtle)
        .min_w(0.0)
        .child(
            text(status.to_owned())
                .font(TYPE_SMALL)
                .color(palette.text_secondary)
                .max_lines(2),
        )
}

/// A titled section: its heading in capitals' place, then `body`.
fn section(heading: &str, body: El, palette: &Palette) -> El {
    col()
        .w_full()
        .px(SPACE_LG_PT)
        .pt(SPACE_MD_PT)
        .gap(SPACE_SM_PT)
        .child(
            text(heading.to_uppercase())
                .font(TYPE_CAPTION_MEDIUM)
                .color(palette.text_muted),
        )
        .child(body)
}

/// A role's chip. Role colours come with their slice.
fn role_chip(role: &str, palette: &Palette) -> El {
    el().px(SPACE_SM_PT)
        .py(SPACE_SM_PT / 2.0)
        .radius(RADIUS_CONTROL_PT)
        .fill(palette.surface_content)
        .border(1.0, palette.line_subtle)
        .child(
            text(role.to_owned())
                .font(TYPE_CAPTION)
                .color(palette.text_secondary),
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
    fn view(&self, name: &str, server: &str, palette: &Palette) -> Vec<El> {
        let mut rows = Vec::new();
        if self.block.is_some() {
            rows.push(self.message_button(palette));
        }
        let copy_id = PersonAction::CopyId;
        let mut items = vec![
            self.item("card-copy-id", strings::card_copy_id(), copy_id, false)
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
                self.item("card-ban", strings::card_ban(), ask, true)
                    .with_icon(&BAN),
            );
        }
        rows.extend(items.iter().map(|item| item_row(item, false, palette)));
        if self.ban == Some(true) {
            rows.push(self.ban_question(name, server, palette));
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
    fn message_button(&self, palette: &Palette) -> El {
        let action = MenuAction::Person {
            key: self.key,
            user: self.user,
            action: PersonAction::Message,
        };
        row()
            .key(MESSAGE_KEY)
            .w_full()
            .h(MENU_ROW_HEIGHT_PT)
            .mb(SPACE_SM_PT)
            .gap(SPACE_SM_PT)
            .center()
            .radius(RADIUS_CONTROL_PT)
            .fill(palette.accent)
            .role("button")
            .label(strings::card_message())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
            .on_press_tx(move |shell: &mut Shell, tx| shell.menu_picked(action.clone(), tx))
            .child(icon(&DIRECT, ROW_ICON_PT).color(palette.accent_on))
            .child(
                text(strings::card_message())
                    .font(TYPE_BODY_STRONG)
                    .color(palette.accent_on)
                    .a11y_hidden(),
            )
    }

    /// "Ban Lia from Cyber Asilo?", unframed, with Cancel and Ban on the
    /// right.
    fn ban_question(&self, name: &str, server: &str, palette: &Palette) -> El {
        let button = |key: &str, label: &str, action: PersonAction, danger: bool| {
            let action = MenuAction::Person {
                key: self.key,
                user: self.user,
                action,
            };
            let (fill, ink) = if danger {
                (palette.danger, palette.accent_on)
            } else {
                (palette.surface_hover, palette.text_primary)
            };
            row()
                .key(key.to_owned())
                .px(SPACE_MD_PT)
                .py(SPACE_SM_PT)
                .radius(RADIUS_CONTROL_PT)
                .fill(fill)
                .role("button")
                .label(label.to_owned())
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
                .on_press_tx(move |shell: &mut Shell, tx| shell.menu_picked(action.clone(), tx))
                .child(
                    text(label.to_owned())
                        .font(TYPE_BODY)
                        .color(ink)
                        .a11y_hidden(),
                )
        };
        col()
            .w_full()
            .px(SPACE_LG_PT)
            .py(SPACE_SM_PT)
            .gap(SPACE_SM_PT)
            .child(
                text(strings::card_ban_ask(name, server))
                    .font(TYPE_BODY)
                    .color(palette.text_primary)
                    .w_full(),
            )
            .child(
                row()
                    .w_full()
                    .gap(SPACE_SM_PT)
                    .justify_end()
                    .child(button(
                        "card-ban-cancel",
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
