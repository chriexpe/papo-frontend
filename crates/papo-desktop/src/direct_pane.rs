//! Draws Direct in the channel pane (`docs/design.md` §10, Direct
//! messages): the pane pill "Direct messages", then a row per direct
//! conversation, newest activity first, across the servers the rail shows.
//! Pure: reads the shell.
//!
//! Owns the pane's look, keyboard and accessibility: the rows are one list
//! the arrows move through, with the picked one as its Tab stop; Enter or a
//! press opens a row and Delete or its ✕ closes it. A row shows the
//! person's avatar with presence, their name, and the server under it, or
//! "typing…" while they type; unread is bold with a dot at its end, and
//! mentions take the dot's place. New message opens the people picker
//! (`new_message`).

use std::borrow::Cow;
use std::sync::Arc;

use papo_sdk_ui::{DirectMessageItem, display_name};
use rune::{El, GrowthSpec, Key, KeyEvent, ScrollSpec, col, el, icon, row, text};

use crate::badges::Badge;
use crate::channel_pane::{chrome, pill_row, row_badge, row_name};
use crate::design::{
    AVATAR_ROW, DM_ROW_HEIGHT_PT, FOCUS_RING_PT, Look, PILL_BUTTON_PT, PILL_ICON_PT,
    RADIUS_CONTROL, SIDEBAR_WIDTH_PT, SPACE_MD, SPACE_SM, Surfaces, TYPE_CAPTION, TYPE_TITLE,
};
use crate::icons::X;
use crate::person_avatar::person_avatar;
use crate::shell::Shell;
use crate::strings;

pub(crate) mod blocked;
pub(crate) mod new_message;

use blocked::{blocked_button, blocked_list};
use new_message::new_message_button;

/// The element key of a direct conversation's row.
pub(crate) fn dm_row_key(row: &DirectMessageItem) -> String {
    format!("dm-{}-{}", row.id.account.user(), row.id.channel)
}

/// Direct: the pane pill, then the conversations, or the people the
/// accounts block.
pub(crate) fn view(shell: &Shell, look: &Look) -> El {
    let blocked = shell.navigation().direct_blocked();
    let title = if blocked {
        strings::direct_blocked_title()
    } else {
        strings::direct_title()
    };
    let pill = chrome(
        pill_row(look)
            .child(
                text(title)
                    .font(look.font(TYPE_TITLE))
                    .color(look.palette.text_primary)
                    .role("heading")
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(new_message_button(shell.shown_dm_picker().is_some(), look))
            .child(blocked_button(blocked, look)),
        look,
    )
    .key(crate::shell::DIRECT_PILL_KEY);
    let list = if blocked {
        blocked_list(shell, look)
    } else {
        conversations(shell, look)
    };
    let servers = shell.navigation().servers();
    let accounts_open = shell.navigation().direct_accounts_open();
    let accounts = direct_account(shell)
        .map(|account| crate::account_switch::direct_pill(account, servers, accounts_open, look));
    // Last, so it draws over the conversations and the pill.
    let grown = accounts_open.then(|| crate::account_switch::direct_grown(servers, look));
    col()
        .key("channel-pane")
        .role("navigation")
        .label(strings::direct_title())
        .w(SIDEBAR_WIDTH_PT)
        .h_full()
        .padding(look.space(SPACE_MD))
        .gap(look.space(SPACE_SM))
        .chrome_fill(&look.palette.sidebar_fill)
        .child(pill)
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                // A scroll here is work outside a header panel; it scrolls
                // as usual.
                .on_wheel(|shell: &mut Shell, _wheel, _tx| {
                    shell.worked_outside();
                    false
                })
                .child(list),
        )
        .children(accounts)
        .children(grown)
}

/// The account Direct's pill shows: the one the picked conversation is
/// from, else the first server's shown one.
fn direct_account(shell: &Shell) -> Option<&crate::server_list::AccountEntry> {
    let servers = &shell.navigation().servers().servers;
    let picked = shell
        .navigation()
        .picked_dm()
        .map(|picked| picked.id.account);
    let shown = || servers.iter().filter_map(|server| server.account.as_ref());
    picked
        .and_then(|picked| shown().find(|account| account.id == picked))
        .or_else(|| shown().next())
}

/// The conversations, newest activity first; a line when there are none.
fn conversations(shell: &Shell, look: &Look) -> El {
    let palette_copy = *look;
    // Only the rows on screen are built: a person can have many
    // conversations.
    let list = col()
        .key("direct-list")
        .w_full()
        .role("listbox")
        .label(strings::direct_list())
        .focus_group()
        .growth(GrowthSpec::default())
        .virtual_rows(
            shell.direct_rows().len(),
            DM_ROW_HEIGHT_PT,
            |shell: &Shell, index| {
                Cow::Owned(
                    shell
                        .direct_rows()
                        .get(index)
                        .map_or_else(|| format!("dm-gone-{index}"), |item| dm_row_key(item)),
                )
            },
            move |shell: &Shell, index| match shell.direct_rows().get(index) {
                Some(item) => {
                    let picked = shell
                        .navigation()
                        .picked_dm()
                        .is_some_and(|picked| picked.id == item.id);
                    Shown::of(shell, item, picked).view(&palette_copy)
                }
                None => el(),
            },
        );
    // With none, a line in the list's place.
    if shell.direct_rows().is_empty() {
        return el().padding(look.space(SPACE_MD)).child(
            text(strings::direct_empty())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        );
    }
    list
}

/// One row, as drawn.
struct Shown<'a> {
    item: &'a Arc<DirectMessageItem>,
    name: String,
    /// The server the conversation is on.
    server: String,
    badge: Badge,
    presence: Option<papo_sdk_ui::PresenceMark>,
    typing: bool,
    picked: bool,
}

impl<'a> Shown<'a> {
    fn of(shell: &Shell, item: &'a Arc<DirectMessageItem>, picked: bool) -> Shown<'a> {
        let account = item.id.account;
        let server = shell.navigation().servers().get(account.server());
        let people = shell.people(account);
        Shown {
            item,
            name: display_name(item.nickname.as_deref(), item.username.as_deref())
                .unwrap_or(strings::someone())
                .to_owned(),
            server: server.map(|server| server.name.clone()).unwrap_or_default(),
            badge: server
                .and_then(|server| server.direct_badges.get(&item.id.channel))
                .copied()
                .unwrap_or_default(),
            presence: people.and_then(|people| people.presence_mark(item.user)),
            typing: people.is_some_and(|people| {
                people
                    .typing_in(item.id.channel)
                    .iter()
                    .any(|typist| typist.user == item.user)
            }),
            picked,
        }
    }

    fn view(&self, look: &Look) -> El {
        let (style, colour) = row_name(self.badge, self.picked, look);
        let sub_line = if self.typing {
            strings::direct_typing()
        } else {
            self.server.as_str()
        };
        let surface = if self.picked {
            look.palette.surface_selected
        } else {
            look.palette.surface_sidebar
        };
        let open = Arc::clone(self.item);
        let close = Arc::clone(self.item);
        let spoken = match self.presence {
            Some(mark) => strings::person_with_presence(&self.name, mark),
            None => self.name.clone(),
        };
        let mut line = row()
            .key(dm_row_key(self.item))
            .w_full()
            .h(DM_ROW_HEIGHT_PT)
            .px(look.space(SPACE_SM))
            .gap(look.space(SPACE_MD))
            .center_y()
            .radius(look.radius(RADIUS_CONTROL))
            .role("option")
            // The server too: two accounts can each talk with the same person.
            .label(
                self.badge
                    .label(&strings::direct_title_label(&spoken, &self.server)),
            )
            .description(strings::direct_close_hint())
            .focusable(true)
            .focus_selected(self.picked)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .hover_group()
            .on_press_tx(move |shell: &mut Shell, tx| shell.pick_dm(Arc::clone(&open), tx))
            .on_key(
                move |shell: &mut Shell, event: &KeyEvent, tx| match event.key {
                    Key::Enter => {
                        shell.pick_dm(Arc::clone(&close), tx);
                        true
                    }
                    Key::Delete => {
                        shell.close_dm(&close, tx);
                        true
                    }
                    _ => false,
                },
            )
            .child(person_avatar(
                &self.name,
                AVATAR_ROW,
                self.presence,
                surface,
                look,
            ))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .a11y_hidden()
                    .child(
                        text(self.name.as_str())
                            .font(style)
                            .color(colour)
                            .max_lines(1),
                    )
                    .child(
                        text(sub_line)
                            .font(look.font(TYPE_CAPTION))
                            .color(look.palette.text_muted)
                            .max_lines(1),
                    ),
            );
        line = line.children(row_badge(self.badge, look));
        line = if self.picked {
            line.fill(look.palette.surface_selected)
        } else {
            line.hover(look.palette.surface_hover)
        };
        line.child(self.close_button(look))
    }

    /// ✕, over the badge while the row is hovered or focused: closes at
    /// once, no question; messaging them again reopens it.
    fn close_button(&self, look: &Look) -> El {
        let item = Arc::clone(self.item);
        row()
            .key(format!("close-{}", dm_row_key(self.item)))
            .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
            .center()
            // Concentric with its row; the corner's gap is the smaller of
            // the inset from the row's end and the space above.
            .radius(Look::nested_radius(
                look.radius(RADIUS_CONTROL),
                look.space(SPACE_SM)
                    .min((DM_ROW_HEIGHT_PT - PILL_BUTTON_PT) / 2.0),
            ))
            .fill(look.palette.surface_hover)
            .role("button")
            .label(strings::direct_close(&self.name))
            .group_opacity(0.0, 1.0)
            .pin(
                None,
                Some((DM_ROW_HEIGHT_PT - PILL_BUTTON_PT) / 2.0),
                Some(look.space(SPACE_SM)),
                None,
            )
            .on_press_tx(move |shell: &mut Shell, tx| shell.close_dm(&item, tx))
            .child(icon(&X, PILL_ICON_PT).color(look.palette.text_secondary))
    }
}
