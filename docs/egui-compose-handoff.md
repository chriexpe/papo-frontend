# Compose lessons and egui compatibility handoff

As of 2026-10-27. Requested as preparation for a separate egui improvement session.
This document records delivered features, architectural lessons, backend contracts,
later fixes and concrete egui integration gaps. It does not implement those gaps.

## 1. Scope and source status

- PR #124, merged as `021b3c4`: official Android Compose client, 86 implementation commits.
- PR #125, merged as `2425d95`: shared call engine and live Android calls, 17 commits.
- PR #126, merged as `52a017f`: read-state publication, unread intervals, media restoration,
  history paging and rendering work, nine commits.
- PR #127 / `661e663`: provider playback/fullscreen repair. **Open, not merged** when audited.
  Current checkout: `fix/android-compose-embeds`, HEAD `661e663`.
- Existing audits of history/media predate Compose in places. Their already-fixed egui
  issues are useful regression cases, not a claim that those bugs are still present.
- `docs/android-compose.md` and `docs/android-aero-parity.md` contain historical progress
  sections. Some old “still to port” and hourglass notes are superseded by #125/#126.
  Use commit scope and current source rather than treating every older paragraph as current.

**The changes are not all in papo-core.** There are four categories:

| Category | Location/examples | What egui receives |
| --- | --- | --- |
| Shared domain fixes | `papo-core`: authorization, read state, history/cache, member paging, media metadata, notification classification | The new implementation when linked, provided egui uses the correct API/contract |
| Shared media engine | `papo-call`: WebRTC, SFU slots, immutable frames, audio mixing | Engine capabilities; the egui controller/presentation still has to wire them |
| Android orchestration | `papo-android`: typed projections, narrow viewport updates, account/admin operation coordinators, call session guards | No automatic frontend feature; extract shell-neutral logic when appropriate |
| Compose/platform presentation | Kotlin: drawers, rendering, state streams, Media3/Coil/WebView, service/PiP | Design/lifecycle lessons, not code egui should depend on |

Do not make egui depend on the Android UniFFI crate to gain business logic. Move reusable
orchestration into core/shared Rust components, retaining separate shell adapters.

## 2. Compatibility comes before visual parity

The current egui library **does compile** against this core: `cargo check -p papo --lib`
passed during this handoff. That proves source compatibility, not complete behavioral
compatibility. The following are source-observed integration gaps or explicit audit items.

| Priority | Current evidence | Required egui work |
| --- | --- | --- |
| P0 | `src/app.rs` still sends legacy `Command::JoinVoice`, which fetches ICE and then joins from the network worker | Adopt attempt-validated ICE preparation (`FetchVoiceIce`) and issue `voice_join` only while the intended account/attempt is current. Cancellation during ICE must not join later |
| P0 | egui leave/rejoin flow has no counterpart of Android's leave-acknowledgement barrier | Serialize room replacement. Server `voice_leave` events have no attempt ID; an old same-room leave must not kill the replacement call |
| P0 | egui message popup uses `mine` for Delete and exposes Pin unconditionally | Use shared `Store::can_delete_message`/`can_edit_message`, and the actual pin policy. Add moderator deletion without granting server-owner moderation over DMs. Gate actions consistently in chat, pins and search |
| P0 audit | `src/app.rs::portable_config_from_local` modifies a typed cached config and queues a full settings replacement | Preserve unknown fields and recent settings from other clients. Reuse/extend the fresh raw-config update pattern in `api/user_display.rs`; account display fields must not be erased by unrelated egui writes |
| P1 | `src/voice/mod.rs::event_callback` manually forwards SDP/ICE but not `VoiceAudioRoutes` | Delegate to or compose with `papo_call::Call::event_callback`, retaining shell-specific PiP events, so track ownership reaches the mixer |
| P1 | egui wrapper/pump sends `Watching` for cameras, but does not wire `WatchingScreens` or participant `Playback` | Add separate camera/screen subscriptions and receiver-side 0–200% gain/mute, with independent sender-microphone status |
| P1 | `VoiceMember.state_known` and `CallState.audio_routes_known` exist, but egui production source does not consume them | Distinguish cold membership from known device status; disable/explain individual mixing when the backend lacks route metadata |
| P1 audit | `CallState::left` now retains room membership; Android removes only its own member before leaving, while egui's local `leave_call` calls `left` directly | Verify immediate local roster behavior, remove only self when appropriate, and retain other occupants. Do not restore whole-room deletion |
| P1 | egui still chooses a cached older page before starting online HTTP, while Android now overlaps both | Adapt paging to new provisional-cache APIs and independent server/display cursors. Keep existing offline ticket/replay behavior; do not copy blocking Android adapter reads into the render thread |
| P1 | egui applies portable theme/notification settings but not the account font/density controls | Consume shared display values, map wire values correctly, migrate local preferences conditionally, and retain system/device scale independently |
| P1 audit | initial cached profile images/emojis now enter through shared runtime metadata, but egui also queues older separate restoration requests | Check for redundant work; preserve freshness/epoch guards. Current core restore helpers already protect authoritative responses—do not claim a proven overwrite bug without reproduction |
| P1 audit | shared unread semantics changed in #126; egui calls the same reducer but has its own scroll/restore/render logic | Recheck marker interpretation, visibility, continuity breaks, saved positions and actual reached-bottom behavior against the new core tests |

Some changes already benefit egui: its Markdown/format-assist/image-processing/audit
modules reuse shared implementations; API fixes such as full user paging and attachment
dimensions are shared; received broadcast classification was corrected across both
frontends; #126's unread reducer change is shared. Avoid reimplementing these fixes.

## 3. Architecture and state ownership

Delivered architecture:

- `papo-core`: authentication, HTTP/WebSocket, account isolation, reconnect, durable
  text outbox, cache reconciliation, read reducer, notifications, parsers and policy.
- `papo-call`: GStreamer workers, SDP/ICE media handling, camera/screen SFU slots,
  immutable RGBA frames, track-to-user audio mixing and platform media policy.
- `papo-android`: typed UniFFI records/actions, operation guards, projections and native
  adapters. It also contains orchestration that is not yet shell-neutral.
- Compose: presentation, Android resources, permissions, audio focus/routing, foreground
  service, notifications, PiP and native media surfaces.
- egui: its own immediate-mode presentation, textures, input and platform integration.

Lessons:

- A session belongs to the process/runtime, not to a screen being collected or painted.
- Network signaling and media must progress without a rendered chat frame.
- Every asynchronous operation needs the relevant server/account/channel identity,
  generation/request ticket and, when applicable, call attempt or draft identity.
- Stable UI callbacks must invoke current state. Retaining an old snapshot to save work
  must not make Edit/Delete/Reply act on obsolete content or a different account.
- Keep translations in the shell. Core should return semantic values/errors rather
  than a sentence Kotlin or egui must reverse-translate.
- The core's guarantees are only as good as each caller's use of its policy and APIs.
  Sharing a crate is not equivalent to having feature parity or identical behavior.

## 4. Reading, hourglass and saved-position lessons

Keep these concepts separate:

1. Channel acknowledgement/badge and notification receipts.
2. Actually observed message spans and the skipped catch-up interval.
3. Saved viewport position (message ID plus offset).
4. Temporary jump/navigation intent and initial restoration.
5. Incoming-message navigation and actual mention targets.

Reaching newest can acknowledge the channel while preserving skipped history. It is
not permission to erase the user's saved position or indiscriminately clear navigation
state. A successful scroll request is not proof that its destination was rendered.

Core/UX invariants established during this work:

- Observe only actually readable messages, while foreground and not covered by a
  viewer/sheet. Pixels behind the composer/header and intermediate jump-animation rows
  must not count as read.
- Initial position restoration and stale online cache must not prematurely advance
  the cursor. A jump breaks continuity; reading upward and downward have distinct effects.
- Keep one catch-up interval. A newer skipped section supersedes the old interval using
  the previously seen/acknowledged head, captured before advancing it.
- Exclude the destination's exposed messages from the unread interval. A fully exposed
  one-/two-message arrival batch must not create a phantom block around itself.
- The hourglass target and markers must derive from the same core boundary. An unloaded
  boundary needs history resolution, not an arbitrary old read-anchor substitute.
- Deleted anchors and legacy timestamp precision need explicit handling.
- Mention navigation is for actual mentions, not every general notification or ordinary
  DM. Local acknowledgements must prevent delayed alerts from resurrecting old targets.
- The newest-message arrow must retain intent until the current bottom is actually
  reached, including a tail that grows while media dimensions resolve.
- Saved positions remain scoped to server/account/channel and survive exit/restart.
  “Open at newest” and “show hourglass” are independent preferences.
- Fetching history is not acknowledgement. Backend `last_read_at` is receipt time;
  resolve the referenced message's creation key before chronological comparison.
- Preserve exact timestamp/nanosecond/ID ordering. Schema 21 read cursor precision must
  survive upgrades; #126 did not introduce a new cache schema.

Status: #124/#125 explicitly deferred the intermittent live incident. #126 later fixed
specific shared interval/visibility problems and added regressions. It does not prove
that every reported stuck-position incident or scrolling issue is permanently solved.
Do not remove meaningful tests or discard saved positions to make a test pass.

## 5. Performance: useful work versus repainting everything

Later commits exposed a costly chain: viewport observation → full native snapshot →
messages/profiles/resources replaced → broad Compose invalidation → more layout/media
work while scrolling. Merely throttling read reports would damage correctness.

The corrections were:

- A narrow typed viewport response uses the normal core reducer, persistence and receipt
  delivery without reconstructing the message/resource graph.
- Reading publishes on a separate server/account/channel stream. Full snapshots seed
  it, but reading-only changes do not republish application/timeline state or resync calls.
- Only marker composition observes moving boundary IDs. The conversation sees derived
  control state such as mention targets, seek status and hourglass availability.
- Equal message/display resources retain identity. Inherited profile contexts and
  callbacks must also be isolated; stable explicit arguments alone were insufficient.
- Resolve actions from current state, even when rendering uses retained presentation.
- Rebuild ID lookup tables on message changes, not each scroll-layout observation.
- Moving markers are overlays positioned at message edges. They do not alter row height,
  author grouping or hit testing, and retain accessibility descriptions.
- Identical database bodies skip SQL updates, but a snapshot becoming proven timeline
  history must still promote its membership. Identical member refreshes skip rewrites.

For egui, adapt the principles rather than importing StateFlow/Compose machinery:
retain parsed layouts/textures and cheap display inputs; isolate read reduction from
expensive content rebuilding; avoid blocking database/disk/HTTP work in the render frame;
keep marker geometry independent of message layout. Immediate-mode repainting itself
is not equivalent to rebuilding all data and resources.

Do not claim the performance problem solved from these changes alone. #126 reports
release/AOT captures around P95/P99 25–26/32 ms, before its final cache/history changes.
They differ from earlier debug captures in build mode, cache and content and are not a
matched causal comparison. Profile an optimized build with matched content/device,
cache state, gestures, font/density settings and media before drawing conclusions.

## 6. History, cache and freshness contracts

- Displayed cached rows are speculative. They do not establish server freshness,
  chronological continuity, a validated HTTP boundary or read acknowledgement.
- Online paging starts HTTP and can display a **connected** local page concurrently.
  Only offline paging should depend entirely on local coverage.
- Keep display/local and server-verified cursors separate in both directions.
- A connected page requires the exact chronological anchor, including timestamp
  precision and ID. An old pin/reply/search snapshot does not prove the gap is cached.
- Context windows remain separate from the live timeline. Live messages continue into
  the head; edits/deletions/reactions/pins/moderation update loaded copies.
- Replay concurrent live mutations after HTTP snapshots so late responses cannot undo
  edits, deletions, reactions or optimistic changes.
- Generation/request/owner guards apply to local DB responses as well as HTTP.
- Preserve visible ID/offset anchors while adding pages at either end.
- Automatically prefetch near either viewport edge without forcing pixel offsets through
  the entire message composition. Pause requests during initial restoration/seeking.
- Initial cached metadata now includes asset identities before transport bootstrap;
  authoritative changes/empty lists/removals still win over delayed cache restoration.
- Current continuous message retention is bounded (500/channel, 100-row DB pages).
  Detached snapshots and media resources have independent bounds/lifetimes.
- For ID-only reply/notification targets, the audited backend cannot locate a message
  from `last_id` alone without `since`. Use a cancellable bounded seek to find its exact
  key; do not inject every intermediate seek page into the timeline.

Core integration points: `Store::preview_cached_history`, `restore_cached_history`,
`cached_history_failed`, `begin_load_older/newer`, `history_ticket`, and the cache's
connected older/newer-page reads. egui already has asynchronous page tickets and mutation
replay; preserve those and adapt orchestration rather than replacing everything.

## 7. Media, embeds, resource lifetime and geometry

Delivered improvements and transferable rules:

- Preserve backend attachment/media dimensions through HTTP models, core state, cache
  and UI projections. A renderer cannot use dimensions discarded by an earlier layer.
- Fit direct images/videos to their actual aspect ratio. Remove artificial minimum-height
  backing boxes and center media inside genuine cards.
- Prefer actual video geometry over poster geometry. For missing metadata, resolve from
  decoded content; a generic fallback must not become a permanent oversized frame.
- Cache geometry independently of the GPU texture. Texture eviction must not collapse a
  row and cause scroll jumps or resize/reload feedback loops.
- Backend card authority includes an intentionally absent image/video. Do not rescrape
  backend-authored cards and resurrect removed media from a retained texture.
- Distinguish direct media, GIFs, authored custom cards, normal link cards and provider
  embeds; these do not all deserve the same enclosing card or playback mechanism.
- Use the same complete message-body/media logic for chat, pins and search. Detached
  hydration must not acknowledge messages or create history continuity.
- Pins refresh from live events/reconnect. Keep cached results on failed refresh and
  offer Retry; no routine refresh button or separate “open conversation” action is needed.
- Tapping outside media/author controls navigates to the source; media/author interactions
  retain their own behavior.
- A single encoded-media budget serves avatars, stickers/GIFs, banners, images, consumed
  audio/video ranges and waveform envelopes. Compose uses a 256 MiB shared Media3 LRU
  with Coil disk caching disabled; egui should preserve the same ownership/budget
  principles, not adopt the Android library itself.
- Immutable resources use account/resource/revision keys. Public images can show stale
  bytes while conditionally revalidating; public requests receive no Papo credentials.
- Do not preload entire long videos. Do not retain decoded animations or GPU frames
  indefinitely or create independent disk caches for every UI surface.
- Pause playback/animation at the **readable** viewport edge, retain nearby resources,
  release distant decoders, and preserve position/manual-pause intent for reconstruction.
- Being hidden behind a frosted control is different from being far away: assets may
  still need loading to appear through blur, while their messages must not count as read.
- Animated occurrences need independent clocks so pausing one occurrence does not freeze
  another. Keep ordinary decoded resource reuse when appropriate.
- Fullscreen should reparent/reuse the existing native player/view and decoder, preserving
  position, controls and session, not construct another player.
- Backend-authorized `pending`/`processing`/`failed` moderation checks do not imply the blob
  is inaccessible. Only genuinely blocked media is rejected; sensitive/spoiler reveal
  stays independent. A clean check can finish without a WebSocket update.
- Parse audio/container metadata carefully: M4A/M4B voice notes should not be rendered
  as video just because their container reports `video/mp4`.

#127-specific lessons (pending merge): AndroidView's default WRAP_CONTENT caused Chromium
viewport/percentage video heights to collapse. Explicit MATCH_PARENT dimensions restored
inline video. Provider gestures must not be stolen by chat scrolling/reply/long-press.
Fullscreen uses WebChromeClient's original custom view, with Back/disposal restoring
orientation and bars and keeping the inline WebView/session alive. The provider receives
an app-identity referrer, never a Papo session/server identity. Supported providers remain
isolated: no arbitrary iframe execution, Papo headers, file/content access or JS bridge.
The same geometry/lifetime/gesture audit applies to egui's platform WebViews, though the
AndroidView-specific fix is not portable verbatim.

## 8. Composer, Markdown, identity and message actions

Delivered feature inventory and egui parity targets:

- Same-author grouping uses real identity, time/day bounds and correct external-source
  identity, not merely display name or the bridge account. Every message keeps its own
  ID, actions, reactions, highlight and selection behavior.
- Normal spacing became the sensible compact baseline; Compact tightens it further.
  Typography, message density and overall device/interface density are separate axes.
- A multiline composer reclaims text width by moving its action row below the editor;
  single-line input stays compact. Keep keyboard focus/caret/selection stable through
  layout changes, font scaling and long placeholders.
- Draft attachment thumbnails are compact and open their options by tapping the tile.
  Use a staged action sheet rather than occupying the entire input width with controls.
- Shared CommonMark parsing supports emphasis/strike, headings, quotes/lists/tasks,
  tables, rules, code/copy, safe links/images and interactive custom stickers.
- Share cached parse trees and formatting rules. Do not reparse the whole history each
  frame or treat code URLs as media players. Bound excessive nesting and reject unsafe
  schemes/HTML behavior.
- Formatting assistance respects IME composition, paste, selection, paired delimiters,
  atomic sticker deletion and programmatic insertion/reset.
- Android's UTF-16 offsets differ from Rust Unicode-scalar offsets. Even outside Android,
  byte/scalar/grapheme boundaries and duplicate names must not corrupt identity bindings.
- Mentions display readable names but preserve canonical user IDs in drafts and edits.
  Editing a label invalidates its binding; deleting one duplicate label must not transfer
  its identity to another. Resolve names in reply previews and message action previews.
- Copy text preserves canonical source even when the preview is human-readable.
- Reply notification defaults and per-reply overrides are separate; changing one reply
  must not change the account/device default. Pass `notify_reply` on text and uploads.
- Hardware/IME Enter-to-send is opt-in and disabled by default. Shift+Enter, paste and
  editor multiline behavior remain correct; all sends use the same validation/outbox path.
- Caption editing and structured custom-card editing follow the target held/clicked.
  Caption saves omit the embed field; card saves preserve caption bindings and the full
  card list. Text editing uses an unmistakable editable field in a staged drawer.
- Custom typing phrases are semantic server content. Single-person typing uses that
  phrase; group typing uses a normal group label. Align the pill to the editor, expire
  transient state and clear it on disconnection.

`@everyone` lesson: permission, word styling, recipient row tint, mention navigation and
OS notifications must agree. Use the **sender's** effective authority; unknown/external
identities gain no implicit broadcast permission. Own valid broadcasts are styled but
must not notify/tint the sender as a recipient. Keep direct bound mentions independent.
Backend Notification is authoritative for broadcast delivery because current role
metadata cannot prove historical send-time permission. The core classifier and egui
received-renderer correction already exist; audit remaining callers and cache keys.

## 9. Attachments, image preparation and delivery certainty

- Options: rename the stem while retaining extension, stable randomization, spoiler,
  crop, and supported metadata removal. Apply/Cancel stages edits atomically.
- PNG/JPEG/WebP/GIF metadata stripping respects orientation and animation; keep the
  original alongside the private derivative until send/removal. Unsupported/oversized
  processing fails clearly and never silently uploads the original as “sanitized”.
- `SPOILER_` filenames are the agreed interoperable convention for Compose and egui.
  The audited backend ignored spoiler flags, so do not present an unenforced field as
  protection. Other clients may show the media without concealment.
- Shared `media_prepare` handles crop/resize/encode, animation bounds and destination
  limits for avatars, server icons, banners, stickers and outgoing images.
- Cropping or cancelling cannot destroy an existing durable draft. Uncertain in-flight
  uploads must not be edited or automatically resent.
- Core durable text enqueue acknowledgement is distinct from final delivery. Preserve
  text until durable enqueue succeeds. Attachments use their acknowledged multipart path.
- Save the in-flight marker before upload. Distinguish definite preparation/rejection
  errors from an ambiguous lost response, server error or decode failure.
- An ambiguous creation/upload requires a deliberate duplicate-delivery retry choice.
  A failed metadata refresh after an acknowledged save is not a rejected save.
- All server/account/channel drafts, bindings, private sources/derivatives and expression
  preferences need scoped cleanup and stale-completion guards.

## 10. Backend contracts that must shape egui

These findings were verified against the pinned backend audit in
`docs/android-aero-parity.md` (backend `af057d5`, 2026-10-23). Recheck if the backend changes.

- Eight role permission fields, including `manage_webhooks`, must round-trip on every
  full-object role replacement; rename-only saves must not reset unrelated bits.
- Moderator deletion requires the explicit channel-role grant. An open channel or
  `manage_server` alone does not grant it; server ownership does not override DM privacy.
- Ban/unban routes use `manage_server`, despite a separately persisted `ban_members` bit.
- Password reset-link generation for another user is owner-only, not a delegated admin bit.
- Webhook entry points use owner/`manage_webhooks`; client gating cannot fix a backend that
  actually authorizes an invalid request. The server must enforce its own permissions.
- Users are paginated, not limited to the first 100. Follow exact time/ID cursors,
  deduplicate inclusive edges, include banned members for unban, reject non-advancing cursors.
- Categories are derived from global order in the audited backend. `parent_id` assignment,
  per-person ACL, role reorder, sticker rename and server banner are not functioning
  operations to invent in the UI. Create-channel ignored `access_roles`; configure real
  role access through the permissions endpoint after creation.
- Audit pagination must advance from raw server pages even when local filters produce
  zero visible matches. Preserve generation/filter ownership and selectable deleted text.
- Custom embed creation **is** backend-supported. Reuse core validation/outbox/upload
  paths, including Unicode limits, attachment references and complete-list semantics.
- Custom embed update replaces the entire list. Omitted list preserves it; explicit empty
  list removes editable cards. Protected media identities are not round-trippable through
  the audited edit input. Disable unsafe list replacement; allow caption-only editing.
- Whole account config PUT is a replacement. Read fresh raw data, change the intended
  field and preserve unknown fields. No frontend can guarantee atomic cross-client writes
  without a backend revision/conditional-update contract.
- Internal/wire values stay untranslated (`medium`/`huge` are the Normal/Large font values).
  English/Portuguese labels, errors, plurals and accessibility strings belong to the shell.

## 11. Accounts, safety, notifications and navigation

- Forget server works while signed out and for the final saved entry. Stop/join transport,
  remove local credentials, DB namespace, drafts/private files, positions, expressions,
  caches, appearance migration state, notifications/jobs and registrations.
- Persist cleanup intent before destructive local cleanup. A tombstone prevents reconnect
  after a partial crash and permits retry. Preserve unrelated servers and user-owned downloads.
  Forgetting never deletes remote accounts/messages. Sign-out is a different operation.
- Keep external author identity separate from native bridge accounts in display, grouping,
  avatar/color, profiles, mentions and interaction targets.
- Block/unblock, hidden DM restore, global DMs and notifications retain source account
  and navigation destination; equal IDs on different servers are not the same identity.
- External-link confirmation shows the full URL/canonical host. Trust is exact-host only;
  subdomains do not inherit it. Apply the policy to Markdown and WebView exits too.
- Recovery links require explicit server confirmation and public, cookie-free, no-redirect
  transport. Never persist recovery tokens/passwords in navigation/preferences/diagnostics.
- Native notifications share core policy and durable deduplication across live messages,
  backend Notifications, Firebase hints and bounded reconciliation.
- Per-account inbox cursors and per-channel bulk policies need partial-failure outcomes,
  not a single optimistic success. Acknowledgements must not cross account replacement.
- Back dismisses the active viewer/sheet/menu/keyboard before leaving chat. Keyboard
  visibility needs the actual platform state, not a stale focus flag. Drafts survive.
- Settings/profile navigation returns to its actual source and preserves ongoing calls/media
  according to the intended surface lifetime.

## 12. Calls: shared engine, incomplete egui integration

Implemented for Android: room-scoped presence, muted/camera-off join, incoming audio,
local camera capture/switching, incoming camera and shared-screen video, speaker state,
receiver volume/mute, foreground notification controls, communication routing/audio
focus, background lifetime, PiP and live voice-channel text history.

Shared lessons:

- A room/channel ID alone is insufficient session identity. Attempts must remain unique
  across leave/rejoin and client replacement; reject stale actions/frames/permission results.
- ICE discovery and room entry are distinct. Discard abandoned discovery results before
  they can join. Do not send an offer before `voice_joined` creates the server peer.
- Server leave acknowledgements have no attempt ID: use a barrier before replacement,
  and retain it after timeout until acknowledgement/disconnect. Timeout is not evidence
  that an old session or its future events are gone.
- Camera UI follows actual capture completion/revision, including failure returning to
  the same boolean, not merely the last button click.
- Local Mute for me/gain changes receiver output, not the sender's mic indicator.
  Track ownership must be authoritative and refreshed when SFU slots are reused.
- Camera and screen from one publisher are separate streams sharing a bounded slot pool.
  Subscriptions/retry/frame identity must include kind as well as publisher.
- Distinguish membership-only cold presence from confirmed mic/camera state.
- Leaving preserves other occupants. Disconnect stops media and does not autojoin.
- Media frame publication must not wake/republish the full chat graph at video frame rate.
- A call stays scoped to its account/room while browsing other chats or servers.

Presentation accepted in this session: independent call pill below channel navigation;
room/server at left, speaker-prioritized participant squircles in remaining space;
sequential 1→2→3→4 view control beside avatars only while videos are shown; subtle close;
no extra empty video area without a source. First avatar-strip tap can reopen previews,
next opens the call. Participant gain/mute belongs in a dedicated action drawer, not the
profile. Profiles/messages remain separate actions. Keep voice roster interaction live.

JNI/UniFFI finding: Kotlin↔Rust uses UniFFI/JNA. GStreamer's Android plugins use JNI
internally. Foreign callbacks from its native media threads could attach/detach a JVM
thread and invalidate GStreamer's cached JNIEnv, crashing later camera operations.
Cache device state natively and notify Java from a separate thread. Do not treat the
fix as replacing UniFFI or route a JNIEnv across threads. Android OpenSL capture/playback
use communication/voice presets to match platform audio routing.

Not implemented by this scope: Android screen capture/publication and a new direct-call
invitation/ringing protocol. Physical acoustics, Bluetooth hardware and internet/TURN
reliability need real-device/server testing beyond synthetic emulator transport.

## 13. Verification, diagnostics and maintainability lessons

- Demo is excellent for reviewing UI and awkward states without touching a live server.
  It does not validate auth, WebSocket, FCM, real permissions or actual media transport.
- Review substantial UI changes with the user before running a large instrumentation
  campaign. Keep formatting-only changes separate from behavior/refactors.
- Formatting is only one layer of hygiene. Split oversized screens, name state transitions,
  separate callbacks/layout/navigation, and comment lifecycle and ownership invariants.
- Test process-owned models through process-owned test seams. Activity-only fixtures
  became invalid when production ownership moved to the Application.
- Use isolated wire fixtures and Pion SFU media tests. Never automatically transmit mic,
  camera, messages or administrative mutations to saved real servers/participants.
- Call tests exercised real SDP/ICE/DTLS/SRTP, decoded remote camera+screen frames,
  synthetic capture/switching, audio packet output, stale controls, rapid rejoin,
  cancellation, disconnect, recreation, PiP and notification actions.
- UI assertions should exercise the real visible media region/controls, not offscreen
  nodes or text labels that can pass while video pixels are absent. #127 tests actual
  local HTML/video pixels, seek gestures and fullscreen lifecycle without YouTube dependence.
- Emulator LOW_MEMORY termination is not proof of an application crash. Match the app PID,
  distinguish system/process reasons, and allow realistic RAM/16-KiB-page ABI testing.
- Performance requires release/benchmark measurements; debug and release/AOT numbers
  are not interchangeable. Preserve comparable cache/content/device conditions.
- Debug read traces are bounded: 128 KiB current + 128 KiB previous per server, no bodies
  or credentials, repeated events suppressed, excluded from release. Avoid unbounded logs,
  recordings, APK copies, retained decoders and source/derivative leaks.
- The 114-minute CI episode was dependency/mirror installation, not Clippy executing
  for 114 minutes. Cache package downloads, keep signed indexes current and bound setup.
- Pin enforcement toolchains while still testing newer Rust. New stable Rust deprecated
  the atomic update API; the compatible compare/exchange loop fixed it without disabling
  warnings. Concurrency tests preserve unique call-ID behavior.
- Keep SDK/UniFFI/GStreamer versions reproducible, checksum/cache large SDKs and generate
  bindings from the exact native library shipped. Verify each ABI and platform build.
- The signed official Compose APK keeps the old release package/key and imports old
  credentials/preferences/cache idempotently. Debug/benchmark use the development package;
  benchmark uses release-profile Rust and profiling support without erasing debug data.
- All twelve #124/#125 CI checks passed before merge. #126 reported 494 core tests,
  76 Android adapter tests and 16 emulator UI checks for its cache/history work. #127
  reports two provider tests and APK/lint/format checks. These are historical results,
  not a claim that this handoff re-executed every suite or profiled the latest phone build.

## 14. Recommended order for the next egui session

1. Establish main/branch/backend revisions and capture baseline behavior. Read this
   compatibility table and the current core reading/history/call APIs first.
2. Fix semantic integration: message authorization, safe full-config writes, call
   attempt/cancel/rejoin ordering, audio routes/screen subscriptions and presence status.
3. Verify new read/history semantics from the actual egui viewport: one/two/media arrivals,
   latest jump then exit/reopen, old/unloaded/deleted targets, both directions, restart,
   reconnect and multiple accounts. Preserve the saved-position feature.
4. Adapt online cache+HTTP overlap, automatic edge loading and narrow invalidation. Use
   shared reducers/cursors/replay; measure egui frame work before/after matched scenarios.
5. Audit media authority/geometry/cache/player/WebView lifetime across chat, pins/search,
   fullscreen and blur. Include provider repair lessons from pending #127 when merged.
6. Fill verified frontend gaps: moderator/pin actions, bound editing/previews, attachment
   options, caption/card editing, typography/density, admin/account/recovery/notification UI.
   Do not assume each is absent—egui already has substantial implementations.
7. Review each substantial UI slice before broad regression runs. Keep protocol, platform
   and presentation commits separate and report unverified hardware/backend limitations.

Acceptance means more than compiling: both frontends interpret the same authoritative
core contracts correctly, user state is preserved, callbacks cannot act on replacement
accounts/calls, and egui no longer reintroduces solved core/backend problems in its shell.

## 15. Source entry points

- `crates/papo-core/src/state/mod.rs`, `state/reading_tests.rs`, `state/history_tests.rs`:
  read spans, positions, continuity, page/context tickets and mutation replay.
- `crates/papo-core/src/cache/{types,store,tests,schema}.rs`: exact cursors, retention,
  cached assets, SQL no-op writes and schema compatibility.
- `crates/papo-core/src/state/authorization.rs`, `notification.rs`, `api/user_list.rs`,
  `api/user_display.rs`, `api/models.rs`: policy and backend contract fixes.
- `crates/papo-core/src/{markdown,format_assist,media_prepare,embed_authoring,audit,search,recovery,inbox}.rs`:
  reusable domain implementations.
- `crates/papo-call/src/{lib,engine,audio,slots,platform}.rs`: shared media engine.
- `crates/papo-android/src/{call,viewport,appearance,administration,message_results,social,webhooks}.rs`:
  working Android orchestration to audit/extract, not a new egui dependency.
- `android/compose/.../{ChatStatePublication,ReadingUpdates,ConversationMarkers,TimelineMessage,
  MessagePresentation,HistoryPrefetch,MediaLifecycle,MediaCache,ProviderPlayer}.kt`:
  state/lifetime/rendering reference implementations.
- `src/app.rs`, `src/voice/mod.rs`, `src/ui/{shell,call,settings,markdown_view}.rs`,
  `src/media/{mod,player,backend,platform}.rs`: egui integration targets.
- `docs/android-compose.md`, `docs/android-aero-parity.md`, `docs/android-calls.md`,
  `docs/message-history-audit.md`, `docs/media-cache-audit.md`: detailed contracts/history.
- `tests/call-sfu/README.md` and Compose instrumentation sources: fixture/test contracts.

## 16. Commit ledger

The ledger below covers the implementation commits of #124, #125 and #126, then the
current open #127 change. Commit subjects are an evidence index, not proof that a
particular feature is missing from egui today.

### PR #124 — Compose client

- `db91fd9` feat(android): add a native Compose messaging client
- `954809d` feat(android): share papo-core through UniFFI and render media embeds
- `69e9748` feat(android): expand channel descriptions from the floating header
- `dc955aa` feat(android): add gallery, camera and file attachments
- `9209801` feat(android): record voice messages inside the composer
- `57dc175` fix(media): classify M4A voice messages as audio
- `d0e7ead` feat(android): render decoded audio waveforms in chat
- `d4ad6c7` feat(android): add server emoji, GIF libraries and reaction pickers
- `e5e8ab0` feat(android): suggest bound mentions and inline server stickers
- `1807d39` fix(reactions): reconcile repeated reaction toggles reliably
- `5dc7d2a` feat(android): show sticker details and favorite reactions
- `3387448` feat(android): navigate and highlight reply originals
- `018a087` feat(android): unify media viewing, downloads and sensor rotation
- `75542fc` feat(android): share the native PlayerView across inline and fullscreen
- `5b1408b` feat(android): pause offscreen media and retain resources by distance
- `3edc40c` feat(android): show user profiles, avatars and author mentions
- `34d1a65` feat(android): distinguish integration authors with app profiles
- `208a4ea` feat(android): unify persistent media caching across images and playback
- `fc88747` fix(android): dismiss Back layers before leaving conversations
- `22580a0` refactor(search): share filter parsing and autocomplete in papo-core
- `0418f49` feat(android): add community controls and global search
- `eb9e483` feat(android): group Chats and Direct beside a separate server switcher
- `a5ea445` fix(core): refresh signed-in identity from profile updates
- `dca7a5d` feat(android): add acknowledged account operations through UniFFI
- `6ccb28b` feat(android): add searchable native client settings
- `cff4bf6` fix(android): reuse decoded media across profile navigation
- `f00686d` fix(android): match server selection shape and enlarge account avatar
- `d0cd8c3` feat(android): expose authenticated backend notification inbox
- `a697e0f` feat(android): add global direct messages and notification navigation
- `5d5e655` feat(android): reply to messages with a horizontal swipe
- `50ac19b` fix(android): float latest-message control above the composer
- `f2952b2` fix(core): separate read receipts from preserved catch-up progress
- `87361ed` feat(android): add durable reading navigation and hourglass preferences
- `a5cdef7` android: share primary role colors across member names
- `0eafdbf` fix(android): render media while asynchronous checks are pending
- `bdfa29d` android: group consecutive authors and tighten compact message headers
- `187e3b2` fix(android): synchronize readiness when reusing a retained player
- `15a07e6` android: wire native alerts and Firebase into official Compose releases
- `22338e6` android: show custom typing phrases in a composer-aligned glass pill
- `6c553ec` test: align egui fixtures with shared read state and mention encoding
- `0eeddfc` test(android): keep video playback checks inside the actual viewport
- `f17a953` fix(android): dismiss the IME before leaving a conversation
- `ae9dbf7` refactor(core): share audit rendering and image preparation across frontends
- `080e870` feat(android): expose scoped administration and shared image editing through UniFFI
- `c4c0138` fix(android): check platform IME visibility when handling chat Back
- `7177389` feat(android): share one crop editor across profiles, stickers and image drafts
- `68edee9` feat(android): add native server administration and searchable audit timeline
- `e479aaa` fix(android): polish administration labels and counts
- `f4beba8` Share Markdown parsing and composer formatting through papo-core
- `f2fc18a` Expose cached Markdown trees and formatting sessions over UniFFI
- `20f7a94` Render rich Markdown with shared Android media and safe code links
- `1d1e912` Add IME-aware composer formatting assistance and Android coverage
- `59576c6` Trim empty caption edges left by hidden media links
- `ce278b9` core: reconcile detached search results and pin snapshots
- `a55fa8e` android: expose scoped live pins and full result media through UniFFI
- `6c379e5` android: add live pins and share playable message results
- `cf7d9ce` android: retire acknowledged new-message dividers using core state
- `98aeb71` android: add bounded debug traces for unread cursor transitions
- `de55e3b` Android: add trusted links, app languages and account recovery
- `b901f69` style(android): format Compose consistently and enforce ktfmt in CI
- `0995082` refactor(android): separate settings pages, UI controls and snapshot projections
- `d8fe802` fix(android): complete localization and preserve rotation after language changes
- `39c84a8` android: add attachment options and shared media privacy
- `be480e0` android: preserve user bindings while editing mentions
- `45b63cf` android: forget saved servers and their local data
- `c14f2ba` android: add reply mention defaults and per-reply controls
- `13920fe` android: preserve webhook permissions and add moderator deletion
- `f029c0f` android: add account-scoped blocking and hidden DMs
- `057930d` android: add permission-aware webhook administration
- `06fb9a5` android: add custom embed authoring and safe editing
- `af9be21` android: add bulk notification policies and paginated inbox
- `75ea64c` android: add permission-aware everyone and Unicode suggestions
- `c5bb89e` fix: enforce sender permissions for received everyone mentions
- `befe375` android: add interface density and adaptive multiline composer
- `d51042b` fix(android): normalize stacked message spacing and tighten compact mode
- `beddd9f` fix(android): resolve mention names in reply previews
- `e365275` fix(android): center embed media and remove direct-media backing
- `f06e704` fix: preserve backend attachment dimensions for Android media layout
- `11ad5fb` fix(android): resolve fallback media frames from decoded geometry
- `998150c` feat(android): sync account typography and message spacing
- `e82b3ae` fix(android): align administration permissions and load every member
- `0c80b91` test(android): update regression fixtures for accepted Compose UI
- `343c40c` docs(android): record administration audit and deferred regression
- `572b0b0` ci: cache Ubuntu package downloads and bound dependency setup
- `fd25c58` fix(android): separate caption and embed editing and add Enter-to-send
- `6c90159` test(android): verify caption targeting and editable field values

### PR #125 — calls

- `c3e7a96` feat(android): preview voice and video call flows in Demo
- `352359f` feat(android): polish call pills and add expandable video previews
- `fad2044` fix(android): cycle call video views and normalize pill spacing
- `f92eb60` fix(android): use an avatar slot for reopening call videos
- `f330726` fix(android): place video view control beside participants
- `f7978a1` feat(android): preview voice rosters beneath channels
- `fdc8bbf` feat(android): add call participant profiles and playback controls
- `137e387` fix(android): move call controls into participant action drawer
- `138998b` refactor(call): extract GStreamer engine from egui into papo-call
- `b183b33` feat(call): route participant playback and screen subscriptions
- `a2fb7e2` feat(android): bridge core call state and package shared media runtime
- `9e34652` fix(android): isolate media workers from foreign JNI callbacks
- `92068a5` fix(call): retain room presence and enable voice channel history
- `e2844a0` fix(call): serialize room replacement and scope media controls to unique attempts
- `089f6c4` feat(android): connect live calls with foreground audio video and picture-in-picture
- `0044a8f` docs: document live call ownership validation and review scope
- `fc7d852` fix(ci): avoid deprecated atomic update API across Rust toolchains

### PR #126 — later reading/cache/rendering changes

- `9dbcf44` fix(android): avoid full snapshots for chat viewport updates
- `6fdd6bc` test(android): exercise the process-owned model in chat UI tests
- `1b5c438` fix(chat): replace superseded unread intervals and exclude viewed rows
- `9eb09eb` perf(android): isolate message content from reading and server updates
- `a5a1ef5` perf(android): draw read markers outside stable message rows
- `9dacb54` perf(android): isolate inherited message profile dependencies
- `afa3c33` refactor(android): publish reading progress independently of chat content
- `13a6a91` fix(android): restore cached media and load chat history automatically
- `fb3feaa` fix(ci): satisfy Compose formatting and strict Clippy checks

### PR #127 — open at audit time

- `661e663` fix(android): restore inline embed playback and fullscreen controls
