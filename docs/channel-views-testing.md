# Passive channel views

Companion backend: [chriexpe/papo-backend#18](https://github.com/chriexpe/papo-backend/pull/18).
The client accepts `views` on channel payloads, including the backend's optional
`name` field. Older payloads display the URL hostname. View metadata is not cached
on disk: restoring a cached channel does not restore an old authorization to a view.

## Platform policy

Only Windows currently constructs an embedded channel view. It requires a dedicated
WebView2 environment and temporary user-data directory, an InPrivate controller,
disabled SSO/host objects/web messages/autofill/password saving/dialogs, and all
security event interfaces. Initialization failure selects **Open in browser**.

The Windows path never uses the authenticated API client or rich-link constructor.
It uses ordinary `Navigate`, adds no request headers, and removes `Referer` through
`WebResourceRequested`. `NavigationStarting` restricts only the main frame to the
initial exact HTTPS origin; subframes and subresources keep their normal origin
behavior. Popups, downloads, permission requests and HTTP authentication are denied.

`WebResourceResponseReceived` checks the current main document, classified using the
browser-owned `Sec-Fetch-Dest: document` request metadata. Iframes and fetches are
not classified by URL alone. Missing main-document classification or a failed
header read prevents readiness and selects fallback. The view stays hidden until
both response checks and successful navigation completion have been observed.
`X-Frame-Options: DENY/SAMEORIGIN`, any CSP `frame-ancestors` value other than exactly
`*`, HTTP errors, navigation/TLS/process failures, and a 15-second load deadline
select fallback. Redirects do not extend that deadline. No script is injected.

Browser-only platforms never construct a channel-view WebView:

- **Linux/WPE:** the available WPE 2.0 navigation action API does not expose a
  reliable main-frame flag. The frame name describes a navigation target, not
  main-frame identity. A response-only check would act after a cross-origin request;
  locking all navigation actions would also police subframes. Neither meets the
  requested policy. See the public [navigation action implementation](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/glib/WebKitNavigationAction.cpp).
- **macOS:** permission denial cannot be guaranteed for all requested permission
  classes on supported OS versions using the existing public WKWebView API surface.
  For example, Apple's public [geolocation permission delegate](https://developer.apple.com/documentation/webkit/wkuidelegate/webview(_:requestgeolocationpermissionfor:initiatedbyframe:decisionhandler:))
  is documented as beta. The existing media-only delegate is insufficient.
- **Android:** the existing WebView shares process storage. A follow-up should
  investigate AndroidX WebKit multi-profile support, checking
  `WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)` before accessing
  [`ProfileStore`](https://developer.android.com/reference/androidx/webkit/ProfileStore).
  API availability alone does not prove non-persistence; profile deletion/lifetime,
  cookies, storage and cache isolation must be verified too. Not part of this PR.

## Reproducible native smoke fixture

```sh
python3 scripts/prepare-winit.py
cargo run --locked --example channel-views-smoke
```

This fixture uses the real shell, tab UI, draft handling, rich-link manager and
channel-view implementation, with an in-memory store and no Papo credentials.
It does not test a live backend or private-channel authorization.

1. Select **Public** (`https://example.com/`) in channel A. On Windows expect either
   the page or fail-closed fallback if a mandatory control is unavailable. On
   browser-only platforms expect **Open in browser**, without a WebView.
2. Return to **Chat**, enter a draft, select the view, type again, then return to
   **Chat**. The draft must remain intact, without the view's keystrokes. Composer
   controls must remain clickable. Do not send real messages.
3. Switch to channel B and select **Frame refusal** (`https://github.com/`). Confirm
   its current response headers before testing; GitHub supplied both
   `X-Frame-Options: deny` and CSP `frame-ancestors 'none'` during the Linux smoke.
   Expect fallback. On browser-only platforms this validates platform fallback,
   not native response-header inspection.
4. Use **Revoke (channel_update)** while viewing the page. The view tab must
   disappear immediately and the chat composer return. Use **Restore** to repeat.
5. On Windows, toggle **Overlay**, switch tabs/channels repeatedly, resize the
   window, move between monitors with different scaling, minimize/restore, and
   close while loading. The browser must not cover chrome or retain keyboard focus.
   Main-frame cross-origin links/redirects, popups, downloads, custom schemes,
   camera/microphone/geolocation/clipboard requests must never escape the policy.
6. Test an unreachable HTTPS site and a slow page; the load deadline must produce
   fallback. Test a same-origin page containing cross-origin subframes/resources;
   these must not be subjected to the main-frame navigation lock or framing check.

For a live backend, repeat with two actual channels (including one private), revoke
the installation/remove its scope, and confirm the `channel_update` event and new
channel payload remove its tabs. Test rich-link activation/playback on **Chat** as
well. A metadata request started before revocation must not restore the old tabs.

## Windows profile cleanup

The view retains a real browser process handle. On destruction it closes the
controller, releases its environment, waits for that browser to exit on a cleanup
thread, and then deletes the unique temporary directory, retrying transient file
sharing errors. Shutdown joins these workers; waits are bounded and failures are
logged without deleting a directory while its browser is still running. The
rich-link profile is untouched.

On a native Windows host, record the initial set of directories:

```powershell
Get-ChildItem -LiteralPath $env:TEMP -Directory -Filter 'papo-channel-view-*'
```

Open/close or switch between the two views at least 20 times. After each browser
process exits, rerun the command and verify no directories added by the test remain.
Also test shutdown during a load. This is a required runtime check; cross-compiling
the Windows module does **not** verify profile isolation or directory removal.

## Checks

```sh
cargo fmt --all -- --check
cargo +1.98.0 clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
python3 -B -m unittest discover -s tests -p 'test_prepare_winit.py'
```

Repository-wide formatting already fails on the base branch. Keep that result
separate from formatting of the new modules and do not count it as passing.
