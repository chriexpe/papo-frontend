//! Removing a server: the confirmation popup the server menus open, and the
//! SDK call it makes once confirmed.
//!
//! Owns the question and its two buttons. The SDK removes the server and
//! signs its accounts out on this device; the rail follows from the server
//! list it posts.

use papo_sdk::ServerId;
use rune::{El, col, el, row, text};

use crate::design::{
    CONFIRM_WIDTH_PT, FIELD_HEIGHT_PT, FIELD_OUTLINE_PT, FOCUS_RING_PT, PILL_OUTLINE_PT,
    POPUP_PADDING_PT, POPUP_SHADOW_PT, Palette, RADIUS_FIELD_PT, RADIUS_POPUP_PT, SPACE_LG_PT,
    SPACE_MD_PT, SPACE_XXL_PT, TYPE_BODY, TYPE_BODY_STRONG, TYPE_DISPLAY,
};
use crate::shell::Shell;
use crate::strings;

/// The element key of the confirmation popup.
pub(crate) const CONFIRM_KEY: &str = "remove-server";

/// "Remove {server}?" over the dimmed app. Escape, Return and Cancel keep
/// the server; only Remove removes it.
pub(crate) fn confirm(server_name: &str, palette: &Palette) -> El {
    let (drop, blur) = POPUP_SHADOW_PT;
    let cancel = button("remove-cancel", strings::remove_cancel(), palette)
        .border(FIELD_OUTLINE_PT, palette.line_strong)
        .on_press(Shell::cancel_remove_server)
        // Return keeps the server: removing it is never the default.
        .default_button()
        .child(
            text(strings::remove_cancel())
                .font(TYPE_BODY_STRONG)
                .color(palette.text_primary),
        );
    let remove = button("remove-confirm", strings::remove_confirm(), palette)
        .fill(palette.danger)
        .on_press_tx(Shell::confirm_remove_server)
        .child(
            text(strings::remove_confirm())
                .font(TYPE_BODY_STRONG)
                .color(palette.danger_on),
        );
    let frame = col()
        .key(CONFIRM_KEY)
        .w(CONFIRM_WIDTH_PT)
        .padding(POPUP_PADDING_PT)
        .gap(SPACE_LG_PT)
        .radius(RADIUS_POPUP_PT)
        .fill(palette.surface_content)
        .border(PILL_OUTLINE_PT, palette.line_subtle)
        .shadow(drop, blur, palette.popup_shadow)
        .role("alertdialog")
        .label(strings::remove_title(server_name))
        .modal(false, Shell::cancel_remove_server)
        .child(
            text(strings::remove_title(server_name))
                .font(TYPE_DISPLAY)
                .color(palette.text_primary)
                .role("heading")
                .level(2)
                .wrap(),
        )
        .child(
            text(strings::remove_body())
                .font(TYPE_BODY)
                .color(palette.text_secondary)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .justify_end()
                .gap(SPACE_MD_PT)
                .child(cancel)
                .child(remove),
        );
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .padding(SPACE_XXL_PT)
        .fill(palette.scrim)
        .child(frame)
}

fn button(key: &'static str, label: &'static str, palette: &Palette) -> El {
    row()
        .key(key)
        .h(FIELD_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .center()
        .radius(RADIUS_FIELD_PT)
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
}

/// Asks the SDK to remove `server`. Holds the registry writes, so a probe
/// sent after it finds the server gone.
pub(crate) fn remove(shell: &Shell, server: ServerId) {
    let Some(sdk) = shell.sdk() else {
        return;
    };
    let writes = shell.registry_writes();
    let held = std::sync::Arc::clone(&writes).try_lock_owned();
    sdk.run(move |client| async move {
        let _writes = match held {
            Ok(held) => held,
            Err(_) => writes.lock_owned().await,
        };
        if let Err(error) = client.remove_server(server).await {
            log::warn!("the server was not removed: {error}");
        }
    });
}
