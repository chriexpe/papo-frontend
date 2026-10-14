# macOS bootstrap (Apple Silicon)

The macOS port is **experimental**. This first milestone checks that the native
Rust desktop application links against GStreamer and starts its media command
line without requiring a MacBook on the developer's desk. It does **not** yet
produce a distributable `.app`, sign or notarize it, or provide a native
WKWebView backend. Rich interactive web embeds currently have no macOS backend.

## Building on a Mac

The CI target is `macos-15` (Apple Silicon / `aarch64-apple-darwin`). On an
Apple Silicon Mac with Xcode Command Line Tools and Rust 1.98+ installed:

```sh
brew install gstreamer pkgconf
# Homebrew currently bundles the former gst-* plugin formulae in gstreamer.
gst-inspect-1.0 webrtcbin
cargo build --locked --bin papo
./target/debug/papo voice-check
./target/debug/papo demo
```

The app relies on Homebrew's GStreamer libraries/plugins at development time.
Do not distribute `target/debug/papo` as a standalone binary: packaging and
runtime relocation are still pending. For a practical test, verify login,
text chat, attachments, scrolling and media; audio/video hardware availability
and camera/microphone permission prompts require testing on a physical Mac.

## Missing pieces for a release

- Native WKWebView implementation of `WebEmbedBackend` (including input,
  clipping, scrolling and lifecycle), rather than the unavailable fallback.
- Native notification and application lifecycle integrations, including macOS
  menu/dock/tray behavior as appropriate.
- `.app` packaging, GStreamer runtime/plugins and license bundling, stable
  loader paths, `Info.plist` privacy usage strings, code signing/notarization.
- Physical MacBook testing for microphones, camera, calls and GPU/Metal
  rendering. Intel architecture is not yet a CI target.

Keep platform-specific functionality behind OS boundaries and preserve the
shared egui/Papo core rather than forking application logic for macOS.
