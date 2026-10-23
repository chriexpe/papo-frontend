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
| 1 | 1, 2; role/permission portion of 6 | Preserve all eight role permissions; shared moderator deletion policy; acknowledged Delete from chat, pins and search | Implemented; awaiting user UI review |
| 2 | 3 | Profile block/unblock, blocked-user settings, hide/reopen DMs, global-list and account synchronization | Pending |
| 3 | 4; webhook authorization portion of 6 | Webhook management, one-time token disclosure, identity edits and revocation | Pending |
| 4 | 5 | Structured custom embed composer/editor, preview, limits and protected-media preservation | Pending |
| 5 | 7, 8 | Apply channel notification policy to all with partial-failure reporting; independent inbox pagination and mark-loaded-read | Pending |
| 6 | 9, 10 | Permission-aware `@everyone` and shared-catalog Unicode emoji suggestions | Pending |
| 7 | 11 | Account-synchronized font size/density, system font scaling and existing-preference migration | Pending |

Item 6 is a cross-cutting audit of existing administration, rather than a new
administration implementation. Its remaining owner/delegate, channel, ban/reset,
sticker, audit and mutation-refresh checks remain open until verified against the
official backend. All eleven brief items must be implemented or explicitly
reported as gaps before the overall pass is complete.

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
