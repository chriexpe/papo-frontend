//! Generates the string catalog from `strings/` (see `src/strings.rs`).

use std::path::Path;
use std::{env, io};

fn main() -> io::Result<()> {
    let manifest_dir =
        env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| missing("CARGO_MANIFEST_DIR"))?;
    let out_dir = env::var_os("OUT_DIR").ok_or_else(|| missing("OUT_DIR"))?;
    rune_i18n::codegen::generate(
        &Path::new(&manifest_dir).join("strings"),
        Path::new(&out_dir),
    )
}

fn missing(variable: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("Cargo did not set {variable}"),
    )
}
