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
//! mentions take the dot's place. The New message picker waits for user
//! search in the backend.

use std::borrow::Cow;
use std::sync::Arc;

use papo_sdk_ui::{DirectMessageItem, display_name};
use rune::{El, GrowthSpec, Key, KeyEvent, ScrollSpec, col, el, icon, row, text};

use crate::badges::Badge;
use crate::channel_pane::{chrome, pill_row, row_badge, row_name};
use crate::design::{
    AVATAR_ROW, DM_ROW_HEIGHT_PT, FOCUS_RING_PT, PILL_BUTTON_PT, PILL_ICON_PT, Palette,
    RADIUS_CONTROL_PT, SIDEBAR_WIDTH_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_CAPTION, TYPE_TITLE,
};
use crate::icons::X;
use crate::person_avatar::person_avatar;
use crate::shell::Shell;
use crate::strings;

/// The element key of a direct conversation's row.
pub(crate) fn dm_row_key(row: &DirectMessageItem) -> String {
    format!("dm-{}-{}", row.id.account.user(), row.id.channel)
}

/// Direct: the pane pill, then the conversations.
pub(crate) fn view(shell: &Shell, palette: &Palette) -> El {
    let pill = chrome(
        pill_row().child(
            text(strings::direct_title())
                .font(TYPE_TITLE)
                .color(palette.text_primary)
                .role("heading"),
        ),
        palette,
    );
    let palette_copy = *palette;
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
    let list = if shell.direct_rows().is_empty() {
        el().padding(SPACE_MD_PT).child(
            text(strings::direct_empty())
                .font(TYPE_CAPTION)
                .color(palette.text_muted),
        )
    } else {
        list
    };
    col()
        .key("channel-pane")
        .w(SIDEBAR_WIDTH_PT)
        .h_full()
        .padding(SPACE_MD_PT)
        .gap(SPACE_SM_PT)
        .fill(palette.surface_sidebar)
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

    fn view(&self, palette: &Palette) -> El {
        let (style, colour) = row_name(self.badge, self.picked, palette);
        let sub_line = if self.typing {
            strings::direct_typing()
        } else {
            self.server.as_str()
        };
        let surface = if self.picked {
            palette.surface_selected
        } else {
            palette.surface_sidebar
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
            .px(SPACE_SM_PT)
            .gap(SPACE_MD_PT)
            .center_y()
            .radius(RADIUS_CONTROL_PT)
            .role("option")
            // The server too: two accounts can each talk with the same person.
            .label(
                self.badge
                    .label(&strings::direct_title_label(&spoken, &self.server)),
            )
            .description(strings::direct_close_hint())
            .focusable(true)
            .focus_selected(self.picked)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
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
                palette,
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
                            .font(TYPE_CAPTION)
                            .color(palette.text_muted)
                            .max_lines(1),
                    ),
            );
        line = line.children(row_badge(self.badge, palette));
        line = if self.picked {
            line.fill(palette.surface_selected)
        } else {
            line.hover(palette.surface_hover)
        };
        line.child(self.close_button(palette))
    }

    /// ✕, over the badge while the row is hovered or focused: closes at
    /// once, no question; messaging them again reopens it.
    fn close_button(&self, palette: &Palette) -> El {
        let item = Arc::clone(self.item);
        row()
            .key(format!("close-{}", dm_row_key(self.item)))
            .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
            .center()
            .radius(RADIUS_CONTROL_PT)
            .fill(palette.surface_hover)
            .role("button")
            .label(strings::direct_close(&self.name))
            .group_opacity(0.0, 1.0)
            .pin(
                None,
                Some((DM_ROW_HEIGHT_PT - PILL_BUTTON_PT) / 2.0),
                Some(SPACE_SM_PT),
                None,
            )
            .on_press_tx(move |shell: &mut Shell, tx| shell.close_dm(&item, tx))
            .child(icon(&X, PILL_ICON_PT).color(palette.text_secondary))
    }
}
