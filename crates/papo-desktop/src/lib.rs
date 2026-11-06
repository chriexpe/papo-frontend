//! Papo's desktop client, built from scratch on GPUI Kit.
//!
//! This crate owns the desktop presentation: windows, views, input, platform
//! integration and the mapping from `papo-core` state to what a view draws.
//! It owns no protocol, no canonical state and no persistence format: those
//! belong to `papo-core`, and calls to `papo-call`. Nothing here is
//! translated from the egui client in the root crate; that client is a
//! behavioural reference only (`crates/papo-desktop/AGENTS.md`,
//! `docs/papo-desktop.md`).
//!
//! The binary (`src/main.rs`) only calls [`run`].

// GPUI's macros (`actions!`, the derives) emit `gpui::` paths. With the
// `gpui-fast` engine they are not rewritten to the Kit, so the Kit is aliased
// here once, as its compatibility notes ask (gpui-fast `compat/README.md`).
extern crate gpui_kit as gpui;

mod app;
pub mod credentials;
mod projection;
mod session;
mod shell;
mod window;

/// Starts the desktop client and blocks until it quits.
///
/// Logging starts first, then GPUI takes over the main thread. The function
/// returns when the last window closes or the user quits.
pub fn run() {
    init_logging();
    app::run();
}

/// The log filter when `RUST_LOG` is not set: Papo's own crates at `info`,
/// everything else at `warn`. At `info`, zbus and the accessibility bridge
/// log every D-Bus handshake (measured on Linux, 2026-11-05), which hides
/// the lines a bug report needs.
const DEFAULT_LOG_FILTER: &str = "warn,papo_desktop=info,papo_core=info,papo_call=info";

fn init_logging() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(DEFAULT_LOG_FILTER))
        .init();
}
