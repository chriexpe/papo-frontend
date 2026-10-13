# Wayland window visibility patch

Papo uses [prodesert22/winit](https://github.com/prodesert22/winit) for runtime
Wayland window visibility. The fork is based on upstream winit 0.30.13 and
adds an isolated patch to four source files. Its behavior is documented in
the fork's `PAPO-PATCH.md`.

The `[patch.crates-io]` entry in `Cargo.toml` pins the fork to a full commit
hash. `Cargo.lock` records the resolved Git source, including the fork's
workspace dependency on `dpi`. Moving the fork's branch does not change a
locked build. The library source is fetched by Cargo rather than stored in
this repository.

## Updating or removing the patch

1. Check whether a compatible upstream release supports the required
   hide/remap lifecycle. If it does, remove the override and refresh the
   relevant lockfile entries.
2. Otherwise, reapply the isolated patch to the desired compatible upstream
   version in the fork. Review conflicts and changes to configure handling,
   visibility state and redraw scheduling.
3. Update `rev` in `Cargo.toml` to the tested commit and run `cargo check --workspace`
   to resolve the replacement source and refresh its lockfile entries.
   Review `Cargo.lock` for unrelated dependency updates.
4. Run workspace tests, strict Clippy, and a workspace build. Run the explicit
   desktop visibility probe in a desktop session to verify hiding and reopening
   the window while background logic remains active.
5. Regenerate Flatpak dependencies with `./scripts/flatpak-sources.sh` and verify
   that the generated sources include the pinned Git revision and support an
   offline Cargo build. The generated source manifests are not committed.

The desktop probe can be run with:

```sh
cargo test --locked --test desktop_visibility -- --ignored --nocapture
```
