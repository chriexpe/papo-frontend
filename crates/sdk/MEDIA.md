# Media and expressions

`Session::media()` returns an account capability for encoded bytes. Rendering,
decoding, animation, player/viewer lifetimes and picker widgets belong to clients.
No SDK method treats a local SHA-256 digest or filesystem path as authorization.

## References and recovery

`MediaSource` describes a backend object, revisioned avatar, emoji or anonymous
HTTPS URL. Each successful fetch commits an `(account, source)` reference to a
shared content file. Authenticated backend fetches verify the current account's
identity first. Public avatar and external/provider requests use a separate
cookie-free transport with redirects/retries disabled. Requests and publications
are fenced by connection generation and account revocation.

`cached` only resolves that account's existing grant. `fetch` reuses such a grant
unless refreshed; it cannot discover another account's grant by hashing content.
Identical authorized content shares a single file. Refresh denial retires the
underlying source family, including bounded ranges and attachment thumbnails.
Previously returned byte buffers remain consumer-owned; remote permission changes
cannot revoke copies already downloaded. The backend authorizes every new request.

User/webhook avatar keys contain an explicit revision. Refresh sends available
ETag/Last-Modified validators; a changed revision fetches unconditionally. User
AvatarUpdate invalidates that account's old user-avatar references. Missing,
hash-corrupt or oversized files discard validators and permit an authoritative
replacement rather than repeatedly accepting an unusable 304 response.
Ranges validate Content-Range and response length; an ignored Range response is
sliced to the requested bounded interval. A partial grant never grants the full
object. Responses are capped at 100 MiB with two concurrent media fetches.
Emoji pages retain metadata only and store bounded decoded image bytes in this
same cache. Server icons are fetched lazily into it.

The NORMAL media index is rebuildable. Unknown schemas/integrity failures rebuild
it; existing content files grant no access after rebuilding. Refs return only
following new authoritative retrieval. Deterministic orphan collection preserves
unindexed files for 24 hours, including staged files interrupted before commit.
Account cleanup immediately deletes only formerly owned files with no other owner;
it never sweeps fresh unindexed content belonging to an index-recovery window.
`clear_cache` retires this account's refs without touching credentials, drafts,
operations, expression metadata or another account's shared files.

Content hashing, private file creation and fsync run outside SQLite writers.
Unix directories/files use 0700/0600; Windows uses the existing private filesystem
ACL boundary. Admitted blocking work retains the profile lease, and blocking
unlink retains media exclusion through cancellation. No encoded bytes enter SQLite.

## Physical budget

`ClientConfig::disk` configures one profile-wide allocated-file ceiling: 1 GiB by
default on desktop and 256 MiB on Android. `disk_usage`, `trim_disk` and
`subscribe_disk` expose measurements, remaining pressure and independent faults.
The automatic check interval defaults to five seconds. Measurement uses physical
file allocation including SQLite/WAL/SHM/journal and protected outgoing blobs.
Shared content counts once. This is not a process-memory limit.

Existing free pages/WAL are reclaimed first, then media files by global LRU,
then oldest chronological prefixes in least recently opened channels. Prefix
transactions are bounded and preserve explicit recoverable gaps, detached pins,
real tombstones and exact cursors. Maintenance-only cold reads never authorize
history presentation. Removal/login and retention are fenced so stale maintenance
cannot recreate a removed account's cache or overwrite a newer live model.
Protected durable state/outgoing data, held readers or orphan grace may exceed a
small ceiling; the report retains `pressure=true` rather than deleting intent.
See [PERFORMANCE.md](PERFORMANCE.md) for executable gates and report scope.

## Expression library

`Session::expressions()` owns account-scoped GIF/sticker favorites, confirmed GIF
recents and reaction usage. State schema v3 migrates these metadata tables forward;
the data survives restart and is removed with its account. Limits are 128 favorites
per kind, 64 recent GIFs and 256 reaction-usage entries. Indexed sequence allocation
avoids scanning other accounts inside the shared FULL writer.

`Gif::message()` constructs an ordinary Session 9 outgoing message. Selection only
stages metadata; it does not send or record a recent. The existing operation queue
records recents in the same FULL transaction as confirmed Sent, before operation
compaction. Failed/Unknown/interrupted sends do not create recents. No second send
path or media cache exists, and ambiguous sends remain never automatically retried.
`ExpressionLibraryService` exposes coalesced metadata snapshots and typed faults;
clients own all native presentation and attribution.

Optional `GiphyConfig` accepts a caller-supplied key. Missing configuration returns
`ExpressionError::Unavailable` for live catalog requests while saved metadata
remains accessible offline. Trending/search preserve provider order and exact
queries with bounded 25-item pages. The provider receives no Papo cookies or
credentials; its key never enters Papo requests, SQLite or diagnostic output.
The production endpoint is fixed; the test-only endpoint override accepts literal
loopback HTTP addresses solely for deterministic credential-separation proofs.
Live provider requests cancel/discard results after generation changes.
