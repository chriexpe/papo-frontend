//! The provider viewer drawn (`docs/design.md` §15.5): only a provider's page uses this shell.
//! Black at 92 % over the whole window, the same player 16 : 9 and centred with 64 pt clear at
//! the top and 24 at the sides, and §14.8's two pills, dark in every theme: on the left the
//! provider's letter tile, the title at 600, then the provider, the sender and the time; on the
//! right Open in browser, Copy link and ✕. There is no zoom, Save or Jump: a page can't offer
//! them. Esc closes; while the player has focus its own keys work. Spoken as a dialog named for
//! the player. Pure: the state is the shell's.

use rune::{Color, Cursor, El, NativeSurface, col, el, row, text};

use super::view::button;
use super::{CLEAR_SIDE_PT, CLEAR_TOP_PT};
use crate::design::{
    Look, RADIUS_PILL, SPACE_LG, SPACE_MD, SPACE_SM, Surfaces, TYPE_BODY_STRONG, TYPE_CAPTION,
    VIEWER_BACKDROP_ALPHA, VIEWER_BUTTON_PT,
};
use crate::icons::{EXTERNAL, LINK, X};
use crate::link_cards::player::page;
use crate::message_media::received::MediaAt;
use crate::provider_players::PlayerKey;
use crate::shell::Shell;
use crate::strings;

/// The provider viewer's element key.
pub(crate) const PROVIDER_VIEWER_KEY: &str = "provider-viewer";

/// What the provider viewer draws.
pub(crate) struct ProviderShown<'a> {
    pub(crate) at: MediaAt,
    pub(crate) key: PlayerKey,
    pub(crate) surface: &'a NativeSurface,
    pub(crate) cursor: Cursor,
    /// The player's box, in points.
    pub(crate) size: [f32; 2],
    /// The provider's name, the allowlist's own.
    pub(crate) provider: &'static str,
    pub(crate) title: Option<&'a str>,
    pub(crate) sender: &'a str,
    pub(crate) when: &'a str,
}

/// The provider viewer over the window; `look` is already the always-dark one.
pub(crate) fn view(shown: &ProviderShown<'_>, look: &Look) -> El {
    let title = shown.title.unwrap_or(shown.provider);
    let label = strings::embed_viewer_label(shown.provider, title);
    let [width, height] = shown.size;
    let player = page::<Shell>(
        shown.at,
        shown.key,
        shown.surface,
        shown.cursor,
        strings::embed_player_label(shown.provider, shown.title),
        look,
    )
    .size(width, height)
    .fill(Color::BLACK)
    .on_wheel(|shell: &mut Shell, wheel, tx| shell.player_viewer_wheel(wheel, tx));
    el().key(PROVIDER_VIEWER_KEY)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .role("dialog")
        .label(label)
        .modal_trap()
        .on_key(|shell: &mut Shell, key, tx| shell.player_viewer_key(key, tx))
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(Color::BLACK.alpha(VIEWER_BACKDROP_ALPHA))
                .a11y_hidden(),
        )
        .child(
            col()
                .pin(
                    Some(CLEAR_SIDE_PT),
                    Some(CLEAR_TOP_PT),
                    Some(CLEAR_SIDE_PT),
                    Some(CLEAR_SIDE_PT),
                )
                .center()
                .child(player),
        )
        .child(top(shown, title, look))
}

/// The two pills at the top.
fn top(shown: &ProviderShown<'_>, title: &str, look: &Look) -> El {
    let details = [shown.provider, shown.sender, shown.when]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let about = row()
        .h(VIEWER_BUTTON_PT + look.space(SPACE_SM))
        .pl(look.space(SPACE_MD))
        .pr(look.space(SPACE_LG))
        .gap(look.space(SPACE_MD))
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .min_w(0.0)
        .child(crate::link_cards::provider_marks::letter_tile(
            shown.provider,
            look.palette.line_strong,
        ))
        .child(
            text(title.to_owned())
                .font(look.font(TYPE_BODY_STRONG))
                .color(look.palette.text_primary)
                .truncate(),
        )
        .child(
            text(details)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_secondary)
                .truncate(),
        );
    let actions = row()
        .h(VIEWER_BUTTON_PT + look.space(SPACE_SM))
        .px(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM) / 2.0)
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .child(button(
            "provider-viewer-browser",
            &EXTERNAL,
            strings::embed_open_browser(),
            look,
            Shell::player_viewer_open_page,
        ))
        .child(button(
            "provider-viewer-copy",
            &LINK,
            strings::link_menu_copy(),
            look,
            Shell::player_viewer_copy_link,
        ))
        .child(button(
            "provider-viewer-close",
            &X,
            strings::viewer_close(),
            look,
            Shell::close_player_viewer,
        ));
    row()
        .pin(
            Some(CLEAR_SIDE_PT),
            Some(look.space(SPACE_LG)),
            Some(CLEAR_SIDE_PT),
            None,
        )
        .items_center()
        .gap(look.space(SPACE_MD))
        .child(about.shrink())
        .child(el().grow(1.0))
        .child(actions)
}
