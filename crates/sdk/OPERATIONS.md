# Account operations

`Session::operations()` (`papo_sdk_ui::OperationService`) binds typed management
commands to one revocable account. Domain inputs live under
`papo_sdk::operations::{administration,channels,roles,moderation,profile,settings,
social,webhooks,emojis,audit,interactions}`. Permission affordances remain in
`CapabilityService`; every request is authorized by the pinned Go backend.

A returned `OperationReceipt` means the server positively acknowledged the
mutation. Its separate `RefreshStatus` reports whether the local projection was
updated, was unnecessary, or encountered a request/cache/generation/concurrent
publication warning. A warning never changes the acknowledged mutation into a
failure. `refresh_metadata(MetadataScope)` retries only reads/publication, never
the mutation. Validate a receipt against the original session before presentation.

Pre-dispatch failures and explicit backend rejections are typed errors. Timeout,
ambiguous transport, interrupted dispatch, successful-but-undecodable responses,
HTTP 408 and server failures are `OperationError::Unknown`. There is no automatic
retry. Dropping a caller does not undo a dispatched mutation; the tracked SDK task
may finish. Requests have the existing request deadline and account cancellation.

Management mutations share an account-local ordering lane. They do not lock the
shared durable writer, block another account, or replace the durable human send
queue. Metadata is fetched outside the resource publication lock; version checks
preserve newer gateway images. Refresh warnings make that conflict observable.
Member updates use bounded requested summaries, never a full roster refresh.
Requested profile snapshots omit inline avatar blobs. Role changes update labels
across existing members, profiles, DM participants, blocks and channel overrides.

| Domain | SDK contract |
| --- | --- |
| Administration | Server PATCH/full replacement; name, visibility, password and encoded icon |
| Channels | Text/voice/category creation, edits, deletion, position/parent and role overrides |
| Roles | Complete eight-bit replacement, rename preserving all bits, assignment/removal/deletion |
| Moderation | Ban/unban and authenticated message deletion; no owner exemption for DM moderation |
| Profiles | Own nickname/status/bio/typing phrase, encoded avatar/banner; explicit reset preparation |
| Sessions/security | Existing `Session::auth()` device listing, explicit revocation and password/reset operations |
| Settings | Backend account/channel settings, separate from durable device notification overrides |
| Social | Verified DM listing/open/hide and block/unblock; no inferred reverse-block permission |
| Webhooks | Management and scoped bearer execution/edit/delete with typed foreign identity/custom embeds |
| Stickers | Encoded emoji upload/delete; media service owns metadata, favorites and encoded media |
| Audit | Actor/action/entity/date backend filters, raw pagination and explicitly local channel/deletion filters |
| Pins/reactors | Confirmed pin/unpin, canonical pin refresh and exact grouped reaction-user pagination |

Batches are bounded to 100 items, retain every ordered item result, and do not
retry errors or silently move remaining work into a new connection generation.
Role creation accepts up to 1000 explicit initial member IDs, refreshed in batches
of at most 50. Role replacement is full replacement: the backend has no CAS or
rename-only endpoint. `RoleUpdate::rename` rejects incomplete permission evidence.
The pinned backend honors channel parent/access options at creation; it offers
neither sticker rename nor role reorder.

`Session::reactions()` is an account-shared bounded ephemeral coalescer. Adjacent
pending desired-membership commands for one message/emoji/generation collapse
before FULL outgoing admission. Acknowledgments explicitly distinguish Queued
from CoalescedInto; they never claim Sent. Already admitted intents are immutable.
HTTP dispatch, ordering barriers, confirmed usage and Failed/Unknown remain in the
existing outgoing service. The SDK does not optimistically alter canonical totals.

Audit `since` is an inclusive lower date bound in either order, not a moving page
boundary. It remains unchanged while the backend resolves raw `last_id` to its
ordered tuple. The continuation comes from the last raw row before local filters,
including an empty visible page. Cursor account, generation and query are bound.
Reaction users are grouped after raw backend pagination; the cursor uses the last
raw tuple across all groups. Page-local group counts are not total reaction counts.

Webhook credentials and reset links remain explicit caller-owned secrets without
Debug/Serialize. Webhook requests use the scoped bearer and no account cookie.
External authors never become person/DM/role targets through bridge attribution.
Reset links carry their issuing account and target; consuming a link uses explicit
server selection, never a server inferred from the link's host.

The backend accepts self-owned DM channel settings but exposes no DM policy
readback. Such mutation receipts report `Warning(Unavailable)`, not a successful
policy projection. This does not change independent device notification settings.
Use shared `NotificationSettings.channels`/`muted` to suppress DM alerts on this
device; the SDK does not invent a server endpoint or silently overwrite device
policy. Ordinary channel settings are refreshed through the real channel list.

Native editors, unsaved widgets, confirmations, image cropping/decoding and
presentation remain client responsibilities. No native UI or new persistence
layer is introduced. Loopback faults/races complement actual Go/PostgreSQL
handler proofs in `scripts/papo-backend-proof.py`; fixture authorization is not the
permission specification.

The isolated production proof provisions an auth burst of 256 because all
independent test accounts share one loopback IP. It uses the real authentication
and permission middleware; this suite is not a rate-limit throughput benchmark.
