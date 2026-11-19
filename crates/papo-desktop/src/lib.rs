//! Papo's desktop client, on Rune.
//!
//! This crate owns the desktop presentation: windows, views, input, focus,
//! and platform integration. Accounts, sessions, messages, history, drafts
//! and media storage belong to the Papo SDK; nothing here keeps its own copy
//! of them. Rune types stay in this crate.
//!
//! The binary (`src/main.rs`) only calls [`run`].

mod design;
mod shell;
mod strings;
mod window;

/// Starts the desktop client and blocks until its window closes.
///
/// # Errors
///
/// Returns Rune's error when the window cannot open, for example without a
/// Wayland or X11 display.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();
    strings::use_system_language();
    rune::set_app_id(window::APP_ID);
    rune::App::new(window::spec()).try_run()?;
    Ok(())
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
