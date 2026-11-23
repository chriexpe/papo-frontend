//! The top of each step: the server step's title, and on the account steps
//! the server's icon, name and address, centred, with the notice under them
//! (`docs/design.md` §7, Layout). No card or badge around the server.
//!
//! Owns how the server is shown; which step shows is the view's.

use rune::{El, LiveRegion, col, el, text};

use crate::design::{
    Look, RADIUS_SERVER_ICON, SERVER_ICON_PT, SPACE_SM, TYPE_CAPTION, TYPE_DISPLAY, TYPE_SMALL,
};
use crate::strings;

use super::{AccountForm, Form, Server};

/// The server's icon, name and address, centred, with the notice under them.
pub(super) fn server_header(server: &Server, form: Form, look: &Look) -> El {
    let icon = match &server.icon {
        Some(image) => el()
            .size(SERVER_ICON_PT, SERVER_ICON_PT)
            .radius(look.radius(RADIUS_SERVER_ICON))
            .pixels(image.pixels())
            .a11y_hidden(),
        None => initial(&server.name, look),
    };
    let mut header = col()
        .key("server")
        .w_full()
        .items_center()
        .gap(look.space(SPACE_SM))
        .child(icon)
        .child(title(&server.name, look))
        .child(
            text(server.address.as_str())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
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
                .font(look.font(TYPE_SMALL))
                .color(look.palette.warning)
                .role("status")
                .live_region(LiveRegion::Polite),
        );
    }
    header
}

/// A server without an icon: the first letter of its name.
fn initial(name: &str, look: &Look) -> El {
    let letter: String = name
        .chars()
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    el().size(SERVER_ICON_PT, SERVER_ICON_PT)
        .center()
        .radius(look.radius(RADIUS_SERVER_ICON))
        .fill(look.palette.surface_code)
        .a11y_hidden()
        .child(
            text(letter)
                .font(look.font(TYPE_DISPLAY))
                .color(look.palette.text_secondary),
        )
}

pub(super) fn title(words: &str, look: &Look) -> El {
    text(words)
        .key("title")
        .font(look.font(TYPE_DISPLAY))
        .color(look.palette.text_primary)
        .role("heading")
        .level(2)
}
