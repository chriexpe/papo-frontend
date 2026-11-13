# Notifications, background work and search

The pinned Go backend is the wire and authorization authority. This SDK owns
policy and persistence; clients own native permissions, channels, scheduling,
Firebase integration, presentation and navigation.

## Notification handoff

`Session::notifications()` exposes the authoritative paginated inbox, explicit
read acknowledgment, device policy, push registration and a consuming delivery
queue. `papo-sdk-ui::NotificationClient` combines independent account results;
a failed account has a retry result without erasing other accounts.

All live, push-wake and background candidates use the same policy and the same
existing FULL `state.sqlite.notification_ledger`. The key is
`(server_id, user_id, message_id)`, including for ephemeral gateway notification
IDs and backend rows with different IDs. A claim commits before the SDK returns
or queues a platform handoff. A crash after commit can lose a banner. It cannot
replay that banner. This is **at most once**, not exactly once or at least once.
The reliable inbox and read state do not depend on banner handoff.

Claims use authoritative **message** creation time for their 30-day admission
and collection horizon, consistently across live and inbox producers. Notification
row creation time is never substituted for message time. Identities are collected in bounded 100-row account-local
turns. Bounded detached history reads resolve inbox message-time evidence;
budget exhaustion returns More without advancing the inbox cursor. The next wake
resumes that exact history cursor in the same generation, retaining at most one
channel/page of evidence; reconnect starts fresh.
Absent or inaccessible rows remain in the reliable inbox and produce an
observable fault rather than an unproven platform candidate. A persisted monotonic admission floor rejects older candidates, including
following a wall-clock rollback. Collection cannot resurrect an old identity.
Removing an account deletes its policy, polling boundary and ledger entries.

Policy combines backend account switches, backend channel modes, durable device
account/channel settings, mutes, direct conversations, own-message suppression,
explicit foreground-visible channels and known read points. Preview delivery is
off by default; platform lock-screen presentation still belongs to the client.
Only eligible candidates consume a claim; suppression of an eligible candidate
also consumes it, so an unmute does not replay old alerts. An ordinary message
under a mentions-only policy leaves its identity available for a later reply
trigger. Backend `all` events may be ephemeral: they do not prove a mention or
reply. Actual inbox rows supply that evidence. The gateway's `user_id` identifies
the **author**, not recipient; the authenticated unicast session supplies account
ownership. Inbox `actual_mention` distinguishes direct mention tokens in the
returned preview from ordinary DM/reply records; backend previews are truncated
at 512 characters, so this flag cannot prove absence beyond that preview.

Cache hydration and ordinary history/reconnect REST pages are never producers.
Live input and output queues each hold 64 entries; overflow is an observable
notification fault. Canonical history remains independent. Old generations are
discarded before delivery. Validate the scoped target again before native
presentation or tap navigation; fetching/tapping does not acknowledge a read.
Only explicit `mark_read` calls or existing channel read reporting acknowledge
backend state. A stale notification read returning 404 is accepted.

## Background lifecycle

`ClientHandle::background_sync(deadline)` and its explicit-cancellation variant
run on the existing SDK runtime. `push_wake(account, deadline)` treats the push
payload solely as a wake hint for an explicitly resolved account. It does not
trust push previews, IDs or the current active account as delivery authority.
An absent account hint resolves only when exactly one signed-in account exists.

Four account jobs may execute concurrently across all background invocations.
Each account serializes its own polling cursor, verifies identity, refreshes
backend account policy and reads fresh channel/DM metadata independently of UI
state. Already verified transports are reused only in their connection generation.
Fresh policy versions cannot be overwritten by older same-generation responses.
Inbox rows outlive access: a bounded history authorization read validates each
candidate channel without inserting history or reporting reads. Authoritative
403/404/410 rows are skipped with cursor progress; transient failures remain
retryable. One account's failure never cancels another's work.

The first inbox observation establishes a quiet durable baseline. Later runs use
ascending exact `(notification.created_at, notification.id)` pages from that
baseline, at most four 100-row pages/account/invocation. Empty baselines are
explicit local state, not manufactured backend cursors. Each processed row
advances the FULL checkpoint. Deadline/caller cancellation, dropping the caller
future, generation replacement and `interactive_sync()` preemption stop admission
and network work. Claims already committed may lose their banner. Final report
handoffs are revalidated after all account jobs, including early results that
became stale while another account was pending. Keep the interactive guard while
foreground ownership should exclude background work.

## Push ownership boundary

Only the existing authenticated PUT/DELETE `/users/me/push-device` routes are
used. The backend owns a token globally within one server: registering it for a
second user transfers ownership. The SDK serializes transitions, removes an old
token on rotation, preserves known ownership on a definite rejected transfer,
retains both possible owners after an ambiguous transfer,
and closes registration admission before unregistering on logout. An old account cannot delete
a newer account's registration. Tokens are held in memory and never logged or
persisted by the SDK.

Timeouts, transport failures, HTTP 408 and server failures are `Unknown`, not a
successful registration/removal. No automatic mutation retry is introduced.
`unregister_push_device(host_supplied_device)` permits explicit owner-scoped
cleanup/reconciliation after restart or an uncertain result. An empty local owner
map cannot establish remote removal: `unregister_push()` then reports Unknown.
The platform must restore its provider token and explicitly reconcile it on
startup, permission changes and account changes. If remote cleanup fails, local
logout still proceeds and reports the failure. Offline deletion cannot promise
remote erasure; the host must retain the token for later cleanup. Real Firebase
delivery is **not** claimed without a real-device test.

## Detached authoritative search

`Session::search(SearchQuery)` owns backend requests; `papo-sdk-ui::SearchService`
owns an independent observable surface. Different search surfaces do not share
query cancellation or a current DM. Replacing a query cancels old work; page
publication is serialized; cancellation-on-drop also stops dispatched requests.
Subscriptions invalidate idle pages on generation change and close on revocation.

The query supports the actual POST `/search` fields: text, author, explicit
channel/DM, mention UUID, `has=link`, attachment presence **or absence**, inclusive
UTC date bounds and asc/desc order. The parser resolves `de/from`, `em/in`,
`tem/has`, `mentions` and quoted names against lightweight current metadata;
ambiguous labels fail. Local channel/person matching never fetches a roster or
profile list. The backend implements full-text matching and permission filtering;
there is no client full-text index. Global backend search excludes DMs; explicitly
scoping a DM invokes its membership/block authorization.

Each page contains at most 100 results in strict `(created_at, id)` order, not
score order. Exact backend timestamp spellings/IDs form the next cursor only when
`has_more` is true. Invalid ordering/nonprogress fails instead of manufacturing a
cursor. Pages, hits and original navigation targets retain account/generation
identity. Clients retain their own query/results for Back restoration.

`hydrate_search_hit` retrieves full messages, including embeds/attachments,
without opening a timeline or changing canonical history, coverage, reads or the
viewport. The pinned backend has no message-ID/context GET. Hydration therefore
walks actual history pages, at most four/turn, returning Pending for explicit
resumption. It preserves exact real cursors, rejects nonmonotonic pages, uses two
concurrent slots/account and retains at most 32 detached messages/account.
Original result navigation is revalidated before use. The backend still authorizes
actual history requests, and detached data never authorizes read reporting.

## Validation

`bash scripts/papo-sdk-check.sh` runs the complete SDK gate. Notification unit
and loopback tests cover canonical claim races, quiet/paginated catch-up, mute and
trigger distinctions, partial outages, preemption, cancellation, account removal,
replacement and push ownership. Linux tests perform actual SIGKILLs after FULL
claim both before and after handoff and verify no duplicate after restart.
Existing outgoing process-kill tests retain their ambiguous-send acceptance.
Search tests cover exact tuple pages, cancellation/replacement, navigation source,
query lifecycle and multi-turn detached hydration with unchanged canonical
snapshots. `scripts/papo-backend-proof.py` additionally exercises actual pinned
Go/PostgreSQL notification, search and push handlers, alongside the existing
permission/cursor/upload proofs. Android cross-compilation is independent of
native Firebase, notification presentation and device testing in Sessions 15–16.
