# macOS (Apple Silicon, experimental)

The first macOS target compiles natively on GitHub Actions and provides a
Finder-launchable **test** `Papo.app`. It is ad-hoc signed, **not** signed
with an Apple Developer ID, notarized, or self-contained. It requires Homebrew
GStreamer and its libnice plugin installed on the destination Mac. This
version is for development and hands-on compatibility tests only.

## Download the app from CI

1. Open the latest successful [CI run](https://github.com/chriexpe/papo-frontend/actions/workflows/ci.yml)
   for the branch/PR, and download artifact `papo-macos-arm64-adhoc`.
2. On an Apple Silicon MacBook, install the external runtime:
   `brew install gstreamer libnice-gstreamer`.
3. Extract `Papo-macos-arm64-adhoc.zip` and move `Papo.app` to
   `/Applications` or another convenient directory.
4. Open it via Finder. This test build is **not notarized**, so macOS may
   block the first launch. Use the standard system **Open Anyway** control in
   Privacy & Security after attempting the launch; do not disable Gatekeeper
   globally. Apple Silicon only — no Intel test bundle yet.

This app depends on a working Homebrew installation (usually `/opt/homebrew`).
It does not ship any GStreamer dynamic libraries, plugins, or helper binaries.
If Homebrew is uninstalled or moved, the app may stop launching. Neither the
zip nor the binary is an official signed release.

## Build from source

On an Apple Silicon Mac with Xcode Command Line Tools and Rust 1.98+:

```sh
brew install gstreamer libnice-gstreamer pkgconf
gst-inspect-1.0 webrtcbin
gst-inspect-1.0 nicesrc
cargo build --locked --bin papo
./target/debug/papo voice-check
bash packaging/macos/build-app.sh target/debug/papo 0.7.5 dist
open dist/Papo.app
```

Replace `0.7.5` with the current numeric version in `Cargo.toml`.
The CI job builds and smoke-tests the packaged executable, validates its
`Info.plist`, generates a native icon and applies an ad-hoc signature.
The `Info.plist` includes privacy usage descriptions for the microphone and
camera, but hardware permission prompts still need a real MacBook test.

## Still missing for a production macOS release

- WKWebView implementation of `WebEmbedBackend`; rich interactive web
  embeds are not supported on macOS yet.
- Native notifications, macOS menu/dock/window semantics, and full hardware
  testing, including microphone/camera permissions and Metal rendering.
- Self-contained GStreamer runtime and plugin distribution, relocatable
  library paths, embedded helpers, developer signing and notarization.
- Apple Intel build and validation, if we elect to support Intel Macs.

Keep platform integration isolated behind existing platform interfaces. The
shared Papo UI, state and networking must remain common to all platforms.
