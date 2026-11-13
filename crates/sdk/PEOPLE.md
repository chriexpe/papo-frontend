# People, permissions and live activity

`papo-sdk` owns resource reconciliation, bounded profile requests, permission
predicates and ephemeral activity. `papo-sdk-ui` exposes `PeopleService`,
`CapabilityService`, `DirectMessageService` and `ActivityService`. None owns
native widgets, navigation, media downloads, notification policy or search.
Administrative/profile/block mutations remain separate operations work.

## Authorization specification

The specification is Go revision
`9da01ccc191a764517441f7dde743beb747ea516`, pinned in
`api/protocol/backend-commit`. Relevant implementation paths are
`backend/internal/services/messages.go` (`userHasChannelPermission`,
`userHasRolePermission`, create/edit/delete/pin/unpin), `services/voice.go`,
`services/reactions.go`, `middleware/permissions.go`, `handlers/routes.go`,
`services/direct_messages.go`, `storage/direct_messages.go`,
`storage/users.go` and `websocket/client.go` (`handleTyping`).

These are affordances, never authorization tokens. The backend authorizes every
request. `CapabilityService` reports typed unavailable inputs before reconcile,
while offline, after replacement, or when a required role/overwrite is absent.
Individual omitted Boolean fields are false; an absent whole permission list
is not a known-empty/open list. Cached definitions can become stale even while
online: this backend has no role-definition, overwrite-only or block-change
events. Reconnect replaces them; membership events also refresh role definitions.

| Predicate or operation | Pinned backend rule |
| --- | --- |
| Global permission | Owner, or any explicitly assigned role with that bit |
| Nonempty channel overwrites | Independent OR of each field across assigned roles; false never overrides another true |
| Empty ordinary-channel overwrites | Read/send/voice granted, deleting others denied |
| Role hierarchy/default everyone/admin implication | None |
| Sending | Send grant and channel type text/voice/dm; read is independent |
| Uploaded attachments | Sending plus global send_attachment, except DMs |
| Voice join | Voice grant and channel type voice; read is independent |
| Ordinary pin | Global pin_message, without an additional read check |
| Ordinary unpin | Read plus global pin_message |
| Add/remove own reaction | Send/read respectively |
| Typing | Read, even if sending is denied |
| Edit own message | Authorship; DM edits additionally require unblocked membership |
| Delete own/other message | Authorship/channel delete grant; DM never grants deleting others |
| Ban/unban | Owner or manage_server; owner targets protected. The stored ban_members bit is not this route's gate |
| Reset another person's password | Owner; manage_server alone does not suffice |
| Everyone mention | everyone_message affects notification expansion, not submission of mention text |
| DM read/send/pin/unpin | Membership and no block in either direction; owner has no bypass |
| Hidden DM | Hiding changes list visibility, not membership/access |

The pinned `storage.ListDirectConversations` SQL filters membership, hidden_at,
and a NOT EXISTS block query in **both directions**. Thus a freshly reconciled
DM list proves access for its owning session without requiring reverse blocks
in the own block list. A queued DM event can predate that REST image: each
`DMUpdate` therefore refreshes the authoritative DM list and own blocks instead
of reinserting its payload. This adds one DM-list request per event, avoiding
stale block/visibility inference. The targeted GET-by-ID endpoint alone would
not suffice because it intentionally returns hidden conversations.

A DM list proves access only for its owning session. The public raw
`capabilities(snapshot, user, channel)` evaluator handles ordinary channels and
returns unavailable for DMs. Session-bound `CapabilityService` can use its own
backend DM list. `direct_grants` accepts explicit membership/either-direction
block evidence for cases such as an already-known hidden conversation. The own
block list alone cannot establish absence of a reverse block. Missing DM list
evidence remains unavailable, never an invented nonmembership decision.

`AuthorIdentity` handles external/webhook identity before any bridge account ID.
Clients obtain no person roles, mention identity or DM target from that branch.
The backend has no invented system-account permission flag.

## Incremental people and bounded work

Reconnect loads lightweight member pages; individual join/role/avatar events
continue to request exactly one lightweight summary. They replace/remove only
that person and invalidate only their requested profile. Role assignment events
also refresh the small role definition list, including a newly created role.

Full profiles load only for caller-requested IDs, with at most 1000 input IDs per
call, deduplication, and at most 50 IDs per HTTP request. Each batch releases the
resource serialization lock before admitting another batch so live invalidation
can progress. Replies outside the requested batch or with duplicate identities
are rejected. Avatar bytes from the existing wire API are excluded from the UI
profile projection and SQLite; media handling remains separate.

People lists keep unchanged item Arcs, use generation/version-stamped diffs and
Reset after source discontinuities. Relevant resource Arc identities prevent
presence-only publications from projecting/diffing a roster or sorting DMs.
Sources coalesce at capacity one and close on session revocation. Inline avatar
blobs remain only in the explicit owned profile response for a future media
handoff; the shared in-memory profile snapshots also exclude them. Global DMs use
(account, channel) identity and sort parsed activity instants, retaining exact
backend timestamps. Removing/replacing an account removes its rows; actions must
use the originating session and validate its generation again.

## Ephemeral activity

Presence and typing use memory-only, generation-scoped `ActivitySnapshot` as
their single source. `ResourceSnapshot` has no duplicate presence vector, and
presence events do not advance resource versions or write the account cache. Offline users
are retired, connection loss clears activity immediately, old generation events
cannot restore it, and removal closes subscriptions. There is no SQLite table,
restart restoration, or background send retry for activity.

Typing expires exactly ten seconds after receipt, with refresh/stop handling and
an independent SDK timer, even without another gateway event. At most 1024 active
(channel, user) entries are retained. A custom typing phrase is metadata, never
proof of activity. UI projections omit the current user and supply raw phrase and
nickname metadata; pluralization, ellipsis and avatars belong to clients.

`report_typing` coalesces best-effort starts onto the existing session gateway.
It requires a current online generation, discards stale commands, and does not
replay them after reconnect. The backend has no outbound stop command.

## Validation

Run `bash scripts/papo-sdk-check.sh` for deterministic unit/property, public
loopback, storage and existing crash gates. New regressions cover the real expiry
task on an injected paused clock, generation replacement/offline/removal,
profile admission/deduplication, member invalidation and stable rows, unavailable
capabilities while disconnected, account-bound DM inference and unified DM
revocation/sorting/isolation. A 10,000-row unit fixture proves unrelated presence
publications perform zero additional roster projections.

`python3 scripts/papo-backend-proof.py --backend <exact-pin-checkout>` requires a
fresh disposable loopback PostgreSQL database through `DATABASE_URL` and a runtime
`PAPO_PROOF_PASSWORD`. It runs the existing tuple/upload/outgoing proofs plus:

- 1024 channel cases: all pairs of four-bit overwrites and all four membership
  combinations of two roles, compared with production Go service outcomes.
- 1024 global cases: all eight-bit combinations split across two roles, with all
  four membership combinations, compared with production permission middleware.
- 12 ordinary cases: owner/nonowner, open/restricted, text/voice/category.
- 12 DM cases: both participants and unrelated owner under unblocked, own-block,
  reverse-block and hidden states, compared with production operations.
- A real SDK typing command observed through its session gateway against Go.

Unexpected Go errors fail the proof rather than being counted as denials. CI's
existing PostgreSQL job runs this expanded harness. Unit projection tests also
cover independent typing/reaction/attachment gates and external author separation.
Actual REST regressions also create a DM, block in each direction, hide it and
reconcile a real Session before checking CapabilityService. A held-reconcile
fixture replays an obsolete DM event after a newer empty list and proves it
cannot restore list rows or grants. A 10,000-user/100-update presence fixture
asserts unchanged resource publication/cache revisions; a 2 MiB avatar fixture
asserts metadata-only shared retention and intact owned response bytes.

This proves shared SDK behavior, not native client presentation or power-loss
protection. Ambiguous outgoing sends remain Unknown and are never automatically
retried.
