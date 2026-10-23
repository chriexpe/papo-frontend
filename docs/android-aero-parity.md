# Aero parity follow-up for PR #124

Source brief: `/home/christian/Downloads/aero-plan.md`. Work stays on
`feat/android-compose` / PR #124. Official repositories are read-only references.
Calls, voice-channel text chat and the deferred hourglass investigation are outside
this pass. Each part with substantial UI changes stops at a testable APK for user
feedback before continuing to the next part.

## Contract baseline

Verified against the official sources on 2026-10-22:

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
| 2 | 3 | Profile block/unblock, blocked-user settings, hide/reopen DMs, global-list and account synchronization | Built; awaiting user review |
| 3 | 4; webhook authorization portion of 6 | Webhook management, one-time token disclosure, identity edits and revocation | Pending |
| 4 | 5 | Structured custom embed composer/editor, preview, limits and protected-media preservation | Pending |
| 5 | 7, 8 | Apply channel notification policy to all with partial-failure reporting; independent inbox pagination and mark-loaded-read | Pending |
| 6 | 9, 10 | Permission-aware `@everyone` and shared-catalog Unicode emoji suggestions | Pending |
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
