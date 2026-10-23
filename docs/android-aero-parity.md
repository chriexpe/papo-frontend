# Aero parity follow-up for PR #124

Source brief: `/home/christian/Downloads/aero-plan.md`. Work stays on
`feat/android-compose` / PR #124. Official repositories are read-only references.
Calls, voice-channel text chat and the deferred hourglass investigation are outside
this pass. Each part with substantial UI changes stops at a testable APK for user
feedback before continuing to the next part.

## Contract baseline

Verified against the official sources on 2026-10-23:

- Backend `af057d5e0e107ae666c4f6776b36222ef18b2105`.
- Aero frontend `3817e3e1760b26c470eb7deefb32f9d4f616395c`.

Role permissions have eight fields, including `manage_webhooks`. Role updates
replace the complete permissions object. Moderator deletion uses an explicit
channel grant on an assigned role; open-channel defaults and `manage_server`
alone do not grant it. The server owner can moderate server channels, but cannot
delete another participant's DM. Editing remains author-only.

References:
[role model](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/models/role.go),
[role update](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/services/roles.go),
[message authorization](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/services/messages.go),
[Aero message actions](https://github.com/Papo-Chat/papo-frontend/blob/3817e3e1760b26c470eb7deefb32f9d4f616395c/src/lib/components/Message.svelte).

## Delivery parts

| Part | Brief items | Scope | Status |
| --- | --- | --- | --- |
| 1 | 1, 2; role/permission portion of 6 | Preserve all eight role permissions; shared moderator deletion policy; acknowledged Delete from chat, pins and search | Accepted by user on 2026-10-23 |
| 2 | 3 | Profile block/unblock, blocked-user settings, hide/reopen DMs, global-list and account synchronization | Accepted by user on 2026-10-23 |
| 3 | 4; webhook authorization portion of 6 | Webhook management, one-time token disclosure, identity edits and revocation | Accepted by user on 2026-10-23 |
| 4 | 5 | Structured custom embed composer/editor, preview, limits and protected-media preservation | Functionality accepted by user on 2026-10-23; UI provisional |
| 5 | 7, 8 | Apply channel notification policy to all with partial-failure reporting; independent inbox pagination and mark-loaded-read | Accepted by user on 2026-10-23 |
| 6 | 9, 10 | Permission-aware `@everyone` and shared-catalog Unicode emoji suggestions | Built; awaiting user review |
| 7 | 11 | Account font size/message density, whole-app UI density, adaptive multiline composer, accessibility and preference migration | Pending |

Item 6 is a cross-cutting audit of existing administration, rather than a new
administration implementation. Its remaining owner/delegate, channel, ban/reset,
sticker, audit and mutation-refresh checks remain open until verified against the
official backend. All eleven brief items must be implemented or explicitly
reported as gaps before the overall pass is complete.

## Expanded appearance scope (user feedback, 2026-10-23)

Part 7 covers the entire Compose UI, beyond message typography. Its compact
option must reduce the visual height/padding of the channel header, server
header, channel/DM rows, navigation pills, composer and related controls. Keep
Papo's frosted pills and Android-native behavior, preserve accessible touch
regions and system font scaling, and verify the ownership of account-synchronized
versus device-wide preferences against the backend before choosing storage keys.
Use shared sizing/spacing tokens rather than independent per-screen scaling.

The supplied Discord/Papo screenshots also identify a layout problem independent
of font size: the current composer reserves 40 + 40 + 48 = 128 dp beside every
line for emoji, GIF and Send, plus the separate gallery button and editor padding.
The multiline editor needs more usable text width. Prototype a separate bottom
action row first, and compare with a compact vertical action stack if necessary;
choose based on actual text width, total composer height and reachable controls.
Keep the single-line composer compact and avoid layout oscillation while typing.

Review the prototype with the user before proceeding. Check narrow/regular phones,
light/dark, keyboard open/closed, single/multiple lines, long channel placeholders,
attachments/replies/recording, mentions/stickers and enlarged system text. Verify
caret/selection, text and control clipping, Send visibility, draft restoration and
existing chat geometry/shortcuts without changing the reading/hourglass reducer.

## Part 1 implementation and review

- Shared models and effective role aggregation retain `manage_webhooks`; typed
  UniFFI capabilities, Compose and egui role editors expose it. Older seven-bit
  saved editor state is reseeded from the server role rather than guessing that
  the previously unrepresented permission is false. Existing-role editors wait
  for the acknowledged administration snapshot, so pre-upgrade cached roles cannot
  supply an incomplete replacement permissions object.
- Shared core helpers decide whether a message can be edited/deleted. Snapshot
  flags drive the UI, and native mutation entry points recheck current authority.
- Chat, pin results and search results use one deletion confirmation. Real
  deletion waits for the backend response; rejection leaves the message in place
  and keeps an English/Portuguese error in the confirmation dialog. Requests and
  response projection remain account-scoped.
- Host tests cover authors, assigned/unassigned moderator roles, revocation,
  owners, DMs, signed-out/pending messages, eight-field serialization and actual
  role-update payloads. A real loopback HTTP fixture checks denied deletion,
  successful deletion and reconciliation of history, detached results and pins.

User checks for the first APK:

1. Open a role, find **Manage webhooks** (also searchable), rename it and verify
   the existing permission remains unchanged. Check editing/recreation/Cancel.
2. Hold someone else's message as an owner or authorized moderator. **Delete**
   should appear; **Edit** should remain absent. Check Cancel and confirmation.
3. Check the same Delete action from pins and search. A server rejection must
   leave the message visible and the confirmation recoverable.
4. As an ordinary member, other people's Delete actions remain hidden. Even a
   server owner cannot delete another participant's DM.

Live production moderation/role mutations are left to the user's test account;
the build verification uses demo data and loopback fixtures. The complete Android
regression suite and final PR CI verification remain due after UI acceptance.

## Part 1 validation

- `cargo test --locked --workspace`: 792 passed (275 egui, 52 Android adapter,
  465 shared core); one pre-existing ignored smoke test remains ignored.
- Strict workspace Clippy and `cargo check --locked -p papo --lib`: passed.
- Compose formatting check, universal debug APK, instrumentation APK compilation
  and Android lint: passed.
- Test APK: `target/android-apks/papo-android-0.8.5-aero-permissions.apk`, containing
  arm64-v8a and x86_64. Installed on the phone emulator; startup verified with the
  existing account. No production messages/roles were modified for verification.
- No Android instrumentation suite was executed for this UI slice before the
  requested user review. Its execution and final PR CI verification remain open.

## Part 2 implementation and review

- Native profiles expose confirmed block/unblock actions, with self and external
  integration profiles excluded. Blocking applies to DMs only; shared-channel
  messages remain visible. Unblocking does not automatically reopen a hidden DM.
- Client settings includes a searchable, account/server-scoped blocked-users page
  with canonical names, loading, empty, retry and unblock states.
- Holding a conversation in either DM list opens the shared Hide action. Hiding
  preserves history; profile messaging reopens the canonical conversation. The
  backend can also show a hidden conversation after a new incoming message.
- Acknowledged core updates remove hidden/blocked DMs and their unread indicators
  immediately, even without a successful follow-up list refresh. Late DM events
  cannot reinsert an identity still in the current account's block list. Removing
  the active DM exits its route; empty bootstrap snapshots do not close restored
  conversations. This does not modify reading/hourglass state transitions.
- Awaitable UniFFI operations use the existing core HTTP API, freeze credentials,
  and reject stale account responses. Semantic errors have English/Portuguese
  presentation, including a block by the other participant. An inaccessible DM
  is removed without incorrectly adding the peer to our own block list.

Validation:

- Rust workspace: 795 passed (275 egui, 55 Android adapter, 465 core); the existing
  ignored smoke test remains ignored. Strict workspace Clippy passed.
- New host checks exercise actual HTTP paths, cookies and request payloads,
  rejected hides/blocks, successful mutations without list refresh, bidirectional
  block failures, unblock/reopen, retained history, late events and account swaps.
- Compose formatting, universal debug APK, instrumentation APK compilation and
  Android lint passed. Two new UI regression flows are compiled; execution is
  deferred until this UI slice has been reviewed by the user.
- APK: `target/android-apks/papo-android-0.8.5-aero-social.apk` (arm64-v8a/x86_64),
  installed on the phone emulator without clearing saved accounts. Production
  block/hide mutations are left to the user's test account.

User checks:

1. Open a native profile, block and unblock. For an open DM, blocking should
   return to its list while keeping the profile usable. Shared channels stay visible.
2. Search client settings for **Blocked users** and unblock there. The DM should
   stay hidden until explicitly opened again (unless a new message arrives).
3. Hold a DM in the server and global lists, hide it, then reopen from the user's
   profile. Its history should remain, and other servers/accounts should be unchanged.

Contract references:
[block service](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/services/user_blocks.go),
[block storage](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/storage/user_blocks.go),
[DM storage](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/storage/direct_messages.go).

## Part 3 implementation and review

- Server settings and its individual-setting search expose Webhooks only with
  `manage_webhooks` (or server ownership). Delegates with only this permission
  can reach administration without receiving other administration privileges.
- Management lists active integrations with channel and persistent nickname.
  Creation permits text/voice channels, excludes DMs/categories, and enforces
  the backend's 80-character name limit. Identity editing supports a separate
  100-character display nickname, preserving untouched avatar fields; empty
  values explicitly clear nickname/avatar as defined by PATCH.
- Avatar selection reuses the shared crop/animation pipeline and integration
  avatar renderer. An acknowledged avatar edit invalidates only its existing
  account-scoped cache entry; it does not create another media cache.
- The new token exists only in an unpersisted one-shot UniFFI receipt and a
  transient disclosure dialog. Metadata contains no token. Receipt debugging
  redacts credentials; the dialog prevents screenshots, discards disclosure on
  leaving/backgrounding, and marks explicit clipboard copies sensitive.
- Revocation requires confirmation and preserves prior messages. Successful
  create/edit/revoke changes reconcile from acknowledged responses, without
  depending on another GET. Unconfirmed create responses are not automatically
  retried; the UI requires checking the list before another creation.
- Core exposes semantic HTTP permission rejection for localized presentation.
  Existing audit sentences/icons recognize webhook create/update/revoke; server
  audit filters include these events and revocation receives danger styling.

Validation:

- Rust workspace: 799 passed (275 egui, 58 Android adapter, 466 core); one existing
  ignored smoke test. Strict workspace Clippy passed.
- Host tests cover webhook-only delegates, insufficient server-management
  permission, owner isolation, channel/name/image validation, one-time token
  consumption/redaction, omitted versus cleared PATCH fields, actual avatar
  upload, canonical HTTP routes/cookies, server denial, unconfirmed creation,
  session changes and readable audit events.
- Compose formatting, universal APK, instrumentation APK compilation and Android
  lint passed. New UI/UniFFI/cache regression flows compile; their execution is
  deferred until the user's review of this UI slice. Android CI for Part 2 passed
  compilation checks; it does not run the emulator instrumentation suite.
- APK: `target/android-apks/papo-android-0.8.5-aero-webhooks.apk` (arm64-v8a/x86_64),
  installed on the phone emulator without clearing saved accounts. Production
  webhook creation/editing/revocation is left to the user's test account.

User checks:

1. Open **Server settings → Webhooks**, or search server settings for Webhooks.
   Create one in a text/voice channel and copy the one-time token before closing.
2. Reopen its row: the token must not be retrievable. Edit the nickname/photo;
   verify Cancel/discard, an untouched photo, removing a photo and animated images.
3. Cancel a revocation, then confirm it. Previous messages should remain. A user
   with only Manage webhooks should see this section; ordinary users should not.

Contract references:
[webhook handlers](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/handlers/webhooks.go),
[webhook service](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/services/webhooks.go),
[Aero management](https://github.com/Papo-Chat/papo-frontend/blob/3817e3e1760b26c470eb7deefb32f9d4f616395c/src/routes/admin/webhooks/+page.svelte).

## Part 4 implementation and review

- Send's long-press menu opens **Create or edit embeds**. An empty composer exposes
  the same options on tap. A compact draft chip opens the editor again without
  expanding the main textbox. The editor stages changes until Apply; Cancel and
  its discard confirmation preserve the previous draft. Cards can be added,
  removed and moved within the backend's ten-card limit.
- Typed UniFFI drafts cover title, description, link, accent, site, author identity,
  footer text, image, thumbnail, direct video and up to 25 fields per card. Shared
  core validation counts Unicode code points, including the backend's 4192-character
  description and 6000-character aggregate text limits. Presentation errors and
  counts use English/Portuguese Android resources and plurals.
- Explicit Preview uses the received-card layout, shared media cache/decoder,
  player and viewer. Image/thumbnail preview follows the backend's primary image
  priority. Previewing, rather than typing a URL, starts remote media requests;
  actual server downloading/resizing can still reject or change the media.
- Images can use HTTPS URLs or uniquely named image files already attached to
  the same message. The native adapter resolves the actual multipart filenames;
  changed/removed references fail validation rather than silently losing media.
  Referenced uploads become embed media rather than duplicate attachments.
  Spoiler files remain independent attachments because embed media has no filename
  on which to preserve the agreed spoiler cover. Video uses the supported HTTPS
  URL plus MIME input, not a fabricated attachment URL.
- Embeds use the existing durable text outbox or attachment upload path, retaining
  reply bindings and the per-reply notification choice. Owner-scoped drafts survive
  navigation/restarts and participate in Forget server cleanup. A late send ACK
  compares the persisted draft before clearing it, preserving later edits made
  after leaving and re-entering the conversation.
- Existing-message editing keeps stable mention bindings and now waits for the
  acknowledged server result. Custom metadata and direct-video cards are editable;
  an untouched embed list is omitted from PUT, while an explicit empty list removes
  editable custom cards. Ordinary automatic previews remain backend-managed.

**Confirmed backend limitation:** received custom image/thumbnail/author media is
protected and its identity is absent from authoring inputs. `PUT /messages/:id`
replaces the full custom list and accepts neither existing embed IDs/protected
media identities nor multipart replacements. Footer icon input is also absent.
If any custom card cannot be represented without data loss, Compose disables
replacement of the entire list and explains why. Message text can still be edited
with the list omitted, preserving the original IDs and media. No public URL is
invented and no editable subset is sent in place of the complete list.

References:
[custom input and limits](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/services/embeds_custom.go),
[create/edit service](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/services/messages.go),
[edit handler](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/handlers/messages.go).

Validation: 805 workspace Rust tests pass (275 egui, 60 Android adapter, 470 core;
one pre-existing ignored test). Host checks cover full typed round-trips, Unicode
limits, multipart attachment references, the real HTTP omission/replacement/removal
contract, rejected edits, author isolation and preservation of protected media.
Strict Clippy, Compose formatting, universal debug APK, instrumentation APK
compilation and Android lint pass. Two new UI flows are compiled, not executed;
local instrumentation remains deferred until user acceptance.

User checks:

1. Hold Send, or tap Message options with an empty textbox. Create a card, preview
   it, Apply and send with/without ordinary message text.
2. Add/move/remove multiple cards, inline fields and author/footer details. Try an
   invalid URL/color and overlong text; Apply must explain the problem.
3. Attach an image first and reference it from Media. Rename/remove that file after
   Apply; the composer must request correction before sending. Check an HTTPS image,
   GIF or direct video in Preview, including the shared player/viewer.
4. Leave the conversation and return with an unsent embed. Cancel an editor change
   and verify the original draft remains. Forgetting the server removes this draft.
5. Hold an authored message and edit its card/title/body. Preserve any mentions and
   unsent composer draft. A protected-image message must explain the editing limit;
   a text-only change must retain its original media. Server rejection keeps the
   editing dialog open.

## Part 4 user feedback

The user accepted the working functionality on 2026-10-23 and asked to continue.
The editor's visual design remains provisional; retain that feedback for the
appearance/polish review rather than treating the current layout as a final design.

## Part 5 implementation and review

- Server information retains per-channel notification choices and adds one policy
  selector plus **Apply to all** for its current text/voice channels. Categories and
  DMs are excluded. The native adapter calls the existing account/channel endpoint
  sequentially through the shared request governor. A rate-limited request can be
  retried once after that governor's deadline. Every confirmed channel is projected
  independently, without relisting chat/navigation/read state.
- Success counts require backend ACKs, including when the requested policy matches
  the local value: another client may have changed it. Partial results name the
  unconfirmed channels and distinguish missing channels, denial, rate limits and
  unconfirmed transport outcomes. Retry targets only those channels and retains
  previously confirmed changes. The operation is idempotent; the UI never claims
  an unconfirmed change left the server untouched.
- The core API now exposes `notifications`, `has_more` and an exact nanosecond
  timestamp/ID cursor. The shared inbox pager owns raw-page cursors, descending
  ordering, deduplication, request tickets and monotonic read flags. Filtering an
  own-author notification cannot erase the raw cursor. Empty/repeating pages that
  claim more data produce a retryable error rather than an infinite loading loop.
- Native pagers belong to their server/account. Account changes reset history and
  invalidate tickets; credential renewal invalidates in-flight tickets while
  retaining that same owner's loaded history. The authenticated response and cache
  update are checked under one lock. Deleted message notifications cannot be
  resurrected by an older response.
- A live head refresh merges into previously loaded pages. If a new head has no
  overlap, pagination resumes from that new head so an intervening gap is not
  skipped. A refresh no longer depends solely on the unread count: native inbox
  revisions include live notifications, reconciliations and deletion events. REST
  page application deliberately does not increment this signal and refetch itself.
- The ViewModel owns per-account UI requests across navigation, queues a head
  refresh behind an active page and suppresses duplicate loads. Hidden inboxes
  retain state and mark refreshes pending without fetching further pages. The
  global list loads older pages incrementally per server, with scoped Retry on
  failure. It keeps existing author/avatar, source-message and read presentations.
- The header now marks **all loaded notifications as read**, replacing the refresh
  icon. It captures only loaded unread IDs, splits requests at the backend's 1000-ID
  limit, keeps earlier ACKs if another batch/server fails and reports the remaining
  unconfirmed count. Individual read controls use the same path. An incoming page
  cannot undo a simultaneous read ACK. Inbox pagination never generates OS alerts.
- All new labels, counts, busy/error states and accessibility text use Android
  English/Portuguese resources. Chat frontier/hourglass transitions are unchanged;
  the inbox uses a read-only projection of existing acknowledgement state.

References:
[notification endpoint contracts](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/handlers/notifications.go),
[service authorization and limits](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/services/notifications.go),
[keyset ordering](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/storage/notifications.go),
[Aero inbox reference](https://github.com/Papo-Chat/papo-frontend/blob/3817e3e1760b26c470eb7deefb32f9d4f616395c/src/lib/store/notifications.svelte.ts).

Validation: 814 Rust tests (275 egui, 64 Android adapter, 475 core; one existing
ignored test), strict workspace Clippy, Compose formatting, universal debug APK,
instrumentation APK compilation and Android lint. New core/HTTP tests cover exact
submillisecond cursors, read/page races, refresh retention, no-progress responses,
raw filtering, deletion races, credential renewal, duplicate requests, account
replacement, real partial policy ACKs and 429 retry. Two new Android interaction
flows are compiled but not run before user acceptance. Final device regression
execution remains due; CI only compiles instrumentation APKs.

User checks:

1. Tap a server name, choose a notification policy and **Apply to all**. All current
   text/voice channel rows should reflect confirmed settings. Override one channel
   afterwards and check the others retain their values.
2. If a batch partially fails, verify confirmed rows stay changed and Retry names
   only the unconfirmed channels. Signing out/forgetting must isolate the old work.
3. Open global Notifications. Scroll toward the oldest entries to load more; each
   account retains an independent cursor, loading state and retry control. New
   incoming notifications must not discard those older loaded entries.
4. Use the header's double-check button. Only loaded notifications become read;
   unseen/unloaded pages and notifications arriving after the action remain eligible.
   Check individual reads, leaving/reopening during a load, and multiple servers.

## Part 6 implementation and review

- The existing scrollable `@` suggestions now offer `@everyone` first for a
  matching prefix, only in a known server text/voice channel with the current
  effective `everyone_message` capability. DMs, categories, missing channels and
  signed-out snapshots do not offer it. Live permission snapshots recompute the
  list without rewriting the draft. The backend remains authoritative.
- The global mention inserts plain `@everyone`, the backend's expected token,
  rather than a fabricated user binding. A real member named everyone remains a
  separate, ID-bound suggestion. Global mentions use the composer's existing bold
  accent treatment when permitted, with offsets mapped through inline stickers.
- Unicode suggestions share the picker's existing `emoji.json` through one cached
  catalog and one matcher. Bilingual keywords support case/accent-insensitive
  matching and aliases separated by underscores/hyphens, such as `:thumbs_up`.
  Repeated glyphs retain all their keywords while producing one result. Server
  stickers retain priority; matching Unicode results follow in the same list.
- Both the main composer and existing-message editor reuse the same suggestion
  builder and insertion operation. A pick replaces the token at the caret,
  consumes its closing colon and subsequent horizontal whitespace, leaves one
  space, and preserves unrelated mention identities, UTF-16 caret offsets and
  custom-sticker rendering. Ordinary IME/selection changes retain the original
  TextFieldValue. An empty query offers the existing catalog; no matches hide the
  list. New queries reset suggestion scrolling to the first result.
- Presentation is separated from matching/selection helpers. Suggestion action
  labels and the global-mention explanation use Android English/Portuguese
  resources; protocol tokens and the catalog's bilingual search aliases retain
  their original values.

References:
[backend mention triggers and authorization](https://github.com/Papo-Chat/papo-backend/blob/af057d5e0e107ae666c4f6776b36222ef18b2105/backend/internal/services/notifications.go),
[Aero composer permission and insertion reference](https://github.com/Papo-Chat/papo-frontend/blob/3817e3e1760b26c470eb7deefb32f9d4f616395c/src/lib/components/Composer.svelte).

Validation: Compose formatting, universal debug APK, instrumentation APK
compilation and Android lint pass. The existing native wire/edit regression test
passes on the host, confirming plain `@everyone` and bound adjacent user mentions.
Four new algorithm/UniFFI regression cases and
two UI flows are compiled for permission changes, DM/category exclusion, member
name collisions, duplicate glyph aliases, empty/no-result queries, replacement
inside a shortcode, adjacent bound mentions, projected highlights, IME state and
navigation persistence. Their device execution remains deferred until UI review;
no reading/hourglass state transitions are changed in this part.

User checks:

1. In a server channel where you can mention everyone, type `@ev` and select
   `@everyone`. Check the final space and caret. An ordinary member without that
   permission, or a DM, must not offer the global action.
2. Type `:thumbs_up`, `:smile` or a server sticker's prefix. The same list should
   offer custom stickers first, followed by matching Unicode glyphs. Selecting a
   glyph inserts the actual emoji; selecting a sticker preserves its inline image.
3. Try a pick between existing text/mentions, an empty `:` and a query with no
   matches. Leave/reopen the chat with the draft and repeat inside message editing.

APK: `target/android-apks/papo-android-0.8.5-aero-suggestions.apk`, containing
arm64-v8a and x86_64, installed on the phone emulator without clearing saved data.

## Part 6 follow-up: received global mentions

The user's two-account test exposed a split between composer authorization,
received-word styling and recipient classification. The composer already checked
`everyone_message`, while the shared recipient helper accepted any literal
`@everyone`/`@todos`. Egui used that helper for the row band but checked the author
for its word pill, producing contradictory visuals. Compose had no received
`@everyone` span or persistent mention-row projection at all.

- Shared recipient classification now requires the author's effective permission
  for `@everyone`, retains direct ID-bound mentions, checks whole readable names
  and prevents a member/role named everyone from bypassing the reserved token.
  Unknown/external authors do not gain broadcast permission. Unsupported `@todos`
  no longer masquerades as the backend's broadcast token.
- Native Markdown projections receive the author identity and include the
  permission result in the bounded document cache key. Valid global text gets an
  accented, bold span while code, links and bound user mentions retain their
  existing semantics. Permission changes cannot reuse the opposite cached result.
- Compose consumes one core `mentionsMe` flag for a persistent row tint, with the
  existing reply-navigation spotlight taking precedence. Own permitted broadcasts
  keep a valid word accent but do not generate a self-recipient band or self-alert.
  The same received-word renderer covers timeline, search and pins.
- OS broadcast delivery waits for the backend's authoritative Notification rather
  than inferring send-time authorization from cached roles. Ignoring the raw Message
  consumes no ledger entry, so a subsequent valid Notification still delivers once.
  Direct mention fallback and Message/Notification deduplication remain covered.
- Author validity uses currently known role metadata: the backend does not attach
  a historical permission decision to each message. Broadcast delivery remains
  backend-authoritative independently of that presentation metadata.

Validation: 819 workspace Rust tests pass (275 egui, 67 Android adapter, 477 core;
one existing ignored test), including permission inversion, own-versus-recipient
presentation, grant/revocation cache invalidation, reserved-name collisions,
formatted text and backend-confirmed broadcast delivery. Strict Clippy, Compose
formatting, universal APK, instrumentation APK compilation and Android lint pass.
The device suite remains deferred; no production messages or role settings were
changed for automated verification. Reading/hourglass transitions are unchanged;
their existing mention classification now uses sender-aware input.

Two-account review: send plain `@everyone` from an ordinary member and then from
the owner/permitted delegate. The first stays ordinary; the second has an accented
word on both accounts and a recipient tint only on the other account. Confirm
notification delivery with the receiver outside the chat and channel notifications
enabled. APK: `target/android-apks/papo-android-0.8.5-everyone-fix.apk`.
