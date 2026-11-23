//! The reactions list's pieces (`docs/design.md` §10, Reactions list):
//! "← Reactions", the tabs, a person's row and a row waiting for its page.
//! What they show is the list's (`reactions_list`).

use papo_sdk_ui::operations::ReactionEmoji;
use papo_sdk_ui::{ReactionChip, Reactor};
use rune::{El, el, icon, row, text};

use super::super::Shell;
use crate::design::{
    FOCUS_RING_PT, PILL_AVATAR_PT, PILL_BUTTON_PT, Palette, RADIUS_AVATAR_PT, RADIUS_CONTROL_PT,
    ROW_ICON_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_CAPTION, TYPE_SMALL_MEDIUM, TYPE_TITLE,
};
use crate::strings;

/// A row's height, in points.
pub(super) const ROW_PT: f32 = 36.0;
/// The element key of the back button.
const BACK_KEY: &str = "reactions-back";

/// The element key of the tab for `filter` (All when none).
pub(super) fn tab_key(filter: Option<&ReactionEmoji>) -> String {
    match filter {
        Some(ReactionEmoji::Unicode(emoji)) => format!("reactions-tab-{emoji}"),
        _ => "reactions-tab-all".to_owned(),
    }
}

/// "← Reactions".
pub(super) fn header(palette: &Palette) -> El {
    let back = row()
        .key(BACK_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(strings::reactions_back())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .hover(palette.surface_hover)
        .on_press_tx(|shell: &mut Shell, tx| shell.reactions_back(tx))
        .child(icon(&crate::icons::ARROW_LEFT, ROW_ICON_PT).color(palette.text_secondary));
    row()
        .w_full()
        .gap(SPACE_SM_PT)
        .center_y()
        .child(back)
        .child(
            text(strings::reactions_title())
                .font(TYPE_TITLE)
                .color(palette.text_primary)
                .role("heading"),
        )
}

/// All, then each emoji with its count, in the chips' order; the chosen
/// one in the link colour on `accent.soft`.
pub(super) fn tabs(
    chips: &[ReactionChip],
    chosen: Option<&ReactionEmoji>,
    palette: &Palette,
) -> El {
    let mut tabs = vec![tab(
        None,
        strings::reactions_all().to_owned(),
        None,
        chosen.is_none(),
        palette,
    )];
    for chip in chips {
        let ReactionEmoji::Unicode(emoji) = &chip.emoji else {
            continue;
        };
        let selected = chosen == Some(&chip.emoji);
        tabs.push(tab(
            Some(chip.emoji.clone()),
            format!("{emoji} {}", chip.count),
            Some(strings::reactions_tab(emoji, chip.count)),
            selected,
            palette,
        ));
    }
    row()
        .w_full()
        .wrap()
        .gap(SPACE_SM_PT)
        .role("tablist")
        .focus_group()
        .children(tabs)
}

fn tab(
    filter: Option<ReactionEmoji>,
    words: String,
    spoken: Option<String>,
    selected: bool,
    palette: &Palette,
) -> El {
    let (fill, ink) = if selected {
        (palette.accent_soft, palette.link)
    } else {
        (palette.surface_pill, palette.text_secondary)
    };
    let key = tab_key(filter.as_ref());
    row()
        .key(key)
        .h(PILL_BUTTON_PT)
        .px(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .fill(fill)
        .hover(palette.surface_hover)
        .role("tab")
        .label(spoken.unwrap_or_else(|| words.clone()))
        .focusable(true)
        .focus_selected(selected)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .on_press(move |shell: &mut Shell| shell.reactions_tab(filter.clone()))
        .child(text(words).font(TYPE_SMALL_MEDIUM).color(ink))
}

/// One person: the avatar, the name, "you" by yours, and on All every
/// emoji they used.
pub(super) fn reactor_row(reactor: &Reactor, name: &str, all: bool, palette: &Palette) -> El {
    let mut line = row()
        .w_full()
        .h(ROW_PT)
        .px(SPACE_MD_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .role("listitem")
        .child(person_avatar(name, palette))
        .child(
            text(name.to_owned())
                .font(TYPE_BODY)
                .color(palette.text_primary)
                .max_lines(1),
        );
    if reactor.you {
        line = line.child(
            text(strings::reactions_you())
                .font(TYPE_CAPTION)
                .color(palette.text_muted),
        );
    }
    if all {
        let emoji: String = reactor
            .emoji
            .iter()
            .filter_map(|emoji| match emoji {
                ReactionEmoji::Unicode(emoji) => Some(emoji.as_str()),
                ReactionEmoji::Custom(_) => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        line = line
            .child(el().grow(1.0))
            .child(text(emoji).font(TYPE_BODY));
    }
    line
}

/// A person's 24 px avatar, centred on its row (the timeline's sits lower,
/// on its message's first line).
fn person_avatar(name: &str, palette: &Palette) -> El {
    el().size(PILL_AVATAR_PT, PILL_AVATAR_PT)
        .radius(RADIUS_AVATAR_PT)
        .center()
        .fill(palette.surface_hover)
        .a11y_hidden()
        .child(
            text(crate::rail::initial(name))
                .font(TYPE_CAPTION)
                .color(palette.text_secondary),
        )
}

/// A row while its page is on its way.
pub(super) fn skeleton_row(palette: &Palette) -> El {
    row()
        .w_full()
        .h(ROW_PT)
        .px(SPACE_MD_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .busy(true)
        .label(strings::reactions_loading())
        .child(
            el().size(PILL_AVATAR_PT, PILL_AVATAR_PT)
                .radius(RADIUS_CONTROL_PT)
                .fill(palette.surface_hover),
        )
        .child(
            el().w(120.0)
                .h(10.0)
                .radius(5.0)
                .fill(palette.surface_hover),
        )
}
