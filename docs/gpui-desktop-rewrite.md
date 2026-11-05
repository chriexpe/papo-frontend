# Papo desktop rewrite: GPUI Kit + GPUI Fast

Status: **architecture/handoff contract**, not an implementation. Established 2026-11-05.

This document is the source of truth for a **clean replacement** of the desktop egui client. It is not a request to incrementally port egui, preserve its app object, or reorganize legacy code in place. The supported Android client is Kotlin/Jetpack Compose and remains unaffected. Existing desktop egui stays buildable and shippable until the GPUI replacement passes the release gate.

## 1. Decisions and non-goals

1. Build a **new** desktop application under `crates/papo-desktop/`, driven by **GPUI Kit + GPUI Fast**. Initial documented compatible version pair: `gpui-kit 0.9.0` / optional `gpui-fast 0.1.13` (2026-11-05). Verify the published compatibility matrix before changing versions.
2. Use `gpui-kit = { version = "=0.9.0", features = ["gpui-fast"] }` initially. For native browser embedding, add `gpui-webview = { version = "=0.9.0", features = ["gpui-fast"] }` only when needed; the wrapper is experimental. Let Kit provide the GPUI types, do not add a mismatched direct `gpui` dependency or patch upstream GPUI as a default strategy. Commit Cargo.lock when the new crate is ready.
3. Retain `crates/papo-core` as the canonical source for protocol, sessions, REST/WebSocket, message state, synchronization, read state, persistence, previews, notifications and authorization. Retain `crates/papo-call` as the shared GStreamer/WebRTC call engine. The GPUI application must not develop a divergent protocol/state implementation.
4. Treat `src/app.rs`, `src/ui/`, `src/render/`, `src/webembed.rs` and the current egui media adapters as **behavioral references, not source to translate**. Carry over requirements and verified algorithms only when they are UI-independent.
5. Create new desktop-owned state, navigation, components, theme tokens, media presentation and platform integration with clean ownership. No egui/eframe types, IDs, contexts, textures, styles, geometry or widget callbacks in new public contracts.
6. No GPUI types in `papo-core` or `papo-call`. Android Compose and its UniFFI contracts must continue working. Shared-core changes require demonstrated multi-client benefit and independent tests.
7. Preserve the Papo visual identity and all functional semantics, **not** an identical widget tree, pixel-for-pixel legacy quirks or existing implementation details.
8. Keep GPUI Fast an **explicit, reversible compile-time backend choice** during development where Kit supports it. Do not combine GPUI-Fast and upstream-GPUI types in one dependency graph. It is experimental; it is not assumed to improve resident memory without measurement.
9. The eventual desktop runtime should not carry any egui/eframe dependency, vendor patches, build preparation, binaries or platform shims solely retained for the old UI. Remove those **at cutover**, not in the preparation PR.
10. Do not convert this into a web app, Electron/Tauri shell, a GPUI-on-Android project, or an unsolicited backend rewrite.

## 2. Current boundaries (verified against main)

- `crates/papo-core` already has no egui/eframe/GStreamer dependency, exports `api`, `cache`, `runtime`, `state`, `preview`, `notification`, `storage` and related modules. `papo-core::runtime::ServerRuntime` is a useful integration boundary, not a reason to clone state into every component.
- `crates/papo-call` already isolates substantial call-engine logic from the UI.
- Android Compose consumes shared Rust contracts through UniFFI. Never change these contracts incidentally while moving desktop code.
- The egui desktop controller, shell, widgets, media presentation, webview geometry and GPU effects are intertwined. Moving them into a shared desktop abstraction before writing GPUI would create avoidable transitional code.
- Existing `Cargo.toml`, `Cargo.lock`, desktop build/packaging and CI target the `papo` egui executable. Preserve these until a validated GPUI executable can replace them. Do not register a placeholder/unbuildable crate in the workspace or add GPUI dependencies to the legacy root package.

## 3. Target structure (guidance, not a freeze on file names)

```text
crates/
  papo-core/                 # existing shared domain/runtime/cache
  papo-call/                 # existing shared call engine
  papo-desktop/              # new, independent desktop binary
    Cargo.toml
    src/
      main.rs                # bootstrap; no business logic
      app/                   # lifetimes, commands, session orchestration
      projection/            # narrow core -> view model mapping
      navigation/            # routes and navigation intent
      ui/
        shell/               # main layout, rails, headers and panes
        chat/                # virtualized timeline, message views, composer
        settings/            # appearance / notifications / account
        administration/      # roles, permissions, moderation, webhooks
        overlays/            # sheets, dialogs, popovers, menus
        design/              # colors, type, spacing, motion, icons
      media/                 # UI-neutral media services + GPUI texture adapter
      embeds/                # policy, viewport, native WebView ownership
      platform/
        linux/
        windows/
        macos/
      diagnostics/           # lifecycle / perf / memory instrumentation
```

Keep feature modules cohesive, avoid a single enormous `shell.rs` or `app.rs`, and avoid creating a speculative framework with dozens of layers before a real chat screen exists.

**Dependency direction:** `papo-desktop` -> `papo-core`/`papo-call`. Feature views -> slim application interfaces/projections -> canonical core runtime. Platform backends implement desktop-owned interfaces and do not depend on feature-specific UI elements. GPUI entities and handles do not cross into shared crates.

**Threading:** work, network and decoding happen off the UI thread; marshal typed results back through GPUI scheduling/notifications. Bind callbacks to an explicit server/account/channel generation; ignore stale results after navigation, logout or session replacement. Avoid per-frame polling, wholesale snapshot copies, nested broad locks and unbounded tasks. Keep the persistent view state minimal and keyed by stable IDs.

## 4. UI behavior contract

- Use GPUI Kit components for menus, dialogs, tooltips, text inputs and virtual lists when they provide correct behavior. Wrap small app-specific behavior, not the entire toolkit. Make local replacements only if the component has a documented gap.
- The message timeline needs stable message IDs, variable-height rows, anchor preservation on prepend/reflow, bottom-following only when appropriate, deterministic navigation to message/latest/unread, and explicit *visible interval* reporting to `papo-core` read-state logic. The Kit MessageScroller alone does not implement Papo's read acknowledgements or unread-gap semantics.
- Preserve multi-server isolation, cross-server DM list, per-channel unread state, offline history and reconciliation, optimistic/outgoing handling, authorization checks, notifications and delayed event ownership.
- The composer requires verified IME, selection, clipboard, undo/redo, multiline growth, markdown, mentions, custom and Unicode emoji, replies, edits, attachments, paste and drag/drop. It must not conflate keypress handling with server send acknowledgements.
- Settings, administration, profiles, calls and rich messages should share basic design primitives without sharing mutable state by accident.
- Preserve Papo's floating pill / restrained glass appearance with a centralized token system. Glass must have a deterministic opaque fallback. Do not revive the egui-specific framebuffer or `egui-wgpu` patch.
- Native WebViews must have a bounded lifecycle (activate/present/suspend/destroy); overlay order, clipping, input focus, fullscreen and offscreen behavior are part of correctness, not decoration.
- Media is a first-class, bounded resource with explicit ownership for encoded bytes, decoded frames, textures, active decoders and native browser processes. Separate visible, near-viewport, offscreen and disposed states. Never decode on render callbacks or preload long videos in full.
- Reuse shared core policy and reusable rendering models for posts and embeds. Do not implement separate slightly different message semantics for timeline, pins, search and DMs.

## 5. Platform risks to validate early

| Platform | Mandatory validation |
| --- | --- |
| Linux X11/XWayland | GPUI startup, WGPU/driver support, selection/IME, WebKitGTK wrapper, overlay/z-order/clipping, file portals, notifications, tray, GStreamer, packaging |
| Linux native Wayland | GPUI window and lifecycle, fractional DPI, menu/overlay behavior, clipboard, screen capture, close-to-tray; **gpui-webview currently does not support native Wayland**, so define a tested alternative or explicitly retain an XWayland requirement before release |
| Windows | GPUI graphics backend/driver fallback, text/IME, WebView2 child vs composition overlay issues, tray, updater, install/portable packages and GStreamer |
| macOS (Apple Silicon initially) | WKWebView ownership and overlay composition, Retina DPI, window activation, app menu/Dock, notifications, media and signing/bundle behavior |

The GPUI Fast low-level native composition example is **not** the same thing as cross-platform support in the `gpui-webview` wrapper. Its Windows and Linux Wayland integration limitations must be proven against Papo, not inferred from the renderer's feature list. Do not require the new app to use the old WPE/WebView2/WKWebView bridging code if a cleaner working implementation exists.

## 6. Local implementation sequence

These are *vertical slices*, not months of scaffolding. A slice is complete only when a user can run it against a real backend.

1. **Bootstrap:** local GPUI Kit + Fast window, theme tokens, logging, diagnostic hooks, core runtime connection and clean shutdown. Keep the old egui binary working. Demonstrate Linux window/input/launch first, then ensure dependency choices are viable on Windows/macOS.
2. **Real account + timeline:** authentication, multiple sessions, server/channel navigation, cache hydration, virtualized messages, history prepend, unread jump, read receipts, offline/reconnect, and a basic composer against real Papo endpoints.
3. **Composer + interactions:** reply/edit/delete, rich text and mentions, emoji, attachments, search/pins, role-aware actions, DMs, menus, popovers and keyboard access.
4. **Media stress slice:** photos/animated media, GStreamer direct playback and call tiles; run repeated open/close/scroll/reconnect memory checks, not merely a single playback smoke test.
5. **Native embeds:** WebView playback and floating/inline policies on each desktop OS; test overlapping menus, tooltips, dialogs, fullscreen, scrolled clipping and cleanup.
6. **Remaining parity:** settings, account/server admin, moderation/audit, notification policies, calls, presence, updates, tray, file dialogs, window appearance and packaging. Use a per-feature parity matrix from egui and Compose; neither is an unquestioned design reference.
7. **Cutover only after gate:** switch build/release entry point; remove egui, egui-only patches and dead shims; update README, CI, docs, packages and smoke tests. Remove stale migration prose that claims egui remains the target.

Each slice should be a focused PR, with evidence of real builds/tests and clear limitations. Do not let a single giant PR become the only reviewable artifact.

## 7. Correctness and performance gates

**Automated**
- Unit and integration tests for core runtime compatibility, projections, account/channel generation scoping, subscription teardown, unread/read boundaries, and media/WebView lifecycle state machines.
- Component tests for deterministic row IDs, preserve-anchor behavior, changing heights, optimistic updates, composer keyboard and IME behavior where test infrastructure allows.
- Existing Android Compose/UniFFI and `papo-core`/`papo-call` tests continue to pass. `cargo test --locked --workspace`, platform builds and Clippy must remain green as crates become workspace members.
- A UI environment test is not a substitute for real desktop validation. New GPUI jobs should be introduced only once the corresponding target is locally buildable.

**Manual, on real desktop machines**
- Log in/out; multiple servers; offline/reconnect; 10k+ message channel; switch channel while history loads; incoming message during reading; unread hourglass; scroll up/down; changing row heights, DPI and font scaling; every density/grouping variant.
- Composition: embed under popover/context menu/tooltip/modal, float/offscreen behavior, rapid activation/destruction, YouTube, native video/GIF, call join/leave/rejoin, camera/screen sharing.
- Repeated opening/closing of profiles, settings, emoji/sticker/GIF pickers, native dialogs, media viewer and notifications.
- Crash-free suspend, restore, close-to-tray, logout/restart and packaging/updates on every supported OS.

**Memory / performance**
- Compare optimized GPUI and current egui builds on the *same data and machine*. Measure process RSS/PSS/private bytes where available, GPU allocations, GStreamer decoder buffers and WebView child processes separately. Record exact scenario, duration and graphics backend.
- Start with warm idle; scroll image-heavy history; play and close media; repeatedly switch channels, open embeds, join/leave calls; then wait for settled idle. Repeat cycles to detect monotonic growth and resources that are released too late.
- Target bounded, stable resource ownership and no noticeable responsiveness or CPU regression. Do **not** promise a numeric memory improvement from framework selection or misinterpret allocator retention as a proven leak.
- GPUI Fast caches rendering and GPU tiles: retain only what the user experience needs; cap image and decoder budgets with measurable eviction. Check missing `cx.notify()` when externally held state changes, to avoid invisible/stale updates.

## 8. First local-agent handoff

Work from a branch based on the latest `main` **containing this preparation PR**. Read this document, the repo's `AGENTS.md` and GPUI Kit's release-matched documentation before implementation.

1. Inspect `papo-core::runtime::ServerRuntime`, `papo-core::state`, `papo-core::api` and `papo-call`. Draft only the minimal UI-facing adapter needed for a real chat vertical slice; keep the canonical state/commands in core.
2. Create `crates/papo-desktop` as a distinct Rust binary and GPUI app. Use Kit's published GPUI Fast feature. Enable the optional WebView dependency only when the embed work starts.
3. Build and run locally first. For Linux test native Wayland and X11/XWayland explicitly, logging the selected display backend. Keep the old `papo` binary and packaging unchanged.
4. Implement auth/server/channel navigation, a virtualized live timeline, read viewport accounting and text composer. Use actual backend data, not throwaway mocked UI as a substitute for integration.
5. Commit source, tests and lockfile only when the new crate builds; then add it to workspace and a targeted CI job when dependencies and runners are known to be compatible. The existing CI should not be broken by scaffolding.
6. Record what was tested locally, the GPU/display backend, known WebView constraints and a before/after memory scenario. Send small, reviewable PRs; do not merge the final switch-over before parity gates.

Potential docs: https://gpui-kit.com/docs/installation/ ; https://gpui-kit.com/docs/webview/ ; https://gpui-kit.com/component/message-scroller/ ; https://github.com/longbridge/gpui-fast .

### What this preparation PR intentionally does not do

- No Cargo manifest, lockfile or vendor patch changes; no compiling experimental GPUI on existing CI.
- No moves or rewrites of egui files; no renaming existing desktop binaries; no changes to Android.
- No claim that native Wayland WebViews, Windows overlay composition or cross-platform graphics are already solved.
- No global decision to freeze the UI at the initial library versions. Revisit versions explicitly with local testing and a reviewed dependency update.
