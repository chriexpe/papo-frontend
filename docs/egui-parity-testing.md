# egui parity batch — manual test checklist

Started from the Compose handoff checkout (`661e663`) on 2026-10-27; the PR is based on main (`a1903f5`, including merged #127). This batch
keeps the desktop call layout, as requested. The running app is a development
build, not a release performance benchmark.

## Handoff status

The concrete compatibility work from the handoff is implemented in this batch:
call attempt/cancellation and leave ordering, audio routes, separate screen
subscriptions, participant playback controls, unknown presence, message policy,
fresh settings patches, account appearance, online cache/HTTP overlap, roster
teardown and startup asset restoration. Reading/marker integration was adjusted
without discarding saved positions. Implementation and focused checks do not
establish complete live integration or full frontend parity.

The broader handoff remains open. Next audit areas, in order:

1. Media across chat, pins/search and fullscreen: backend authority, geometry,
   player reuse, viewport pause/resume and bounded resource retention.
2. Composer/attachments: multiline width, mention identity through editing,
   staged rename/randomization/spoiler/metadata/crop, and ambiguous upload retries.
3. Accounts/administration: recovery and trusted-link transport, block/hidden-DM
   ownership, role-field preservation, complete member/audit pagination, and
   acknowledged mutation outcomes. Existing implementations need comparison;
   this list is not a claim that those features are absent.
4. Remaining lifecycle verification: actual call transport, reconnect/account
   replacement, unread intervals and saved positions with real media/history.
   Performance conclusions require a matched optimized-build measurement.

Known unimplemented additions are webhook avatar editing and a combined
bulk-notification partial-failure summary. Complete private attachment-derivative
cleanup during server forgetting also needs verification. The handoff's PR #127 status is historical; it is merged in the PR base.

## Changes available to test

| Area | Exact change | Suggested check |
| --- | --- | --- |
| Call entry | ICE discovery no longer joins automatically. Only a current account/attempt may send `voice_join`; SDP waits for `voice_joined`. | Start joining, cancel immediately, then join again. The cancelled attempt must never enter later. |
| Call replacement | A shared leave-acknowledgement barrier serializes replacement calls, including same-room rejoin. Timeout keeps the barrier; disconnect cancels queued entry. A waiting replacement has Cancel. | Leave/rejoin quickly, switch rooms, cancel the waiting replacement, and disconnect during entry. |
| Call roster | Local leave removes only self. Other occupants remain. Cold presence no longer claims a known microphone/camera state. | Leave a room containing another user, then inspect its roster and a newly loaded room. |
| Call audio | The adapter forwards shared audio-route signaling. The entire participant roster row or tile opens an anchored dropdown for local mute and 0–200% playback gain, profile and DM actions. Mixing is disabled with an explanation if the server does not publish track ownership. | Adjust a remote participant while confirming their sender-microphone indicator remains independent. |
| Call video | Camera and shared-screen subscriptions are independent. A publisher can have both tiles. Screen frames retain their aspect ratio; texture identity includes publisher and stream kind. | Receive camera and screen from the same participant, stop either, and let another publisher reuse a slot. |
| Message permissions | Edit/Delete use shared policy. Explicitly authorized moderators can delete others' server messages; server ownership does not override DM privacy. Pin follows shared policy. Dispatch rechecks permission and preserves the originating owner. | Compare author, ordinary member, channel moderator and owner menus in chat, pins/search and DMs; change permissions with a menu open. |
| Profile administration | Ban/unban is gated by server-management permission; another user's password reset link is owner-only. | Compare ordinary, delegated manager and owner profile menus. |
| Appearance | Account font size Small/Normal/Large maps to `small`/`medium`/`huge`; message spacing Compact/Normal/Comfortable maps to the shared wire values. System/device zoom remains separate. New controls are in Settings → Appearance. | Change each independently, switch accounts/servers, restart, and change the account setting from another client. |
| Extra compact chat | Extra compact uses a timestamp gutter, repeats the colored sender beside every message, omits avatars, and keeps replies above the body. Text, Markdown and links share the sender line; emoji stay inline. The original Compact keeps avatars and grouped sender headings. Extra compact is saved locally and maps to Compact for account sync. The appearance preview follows the selected layout. | Select Extra compact and compare consecutive authors, replies, wrapping, links, formatted text and attachments. |
| Composer Markdown preview | The desktop textbox styles headings, bold, italics, strike-through, code, quotes and safe links while keeping markup visible and subdued. Layout uses the real styled height; raw source, mention bindings and inline sticker editing are retained. | Type `# Test`, `**bold**`, mixed Unicode and multiline code; edit/copy/undo, add stickers, and confirm the sent Markdown matches. |
| Chat typography | Messages use bundled Inter Regular with softer body color; names and Markdown emphasis use Inter Semibold. Reply text and metadata use smaller chat styles. System UI font and account text sizing remain available. | Compare names/body, Markdown emphasis, replies, links, light/dark appearance and account text sizes. |
| Settings writes | Cached full-config replacement was replaced with fresh raw-config field patches. Unknown and unrelated fields survive. Owner checks and write revision acknowledgements protect account replacement and rapid reversal of a setting. The menu notification toggle also syncs. | Change theme/alerts here while changing typography elsewhere; rapidly toggle an option off/on; switch accounts before a response returns. |
| History | Online paging starts HTTP immediately and previews connected cached rows concurrently, in both directions. Offline older paging retains its cache-only path. Exact cursors, tickets and mutation replay remain shared. | Scroll into cached history online/offline; prepend pages without losing the visible anchor; edit/delete a row while a page is loading. |
| Startup assets | Removed the redundant separate profile-image/emoji restoration reads; shared startup metadata already restores them. | Restart with populated cache and confirm avatars/stickers appear before the connection refresh completes. |
| Reading/markers | Read markers are overlays, so moving them no longer changes row heights or breaks author grouping. Covered chat does not count as observed reading. Saved-position logic is retained. Day grouping uses the full calendar date. | Move through the unread interval; open a viewer/profile/panel/settings or call surface; jump newest, leave/reopen, and test media whose dimensions resolve late. |
| Composer durability | Text, mention bindings, reply and custom-card draft remain until the durable outbox acknowledges enqueue. Repeated Send while waiting cannot enqueue twice. A late acknowledgement cannot erase a newer edited draft. | Send online/offline, press Send repeatedly during enqueue, and navigate away before acknowledgement. |
| Custom-card editing | Caption-only edits omit the embed list. Structured edits now use the shared all-or-nothing safety check, including supported public-video round-trips. Protected media cannot be silently removed by a replacement list. | Edit captions and editable cards, then try a message containing protected image/author media. |
| Server forgetting | Forget works signed out and for the final saved entry. A durable journal suppresses stale credentials and late credential writes, retries local cleanup on startup, clears that server's account settings/read marks/cache, and allows explicit re-add after cleanup. Final removal returns to an empty sign-in surface. | Remove a disposable saved server; restart; confirm unrelated servers remain; re-add it. |
| Notification inbox | The bell in the server rail opens an account-scoped inbox. Each account uses the shared pager with exact timestamp/ID cursors, older-page loading, refresh, monotonic read flags, deletion reconciliation and stale-result rejection. Selecting a record navigates/highlights its source message. | Load older pages, refresh, open a source, delete its message, then switch/sign out while a request is pending. |
| Notification policy | Channel policies now change locally only after server acknowledgement, rather than claiming an unconfirmed write succeeded. | Change several channel policies and compare successful versus rejected requests. |
| Webhooks | Server settings gains a permission-gated Webhooks pane: list, create in text/voice channels, edit display name, revoke with confirmation, and copy/dismiss the one-time creation credential. The list, create form and inline display-name editor reuse channel settings components. Acknowledged results are account/request scoped; failures retain the loaded list. Credentials are not serialized and Debug output redacts them. | Compare a webhook delegate with an ordinary user; create, rename, copy/dismiss and revoke; switch accounts while loading. |

## Verification and limits

- The desktop binary was built with `cargo build -p papo --bin papo`.
- All 18 focused `parity` regressions passed (11 egui + 7 core). They cover call cancellation/replacement/disconnect,
  settings patch preservation/ownership/revisions, marker geometry, reading
  coverage, draft acknowledgement, forgetting and webhook receipt lifetime.
- The extra compact layout also passed a focused geometry check for repeated senders, inline plain/Markdown text, wrapped bodies, shared minute timestamps and preservation of the original Compact layout. Three focused reply-preview checks passed after the typography changes.
- The initial compatibility run also exercised the pre-existing egui and core
  suites. Those runs preceded later edits; they are not a claim of a final full
  regression campaign.
- No automated live-server moderation, webhook creation, camera/microphone
  transmission or hardware call test was performed. Manual live integration is
  still required.
- This batch does not establish that every intermittent hourglass or scrolling
  performance issue is solved. Release profiling and other OS/device checks remain.
- Webhook avatar editing and a combined bulk-notification partial-failure summary
  were not added; existing avatar state is preserved and each policy ACK is applied
  independently. Existing Markdown/media/attachment/recovery features were reused,
  not reimplemented or counted as new features.

## Isolated desktop demo pass

The app was exercised through desktop automation with separate XDG config/data/cache
under `/tmp/papo-parity-demo-check`, without saved account credentials. Generated
media exercised the real local media rendering/player paths.

- Pinned video: visible decoded frames, play, seek, fullscreen and return; the
  paused 0:04 / 0:05 position was retained when reopening pins.
- Chat audio: play activated the real player and resolved an eight-second duration.
- Composer: multiline entry, local demo send, bold Markdown and mention rendering.
- Appearance: normal/large text, light/dark themes, original Compact and Extra compact.
- Calls: simulated roster/tile presentation and clicking the nickname to open an
  anchored participant menu; unavailable audio mixing remains disabled/explained.
- Webhooks: simulated integration list and compact inline display-name editor.
  The demo now supplies local webhook create/update/revoke outcomes for UI review.
- Fixed issues found during the pass: submenu selection outside the settings sheet
  closed the sheet, saved density was not initialized into chat at startup, and
  webhook footer layout consumed the remaining height.

Demo Search has no simulated request results; search retrieval, real calls, server
acknowledgements and long-running performance are not established by this pass.

## PR follow-up validation

- `cargo clippy --locked --workspace --all-targets -- -D warnings` passed locally.
- `cargo test --locked --workspace` passed: 867 tests (273 egui, 76 Android adapter,
  15 call engine, 503 core); the existing integration test remains ignored.
- The running isolated demo showed styled multiline composer text and an inline
  sticker with subdued editable delimiters. Saved Extra compact restored at startup.
- CI's reported Rust failures shared three Clippy causes: item ordering after a
  test module, identical empty branches, and an oversized render-helper signature.
  Those were corrected without disabling lint checks.
