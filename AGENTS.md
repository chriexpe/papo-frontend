# Repository agent guidance

This repository ships the **egui desktop client** and the supported **Kotlin/Jetpack Compose Android client**, backed by Rust `papo-core`. Changes unrelated to the desktop rewrite should follow their existing scope and must not be redirected into GPUI work.

## New GPUI desktop rewrite

**Required reading:** [docs/gpui-desktop-rewrite.md](docs/gpui-desktop-rewrite.md). It records the architecture decisions, compatibility risks, local handoff, performance and feature-parity gates.

- Treat the GPUI desktop as a **clean replacement**, not a transliteration or refactor of existing `src/ui`, `src/app.rs`, egui media, WebEmbed or rendering code. The old desktop is a behavioral reference only.
- New desktop UI belongs in `crates/papo-desktop/` once a real, locally validated crate is created. Use GPUI Kit + GPUI Fast as documented, with compatible dependency versions and one GPUI type universe per build.
- Keep `papo-core` and `papo-call` independent of egui, GPUI and desktop UI types. They own protocol, canonical state, synchronization, authorization, caching and call engine semantics. Do not fork their behavior in the new UI.
- Preserve the Android Compose app and UniFFI contracts. Core changes require a concrete cross-client reason and coverage.
- Make state ownership, cancellation, stale-result protection, and shutdown explicit. Restrict expensive work and media decoding to non-UI threads. Bound memory/media/browser lifetimes.
- Use established GPUI Kit components when suitable; do not recreate focus, text inputs, lists or popup foundations without a documented reason. Preserve Papo's visual identity without copying egui widget internals.
- Verify Windows, Linux (both native Wayland and X11/XWayland), and macOS separately. GPUI Fast native composition is not proof the experimental GPUI Kit WebView wrapper works identically across platforms.
- Do not prematurely remove egui, rename the production binary, alter releases or add an unbuildable GPUI crate to the workspace/CI. Replace the shipping desktop only after the parity, integration and memory gates in the rewrite document.
- Prefer incremental **working vertical-slice PRs** with builds, tests, graphics/backend notes and resource-lifecycle validation over a giant rewrite PR.

## General guardrails

- Respect existing CI and Cargo.lock discipline; avoid unrelated formatting or code movement.
- Do not put secrets, user tokens, identifying server data or private logs in committed fixtures and screenshots.
- For work outside the desktop migration, keep its independent requirements and tests intact.
