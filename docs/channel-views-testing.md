# App surfaces (plan v7)

There are no channel-view tabs or `views` payloads. App channels occupy the content
rectangle below the normal header, with no timeline/composer. Inline app cards use
the same inline viewport calculation, paint/input, clipping and occlusion data as
rich-link players. One isolated app instance lives beside the existing ActiveEmbed;
activation destroys the other instance. App embeds stop on scroll-out and channel
switch, and never float. Normal rich-link playback/floating preferences remain in place.

## Local smoke

Use the backend's `feat/wasm-plugins` after #18/#19, Postgres 16 and goose migrations.
Set `PLUGINS_DIR` to a disposable folder containing copies of
`examples/plugins/bluemap` and `examples/plugins/reference` (including plugin.wasm).
Follow the backend's `docs/wasm-plugins.md` for starting the server and creating an
owner account. No backend or third-party response headers are changed by this frontend.

```sh
bash scripts/wpe-runtime.sh debug
PAPO_WPE_RUNTIME="$PWD/target/debug/waterui-browser/wpe" \
  cargo run --locked --example channel-views-smoke -- http://127.0.0.1:18087
```

The example launches the **real Papo app** with temporary UI preferences and the
specified server. It has no alternative tab UI or test controls. Use an isolated
`XDG_DATA_HOME`/`XDG_CONFIG_HOME` when testing accounts and local media caches.

1. Sign in as an owner (or with `manage_apps`); open Server settings → Plugins.
   Rescan lists discovered plugins and validation errors. Invalid candidates cannot
   be selected for enablement. Review the backend consent preview before approving.
2. Enable BlueMap with an approved public HTTPS URL. Its avatar/channel appears live.
   Rename through normal channel settings/API; disable and confirm it disappears.
   Channel creation appends to the list; moving later uses normal channel settings.
3. Enable reference with a normal target channel, invoke its trigger using the backend
   test tooling, and open its channel and inline embed. Both must report:
   `CSS, módulo JS, imagem, fonte e fetch JSON carregados.`
4. Activate the inline card next to a YouTube rich-link. Activate each in turn;
   only one browser may remain live. Scroll the app card fully out, return, and confirm
   it requires activation again. Change channels; no app surface may survive.
5. Type a draft in the composer while the app card is live. Verify clicks, scrolling,
   keyboard focus and overlaid header/composer controls remain usable. Disable reference
   while its inline surface is open: it must disappear and the card become inert.
6. Configure a frame-refusing remote page (tested with `https://github.com`), then
   confirm the fallback and the existing external-link confirmation. TLS/load failure,
   missing mandatory controls and the 15-second deadline must also fail closed.
7. On compact layouts, reserve 24-point edge strips and the header for egui/drawer
   gestures. Actual Android/iOS touch hardware was not available in this run.

The official BlueMap demo currently returns `X-Frame-Options: SAMEORIGIN` in this
host's response; it must fall back. Lifecycle checks first used `https://example.com`;
the actual user-hosted map was subsequently tested (see below).

## Native WPE policy and teardown regression

ABI 5 adds an isolated constructor. It creates its own ephemeral network session
and web context, denies permissions/authentication/downloads/popups, locks **all**
navigations to the exact origin, and inspects main-document response headers. Bundled
responses additionally require the backend's opaque-origin sandbox header. No app
API client, cookies, custom headers, injected JS or JS-callable IPC handlers enter it.

`lib/extensions/libpapo_app_requests.so` is a native WebKit request hook that removes
Referer from every request. The page first loads only `about:blank`; the target is
not requested until that native module acknowledges installation. The acknowledgement
is internal WebKit IPC, with no JavaScript binding or application SDK. Missing/broken
modules never allow a weaker embedding policy.

The regression driver needs the distro ATK development package in addition to WPE:

```sh
cc native/wpe/tests/app_surface_lifecycle.c -o /tmp/papo-app-lifecycle \
  $(pkg-config --cflags --libs atk) \
  -Ltarget/debug/waterui-browser/wpe/lib -lwaterui_wpe \
  -Wl,-rpath,"$PWD/target/debug/waterui-browser/wpe/lib"
G_DEBUG=fatal-criticals /tmp/papo-app-lifecycle \
  http://127.0.0.1:18087/apps/INSTALLATION_UUID/ui/result.html bundled
```

It loads/destroys 24 isolated pages and queries accessibility after each destruction.
This covers WPE 2.54's lazy accessibility-root lifetime problem. The bridge materializes
live toplevel accessibles before disposal and owns the reference transferred by the
preferred-buffer-formats vfunc. Signal handlers are disconnected before freeing a page.

## Verification recorded on Linux

- Updated backend: `b013e17`, with real plugin folders, Postgres 16 and goose version 21.
- WPE 2.54.1: index.html and result.html each loaded CSS, ES module, SVG image,
  TTF font and fetched JSON (all HTTP 200, functional success marker, no resource blocks).
- Production shell: app channel, native isolated rendering, inline activation,
  rich-link replacement, scroll-out, channel switching, editable composer, live
  rename/delete and disable-while-open checked. Frame refusal and external-link
  confirmation checked. Screenshots: [channel-apps](screenshots/channel-apps/).
- 24 native lifecycle/accessibility cycles passed with fatal GLib criticals enabled.
- Rust workspace tests and pinned Clippy passed; Python winit preparation tests passed.
- Full workspace `cargo fmt --all -- --check` still reports the inherited formatting
  baseline (the untouched HEAD version of shell.rs alone also fails). New modules and
  touched browser modules were formatted; unrelated files were not reformatted.

## Platform limits

Windows: COM modules type-checked and linted through a Windows-target harness; no
native Windows machine was available. Temporary-profile removal after repeated browser
shutdown, frame/TLS/timeout handling, native focus/DPI, and permission/download denial
remain unverified at runtime. Closing-browser events use a separate queue from the next
surface, so late callbacks cannot approve a newer surface.

macOS 12+: app surfaces use a separate `WKWebsiteDataStore.nonPersistent()` and no
script injection or `WKScriptMessageHandler`. Navigation actions are origin locked;
main response headers enforce frame refusal and bundled sandbox policy. Camera and
microphone requests are denied by `WKUIDelegate`, downloads are cancelled, and popups
are denied/external. Location authorization is prevented by the absence of every
`NSLocation*` Info.plist key: packaging, CI, regression tests and the runtime guard
reject those keys. Clipboard remains mediated by WebKit's user gesture/paste
confirmation. Below macOS 12, use fallback. The macos-15 CI job has compiled and
packaged the native implementation; **no Mac was available for runtime/UI testing**.

Android: real embedding requires runtime support for both
`WebViewFeature.MULTI_PROFILE` and `DELETE_BROWSING_DATA` (AndroidX WebKit 1.13.0).
Each surface gets a unique profile, never reused. On destroy its WebView is destroyed
and all cookies, cache and site storage are erased asynchronously. Chromium cannot
delete a profile record loaded in the current process; the empty record is removed
at the next process startup. This lifecycle adjustment was explicitly approved after
testing the device limitation. Otherwise use fallback.
No JavaScript interface is installed. Navigation/resources are origin locked;
permissions, geolocation, file selection, downloads and popups are denied. Remote
frame headers are probed with a credential-less GET, disconnected after headers;
this replaces HEAD because the real BlueMap server returns HEAD 400 but GET 200.
Redirects, errors, timeouts and embedding-refusal headers cause fallback. Bundled
surfaces skip this probe and validate the actual document's sandbox response.

An unrelated, pre-existing UTF-8 slicing panic in the rich-preview fetcher was observed
while fetching YouTube metadata. It did not prevent native YouTube activation or the
chat/composer smoke; this change does not alter that resolver.

Revocation uses the backend `app_invalidate` lifecycle event (backend commit
[`732723c`](https://github.com/chriexpe/papo-backend/commit/732723c)), restricted to
current readers of app channels or channels containing that installation's inline
embeds. Disable, removal, quarantine and approval replacement destroy the matching
live surface immediately and make loaded cards inert, including inline-only plugins.
Cached embed payloads cannot authorize activation; a fresh server payload is required.
Backend ACL delivery, inline-only audience and lifecycle ordering have regression tests;
the backend's full tests, race tests, build and vet pass.

Focused WPE app surfaces forward native key-down/up events, modifier combinations,
function/navigation keys and physical key codes. Tab/arrows/Escape stay with the page;
blur/hide destroys held-key state by sending releases. Key/Text events are deduplicated.
Translation/color-layout tests pass. The user confirmed keyboard input in the actual
BlueMap smoke; modifier combinations and Windows keyboard behavior remain unverified.
Pointer-down/drag also takes keyboard focus, avoiding a click-only focus delay.

Actual BlueMap 5.11 at `https://map.chriexpe.com` rendered in the app channel. The user
confirmed noticeably better panning/zooming after bulk-copying GPU rows into RAM before
colour conversion. Host CPU/readback remains a performance limitation; matching WPE to
the host GPU alone did not fix it. Direct GPU import is a separate renderer follow-up.

## C6: plan v7 app interactions (Linux, 2026-10-16)

Frontend verification uses backend `feat/wasm-plugins` after C4/C5, including
`71af671` (authorized launch metadata and bundled-poster consent validation).
The reference WASM tic-tac-toe plugin runs against a disposable PostgreSQL database
and two independent native Linux clients/accounts. Additional copies of its manifest
exercise inline `click`, inline `visible`, and bundled SVG posters without changing
the reference game module.

Verified in the native production shell:

- Member list hidden by default in app channels; header toggle and device-local,
  per-channel persistence. Only other members receive the plugin's native action.
  Actions sit in the member row as compact icons with the plugin label as tooltip.
- Member action → invite notification → accept → both users play. A completed
  X-winning game followed by the native **Revanche** button resets the board and
  updates the message/buttons through `message_edit`.
- Invite avatar/inviter/text, accept/decline, and mute. Mute persisted through the
  backend user-settings endpoint; accepted/read invites stay hidden on reconnect.
- Inline poster stays idle until click, payload proportions preserved, no nested
  provider header/play decoration. `visible` waits 300 ms; the observed first launch
  followed message delivery by approximately 559 ms, with no repeated idle launches.
  Scrolling away or switching channels destroys the live surface.
- Overlay opens from its card and `opens=` button, has a thin Papo header and channel
  margins, and closes with close/Escape. Launch metadata also supports buttons whose
  surface has no corresponding frame in the message.
- Only the latest app activation remains live. Disabling the plugin during play
  destroys both clients' surfaces, disables message buttons, and removes its channel.
- Opening a member profile preserves the app texture behind the profile while
  blocking app input. A loaded native YouTube player also remains visible behind
  the profile; actual video playback was not part of this regression check.
- Tickets enter only the `#papo_ticket=` fragment after authenticated native launch.
  Native Papo credentials are never passed to the embedded page.

Automated validation: 624 Rust tests passed (one existing integration test ignored),
10 Python window-input tests passed, pinned Rust 1.98 workspace/all-targets Clippy
with warnings denied passed. Backend full Go tests, services/handlers race tests,
and `go vet ./...` passed. The WPE runtime was rebuilt with ABI 5 and strict C warnings.
Unit tests cover visibility debounce, stale replies, revocation, timeout, ticket URL
handling, invite hydration, and button edits including explicit empty components.

Screenshots are in `docs/screenshots/app-interactions/`. Physical mobile back gestures
and native Windows/macOS surfaces were not exercised in this Linux smoke; the platform
isolation/fallback limitations described above still apply. The original `test/pr100`
checkout and its two modified files were left untouched.

## C7: Android and macOS embedding (2026-10-16)

| Platform | Implementation | Verification |
|---|---|---|
| Android 16 / GM1910 / System WebView 154.0.8037.106 | Isolated profiles, origin lock, no JS bridge, permission/download denial, frame refusal and fallback | Debug APK built and installed on the user's phone; real embedding exercised. Full security/permission matrix remains unverified. |
| macOS 12+ | Ephemeral WKWebView, navigation/response delegates, no JS bridge, media/download denial, location packaging guard, user-mediated clipboard | macos-15 CI native build and packaging passed; see PR checks for the latest revision. Runtime/UI **unverified**: no Mac available. |
| macOS <12 / Android missing required WebView features | External fallback | Runtime checks implemented; older devices unavailable. |

Phone smoke used the same disposable backend `feat/wasm-plugins` as C6 and separate
Alice/Bruno accounts on Android and Linux. Inviting from the member row, accepting the
notification and exchanging game moves worked. Native inline and overlay reference
surfaces rendered; card and `opens=` activation were exercised, along with keyboard
Tab/Enter input, background/resume and the overlay Back path. Revanche and plugin-disable destruction were verified in the C6 Linux smoke; repeat
those steps on Android when checking lifecycle variants.

The user confirmed BlueMap panning works after the viewport fix. Navigation now waits
until the native view has real dimensions, avoiding BlueMap camera NaN/Infinity from
an initial 1x1 viewport. App WebViews keep page gestures. Native occlusion rectangles
clip both drawing and hit testing, so the timeline jump button no longer hides the
whole inline surface. Profile cleanup callbacks reported data erasure; subsequent
process startup removed stale profile records. Storage isolation across every browser
API and all permission variants were not exhaustively probed.

Screenshots: [BlueMap](screenshots/app-platforms/android-bluemap.png),
[inline reference](screenshots/app-platforms/android-inline.png),
[overlay reference](screenshots/app-platforms/android-overlay.png).

### Reproduce on Android

1. Build with `bash scripts/android.sh`, then
   `android/gradlew -p android :app:assembleDebug -PpapoDebugSuffix=.appsurfaces`.
   Install `android/app/build/outputs/apk/debug/app-debug.apk`. The separate
   **Papo surfaces test** application preserves the normal Papo install and its data.
2. For a local development backend, use `adb reverse tcp:18096 tcp:18096` and set
   `http://127.0.0.1:18096` in the debug application. Production remote surfaces still
   require HTTPS. Sign into two independent accounts on the phone and a second client.
3. Enable the reference plugin and BlueMap, open BlueMap, then pan/zoom, background
   and resume. Check keyboard focus/input and that switching channels destroys the map.
4. Show members with the header button, invite the other user using the row action,
   accept their notification, play from both clients, then press Revanche.
5. Exercise inline click and visible variants (visible launch waits about 300 ms),
   overlay from its card and from an `opens=` button. Back/close must dismiss the
   overlay without leaving the channel; switch channels and verify it is destroyed.
6. Disable the plugin while playing: surfaces must be destroyed and message buttons
   disabled. Re-enable and verify a fresh launch. Repeat on a WebView without the two
   required features to verify external fallback.

Automated regression checks: `python3 -m unittest discover -s tests -p
"test_app_surfaces.py"` checks forbidden location keys and runs the pure Java
HTTP/header policy against a local server (no cookies/auth/referrer/fragment,
redirect/status/header refusal, and disconnecting without consuming a streamed body).
