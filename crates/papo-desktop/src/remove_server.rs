//! Removing a server: the confirmation popup the server menus open, and the
//! SDK call it makes once confirmed.
//!
//! Owns the question (`confirm` draws it). The SDK removes the server and
//! signs its accounts out on this device; the rail follows from the server
//! list it posts.

use papo_sdk::ServerId;
use rune::El;

use crate::confirm::{Question, confirm as ask};
use crate::design::Look;
use crate::shell::Shell;
use crate::strings;

/// The element key of the confirmation popup.
pub(crate) const CONFIRM_KEY: &str = "remove-server";

/// "Remove {server}?" over the dimmed app. Escape, Return and Cancel keep
/// the server; only Remove removes it.
pub(crate) fn confirm(server_name: &str, look: &Look) -> El {
    let title = strings::remove_title(server_name);
    let question = Question {
        key: CONFIRM_KEY,
        title: &title,
        body: Some(strings::remove_body()),
        shown: None,
        note: None,
        cancel_key: "remove-cancel",
        cancel: strings::remove_cancel(),
        confirm_key: "remove-confirm",
        confirm: strings::remove_confirm(),
        danger: true,
    };
    ask(
        question,
        Shell::cancel_remove_server,
        Shell::confirm_remove_server,
        look,
    )
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
