# Native Android client

The official Android client lives in `android/compose`; the egui Android shell
remains in `android/app` for reference. Signed Compose releases keep the existing
`io.github.chriexpe.papo` package and release key. Debug builds use the
`.compose` suffix, so existing Compose development installations keep their data
and can run beside the official app.

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
the APK. Generated sources stay under `compose/build/generated/uniffi/{debug,release,benchmark}`. A direct build supports both ABIs by default:

```bash
cd android
./gradlew :compose:assembleDebug
# Or only the emulator ABI:
./gradlew :compose:assembleDebug -PpapoAbis=x86_64
```

The APK is `android/compose/build/outputs/apk/debug/compose-debug.apk`.
`scripts/android-compose.sh --arm64 --release` builds the official release
variant. Signing uses the existing `PAPO_ANDROID_KEYSTORE`,
`PAPO_ANDROID_STORE_PASSWORD`, `PAPO_ANDROID_KEY_ALIAS` and
`PAPO_ANDROID_KEY_PASSWORD` inputs. Without a key, a local release APK is unsigned.
The tagged release workflow now builds and signs Compose with the existing
artifact name, package ID and Firebase secrets; it requires complete Firebase
configuration. PR builds upload a separate debug APK.

For scrolling measurements, `./gradlew :compose:assembleBenchmark -PpapoAbis=arm64-v8a`
builds optimized Kotlin and release-profile Rust with shell profiling enabled.
This non-debuggable variant uses the development package ID, Firebase identity
and debug signing key, so it can replace a local debug installation while
preserving its data. Its APK is `compose/build/outputs/apk/benchmark/compose-benchmark.apk`.
The normal release variant does not enable shell profiling.

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
  excluded from backup. Official upgrades import egui's private `data/app.ron`
  server list and compatible preferences, and copy session/server-password files
  directly into encrypted credential storage without returning secrets to UI.
  Existing Compose credentials are never overwritten. The old
  `data/papo-cache.db` is reused when no Compose database already exists, keeping
  cached history, read state and the outbox. Live and headless paths share the
  same database selection. The import is idempotent and source files remain intact.
- The composer keeps its draft until the core acknowledges durable outbox
  storage. Failed enqueue leaves the text available. Ambiguous outcomes require
  a deliberate retry, with a duplicate-delivery warning.
- Read state advances from actually visible message IDs while the Activity is
  resumed. Opening a conversation alone does not mark the whole channel read.
- Network availability/path changes are sent to the shared transport, and
  foreground return probes the connection. Native notifications reuse core policy
  and durable deduplication. Firebase handles background push when configured;
  otherwise WorkManager runs bounded REST reconciliation.

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

Tapping a pending attachment opens its options sheet: rename the stem while
keeping the extension, generate a stable random name, mark it as a spoiler,
optionally strip image metadata, or open the existing crop editor. Apply commits
all options/edits together; Cancel leaves the durable draft unchanged. Options
survive channel changes and process death. Unsupported file types omit metadata
removal; renaming and spoiler marking remain available. Metadata removal keeps a derived
private copy alongside its source until the attachment is sent/removed; turning
it off restores the source. Failed processing never falls back to sending it.
PNG, JPEG, WebP and GIF use the shared core pipeline (also used by egui), applying
EXIF orientation and retaining animation frames/delays. Animated WebP/APNG export
as GIF. Re-encoding may change bytes, color profiles, format or size; it does not
resize the image to fit the metadata-removal budget. Unsupported/oversized inputs
fail explicitly. Cropping separately follows the existing editor budgets.

Spoilers use an explicit `SPOILER_` filename convention recognized by Compose
and egui, independently of moderation status. The backend currently ignores the
previous `spoiler_attachments` field; the upload now uses the preserved filename
instead. The UI previews the actual outgoing name. Other clients may display
these attachments without concealing them; this compatibility limit is shown
in the sheet. Incoming spoiler media is concealed before rendering/playback.


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
Editing uses the same bound composer: core decodes canonical mentions to readable
labels with saved user IDs, and Kotlin preserves them through selections, formatting,
suggestions and recreation. Duplicate labels cannot exchange identities when one is
deleted. Unknown/departed users retain their original binding using the ID as a label.
The normal unsent composer draft stays separate from the message editor.

New replies initialize their @ toggle from **Notifications → Mention when replying by
default**. The per-reply choice survives recreation without changing the device default;
text outbox entries and attachment uploads both carry it through UniFFI to `notify_reply`.
Existing canonical mentions in the message body are independent of this reply toggle.

The wire format comes from core’s mention encoder, including in direct messages;
DMs no longer discard the selected identity. Sticker tokens render as real
animated images in the editor’s measured text layout; cursor mapping, wrapping,
scrolling and atomic backspace use that same layout. Images stop animating when
the composer leaves the foreground. Decode failures retain editable `:name:` text.

Messages use the same `papo-core` CommonMark parser as egui: emphasis,
strikethrough, headings, nested quotes/lists, task lists, tables, rules, code
and fenced code with copy actions. Mentions stay bound to IDs even when names
contain Markdown characters, and custom animated stickers remain interactive.
HTML is discarded and link schemes are checked. Markdown images use the shared
media cache/viewer and server dimensions; code URLs remain literal instead of
creating inline players. The native snapshot contains the parsed layout, with
a bounded account-scoped parse cache, so parsing does not cause a later reflow.
Excessively nested Markdown falls back to readable literal text.

Composer auto-pairing also lives in core. Typed backticks, bold/strikethrough
and the existing egui markers receive closers; typing a closer advances through
it, Backspace removes empty pairs, and hardware Tab exits a pending pair. IME
compositions and paste preserve native text and selections while tracking an
existing closer. Programmatic inserts/reset do not infer formatting pairs.
Android converts UTF-16 cursor positions to core's Unicode scalar offsets;
formatting edits retain mention bindings and atomic sticker deletion.

The chat header has a pin button immediately left of People. Pins open an
expandable sheet with full Markdown, playable attachments and supported provider
embeds. Tapping the card outside media and author controls jumps to the original
and highlights it through the existing reply/history navigation. Pin/unpin actions only appear
with the backend's pin permission (or DM participation); mutations await server
acknowledgement and reconcile an authoritative pin refresh. Cached pins remain
visible on a failed refresh, with an inline Retry action. Live pin events and
reconnection refresh the visible list, including pins outside loaded history;
no manual refresh button is needed. Opening the sheet does not acknowledge messages or
alter a saved reading position/catch-up block.

Search results use the same complete message body, media components, player,
viewer, URL-copy actions and viewport retention as chat and pins. The backend's
text search response is hydrated only for visible results, with at most two
requests in flight through the core API. Core retains up to 128 detached result
snapshots; these never create continuous history coverage or read receipts.
Late responses cannot overwrite a changed query/account, deletion or live edit;
WebSocket media/moderation/pin updates also reach detached results. Provider
WebViews retain their inline controls and the existing provider restrictions.

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
missing/deleted originals report a clear error. Older contexts page automatically
in both directions; Latest messages returns to the live window. Sending successfully
also returns to latest. Navigation commands are scoped to their conversation so
a delayed callback cannot reset another conversation’s history.

## Cache-first startup and history

Startup metadata restores the bounded Turso profile-image and emoji tables before
starting the transport. The first projection can identify the saved server icon,
avatars and custom emojis without waiting for REST bootstrap. Authoritative
updates still replace changed assets and remove deleted ones. The 128-avatar
retention limit protects the signed-in user's avatar. Server icons and avatars
use stable server/account/revision identities in Coil's shared bitmap cache.
Profile banners consume the first cached image emission rather than waiting for
the image flow to finish refreshing.

Chat history has no Load earlier/later buttons. A `snapshotFlow` observer requests
pages near either end of the reverse-layout list, without feeding pixel offsets
into message composition. Requests are scoped to server, account and channel and
wait for initial positioning and hourglass navigation. Stable message keys retain
the visible anchor when pages are added at either end.

Online paging starts HTTP and reads a connected local page before publishing.
Cached rows are provisional and never advance the HTTP cursor. The database must
contain the exact timestamp/nanosecond/ID anchor in continuous history before it
can extend a window; a detached pin or disjoint cache is not a bridge over a gap.
HTTP pages reconcile edits and missing rows, then replay concurrent live mutations.
Detached reply windows keep separate cursors for both directions and do not become
continuous cached history. Offline paging continues through retained local rows.
Retention remains 500 confirmed messages per channel, with 100-row database pages.

Unchanged message upserts skip SQL updates while still promoting newly proven
chronological rows. Unchanged member/profile refreshes skip member-table rewrites.
The existing presentation retention and state publication keep equal message and
resource objects, and unchanged public-image revalidation emits no replacement
bytes. Attachments, images/GIFs, external avatars, audio/video ranges and waveform
data continue using the existing shared, account-scoped 256 MiB media LRU; a
restart does not discard it. This is functional cache/pagination work, not a claim
that physical-device scroll performance has been resolved.

## Scope

Included: server setup, authentication, channels, DMs/member list, message
history, offline cache, text/mention drafts, durable text sends, replies,
Unicode/custom reactions and reactor lists, server stickers, GIF pickers,
inline suggestions, edit/delete, themes, glass controls, image/GIF viewing with zoom, native video/audio playback,
file downloads/uploads, gallery selection, camera capture, voice notes,
link/custom cards, and opt-in YouTube playback.

Still to port: voice channels and voice/video calls; custom embed authoring/editing;
and the other remaining parity gaps. Server administration now includes channel/category creation and ordering,
roles and membership, moderation, stickers and audit (see below). Background delivery
still needs a real configured Firebase
project/device check; demo fixtures cannot validate external FCM delivery.

Demo checks presentation and local interactions; it cannot validate backend
authentication, WebSocket delivery, or reconnect behavior. Verify those against
a real server before using this as the primary client.

## Checks

```bash
cargo test --locked -p papo-core -p papo-android --lib
cargo clippy --locked -p papo-android --all-targets -- -D warnings
./android/gradlew -p android :compose:checkComposeFormatting
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
Core HTTP fixtures verify multipart bodies and failure classification. A local
REST/WebSocket fixture also exercises actual incoming messages from another
user: single arrivals, pairs, tall media, late embed resizing, keyboard resizing
and the first-new shortcut. The new-message divider follows core's unread
boundary rather than retaining a Compose-local arrival marker after receipt.
 Pins and
search checks cover permission gates, full detached snapshots, concurrent pin
updates, source navigation, independent author controls and shared inline /
fullscreen playback. Browsing result lists never establishes history coverage
or acknowledges unread messages.
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


### Forgetting saved servers

Every saved server has an overflow action to forget it, including signed-out entries
and the final server. Confirmation removes its local credentials, SQLite namespace,
message/mention drafts, pending private files, saved reading positions, expression
favorites/recents, cached media/waveforms, push registration and notification jobs.
Accounts and messages on the remote server are untouched. Explicit MediaStore downloads
remain user-owned files outside Papo's private storage.

The network worker is stopped and joined before deleting durable state. A persisted
cleanup tombstone prevents reconnecting after process death until deletion is retried;
other saved servers remain usable if one cleanup fails. Legacy egui settings, session
files and server cache directories are cleaned too, without replacing unrelated settings.

### Shared media cache

`MediaCache` is the sole Compose client cache for encoded media: native and
integration avatars, banners, stickers, GIFs, attachment images, direct image
links, video/audio ranges, embed image bytes and derived waveform envelopes.
A single Media3 `SimpleCache` has a 256 MiB LRU budget across those resources.
Keys include the server namespace and a revocable generation. Forgetting closes its
readers and rejects late writes without adding another cache or evicting other current
server namespaces. The older unscoped v1 cache is discarded once because its owner
cannot be recovered; v2 remains one shared LRU store.
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
Desktop tray/startup controls and desktop activity integration are omitted.
Notifications uses real Android permission/channel settings
and reports the active delivery method. Demo has no password,
banner-upload or authenticated-session controls.

The shared Coil decoder retains up to 24 MiB of shareable static images. Stable
account/resource/revision keys let profile images and banners reuse decoded
bitmaps across screens before any disk/native work. Non-shareable animated
images are decoded independently, retaining the existing viewport pause/release
rules. Encoded bytes still use the same 256 MiB shared media cache; no additional
avatar-only disk store is introduced.

## Language, external links and recovery

Language settings offer System, Português (Brasil) and English, including from
the welcome screen before joining a server. Android 13+ uses per-app locales;
Android 12 persists the same preference and recreates the Activity with the
selected configuration. App labels, dialogs, accessibility descriptions and
counts use Android strings/plurals. Server content, custom typing phrases,
protocol values and identifiers retain their original values. Notification
channel names update without resetting the user's notification settings.

External HTTP(S) and email links share one confirmation path through Compose's
URI handler, including Markdown and links leaving a provider WebView. The
dialog shows the full URL and canonical hostname. Trust applies only to an
exact hostname; subdomains are separate. Links settings can add/remove trusted
domains or disable confirmation. File downloads and in-app media viewing keep
their existing behavior.

Welcome and sign-in offer account recovery. The app accepts the backend's
`https://papo.cyberasilo.online/passwordchange/<token>` and
`papo-reset://passwordchange/<token>` links. The user explicitly confirms the
server; a server hint cannot silently redirect the request to another backend.
The public core reset route sends no signed-in cookies and does not follow
redirects. Tokens and passwords are never saved in preferences, navigation state
or diagnostic actions. A reset ends the previous sessions; sign in afterward.

A server owner can generate a single-use reset link from another native member's
profile. This is owner-only in the backend, independent of assignable role
permissions. Generation requires an explicit action and copying requires a
separate tap; nothing is sent to the member automatically. Recovery validation
and transport failures cross UniFFI as semantic variants translated by Android.
Backend-supplied problem details remain server text.

## Compose maintenance

Run `./android/gradlew -p android :compose:formatCompose` before committing.
The pinned ktfmt formatter uses Kotlin style across checked-in module sources,
instrumentation tests and the module build file. Generated UniFFI sources are
excluded. `checkComposeFormatting` verifies the same files without writing them;
Gradle `check` and the PR's dedicated formatting job enforce it.

`ClientSettings` owns settings navigation and search; individual pages and shared
controls have separate files. Role permissions and colors, message author rows
and reactions use components with explicit values and callbacks. `MainActivity`
keeps lifecycle and app routing while onboarding, navigation and shared surfaces
live separately. Rust's read-only `projection` module builds snapshots using the
same message projection for the timeline, search and pins. Keep data reduction,
read/navigation transitions and account authorization out of presentation helpers.

## Global direct messages and notifications

Servers opens with two global actions: Direct messages and Notifications. The
DM list combines signed-in servers, reuses the same conversation rows as the
server-scoped list, and shows the server name beneath each nickname. Search
matches nicknames, usernames and server names. Identical user/channel IDs on
different servers remain distinct, including avatar, profile and navigation scope.

Notifications reads each account's backend notification records through the
canonical core API and displays their actual source server, author, channel,
content, date and read state. A failed account gets its own retry row while other
accounts stay usable. Opening a row jumps to its original message; the existing
visibility-based read path applies. Explicit mark-as-read waits for backend
acknowledgement and reconciles the core's unread counters. These are in-app inbox
records; native alerts and background push are described below.

Global conversation navigation validates the expected account, selects the target
and switches servers in one actor turn. Back returns to the originating global
list and then Servers. Profiles from the global DM list retain their source
server without activating an unrelated account. The chat-list account button now
uses a larger squircle image, and the Servers selection/ripple follows its inset
rounded-square shape.

## Chat touch controls

Swipe a message from right to left to reply and focus the composer with the
keyboard open. The gesture preserves the existing draft, gives haptic feedback
at the reply threshold and commits on release. Short, cancelled or vertical
swipes do not reply; media scrubbers retain their own gestures. TalkBack exposes
the same Reply action. Pending messages and active recording/sending states do
not accept swipe replies. Left-to-right navigation retains Android's system Back
gesture.

Latest messages floats above the Send button as a separate frosted control. It
does not participate in composer measurement, so appearing or disappearing does
not resize the input or push the timeline. Its position follows the complete
composer height, including reply/attachment previews and outer padding.

## Reading and catch-up

Channel unread badges and catch-up history have independent durable cursors.
Reaching the live head marks the channel read and clears its outstanding alerts,
but preserves skipped history for the hourglass. Reading that interval from
its beginning advances its start; reading upward from its end retracts its end.
Jumps reset continuity and exclude the animation's intermediate rows from read
reports. Covered viewers/sheets, background activities, stale online snapshots
and an unfinished position restore do not advance reading.

The mention bell contains actual @mentions, newest first, with a count when
there is more than one. General `all` notifications and ordinary DMs do not
become mention targets. A mention is removed when exposed, or when the channel
is acknowledged at the live head. Cached mention timestamps preserve the order
of targets whose messages are not loaded.

Viewport observations use a typed read-state response rather than polling every
server and rebuilding message, member and resource projections. The normal core
reducer still receives every report, persists read spans and sends read receipts.
Android patches only reading state, channel badges and the notification count,
retaining message/resource objects. Account, channel and live-message IDs guard
against applying a response to a stale timeline. Transport wakes and other
commands continue to drain updates through full snapshots.

Android publishes reading progress on a separate server/account/channel stream.
The reading field of an application snapshot is only a seed; UI consumers use
`PapoViewModel.reading(server)` for current progress. A boundary-only change does
not emit application/timeline state or synchronize calls and the notification
inbox. Full polls also update the reading stream without republishing content
when only reading differs. Badge/count changes still publish normally.
Only the marker composition observes moving boundary IDs. The conversation
observes a derived set of control values (hourglass visibility, mentions, seeking
and live IDs), so moving the boundary does not rebuild the list or composer.

Message rendering retains its account/display resources independently of reading,
badge and background-work changes. Full snapshots retain equal message/resource
objects, while edits, reply updates and changed display resources still invalidate
the affected rendering. Timeline ID lookups are rebuilt when messages change,
rather than on every scroll layout. Message actions resolve the current snapshot.
Inherited avatar/author contexts use the same retained display resources, and
stable profile callbacks forward to the latest navigation handlers.

Unread and new-message indicators are overlays anchored to visible message edges.
Moving a boundary changes neither row height nor author grouping. Pixel movement
repositions the overlay during placement; labels retain accessibility descriptions
and do not intercept message taps. All viewport read reports remain immediate.

While reading behind the head, the down arrow counts incoming messages that
have not appeared in the viewport. Its first press opens the oldest remaining
arrival. The next press jumps to the live head, preserving skipped content as
catch-up history; once at the head, another deliberate press clears the block.
A small arrival batch that is fully exposed must not create a phantom block.
Only one catch-up interval is retained. Skipping a newer unread section replaces
an older unfinished interval, starting after the saved previously seen head.
Opening at newest captures that boundary before advancing the head. The newer
boundary excludes the destination viewport's read messages, so the unread-end
marker cannot be attached below the newest message just exposed. An unloaded
interval keeps the hourglass available while history resolves its destination.

Hourglass navigation and its markers use the same unread boundary. When the
prefix is outside the loaded window, the existing cache/server history
pagination resolves it before the jump. The saved read anchor is not a substitute
navigation destination. Leaving the chat or cancelling stops further catch-up
pagination. Deleted anchors and legacy millisecond timestamps must not reopen
an already-read anchor or prevent progress through continuous history.

Appearance settings include **Open at newest message** and **Catch-up hourglass**.
Turning the former off restores a message ID and pixel offset scoped to the
server, account and channel. That position is independent of the hourglass and
survives leaving the chat or restarting the app. Turning the latter off hides
the catch-up controls, block markers and explicit clear action; position saving,
ordinary unread state, mentions and new-message navigation continue to work.

The core uses the backend's explicit `PUT /channels/{id}/read` contract. Merely
fetching history does not acknowledge reading. Server `last_read_at` is receipt
wall-clock time, so a remote cursor is resolved to its referenced message's
creation key before comparison. Read cursors preserve nanosecond ordering in
cache schema 21, including upgrades from earlier Android builds. Channel and
notification acknowledgements are owner scoped, retained until successful and
retried with one request in flight per stream. Duplicate notification reads
returning 404 are already complete. Late notifications cannot resurrect a
locally acknowledged mention. Generic channel read synchronization from another
client is picked up through metadata refresh; the current backend has a live
read-state update for DMs.


## Author grouping and attachment checks

Chat, people, search and direct-message author labels use the role color already
chosen by `papo-core` and retained in its member cache. Compose does not infer
role priority from profile-sheet ordering. External identities keep their APP
label and do not inherit their bridge account's role color.

Consecutive messages from the same real author share one avatar and heading
within five minutes on the same local calendar day. External authors group by
source and external ID, never just their bridge account or display name. Unread
boundaries split groups. Every message retains its own lazy-list key, highlight,
reactions, reply action, swipe and long-press menu. Compact messages reduces
vertical padding and the author-header height/spacing; grouping works in both modes. The action sheet includes the
message author and timestamp for continued rows that omit a repeated heading.

Attachment moderation is asynchronous. Backend-authorized `pending`,
`processing` and `failed` checks do not mean a blob is inaccessible. Compose
renders them like egui, retains the sensitive-media reveal control and rejects
`blocked` resources. The backend can finish a check clean without a WebSocket
update, so waiting for that event would leave media permanently covered.
Attachments and embed requests also resolve against the active history context,
not just the live timeline.

The typing indicator shares the composer column and uses the same frosted pill surface. A single person uses the core-provided custom typing phrase, falling back to “is typing…”. Multiple people use the standard group phrase. Names are ordered consistently, long labels truncate to one line, and the core expires stale typing signals after ten seconds. The account settings typing-phrase control updates the same backend field.

## Native alerts, Firebase and background checks

Both Android shells use `android/shared`'s notification renderer and the shared
core notification policy/ledger. Compose supplies an Android lifecycle probe and
an owner-scoped UniFFI alert callback. Visible-chat, own-message, disabled-account
and disabled-channel suppression stays in core. Native notifications use stable
server/account/message identities, an explicit destination and private
lock-screen visibility. Reading a channel clears its alerts. Stale account
notifications cannot navigate into a replacement account.

The Notifications settings page has a device enable switch, Android 13+
notification permission request, system notification-channel settings and actual
push/fallback status. Desktop tray/startup controls are not exposed.

Official releases reuse these existing Firebase inputs, locally or in CI:

- `PAPO_FIREBASE_APP_ID`
- `PAPO_FIREBASE_API_KEY`
- `PAPO_FIREBASE_PROJECT_ID`
- `PAPO_FIREBASE_SENDER_ID`

`PAPO_REQUIRE_FIREBASE=1` rejects an incomplete official configuration. Debug
builds have a separate package ID; to enable their FCM, supply
`PAPO_COMPOSE_FIREBASE_APP_ID` for that debug Firebase Android app, alongside the
same project values. `PAPO_COMPOSE_REQUIRE_FIREBASE=1` makes that mandatory.
No Firebase values are committed. A configuration change invalidates the cached
token before a new registration. Existing GitHub secrets are sufficient for the
official package; no new Firebase app is needed for that release.

Firebase token registration uses the core's authenticated, frozen account scope.
Only a confirmed backend registration suppresses duplicate live background
alerts. Token changes, account changes, permission changes and device preference
changes reconcile registration, with bounded retry spacing. Foreground Firebase
wakes fetch authoritative account notifications and use the same ledger as live
events. Android-rendered background push taps and native notification taps both
resolve the signed-in server/channel and open the original message. Payloads
without a server are resolved only when the channel has one matching account.

Without confirmed push, WorkManager keeps a unique 15-minute periodic job per
signed-in server; Android can delay execution, so this is not realtime delivery.
Jobs use the existing encrypted credentials, core cache, REST transport and
notification ledger. Each run has a 25-second core deadline and cancellation;
interactive startup preempts and waits for a headless transport before taking
ownership of its refresh token. No permanent foreground service or duplicated
messaging transport is added. Offline/transient failures retry; a missing or
expired session exits cleanly.

Firebase's notification/data routing follows its
[Android message delivery contract](https://firebase.google.com/docs/cloud-messaging/android/receive-messages).

## Server administration and image editing

The server header opens a permission-scoped settings dialog. Its search finds
individual controls and channel/role editors. General settings save the server
name, public/private access, password and picture. Channel editors create text,
voice and category entries, edit descriptions, change order and confirm deletion.
Channel access reads and writes the backend's role rules (read, send, delete and
voice). Roles support names, colors, all eight backend permissions, membership
and deletion. Moderators can ban/unban people; the owner and current account are
protected. Sticker administration adds static or animated images and confirms
deletion. The audit timeline supports person/name/ID, action family, channel,
date and deleted-message filters, selectable deleted text and pagination.

Role editing preserves `manage_webhooks` with the other seven permission fields,
including rename-only updates. A shared core policy exposes moderator Delete from
chat, pins and search using the backend's channel-role grants and owner rules;
open channels do not grant moderation, and DMs cannot be moderated by a server
owner. Edit remains author-only. Deletion waits for backend acknowledgement and
retains the message/confirmation on rejection. The remaining Aero parity work is
tracked in [android-aero-parity.md](android-aero-parity.md), with UI review between
parts.

Changes use the canonical Rust HTTP client through account-scoped UniFFI calls.
A save is acknowledged by the server before leaving its editor. Refresh failure
after an accepted mutation is a warning, rather than an invitation to submit the
same creation/upload twice. Permission checks run at the native boundary as well
as in the UI. Audit sentences and filtering now live in `papo-core` and are shared
with egui. Pagination advances using the raw server cursor even when a locally
filtered page has no matches.

The UI exposes only working backend operations. There is no sticker rename,
per-person channel ACL, role reorder or server banner control. Categories can be
created and ordered, but the checked-in backend does not persist the frontend's
`parent_id` field, so category assignment is not exposed. Its create-channel
handler also ignores `access_roles`; role access is configured using the actual
channel-permissions endpoint after creation. No local-only access rules are
presented as server security.

One crop editor serves profile photos, profile banners, server pictures,
stickers and outgoing draft images. It supports drag, pinch, an accessible zoom
slider/reset, destination-shaped masks, square/original sticker crops, names,
and conversation/picker previews. `papo-core::media_prepare` owns the actual
orientation, crop, resizing, encoding and byte budgets; egui reuses the same
processor. GIF, animated WebP and APNG retain frames and timing. Static images
use WebP or JPEG as appropriate. Inputs are bounded to 32 MiB/40 megapixels and
animation decoding has frame/pixel limits. Outputs follow backend limits:
512-pixel/256-KiB stickers, 512-pixel/2-MiB avatars and server icons, and
1536×512/2-MiB profile banners.

Picked editor sources are private temporary files. Existing attachment drafts
stay intact until their replacement is durably saved; cancellation and failed
editing cannot delete the original. An upload with an uncertain outcome cannot
be edited. The editor uses the same image loader and animated-media lifecycle
as the rest of the app, and reusable remote resources keep using the shared
media cache.

Debug APKs keep a local reading-cursor trace beside the message database:
`reading-debug-<server-key>.jsonl` and its `previous.jsonl` rotation. Each file is
limited to 128 KiB (256 KiB per server total). It records cursor IDs/timestamps,
viewport IDs, read spans, history coverage, arrival IDs and before/after catch-up
boundaries; message bodies, credentials and profile data are excluded. Repeated
identical reports are omitted. Release builds do not compile this recorder.
This is diagnostic evidence for the reported false catch-up creation; adding the
trace does not change the hourglass state machine or establish its root cause.
