# Wayland window visibility patch

Papo keeps the winit 0.30.13 correction in `patches/winit+0.30.13.patch`.
Only the diff is committed. `scripts/prepare-winit.py` retrieves the published
crate (from Cargo's archive cache when possible), verifies its SHA-256, extracts
it into the ignored `.papo/patch/winit-0.30.13` directory, and applies the diff.
Cargo compiles that local path through `[patch.crates-io]`.

The patch changes four source files: runtime visibility, Wayland window
state/configure handling and redraw scheduling. Hiding unmaps the toplevel;
showing restores its metadata and waits for a fresh configure. Background
logic remains active while hidden painting is suspended. Initial visibility
semantics remain unchanged.

## Preparing a checkout

Prerequisites: Rust 1.98+, Python 3.11+, Git, and normal platform build
dependencies. On Windows, use `python` instead of `python3`.

```sh
python3 scripts/prepare-winit.py
cargo build --locked
```

Re-run preparation when modifying the patch or updating winit; already verified
prepared sources are reused. `cargo clean` does not remove the prepared source.
Failed or incomplete preparations do not publish a marker; concurrent
preparation is rejected. Cargo.lock and the Cargo registry source tree are
never modified. No `cargo install patch-crate` step or Git checkpoint commits
are needed.

The original `.crate` archive is checked against SHA-256
`a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`.
When not in Cargo's cache, the script downloads the immutable crate from
static.crates.io and checks the exact same checksum before extraction.
The helper rejects unsafe archive members, verifies the complete applied patch
and marker, and stages replacements in a temporary directory before publishing.

```sh
python3 scripts/prepare-winit.py --check  # validate without rebuilding
python3 scripts/prepare-winit.py --force  # recreate even a verified copy
```

The build script checks that the applied-patch marker matches the committed
diff. A stale directory cannot produce a successful Papo build.
Preparation must precede Cargo dependency resolution.

## CI and packaging

The shared GitHub action runs the Python helper before project Cargo commands.
Normal `target/` paths and existing Rust cache configuration remain intact;
the generated winit source lives outside `target/` to avoid rust-cache cleanup.
Android, watch and Flatpak source-generation scripts also run preparation.
The Arch package runs the same helper without compiling a Rust patch CLI.

Flatpak source generation runs preparation on the host. Inside the sandbox,
flatpak-builder downloads the published winit archive with the same checksum,
applies the same diff through its native patch source, and copies the marker
after patch application succeeds. Cargo builds offline with Cargo.lock.
The sandbox installs no patching CLI and downloads no dependencies during build.

## Updating or removing the correction

1. Check whether upstream winit now implements the required Wayland hide/remap
   lifecycle. If so, remove the override, marker, preparation integrations,
   and Flatpak patch sources before refreshing Cargo.lock.
2. Otherwise, rebase the small diff onto the published crate. Update its exact
   version, generated override path, diff filename, preparation constants,
   build-script marker, Flatpak URL/destination/checksum and related docs.
3. Recreate the local copy and refresh Cargo.lock for the new dependency.
   Verify the preparation leaves Cargo.lock unchanged after that.
4. Run preparation tests, workspace tests, Clippy and platform builds.
   Verify the desktop visibility lifecycle on Wayland and X11.

```sh
python3 -B -m unittest discover -s tests -p 'test_prepare_winit.py'
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace
cargo test --locked --test desktop_visibility -- --ignored --nocapture
```

Regenerate Flatpak sources with `./scripts/flatpak-sources.sh` and verify
offline builds on the supported architectures before releasing.
