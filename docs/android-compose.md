# Native Android client

The Compose client lives in `android/compose`, alongside the existing egui
client in `android/app`. Its application ID is `io.github.chriexpe.papo.compose`
so both can be installed without replacing the existing app or its sessions.

## Build and run

Use the existing Android SDK (API 37), NDK `30.0.16248370`, Android Studio JDK,
Python 3.11+, Rust 1.98+, and `cargo-ndk`:

```bash
# Prepare the shared workspace patch once (also done by the build script).
python3 scripts/prepare-winit.py
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
backdrop behind controls, leaving their labels sharp. The conversation header centers the channel name; tapping its main surface
expands the description, with a 160 dp scrollable cap. Back and People retain
their own actions. Settings are solid grouped surfaces. A Frosted controls toggle provides an opaque fallback; themes follow
the system or can be selected explicitly.

System Back dismisses the active viewer, sheet, attachment menu or keyboard
before leaving the conversation. Back from fullscreen video restores the same
inline player. Back during recording stops capture into an unsent attachment;
the draft is preserved. Conversation Back returns to its Chats or Direct list. Client settings returns
to the destination it opened from; Servers and Direct return to Chats before
the activity can exit.

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

Automatic direct-media links render without card metadata or visible URL text;
captions and other links remain. Known media extensions work before backend
metadata arrives, including HTTP links and signed queries. Authored custom cards,
webpage previews and YouTube keep their own presentation. Holding media opens the
message actions, where each link has its own Copy URL button; Copy text retains
the complete original message.

Images, GIFs, video and audio share one viewer outside the scrolling rows. It
supports bounded pinch zoom/pan, accessibility zoom actions, tap outside to
dismiss, GIF favorites, and Downloads/Papo saves through MediaStore. Publishing
is atomic; interrupted saves remove their pending download. Video initially
plays inline. Its native fullscreen button moves the original PlayerView,
including its existing controls, into the viewer and back. The view and decoder
are reused, preserving position and playback state. Zoom transforms the
video texture while playback controls keep their normal size.

Every open viewer uses Android's FULL_SENSOR orientation policy, following the
device even with system auto-rotate disabled. Closing restores the prior policy.
Compose handles size/orientation changes directly so rotation preserves the
viewer and player. YouTube retains its isolated provider controls.

Coil decodes images/GIFs, and Media3 plays video/audio only after a tap. Leaving
the app pauses playback; disposed players release their resources. Only one
player is active per conversation.
Offscreen video/audio pauses at the readable viewport edge; GIFs, animated
stickers and animated avatars stop their animation clocks there too. A
conversation-owned cache keeps nearby players and decoded images even when
Compose deactivates their rows. Around two screens away they are released;
playback positions and manual pause intent survive reconstruction. Measured
row heights determine distance, with an average-height estimate for unseen
rows after large jumps. Retention is also capped at three players and 96 image
occurrences; playback bookmarks are capped at 128. Decoded animated images
are independent per occurrence, so pausing one sticker cannot stop another.
Profiles and fullscreen media pause the timeline behind them.
Audio uses compact Compose play/pause, seek and time controls without a video
surface. Its seek control draws a real decoded waveform before playback, with
played/unplayed progress, touch and accessibility seeking. PCM peaks include
all channels; silence stays flat. At most two decoders run, work cancels when
unused, audio/video ranges and account-scoped waveform envelopes share the media cache. M4A/M4B audio names correct the backend’s broad `video/mp4` container
label in the shared core; actual MP4/M4V video keeps its video presentation.
Files download on demand to private cache and open via Android’s file chooser.

Papo media uses authenticated attachment/thumbnail and embed-video relay routes.
Embed images come through the core’s authenticated preview endpoint, with a
bounded compressed image cache scoped to server, account and revision. Native
streaming preserves byte-range requests. The media HTTP client pins credentials
to one server origin and disables redirects. YouTube runs in an isolated WebView
without Papo headers or a JavaScript bridge; arbitrary iframe URLs are not run.

## Attachments and capture

The separate gallery squircle opens Android’s image/video picker on tap. A
long press rolls up Record audio, Choose files and Camera actions. Its ripple
is clipped to the same squircle. The composer placeholder stays on one line,
with ellipsis for long channel names; typed messages can still grow normally.

The gallery and document picker grant access only to selected items. Capture
uses the installed camera app and an app-private FileProvider destination.
Voice notes request microphone permission and replace the textbox with an
inline recording bar: discard, elapsed time, measured microphone waveform,
and Send. The waveform uses actual recorder peak amplitudes sampled every
80 ms. Send stops and attaches AAC/M4A audio before dispatch; leaving the
foreground stops recording and keeps the file in the draft without sending.

Up to ten attachments can be staged. Private file copies and their metadata
are saved per server/account/conversation, including across process death.
Files and captions remain until the shared core confirms the multipart POST.
The in-flight marker is saved before dispatch: an interrupted upload becomes
an uncertain draft on restart and is never automatically resent. Retrying an
uncertain upload requires an explicit duplicate-delivery confirmation. Offline
attachment sends keep the files available; durable text sending is unchanged.

The core now classifies multipart failures: local preparation and documented
rejections are definite, while lost responses, decode failures and server errors
can have an unknown outcome. Demo attachments stay on-device, render through
the same message views, and are discarded when the demo closes.

## Emoji, mentions, GIFs and reactions

The composer has separate emoji/sticker and GIF buttons beside Send, centered
vertically even with multiline input. The shared emoji picker presents server
emoji first, including animated images, followed by searchable Unicode emoji.
Typing `@` opens matching display names/usernames; typing `:` opens matching
server stickers. Both suggestion lists scroll within a bounded glass panel.
Selection replaces the token at the cursor, inserts exactly one trailing space,
and keeps editing focus. Mention labels are bold/accent-colored and retain
stable user IDs, including duplicate display names and UTF-16/Unicode offsets.
Bindings survive draft restoration and are invalidated if their label is edited.
The wire format comes from core’s mention encoder, including in direct messages;
DMs no longer discard the selected identity. Sticker tokens render as real
animated images in the editor’s measured text layout; cursor mapping, wrapping,
scrolling and atomic backspace use that same layout. Images stop animating when
the composer leaves the foreground. Decode failures retain editable `:name:` text.

The GIF picker has Favorites, Recent and Trending tabs, search and pagination.
GIPHY transport lives in `papo-core`; stable `giphy:<id>` messages remain compatible
with egui. Set `PAPO_GIPHY_API_KEY` in the build environment for live search/trending;
Gradle fingerprints configuration changes so Cargo rebuilds when it changes.
No borrowed key or fake live results are shipped. Without a key, the picker
explains availability while Favorites and Recent remain usable. The explicit
demo supplies a local provider fixture for testing.

GIFs have a star in chat and beside Close in fullscreen, without a background
circle. Favorites and Recent persist per server/account. Provider favorites store
IDs and rendition metadata; uploaded/relay GIFs retain a private file copy so they
can be attached again even if the original message is deleted. Recent is updated
after a send is accepted, excluding selected attachments removed before sending.
The picker stages content in the existing draft; it never sends on selection.

Tapping a sticker in chat opens its details sheet: animated image, name,
Favorite and Copy name. Copy name copies the exact server name without colons;
favorites persist per server/account. Holding a message shows favorite stickers
first, then the five most-used emoji (including phone and animated server emoji).
The row scrolls when needed, and More reactions opens the same picker. Tap a reaction chip
to add/remove yours; hold it to see people, with pagination. Only your own reactions
have the blue background. The live adapter updates its own-reaction flag together
with the count, serializes HTTP changes per message/emoji, coalesces rapid taps,
and restores the previous state on rejection. Loopback HTTP regressions exercise
POST → DELETE → POST, custom payloads, rapid taps and failed removals.

## Reply navigation

Both the reply preview inside a message and the active reply strip above the
composer open the original. Visible originals stay in place; clipped rows move
only enough to clear the floating controls, and offscreen originals scroll into
view in one animation. The message box receives a brief accent highlight, which
fades after 1.8 seconds. Repeated taps
highlight again. Jumping hides the keyboard/suggestions and preserves text,
mentions, attachments and the selected reply.

Loaded originals open immediately, including offline. Older originals use the
shared core’s cancellable message-context request and reconciliation tickets;
no second history fetcher lives in Kotlin. A loading pill offers Cancel, and
missing/deleted originals report a clear error. Older contexts expose Load later
messages plus Latest messages to return to the live window. Sending successfully
also returns to latest. Navigation commands are scoped to their conversation so
a delayed callback cannot reset another conversation’s history.

## Scope

Included: server setup, authentication, channels, DMs/member list, message
history, offline cache, text/mention drafts, durable text sends, replies,
Unicode/custom reactions and reactor lists, server stickers, GIF pickers,
inline suggestions, edit/delete, themes, glass controls, image/GIF viewing with zoom, native video/audio playback,
file downloads/uploads, gallery selection, camera capture, voice notes,
link/custom cards, and opt-in YouTube playback.

Still to port: full Markdown, search,
voice/video calls, FCM/background reconciliation, notifications, the full client
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
restrictions and redirect behavior. Attachment tests cover tap/hold actions,
file and photo picker results, camera capture results, real microphone encoding
and playback, and draft recovery after an interrupted upload. Picker/camera
results use local test fixtures; no test messages are posted to a live server.
Core HTTP fixtures verify multipart bodies and failure classification.
Android 17 needs
the test-only Espresso 3.8 alpha because older Espresso reflects a removed
InputManager API. Production dependencies remain independent of Espresso.


### User profiles

Names and avatars in chat, People and reaction lists open a native profile
sheet. DM-list avatars open it too; the settings account row opens the editable account page. The sheet shows the
server avatar/banner, nickname, username, status, roles, bio and member date,
with a scrollable body and a direct-message action for other members. Holding
a chat author inserts a bound mention at the composer cursor, including the
required spacing. Rendered mentions also open profiles.

UniFFI projects profile metadata and avatar revisions; encoded avatar bytes
are fetched on demand from papo-core and cached per server/account/user/revision.
Banner requests use the authenticated content-addressed media route. Missing
or unavailable avatars keep the initials fallback.

Wormhole/webhook authors retain their external identity independently of the
bridge account: supplied avatars, an APP badge, integration attribution and a
separate external-profile sheet. They never issue a Papo profile lookup, DM or
bound mention against the bridge account. Public avatar URLs receive no Papo
credentials; only the documented server-local webhook avatar route is resolved
relative to the connected server. External profile links accept HTTP(S) only.


### Shared media cache

`MediaCache` is the sole Compose client cache for encoded media: native and
integration avatars, banners, stickers, GIFs, attachment images, direct image
links, video/audio ranges, embed image bytes and derived waveform envelopes.
A single Media3 `SimpleCache` has a 256 MiB LRU budget across those resources.
Image decoding uses one shared Coil loader with its separate disk cache disabled;
streaming playback and waveform extraction read through the same cache data source.
Live HLS/DASH playlists bypass storage while their media segments can be cached.
Only consumed video/audio ranges are cached, so starting a long video does not
require downloading it in full. Recently accessed spans survive longest, whatever
their media type; this is a byte budget, not a fixed number of recent messages.

Immutable native blobs use account/resource/revision keys. Public URL images
(including integration avatars) display cached bytes before conditional refresh
with ETag/Last-Modified after 15 minutes. A changed URL is a new cache entry.
Authenticated requests remain pinned to their server origin and account; public
URLs receive no Papo credentials. Custom emoji revisions invalidate stale image
bytes when the server changes them. The shared cache is disposable and does not
contain user-owned GIF favorites or MediaStore downloads. Temporary export copies
exist only to hand complete files to Android document handlers. Provider-controlled
web embeds retain their browser-managed network behavior.

This disk cache is independent of viewport retention: playback and animation
clocks still pause offscreen, and decoded objects/players release around two
screens away. Scrolling far away does not discard reusable disk bytes.

## Community header and search

The conversation list header shows the server icon and name. Tapping it opens
server information, the address copy action, per-channel notification controls
and mark-as-read. It omits the creator attribution. Icons use the same private
media cache and decoder as other images.

The settings gear follows `papo-core`'s owner and role capabilities. General,
Channels and Roles editors appear only for their corresponding permissions;
mutations are validated in the adapter and confirmed by the backend. A normal
member still has the community sheet and notification controls.

The search field finds accessible channels and people locally and messages
through the core's `POST /search` transport. Filter parsing and autocomplete now
live in `papo-core::search`, shared with egui: `de:`/`from:`, `em:`/`in:`,
`tem:`/`has:` and `mentions:`. Quoted channel names, stacked link/attachment
predicates and completion at a UTF-16 caret all use the same grammar. Selecting
a completed filter inserts one space and preserves surrounding text.

Global search does not inherit a selected DM's scope. Pagination uses the
backend cursor and stale responses are rejected through the core's search
generation. A result opens its conversation and jumps to the original message;
Back restores the search query and results. Another Back clears the search.

Chats and Direct share one floating navigation pill. Servers is a separate
button beside it. The redundant Channels/Direct/People filter row is removed;
people and channels remain available through global search (`@` lists people).
The account avatar beside the server pill opens client settings. Search and
list state are kept per destination, server and signed-in account.

## Client settings

The account avatar opens a native settings index. Search matches individual
controls (including descriptions and current values), ignoring case and accents.
A result opens the relevant page, scrolls to the exact setting and highlights it.
Back returns to the search and retains its query.

Account editing loads the complete profile before enabling changes. Nickname,
status message, bio and typing phrase save together; presence, image uploads,
banner removal and passwords have explicit actions. The adapter uses the core
API and reducer, with typed UniFFI calls that acknowledge actual HTTP completion.
Requests freeze their account credentials and reject stale-owner results. Profile
refresh failure after an acknowledged mutation is reported separately. Unsaved
profile edits survive rotation and require a discard decision when leaving.

Appearance settings apply immediately on this device: system/light/dark theme,
frosted controls, message avatars, timestamps, compact spacing, GIF/sticker
animation and an optional six-second channel-description preview. Android controls
font and display scaling. The message preview reflects those layout/theme choices.

Files uses Android's document-tree picker with persisted access, or the default
MediaStore `Downloads/Papo` destination. The shared viewer's download action uses
that single preference and reports the chosen location. A missing/revoked folder
fails rather than silently changing destinations; incomplete copies are removed.

Sessions shows only backend-provided session IDs and creation/expiry dates. It
doesn't invent device names or identify the current session when the API cannot.
Ending a session requires confirmation. Diagnostics exposes actual connection,
shared disk-cache usage/clearing, app/Android versions and Android's app settings.
Clearing media keeps messages, credentials, drafts, favorites and saved downloads.
Desktop tray/startup controls, desktop activity integration, unimplemented language
choices and background notification controls are omitted. Demo has no password,
banner-upload or authenticated-session controls.

The shared Coil decoder retains up to 24 MiB of shareable static images. Stable
account/resource/revision keys let profile images and banners reuse decoded
bitmaps across screens before any disk/native work. Non-shareable animated
images are decoded independently, retaining the existing viewport pause/release
rules. Encoded bytes still use the same 256 MiB shared media cache; no additional
avatar-only disk store is introduced.
