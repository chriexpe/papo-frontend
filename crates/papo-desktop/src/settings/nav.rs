//! The settings panel's nav (`docs/design.md` §8, The panel): the search
//! field, a tile for the subject (the account, opening Profile), the
//! account's pages under "Account on {server}", then the App pages under
//! "App"; each page an icon in a circle, its name and a one-line summary.
//!
//! Owns the nav's look; the pages' summaries are `rows`'.

use rune::{El, col, el, icon, row, text, text_input};

use super::rows::account::AccountFacts;
use super::rows::{self, Context, page_title};
use super::{PageId, SEARCH_KEY, SettingsPanel};
use crate::design::settings::{
    HEAD_BUTTON_PT, NAV_ICON_CIRCLE_PT, NAV_PADDING, NAV_ROW_INSET_PT, NAV_ROW_PADDING,
    NAV_WIDTH_PT, PANEL_RADIUS, SEARCH_HEIGHT,
};
use crate::design::{
    AVATAR_ROW, FIELD_OUTLINE_PT, FOCUS_RING_PT, Look, PANE_EDGE_PT, PILL_ICON_PT, ROW_ICON_PT,
    SPACE_MD, SPACE_SM, SPACE_XS, TYPE_BODY, TYPE_BODY_MEDIUM, TYPE_CAPTION_MEDIUM, TYPE_SMALL,
};
use crate::icons::{CHAT, GLOBE, PALETTE, PULSE, SEARCH, SHIELD, X};
use crate::person_avatar::person_avatar;
use crate::shell::Shell;
use crate::strings;

/// The nav: the search field, the subject tile and the account's pages
/// when an account is signed in, then the App pages.
pub(super) fn view(panel: &SettingsPanel, context: &Context<'_>, look: &Look) -> El {
    let current = panel.query().is_empty().then(|| panel.page().nav());
    let pages = PageId::NAV.into_iter().map(|page| {
        let summary = match page {
            PageId::Appearance => rows::appearance_summary(context.appearance, context.system_dark),
            PageId::General => rows::general_summary(context),
            PageId::Diagnostics => strings::settings_diagnostics_summary().to_owned(),
            _ => rows::conversations_summary().to_owned(),
        };
        nav_row(page, &summary, current == Some(page), look)
    });
    let mut nav = col()
        .w(NAV_WIDTH_PT)
        .h_full()
        .padding(look.space(NAV_PADDING))
        .gap(look.space(SPACE_XS))
        .border_sides([0.0, PANE_EDGE_PT, 0.0, 0.0], look.palette.line_subtle)
        .role("navigation")
        .label(strings::settings_title())
        .child(search_field(panel, look));
    if let Some(account) = context.account {
        nav = nav
            .child(subject_tile(
                account,
                current == Some(PageId::Profile),
                look,
            ))
            .child(group_label(
                &strings::settings_group_account(&account.server),
                look,
            ))
            .child(nav_row(
                PageId::Privacy,
                &rows::account::privacy_summary(account),
                current == Some(PageId::Privacy),
                look,
            ));
    }
    nav.child(group_label(strings::settings_group_app(), look))
        .children(pages)
}

/// A group's name over its pages, in capitals.
fn group_label(name: &str, look: &Look) -> El {
    el().px(NAV_ROW_INSET_PT)
        .pt(look.space(SPACE_MD))
        .pb(look.space(SPACE_SM))
        .child(
            text(name.to_uppercase())
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.text_muted),
        )
}

/// The subject's tile: the account's avatar with presence, its name and
/// Profile's line; it opens Profile.
fn subject_tile(account: &AccountFacts, current: bool, look: &Look) -> El {
    let name = papo_sdk_ui::display_name(account.own.nickname.as_deref(), Some(&account.handle))
        .unwrap_or(&account.handle)
        .to_owned();
    let summary = rows::account::profile_summary();
    nav_entry(
        PageId::Profile,
        &name,
        summary,
        current,
        person_avatar(
            &name,
            AVATAR_ROW,
            account.presence,
            look.palette.surface_tile,
            look,
        ),
        look,
    )
}

/// A nav page's icon.
fn nav_icon(page: PageId) -> &'static rune::Icon {
    match page {
        PageId::Conversations | PageId::Highlights => &CHAT,
        PageId::General | PageId::Trusted => &GLOBE,
        PageId::Diagnostics => &PULSE,
        PageId::Appearance | PageId::Background | PageId::Shape => &PALETTE,
        PageId::Privacy | PageId::Blocked | PageId::Password | PageId::Profile => &SHIELD,
    }
}

/// The search field: the search icon, the field, and ✕ while it holds text.
fn search_field(panel: &SettingsPanel, look: &Look) -> El {
    // Concentric with the panel, whose edge is the nav's padding out.
    let field_radius = Look::nested_radius(look.radius(PANEL_RADIUS), look.space(NAV_PADDING));
    let clear_side = HEAD_BUTTON_PT - SPACE_XS.0 * 2.0;
    let mut field = row()
        .w_full()
        .h(look.space(SEARCH_HEIGHT))
        .px(look.space(SPACE_MD) + SPACE_XS.0)
        .gap(look.space(SPACE_SM))
        .center_y()
        .radius(field_radius)
        .fill(look.palette.surface_tile)
        .border(FIELD_OUTLINE_PT, look.palette.line_pill)
        .role("search")
        .child(icon(&SEARCH, ROW_ICON_PT).color(look.palette.accent))
        .child(
            text_input(&panel.search, look.font(TYPE_BODY))
                .key(SEARCH_KEY)
                .grow(1.0)
                .min_w(0.0)
                .color(look.palette.text_primary)
                .placeholder(strings::settings_search(), look.palette.text_muted)
                .caret(look.palette.accent, look.palette.text_selection)
                .label(strings::settings_search())
                .autofocus()
                .on_edit(Shell::settings_search_mut)
                .on_input_key(|shell: &mut Shell, key: &rune::KeyEvent, tx| {
                    shell.settings_key(key, tx)
                }),
        );
    if !panel.query().is_empty() {
        field = field.child(
            el().key("settings-search-clear")
                .size(clear_side, clear_side)
                .center()
                // Concentric with the field, centred in its height.
                .radius(Look::nested_radius(
                    field_radius,
                    (look.space(SEARCH_HEIGHT) - clear_side) / 2.0,
                ))
                .role("button")
                .label(strings::settings_search_clear())
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press_tx(|shell: &mut Shell, tx| shell.settings_clear_search(tx))
                .child(icon(&X, ROW_ICON_PT).color(look.palette.text_muted)),
        );
    }
    el().w_full().pb(look.space(SPACE_MD)).child(field)
}

/// A page in the nav: its icon in a circle, its name and one-line summary.
fn nav_row(page: PageId, summary: &str, current: bool, look: &Look) -> El {
    let circle = el()
        .size(NAV_ICON_CIRCLE_PT, NAV_ICON_CIRCLE_PT)
        .radius(NAV_ICON_CIRCLE_PT / 2.0)
        .center()
        .fill(look.palette.surface_tile)
        .child(icon(nav_icon(page), PILL_ICON_PT).color(look.palette.text_secondary));
    nav_entry(page, page_title(page), summary, current, circle, look)
}

/// One entry of the nav, led by `lead`: it shows `page`.
fn nav_entry(page: PageId, name: &str, summary: &str, current: bool, lead: El, look: &Look) -> El {
    let fill = if current {
        look.on_panel(look.palette.surface_selected)
    } else {
        rune::Color::TRANSPARENT
    };
    let weight = if current {
        crate::design::TYPE_BODY_STRONG
    } else {
        TYPE_BODY_MEDIUM
    };
    row()
        .key(format!("settings-nav-{page:?}"))
        .w_full()
        .py(look.space(NAV_ROW_PADDING))
        .px(NAV_ROW_INSET_PT)
        .gap(look.space(SPACE_MD) + SPACE_SM.0)
        .center_y()
        // As the search field above it: concentric with the panel, whose
        // edge is the nav's padding out.
        .radius(Look::nested_radius(
            look.radius(PANEL_RADIUS),
            look.space(NAV_PADDING),
        ))
        .fill(fill)
        .role("link")
        .label(format!("{name}, {summary}"))
        .focusable(true)
        .focus_highlight(look.on_panel(look.palette.surface_hover))
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |shell: &mut Shell, tx| shell.settings_page(page, tx))
        .child(lead.a11y_hidden())
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    text(name.to_owned())
                        .font(look.font(weight))
                        .color(look.palette.text_primary)
                        .max_lines(1),
                )
                .child(
                    text(summary.to_owned())
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.text_muted)
                        .max_lines(1),
                ),
        )
}
