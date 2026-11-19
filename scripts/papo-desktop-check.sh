#!/usr/bin/env bash
# The checks every papo-desktop change must pass. CI's papo-desktop job runs
# this same script, so a change that passes here passes there on Linux.
#
#   scripts/papo-desktop-check.sh          papo-desktop only
#   scripts/papo-desktop-check.sh --shared also the crates papo-desktop shares
#                                          the workspace with; required when a
#                                          change touches another crate, the
#                                          workspace manifest or Cargo.lock
#
# Needs the system packages from scripts/deps.sh and the Rust toolchain from
# rust-version in Cargo.toml.
set -euo pipefail

shared=false
for arg in "$@"; do
    case "$arg" in
        --shared) shared=true ;;
        -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

cd "$(dirname "$0")/.."

step() { printf '\n==> %s\n' "$*"; }

step "format"
cargo fmt -p papo-desktop --check

step "clippy"
cargo clippy --locked -p papo-desktop --all-targets -- -D warnings

step "docs"
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p papo-desktop --no-deps --document-private-items

step "tests"
cargo test --locked -p papo-desktop

if $shared; then
    # What CI's workspace jobs run for the other crates. A change made for
    # the desktop must leave the SDK and Android green.
    step "shared crates: clippy"
    cargo clippy --locked --workspace --exclude papo-desktop --all-targets -- -D warnings
    step "shared crates: tests"
    cargo test --locked --workspace --exclude papo-desktop
fi

step "all checks passed"
