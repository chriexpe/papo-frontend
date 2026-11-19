//! Entry point of the `papo-desktop` binary. All logic lives in the library.

use std::process::ExitCode;

fn main() -> ExitCode {
    match papo_desktop::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("papo-desktop could not start: {error}");
            ExitCode::FAILURE
        }
    }
}
