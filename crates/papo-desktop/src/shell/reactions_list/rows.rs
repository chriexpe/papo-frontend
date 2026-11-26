//! The reactions list's pieces (`docs/design.md` §10, Reactions list):
//! "← Reactions", the tabs, a person's row and a row waiting for its page.
//! What they show is the list's (`reactions_list`).

use papo_sdk_ui::operations::ReactionEmoji;
use papo_sdk_ui::{ReactionChip, Reactor};
use rune::{El, Key, KeyEvent, el, icon, row, text};

use super::super::Shell;
use crate::design::{
    FOCUS_RING_PT, Look, PILL_AVATAR_PT, PILL_BUTTON_PT, RADIUS_AVATAR, RADIUS_CONTROL,
    RADIUS_PILL, ROW_ICON_PT, SPACE_MD, SPACE_SM, TYPE_BODY, TYPE_CAPTION, TYPE_SMALL_MEDIUM,
    TYPE_TITLE,
};
use crate::reaction_emoji::{ReactionEmojiLook, ShownReaction};
use crate::strings;

/// A row's height, in points.
pub(super) const ROW_PT: f32 = 36.0;
/// The element key of the back button.
const BACK_KEY: &str = "reactions-back";

/// The element key of the tab for `filter` (All when none).
pub(super) fn tab_key(filter: Option<&ReactionEmoji>) -> String {
    match filter {
        // By kind: a server's "Unicode" text could be "all" or a custom emoji's ID.
        Some(ReactionEmoji::Unicode(emoji)) => format!("reactions-tab-u-{emoji}"),
        Some(ReactionEmoji::Custom(id)) => format!("reactions-tab-c-{id}"),
        None => "reactions-tab-all".to_owned(),
    }
}

/// "← Reactions".
pub(super) fn header(look: &Look) -> El {
    let back = row()
        .key(BACK_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        // Concentric with the menu, whose edge is one `space.sm` out.
        .radius(Look::nested_radius(
            look.radius(RADIUS_PILL),
            look.space(SPACE_SM),
        ))
        .role("button")
        .label(strings::reactions_back())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .hover(look.on_pill(look.palette.surface_hover))
        .on_press_tx(|shell: &mut Shell, tx| shell.reactions_back(tx))
        .child(icon(&crate::icons::ARROW_LEFT, ROW_ICON_PT).color(look.palette.text_secondary));
    row()
        .w_full()
        .gap(look.space(SPACE_SM))
        .center_y()
        .child(back)
        .child(
            text(strings::reactions_title())
                .font(look.font(TYPE_TITLE))
                .color(look.palette.text_primary)
                .role("heading"),
        )
}

/// All, then each emoji with its count, in the chips' order; the chosen
/// one in the link colour on `accent.soft`.
pub(super) fn tabs(
    chips: &[ReactionChip],
    chosen: Option<&ReactionEmoji>,
    emoji: &ReactionEmojiLook,
    look: &Look,
) -> El {
    let mut tabs = vec![tab(
        None,
        (None, strings::reactions_all().to_owned()),
        None,
        chosen.is_none(),
        look,
    )];
    for chip in chips {
        // A custom emoji the catalog doesn't have has no chip and no tab.
        let Some(shown) = emoji.shown(&chip.emoji) else {
            continue;
        };
        let selected = chosen == Some(&chip.emoji);
        tabs.push(tab(
            Some(chip.emoji.clone()),
            (
                Some(shown.glyph(crate::reaction_emoji::FAILED_NAME_MAX_PT, look)),
                chip.count.to_string(),
            ),
            Some(strings::reactions_tab(&shown.name, chip.count)),
            selected,
            look,
        ));
    }
    row()
        .w_full()
        .wrap()
        .gap(look.space(SPACE_SM))
        .role("tablist")
        .focus_group()
        .children(tabs)
}

/// A tab: its filter, what it draws (an emoji, then words), how it is spoken when not as the
/// words, and whether it is chosen.
fn tab(
    filter: Option<ReactionEmoji>,
    (glyph, words): (Option<El>, String),
    spoken: Option<String>,
    selected: bool,
    look: &Look,
) -> El {
    let (fill, ink) = if selected {
        (look.palette.accent_soft, look.palette.link)
    } else {
        (look.palette.surface_pill, look.palette.text_secondary)
    };
    let key = tab_key(filter.as_ref());
    row()
        .key(key)
        .h(PILL_BUTTON_PT)
        .px(look.space(SPACE_MD))
        .center_y()
        // Concentric with the menu, whose edge is one `space.sm` out.
        .radius(Look::nested_radius(
            look.radius(RADIUS_PILL),
            look.space(SPACE_SM),
        ))
        .fill(fill)
        .hover(look.on_pill(look.palette.surface_hover))
        .role("tab")
        .label(spoken.unwrap_or_else(|| words.clone()))
        .focusable(true)
        .focus_selected(selected)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(move |shell: &mut Shell| shell.reactions_tab(filter.clone()))
        .gap(look.space(SPACE_SM))
        .children(glyph)
        .child(text(words).font(look.font(TYPE_SMALL_MEDIUM)).color(ink))
}

/// One person: the avatar, the name, "you" by yours, and on All every
/// emoji they used. A press or Enter opens their card.
pub(super) fn reactor_row(
    reactor: &Reactor,
    name: &str,
    all: bool,
    emoji: &ReactionEmojiLook,
    look: &Look,
) -> El {
    let user = reactor.user;
    let mut line = row()
        .w_full()
        .h(ROW_PT)
        .px(look.space(SPACE_MD))
        .gap(look.space(SPACE_MD))
        .center_y()
        // Concentric with the menu, whose edge is one `space.sm` out.
        .radius(Look::nested_radius(
            look.radius(RADIUS_PILL),
            look.space(SPACE_SM),
        ))
        .role("listitem")
        .label(name.to_owned())
        .focusable(true)
        .focus_highlight(look.on_pill(look.palette.surface_hover))
        .on_press_tx(move |shell: &mut Shell, tx| shell.reactor_pressed(user, tx))
        .on_key(move |shell: &mut Shell, event: &KeyEvent, tx| {
            if event.key != Key::Enter {
                return false;
            }
            shell.reactor_pressed(user, tx);
            true
        })
        .child(person_avatar(name, look))
        .child(
            text(name.to_owned())
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_primary)
                .max_lines(1),
        );
    if reactor.you {
        line = line.child(
            text(strings::reactions_you())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        );
    }
    if all {
        let reacted: Vec<ShownReaction> = reactor
            .emoji
            .iter()
            .filter_map(|used| emoji.shown(used))
            .collect();
        // The emoji are drawn hidden from assistive technology: the row says them.
        let names: Vec<&str> = reacted.iter().map(|shown| shown.name.as_str()).collect();
        if !names.is_empty() {
            line = line.label(format!("{name}, {}", names.join(" ")));
        }
        let glyphs = reacted
            .iter()
            .map(|shown| shown.glyph(crate::reaction_emoji::FAILED_NAME_MAX_PT, look));
        line = line
            .child(el().grow(1.0))
            .child(row().gap(look.space(SPACE_SM)).center_y().children(glyphs));
    }
    line
}

/// A person's 24 px avatar, centred on its row (the timeline's sits lower,
/// on its message's first line).
fn person_avatar(name: &str, look: &Look) -> El {
    el().size(PILL_AVATAR_PT, PILL_AVATAR_PT)
        .radius(look.radius(RADIUS_AVATAR))
        .center()
        .fill(look.palette.surface_hover)
        .a11y_hidden()
        .child(
            text(crate::rail::initial(name))
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_secondary),
        )
}

/// A row while its page is on its way.
pub(super) fn skeleton_row(look: &Look) -> El {
    row()
        .w_full()
        .h(ROW_PT)
        .px(look.space(SPACE_MD))
        .gap(look.space(SPACE_MD))
        .center_y()
        .busy(true)
        .label(strings::reactions_loading())
        .child(
            el().size(PILL_AVATAR_PT, PILL_AVATAR_PT)
                .radius(look.radius(RADIUS_CONTROL))
                .fill(look.palette.surface_hover),
        )
        .child(
            el().w(120.0)
                .h(10.0)
                .radius(5.0)
                .fill(look.palette.surface_hover),
        )
}
