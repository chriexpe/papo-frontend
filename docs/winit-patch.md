# Wayland window visibility patch

Papo keeps the winit 0.30.13 correction in `patches/winit+0.30.13.patch`.
Only the diff is committed. `patch-crate 0.1.13` copies the published crate
into the ignored `target/patch/winit-0.30.13` directory and applies the diff.
Cargo compiles that local path through `[patch.crates-io]`.

The patch changes four source files: runtime visibility, Wayland window
state/configure handling and redraw scheduling. Hiding unmaps the toplevel;
showing restores its metadata and waits for a fresh configure. Background
logic remains active while hidden painting is suspended. Initial visibility
semantics remain unchanged.

## Preparing a checkout

Prerequisites: Rust 1.98+, Python 3.11+, Git, and the normal platform build
dependencies. On Windows, use `python` instead of `python3` below.

```sh
cargo install patch-crate --version 0.1.13 --locked
python3 scripts/prepare-winit.py
cargo build --locked
```

Run preparation again after `cargo clean`, modifying the patch, or updating
its version. The helper reuses a directory only after checking the applied
patch and its recorded input; otherwise it invokes the CLI with `--force`.
A failed application removes the incomplete generated crate. Preparation
does not modify Cargo.lock or the registry source tree, and concurrent
preparation is rejected.

The original `.crate` archive is checked against SHA-256
`a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`.
Temporary Git checkpoint commits use process-local identity/signing settings;
project commits retain their normal configuration.

```sh
python3 scripts/prepare-winit.py --check  # validate without rebuilding
python3 scripts/prepare-winit.py --force  # recreate even a verified copy
```

The build script checks that the applied-patch marker matches the committed
diff. A stale directory cannot produce a successful Papo build. A missing
path after cleaning is rejected by Cargo before build.rs runs, so preparation
must precede Cargo. The patch-crate CLI can prepare this missing path itself.

## CI and packaging

The shared GitHub action installs/caches the pinned CLI per OS, architecture
and toolchain, and prepares the patch before project metadata/build commands.
CI compilation uses `target/ci`, leaving the prepared source in `target/patch`
outside rust-cache cleanup. Android, watch and Flatpak source-generation
scripts also run preparation.
The Arch package prepares its own local copy using the same pinned CLI.

Flatpak source generation runs preparation on the host. Inside the sandbox,
flatpak-builder downloads the published winit archive with the same checksum,
applies the same diff through its native patch source, and copies the marker
only after patch application succeeds. Cargo then builds offline with the
lockfile. The sandbox does not install patch-crate or download dependencies
during the build.

## Updating or removing the correction

1. Check whether a compatible official winit release supports the required
   hide/remap lifecycle. If it does, remove the patch metadata/override, marker
   guard, preparation integration and Flatpak sources, then refresh Cargo.lock.
2. Otherwise, rebase the small diff onto the published crate for the desired
   version. Update the exact dependency version and generated override path,
   diff filename, preparation constants, build-script marker paths, and Flatpak
   URL/destination/checksum together. Preparation intentionally fails when
   these disagree; updates never silently skip the correction.
3. Recreate the local copy, refresh Cargo.lock once for the new dependency,
   and review the dependency changes. Repeat preparation to ensure the lockfile
   stays unchanged thereafter.
4. Run preparation regression tests, workspace tests, strict Clippy, platform
   checks and a build. Verify the visibility lifecycle in a desktop session:

```sh
python3 -B -m unittest discover -s tests -p 'test_prepare_winit.py'
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace
cargo test --locked --test desktop_visibility -- --ignored --nocapture
```

Regenerate Flatpak sources with `./scripts/flatpak-sources.sh` and verify the
offline build for the supported architectures before releasing.
