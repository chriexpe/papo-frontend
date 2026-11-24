//! Draws the conversation's header (`docs/design.md` §3): the title pill on
//! the left and the actions pill on the right (🔍, 📌 and, in a DM, the
//! person button; members come with their slice), floating over the
//! timeline's top.
//!
//! Owns the header's layout and the header buttons' shared shape; the
//! panels they open are `search_panel`'s and `pins_panel`'s.

use rune::{El, icon, row};

use super::Shell;
use super::pins::PINS_BUTTON_KEY;
use super::title::Title;
use crate::design::{
    FOCUS_RING_PT, HEADER_PILL_HEIGHT, Look, PILL_BUTTON_PT, PILL_ICON_PT, RADIUS_PILL, SPACE_LG,
    SPACE_SM, Surfaces,
};
use crate::strings;

/// The header row over the conversation: the title pill and the actions
/// pill. None while no conversation shows.
pub(super) fn header(shell: &Shell, title: &Title<'_>, look: &Look) -> Option<El> {
    shell.tabs.shown()?;
    let title_pill = super::title::pill(title, look);
    // An open panel takes the actions pill's place.
    let panel_open = shell.shown_pins_panel().is_some() || shell.shown_search().is_some();
    let actions = (!panel_open).then(|| actions_pill(false, dm_profile(shell, title), look));
    Some(
        row()
            .key("conversation-header")
            .w_full()
            .px(look.space(SPACE_LG))
            .pt(look.space(SPACE_LG))
            .center_y()
            .justify_between()
            .child(title_pill)
            .children(actions),
    )
}

/// In a DM, whether its profile pane shows: the person button is there.
fn dm_profile(shell: &Shell, title: &Title<'_>) -> Option<bool> {
    matches!(title, Title::Direct { .. }).then_some(shell.layout.dm_profile())
}

/// The actions pill: 🔍, 📌 (in the accent while its panel is open) and,
/// in a DM, the person button.
fn actions_pill(open: bool, profile: Option<bool>, look: &Look) -> El {
    row()
        .key("actions-pill")
        .h(look.space(HEADER_PILL_HEIGHT))
        .px(look.space(SPACE_SM))
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .role("toolbar")
        .label(strings::header_actions())
        .child(super::search_panel::search_button(false, look))
        .child(pins_button(open, look))
        .children(profile.map(|shown| super::profile_pane::button(shown, look)))
}

/// The corner radius of a button on the actions pill or a panel's head
/// (🔍, 📌, the person button, the search's ✕): concentric with the pill
/// or panel. The corner's gap is the smaller of the side padding and the
/// space above the button.
pub(super) fn header_button_radius(look: &Look) -> f32 {
    let above_pt = (look.space(HEADER_PILL_HEIGHT) - PILL_BUTTON_PT) / 2.0;
    Look::nested_radius(look.radius(RADIUS_PILL), look.space(SPACE_SM).min(above_pt))
}

pub(super) fn pins_button(open: bool, look: &Look) -> El {
    let tone = if open {
        look.palette.accent
    } else {
        look.palette.text_secondary
    };
    row()
        .key(PINS_BUTTON_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(header_button_radius(look))
        .role("button")
        .label(strings::pins_title())
        .help(strings::pins_title())
        .help_keys([strings::shortcut_pins()])
        .expanded(open)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .hover(look.on_pill(look.palette.surface_hover))
        .on_press_tx(|shell: &mut Shell, tx| shell.pins_pressed(false, tx))
        .child(icon(&crate::icons::PIN, PILL_ICON_PT).color(tone))
}
