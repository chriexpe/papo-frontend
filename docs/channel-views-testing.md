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

macOS: app surfaces use the explicit cfg fallback. Nonpersistent WKWebsiteDataStore
exists from 10.11; native response-policy inspection is available from 10.10;
camera/microphone denial is public from 12.0. The public geolocation-denial delegate
is declared for **27.0**, and orientation/motion permission is unavailable on macOS.
Header modification in WKContentRuleList has SPI requirements; this PR does not use
private selectors or inject scripts to compensate. No supported/verified public
combination currently enforces the complete policy, so no app WKWebView is created.
Native macOS compilation/runtime and any future SDK combination remain unverified.
Sources: [WKUIDelegate declarations](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKUIDelegate.h),
[modify-headers SPI](https://bugs.webkit.org/show_bug.cgi?id=237784).

Android: browser-only fallback. Follow-up (outside this PR): verify availability of
`WebViewFeature.MULTI_PROFILE` and AndroidX WebKit
[ProfileStore](https://developer.android.com/reference/androidx/webkit/ProfileStore),
then prove ephemeral storage/cleanup and the remaining controls. No Android device
was connected; rules 1–5 are not claimed as enforced by an embedded Android surface.

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
