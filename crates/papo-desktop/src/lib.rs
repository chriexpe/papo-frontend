//! Papo's desktop client, built from scratch on GPUI Kit.
//!
//! The desktop owns presentation, navigation, platform integration and the
//! mapping of SDK services onto GPUI views. Protocol, canonical data and
//! persistence are owned by `papo-api` and `papo-sdk`, and UI projections by
//! `papo-sdk-ui`. Legacy clients are behavioural references only.

// GPUI's macros emit `gpui::` paths; the pinned GPUI Fast Kit needs this alias.
extern crate gpui_kit as gpui;

mod app;
#[cfg(test)]
mod list_proof;
mod shell;
mod window;

/// Starts the desktop client and blocks until it quits.
pub fn run() {
    init_logging();
    app::run();
}

/// Limit third-party logging while keeping Papo diagnostics visible.
const DEFAULT_LOG_FILTER: &str = "warn,papo_desktop=info,papo_sdk=info,papo_call=info";

fn init_logging() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(DEFAULT_LOG_FILTER))
        .init();
}
