//! Papo's desktop client, on Rune.
//!
//! This crate owns the desktop presentation: windows, views, input, focus,
//! and platform integration. Accounts, sessions, messages, history, drafts
//! and media storage belong to the Papo SDK; nothing here keeps its own copy
//! of them. Rune types stay in this crate.
//!
//! The binary (`src/main.rs`) only calls [`run`].

mod account_switch;
mod account_watch;
mod badges;
mod channel_pane;
mod composer;
mod confirm;
mod design;
mod fonts;
mod icons;
mod keyring;
mod lifecycle;
mod links;
mod menu;
mod navigation;
mod notice;
#[cfg(test)]
mod private_bus;
mod profile_dir;
mod rail;
mod reaction_watch;
mod reading_prefs;
mod reconnect_hints;
mod remove_server;
mod sdk_bridge;
mod server_list;
mod shell;
mod sign_in;
mod strings;
mod tabs;
mod timeline;
mod window;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use papo_sdk::{ClientConfig, DataRoot};

use crate::keyring::{Keyring, SecretService};
use crate::lifecycle::{Lifecycle, Start};
use crate::sdk_bridge::{Bridge, BridgeHost, OpenFailure, Post, Profile};
use crate::shell::Shell;

/// Starts the desktop client and blocks until its window closes and the SDK
/// has shut down. When Papo already runs, asks it to show its window and
/// returns at once.
///
/// # Errors
///
/// Returns Rune's error when the window cannot open, for example without a
/// Wayland or X11 display.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    // Before anything else starts: a later launch only hands off and exits.
    let mut lifecycle = match Lifecycle::start() {
        Start::Running(lifecycle) => lifecycle,
        Start::HandedOff => return Ok(()),
    };
    strings::use_system_language();
    rune::set_app_id(window::APP_ID);
    // Rune ends the process the moment the app quits unless told otherwise;
    // the exit fence below runs after the window is gone.
    rune::orderly_exit();
    let keyring = Arc::new(Keyring::start(SecretService::default)?);
    let app = rune::App::new(window::spec())
        .on_close(Shell::window_closing)
        // Read reports wait while the window is off screen.
        .on_visibility(Shell::window_visibility);
    let post = Arc::new(app.handle());
    lifecycle.attach(Arc::new(app.handle()));
    let bridge = Rc::new(RefCell::new(BridgeSlot::NotStarted));
    let starter = Rc::clone(&bridge);
    let secrets = Arc::clone(&keyring);
    // Opening the profile reads SQLite and restores sessions, so it waits for
    // the first frame. `on_ready` runs again when Wayland recreates the
    // window; the bridge starts once.
    let app = app.on_ready(move |_window| {
        let mut slot = starter.borrow_mut();
        if matches!(*slot, BridgeSlot::NotStarted) {
            *slot = start_bridge(Arc::clone(&secrets), &post);
        }
    });
    let result = app.try_run();
    // The exit fence: the window is gone; shut the SDK down before the
    // process ends, then the keyring it used. Papo's name goes last, once the
    // SDK has released the profile, so a launch waiting for it can open it.
    lifecycle.quitting();
    if let BridgeSlot::Running(bridge) = bridge.replace(BridgeSlot::Unavailable) {
        bridge.shutdown();
    }
    keyring.close();
    lifecycle.stop();
    result?;
    Ok(())
}

/// Whether the SDK bridge has started, once the first frame is ready.
enum BridgeSlot {
    NotStarted,
    Running(Bridge),
    /// It could not start, or the app is past its exit fence.
    Unavailable,
}

/// Starts the SDK bridge on the user's profile. When it cannot, the shell
/// hears so as if the client had failed to open; the window stays usable.
fn start_bridge(secrets: Arc<Keyring>, post: &Arc<rune::AppHandle<Shell>>) -> BridgeSlot {
    if let Some(bridge) = try_start_bridge(secrets, post) {
        return BridgeSlot::Running(bridge);
    }
    post.post(Box::new(|shell: &mut Shell, _tx| {
        shell.sdk_failed(OpenFailure::Failed);
        true
    }));
    BridgeSlot::Unavailable
}

/// [`start_bridge`]'s work, logging why the bridge did not start.
fn try_start_bridge(secrets: Arc<Keyring>, post: &Arc<rune::AppHandle<Shell>>) -> Option<Bridge> {
    let Some(root) = profile_dir::profile_dir() else {
        log::error!("no profile directory: neither XDG_DATA_HOME nor HOME is set");
        return None;
    };
    let profile = Profile {
        root: DataRoot::new(root),
        secrets,
        config: client_config(),
    };
    let post: Arc<dyn Post<Shell>> = Arc::clone(post) as Arc<dyn Post<Shell>>;
    Bridge::start(profile, post)
        .inspect_err(|error| log::error!("the SDK bridge thread did not start: {error}"))
        .ok()
}

/// The SDK client's limits. A debug build run with `PAPO_DEV_LOOPBACK=1`
/// lets a server on plain-HTTP loopback (the SDK's test server) keep its
/// Secure sign-in cookie. While it is set the SDK connects only to loopback
/// servers, so every other saved server stays unreachable. Release builds
/// never read the variable.
fn client_config() -> ClientConfig {
    let config = ClientConfig::default();
    if cfg!(debug_assertions)
        && std::env::var_os("PAPO_DEV_LOOPBACK").is_some_and(|value| value == "1")
    {
        log::warn!(
            "PAPO_DEV_LOOPBACK: loopback servers only, Secure cookies allowed over plain HTTP"
        );
        return ClientConfig {
            insecure_loopback: true,
            ..config
        };
    }
    config
}

/// The log filter when `RUST_LOG` is not set: Papo's own crates at `info`,
/// everything else at `warn`.
const DEFAULT_LOG_FILTER: &str = "warn,papo_desktop=info";

fn init_logging() {
    // Only a logger set earlier in the process makes this fail; that one stays.
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(DEFAULT_LOG_FILTER),
    )
    .try_init();
}
