//! The top of each step: the server step's title, and on the account steps
//! the server's icon, name and address, centred, with the notice under them
//! (`docs/design.md` §7, Layout). No card or badge around the server.
//!
//! Owns how the server is shown; which step shows is the view's.

use rune::{El, LiveRegion, col, el, text};

use crate::design::{
    Palette, RADIUS_SERVER_ICON_PT, SERVER_ICON_PT, SPACE_SM_PT, TYPE_CAPTION, TYPE_DISPLAY,
    TYPE_SMALL,
};
use crate::strings;

use super::{AccountForm, Form, Server};

/// The server's icon, name and address, centred, with the notice under them.
pub(super) fn server_header(server: &Server, form: Form, palette: &Palette) -> El {
    let icon = match &server.icon {
        Some(image) => el()
            .size(SERVER_ICON_PT, SERVER_ICON_PT)
            .radius(RADIUS_SERVER_ICON_PT)
            .pixels(image.pixels())
            .a11y_hidden(),
        None => initial(&server.name, palette),
    };
    let mut header = col()
        .key("server")
        .w_full()
        .items_center()
        .gap(SPACE_SM_PT)
        .child(icon)
        .child(title(&server.name, palette))
        .child(
            text(server.address.as_str())
                .font(TYPE_CAPTION)
                .color(palette.text_muted),
        );
    let notice = match form {
        Form::Unlock { lapsed: true, .. } => Some(strings::notice_access_expired()),
        Form::Account(AccountForm::SignIn) if server.renewing.is_some() => {
            Some(strings::notice_session_expired())
        }
        Form::Unlock { .. } | Form::Account(_) => None,
    };
    if let Some(notice) = notice {
        header = header.child(
            text(notice)
                .key("notice")
                .font(TYPE_SMALL)
                .color(palette.warning)
                .role("status")
                .live_region(LiveRegion::Polite),
        );
    }
    header
}

/// A server without an icon: the first letter of its name.
fn initial(name: &str, palette: &Palette) -> El {
    let letter: String = name
        .chars()
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    el().size(SERVER_ICON_PT, SERVER_ICON_PT)
        .center()
        .radius(RADIUS_SERVER_ICON_PT)
        .fill(palette.surface_code)
        .a11y_hidden()
        .child(
            text(letter)
                .font(TYPE_DISPLAY)
                .color(palette.text_secondary),
        )
}

pub(super) fn title(words: &str, palette: &Palette) -> El {
    text(words)
        .key("title")
        .font(TYPE_DISPLAY)
        .color(palette.text_primary)
        .role("heading")
        .level(2)
}
