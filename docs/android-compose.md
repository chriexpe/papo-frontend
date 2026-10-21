# Native Android client

The Compose client lives in `android/compose`, alongside the existing egui
client in `android/app`. Its application ID is `io.github.chriexpe.papo.compose`
so both can be installed without replacing the existing app or its sessions.

## Build and run

Use the existing Android SDK (API 37), NDK `30.0.16248370`, Android Studio JDK,
Rust 1.98+, and `cargo-ndk`:

```bash
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk --locked

# Emulator
scripts/android-compose.sh --x86_64 --run

# Physical arm64 phone
scripts/android-compose.sh --arm64 --run

# Both architectures in one APK
scripts/android-compose.sh --all-abis
```

`ANDROID_HOME` defaults to `~/Android/Sdk`; `JAVA_HOME` defaults to
`/opt/android-studio/jbr`. Set these when your tools are installed elsewhere.
If multiple devices are connected, set `ANDROID_SERIAL` before installing.

Android Studio can open `android` and run the `compose` module. Gradle builds
the Rust adapter and generates Kotlin bindings automatically. UniFFI is pinned
to 0.32.2, and the generator reads metadata from the exact library packaged in
the APK. Generated sources stay under `compose/build/generated/uniffi`. A direct build supports both ABIs by default:

```bash
cd android
./gradlew :compose:assembleDebug
# Or only the emulator ABI:
./gradlew :compose:assembleDebug -PpapoAbis=x86_64
```

The APK is `android/compose/build/outputs/apk/debug/compose-debug.apk`.
This milestone uses development builds; distribution signing and the release
workflow still belong to the existing egui application.

## First launch and demo

First launch is an empty welcome screen. No server is suggested or prefilled.
Add a server using its HTTP(S) address, then sign in, register, or supply its
server password. Several servers can be saved and switched independently.
For a backend on the emulator host, use its reachable address (normally
`http://10.0.2.2:PORT`). HTTP is supported for local/self-hosted development;
remote servers should use HTTPS.

**Explore demo** is an explicit opt-in. The native adapter seeds the real
`papo-core::state::Store` and applies core updates for local interactions. It
opens no sockets, creates no credentials, and is not saved in the server list.
The demo is marked in the interface. Leave it from Settings. Demo messages
last for the current session; demo drafts use the normal draft storage.

## Ownership and lifecycle

```text
Compose screens
  → PapoViewModel: one background actor + conflated wake channel
    → UniFFI: generated records, enums, objects and callbacks (papo-android)
      → ServerRuntime / Store / Net / ClientDb (papo-core)
```

- `papo-core` owns authentication, REST/WebSocket, reconnects, cache
  reconciliation, history boundaries, account isolation, and the durable text
  outbox. The Android adapter has no duplicate messaging transport or reducer.
- Network updates wake the actor through a generated UniFFI callback. No frame loop or
  periodic UI polling is required. Drain batches are bounded.
- Core calls and cache reads run off the UI thread. Kotlin consumes generated
  records and action enums; UniFFI manages native object and callback lifetimes.
  The ViewModel survives rotation and releases its handles when cleared.
- Credentials use AES-GCM with an Android Keystore key and server/type-specific
  authenticated data. Cache/drafts are app-private; credentials and cache are
  excluded from backup. egui credentials are not imported.
- The composer keeps its draft until the core acknowledges durable outbox
  storage. Failed enqueue leaves the text available. Ambiguous outcomes require
  a deliberate retry, with a duplicate-delivery warning.
- Read state advances from actually visible message IDs while the Activity is
  resumed. Opening a conversation alone does not mark the whole channel read.
- Network availability/path changes are sent to the shared transport, and
  foreground return probes the connection. Background push and Android services
  are outside this milestone; background operation is limited to process life.

## Presentation

Material 3 provides native inputs, selection, dialogs, sheets, and accessibility.
Papo's floating header, navigation island, composer, and app-wide feedback use
one `GlassPill` component. Haze captures the content layer and samples a blurred
backdrop behind controls, leaving their labels sharp. Settings are solid grouped
surfaces. A Frosted controls toggle provides an opaque fallback; themes follow
the system or can be selected explicitly.

Form failures stay inline, connection state has a persistent indicator, delivery
state stays on the message, and app-wide failures use one dismissible feedback
pill. There are no per-screen Android Toast styles.

## Media

The demo’s **mídia** conversation includes local image, portrait, animated GIF,
video, audio and file fixtures, plus link/custom cards and a YouTube preview.
These assets work without a server; YouTube playback itself requires internet.

Embed media reserves its width/height from backend metadata before fetching
bytes. Video dimensions take precedence for players; image previews follow the
backend’s thumbnail-first response. Missing or invalid dimensions use a stable
fallback frame. Decode, error and retry events never resize the frame.

Coil decodes images/GIFs, and Media3 plays video/audio only after a tap. Leaving
the app pauses playback; disposed players release their resources. Only one
player is active per conversation. Original image viewing supports pinch zoom.
Files download on demand to private cache and open via Android’s file chooser.

Papo media uses authenticated attachment/thumbnail and embed-video relay routes.
Embed images come through the core’s authenticated preview endpoint, with a
bounded compressed image cache scoped to server, account and revision. Native
streaming preserves byte-range requests. The media HTTP client pins credentials
to one server origin and disables redirects. YouTube runs in an isolated WebView
without Papo headers or a JavaScript bridge; arbitrary iframe URLs are not run.

## Scope

Included: server setup, authentication, channels, DMs/member list, message
history, offline cache, text drafts, durable text sends, replies, Unicode
reactions, edit/delete, themes, glass controls, image/GIF viewing with zoom, native video/audio playback,
file downloads, link/custom cards, and opt-in YouTube playback.

Still to port: attachment uploads, full Markdown and custom emoji rendering, search,
voice/video, FCM/background reconciliation, notifications, the full client
settings and server administration panels, account recovery, and release
packaging. The established egui settings panels are the design reference for
those ports; the Compose settings screen currently covers account, appearance,
and demo exit only.

Demo checks presentation and local interactions; it cannot validate backend
authentication, WebSocket delivery, or reconnect behavior. Verify those against
a real server before using this as the primary client.

## Checks

```bash
cargo test --locked -p papo-core -p papo-android --lib
cargo clippy --locked -p papo-android --all-targets -- -D warnings
scripts/android-compose.sh --x86_64 --test
```

The test script builds both APKs, runs lint, installs them, invokes the explicit
instrumentation suite, and requires a nonzero JUnit passing-test count. This
avoids false success from Gradle UTP reporting zero tests on a preview emulator.

Device tests cover first-launch validation, demo interactions through UniFFI,
Unicode/multiline messages, saved drafts, appearance/demo exit, Keystore
isolation, and wake callbacks from the live network worker. Media tests decode a
portrait image and play a local video while asserting that reserved frames stay
unchanged. A local HTTP fixture checks Range requests, session headers, origin
restrictions and redirect behavior. Android 17 needs
the test-only Espresso 3.8 alpha because older Espresso reflects a removed
InputManager API. Production dependencies remain independent of Espresso.
