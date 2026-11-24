//! The actions on someone else's profile card (`docs/design.md` §10):
//! Message, Copy ID, Block or Unblock, and Ban with its unframed question,
//! as the account may use them. Your own card has none but Copy ID.
//!
//! Owns how they are drawn and what they ask for (a `MenuAction`); the
//! card's handlers carry them out.

use papo_sdk::Uuid;
use rune::{El, col, icon, row, text};

use super::super::Shell;
use crate::design::{
    FOCUS_RING_PT, Look, MENU_ROW_HEIGHT_PT, RADIUS_PILL, ROW_ICON_PT, SPACE_LG, SPACE_MD,
    SPACE_SM, TYPE_BODY, TYPE_BODY_STRONG,
};
use crate::icons::{BAN, BLOCK, DIRECT, ID};
use crate::menu::{MenuAction, MenuItem, PersonAction, item_row};
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

/// The card's actions, as its person and the account allow. Your own card
/// has no Message, Block or Ban.
pub(super) struct Actions {
    pub(super) key: TimelineKey,
    pub(super) user: Uuid,
    /// Whether this account blocks them; none on your own card.
    pub(super) block: Option<bool>,
    /// Whether Ban is asking; none unless the account may ban them.
    pub(super) ban: Option<bool>,
}

impl Actions {
    pub(super) fn view(&self, name: &str, server: &str, look: &Look) -> Vec<El> {
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
            // As the card's rows under it, concentric with the card.
            .radius(Look::nested_radius(
                look.radius(RADIUS_PILL),
                look.space(SPACE_SM),
            ))
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
                // Concentric with the card: below, the question's padding
                // and the card's, one `space.sm` each, is the smaller gap.
                .radius(Look::nested_radius(
                    look.radius(RADIUS_PILL),
                    look.space(SPACE_SM) * 2.0,
                ))
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
