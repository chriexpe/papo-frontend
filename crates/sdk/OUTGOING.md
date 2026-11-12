# Outgoing operations

`Session::outgoing()` is a renderer-independent account capability. It owns a
single dispatcher on the client's runtime; drafts, dependencies, retries and
local echoes do not require a desktop composer or a particular UI framework.

Enqueue takes a caller-generated unique local UUID. Success means the intent
completed a synchronous FULL commit. Reusing a retained UUID returns
`AlreadyExists`; local UUIDs are not backend nonces. After cancellation, look up
that same local identity before deciding to create another intent.

| State | Meaning | Automatic dispatch |
| --- | --- | --- |
| Queued | Durable intent eligible for a new attempt | Eligible in insertion order |
| Sending | FULL-committed claim before network execution | One attempt only |
| Sent | Valid authoritative result and FULL-committed receipt | Never |
| Failed | Definite 4xx rejection (except 408) or discarded dependency | Never |
| Unknown | May have reached the server | Never |

Startup changes every remaining Sending to Unknown, including a crash between
claim commit and the first socket byte. Transport failures after connection,
lost/malformed success bodies, timeouts, canceled generations, HTTP 408 and
5xx remain ambiguous. Only request-construction or connection-establishment
errors classified `NotDispatched` can return to Queued. The transport disables
redirects and policy retries. Preparation failures leave Queued work intact
with a bounded backoff and observable typed queue fault.

Outgoing subscription is nonblocking: queue initialization and attachment-read
failures appear as an observable `OutgoingSnapshot.fault`, without preventing
canonical live or focused history from opening. Mutations still require a
successfully initialized durable queue.

`retry(id, AcceptPossibleDuplicate)` is the explicit user decision needed for
Unknown. `DefiniteFailure` cannot retry Unknown. Neither gateway messages nor
reconciliation resolve operations by matching text, filenames, author or time.

Unresolved work is an ordering barrier for its channel. Other channels can
progress; the account dispatcher performs one request at a time. A local
message target must be an earlier create in the same account/channel. Its
persisted dependency resolves only from a confirmed backend message ID.
Discarding an unresolved parent fails its dependents; it never sends them
without their target. A confirmed receipt required by pending dependents
cannot be dismissed.

Drafts preserve canonical text, bound mention IDs, reply, embeds and immutable
file bytes. `save_draft` returns an optimistic revision; `enqueue_draft` captures
that exact revision atomically. Failed or Unknown leaves the draft untouched.
Sent clears only its captured revision in the receipt transaction; a newer
edited draft survives. Native text controls and editing offsets belong to
clients; the SDK never substitutes an identity by display name.

Attachments are copied into fsynced, digest-verified account-owned staging files,
with only metadata and references in SQLite. Caller paths and platform grants
are never needed after acknowledgement. Their names and bytes remain immutable
until Sent or explicit discard. Committed reference changes precede orphan
collection; account removal deletes its staging directory. Copy/hash/fsync and
collection run outside the SQLite writer on bounded blocking tasks (two staging
slots and a separate collection slot per profile), so bulk copies cannot occupy
the durable writer or prevent metadata-only work from completing. Transactional
reference snapshots serialize collection with accepted commits; canceled or
failed admission leaves collectible orphans. Sent releases
attachment references and retains backend metadata. Admission is capped at 256
unresolved operations (including Sent receipts awaiting projection) and 256 MiB
of input metadata plus attachment bytes per account,
with at most ten files / 100 MiB of input bytes per message. Confirmed receipts compact to the most
recent 256 projected confirmations plus mappings still needed by dependencies.
Confirmed receipts awaiting projection are retained and count toward admission;
they are not compacted away while their visible replacement is missing. Discarded
payloads are cleared immediately and old unreferenced identity tombstones are
bounded. Draft data and consumer-held old snapshots are separate from the
queue admission budget; this is not a process-memory ceiling.

Outgoing state belongs to a verified account and dispatch attempt, independently
of socket generations. In-flight generation cancellation yields Unknown. A
response already received for the same claimed attempt may be durably recorded
after a reconnect, while revoked/replaced account handles cannot commit it.
Canonical history still rejects old-generation data. Current-generation create
acknowledgements enter history through normal live merge rules, so deletes and
newer edits win. Successful receipt persistence never depends on cache success.

`TimelineItem::LocalEcho` exposes unresolved create intents in live windows.
Focused windows exclude them, and read reports cannot acknowledge their local
UUIDs. Sent and presentation are independent: `history_projected` stays false
if cache insertion fails or its epoch is superseded. A Sent local item displays
the authoritative `Operation.message` until canonical history accounts for its
ID, then retires without another mutation request. Canonical tombstones also
retire it, so a late HTTP response cannot resurrect a deleted message. Complete
REST embed payloads supplement sparse gateway messages while newer gateway
edits, embed replacements, reactions and deletes retain precedence.
Subscriptions remain bounded and snapshot/diff replay stays framework-neutral.

Validation is part of the SDK, not deferred native UI work:

- Unit tests cover payload bounds, receipt identity/timestamps and v1 migration.
- Loopback tests cover ordering, local targets, immutable files, optimistic drafts,
  rejection/explicit retry/discard, account removal and bounded receipt retention.
- Linux tests SIGKILL the actual SDK process three times at six boundaries:
  Queued, committed Sending before dispatch, in-flight request, accepted server
  mutation with held response, received response before receipt commit, and
  committed Sent. Restart checks acknowledgements, SQLite integrity/foreign keys,
  ordered delivery and the absence of automatic ambiguous replay. An in-flight
  request's buffered bytes may still be accepted after the process dies.
- Public timeline tests cover independent windows, stable echo identity, read
  exclusion, Unknown preservation and delete-before-HTTP-receipt ordering.
- `cargo run --locked -p papo-sdk --features test-support --example outgoing`
  runs a headless fixture proof. The isolated PostgreSQL workflow also runs it
  against the pinned production handlers for multipart create, reply, edit and
  delete. The test-support credential host is in memory; no real secrets or
  message content are logged. Tests establish process-crash behavior, not power loss.

Desktop composer widgets remain separate until the framework choice; Android
bindings/client wiring follow their planned milestones. Those deferrals do not
relax any outgoing SDK gate above.

Profile privacy is enforced for caller-selected roots. Unix roots and staging
directories are 0700 and staged files/lock/state database are 0600; existing staged
files are hardened on access. Windows uses built-in Windows PowerShell/.NET ACL
creation with an owner-only protected DACL, not inherited directory assumptions.
Existing SDK-owned cache/media/outgoing files and database sidecars are hardened
under the profile lease; ACL failures fail closed. This adds no Rust dependency or
unsafe code. Windows hosts must provide Windows PowerShell 5.1 and filesystem ACL
support. Native Windows tests deliberately grant Everyone access on a parent and
verify protected owner-only rules on the profile and staged file; these require
hosted Windows execution. See Microsoft's [directory ACL creation](https://learn.microsoft.com/en-us/dotnet/api/system.io.directory.createdirectory?view=netframework-4.8)
and [file ACL creation](https://learn.microsoft.com/en-us/dotnet/api/system.io.filestream.-ctor).
