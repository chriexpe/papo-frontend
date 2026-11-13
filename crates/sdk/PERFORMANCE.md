# Headless history performance fixtures

Run from the repository root, using the existing target directory:

```sh
mkdir -p target/papo-sdk-7c
cargo run --locked -p papo-sdk-ui --example history_cost -- history 10000 > target/papo-sdk-7c/history-10000.jsonl
cargo run --locked -p papo-sdk-ui --example history_cost -- history 100000 > target/papo-sdk-7c/history-100000.jsonl
cargo run --locked -p papo-sdk-ui --example history_cost -- residency > target/papo-sdk-7c/residency-500.jsonl
cargo run --locked -p papo-sdk-ui --example history_cost -- reconnect > target/papo-sdk-7c/reconnect-100.jsonl
```

These fixtures use a synthetic loopback backend and temporary profiles. They never
use a configured user server. Output contains counts and measurements only.
Linux SDK CI runs all scenarios and uploads their JSON Lines artifacts. Structural
assertions fail the run; durations are advisory and are not CI thresholds.

## Scenarios and hard assertions

- **History:** hydrate 10k/100k messages, then observe independent live/focused
  windows with a page size of 30. Reaction, append and delete examine exactly one
  publication entry; an edit plus its embed event examines at most two. A reaction
  encodes one payload and writes at most three cache rows, replaces one visible
  item and leaves the disjoint focused window unchanged. Unchanged payload Arcs
  remain shared. Visible reports and paging use the public services.
- **Residency:** three cycles through 500 channels with one active window. Each
  explicit trim satisfies the configured idle count/byte budgets. Raw subscriptions
  and active windows remain usable; reopening restores cache data. This proves an
  idle-model bound, not a bound on every externally retained snapshot.
- **Reconnect:** 100 unread channels each receive 401 disjoint offline arrivals.
  A held REST response permits a 900-event gateway burst to accumulate. Delivery
  handshakes avoid overflowing the fixture's own broadcast buffer. The SDK must
  reach Online without journal overflow, perform at most four history pages per
  channel, upload no read marks, and keep incomplete unread counts as AtLeast.

Small deterministic unit/lifecycle tests additionally cover byte/count eviction,
LRU selection, raw receiver pinning, in-flight ownership, stale generations,
channel removal while evicted and clean regrant. Existing snapshot/diff property
and durable-read/crash tests remain required.

## Report schema, version 1

Each line is one JSON object with `schema_version`, `scenario`, `rows`,
`elapsed_us_advisory`, `diagnostics`, `process_peak_rss_bytes`,
`process_rss_scope`, and `not_measured`.

`diagnostics` is an account-local `HistoryDiagnostics` snapshot:

| Field | Meaning |
| --- | --- |
| pinned_channels / pinned_bytes | Models protected by handles, raw receivers or in-flight references, with estimated retained bytes |
| idle_channels / idle_bytes | Warm models eligible for eviction, with estimated retained bytes |
| work.publication_entries | Cumulative entries inspected for raw publication, including models subsequently evicted |
| work.publication_encodes | Cumulative payload encodings for raw equality checks; a metadata-only update can inspect an entry without encoding it |
| work.publications | Complete accepted raw publications |
| work.persisted_rows / persisted_bytes | Rows and encoded metadata/row bytes submitted to successful cache transactions; not disk sectors, fsync count, SQLite file size or WAL growth |
| work.cache_loads | Cache model hydrations, including reopen after eviction |
| work.evictions | Idle models removed from RAM; disk history remains |
| work.reconnect_journal_peak / reconnect_overflows | Largest queued gateway journal and count of rejected overflowing generations |

Counters are cumulative within one account session, so subtract successive
snapshots to isolate an operation. They contain no resource IDs or payloads.

Idle byte accounting combines encoded entry lengths, publication payload bytes,
and fixed estimates for entry/order indexes, chunks and tombstones. It excludes
allocator overhead, precise persistent-vector node accounting, SDK resource/read
images, old snapshots retained by consumers, media and native rendering. Tombstone
rows restored from disk may conservatively overestimate the retained payload.

`process_peak_rss_bytes` reads Linux VmHWM when available; it is null elsewhere.
It measures the entire fixture process, including the synthetic backend and its
server-side messages, SDK runtime and fixture data. It is not SDK-only memory or
an allocation counter. GPU resources and native rendering are never measured.

## Idle policy and compatibility

`ClientConfig.history` defaults to four idle channels and 32 MiB of accounted
idle history per account. The byte default accommodates roughly two 10k-message
synthetic channels at the observed ~15 MiB estimate each; it is configurable and
is not a universal device recommendation. Zero disables idle retention. Active
capabilities are protected even when their working set exceeds this idle budget.

Maintenance runs after reading consumes accepted snapshots, on history changes,
and once per second. `Session::trim_history()` provides an explicit deterministic
maintenance point, including for memory-pressure handling. Inspection through
`history_diagnostics()` does not hydrate cold channels or trigger eviction.
Closing the last handle can therefore retain an idle model until maintenance.

Raw `HistorySnapshot.messages` now uses the already-locked imbl persistent Vector.
Consumers retain iteration/indexing but should use `front()` / `push_back()` and
persistent slicing rather than Vec-specific APIs. All raw deliveries remain full
coherent Reset snapshots; version/generation and sticky closure rules are unchanged.
Keeping old snapshots is a consumer-owned cost, with unchanged nodes shared.
Rehydrated publications reserve versions above the account's prior high-water mark,
so old same-generation snapshot stamps cannot alias a recreated model. Normal
updates within a resident channel remain consecutive.

A versioned local observation is checked in as `history-performance-baseline.json`.
It records source/provenance and structural work before/after, not timing thresholds.
The baseline counter adapter omits publication bytes from its memory estimate, so
those estimates are deliberately not compared. Whole-process RSS has its own scope.

The retention tradeoff is explicit: the 500-channel baseline keeps 499 idle models
warm; the candidate keeps four. Repeated cycles consequently perform more SQLite
hydrations. This bounds idle memory rather than promising free warm revisits.
Reopening a Timeline can additionally revalidate the restored stale coverage.

## Comparing results

Record source revision, Rust version, Cargo profile overrides, platform/hardware,
scenario size, whether other builds were running, and fixture/report version.
Compare the same workload and instrumentation. Separate hydration from steady
mutation costs, encoded estimates from process RSS, and SDK work from rendering.
A single wall-clock sample does not establish p50/p95 or justify a timing budget.
The checked structural budgets are the stable regression gates.

The 900-event reconnect case leaves headroom under the 1024-entry journal. Bursts
exceeding capacity still deliberately reject their generation and reconcile again;
this fixture does not claim infinite traffic tolerance or authorize dropping events.

## Physical disk budget (Session 11)

The same harness also accepts `disk 10000` and `disk 100000`. Both run in Linux
CI and emit a physical report followed by the existing history diagnostics.
They hydrate one channel, explicitly trim to 2 MiB, and require actual allocated
files to fit that ceiling, bounded history transactions (at most 80 changed
rows), retained messages, and an explicit older-history gap. The small ceiling
is a fixture setting, not a production default. Microsecond cursors and equal
creation timestamps use the existing 7c seed convention.

`physical` includes `budget_bytes`, `allocated_bytes`, `database_bytes`
(main SQLite plus WAL/SHM/journal), `media_bytes`, protected `outgoing_bytes`, profile lock/security-marker `profile_bytes`,
`reclaimed_bytes`, `media_evictions`, `history_turns`,
`max_history_rows_per_turn`, `reclaim_blocked` and `pressure`. Allocation uses
filesystem allocated-size APIs, not SQLite row sizes or logical file lengths.
Content-addressed files count once. Directory entries, exported client files,
OS credentials and process memory are outside this file-allocation report.
Concurrent writers can change allocation during maintenance; final pressure is
always measured again, never inferred from deleted row counts.

Before eviction, maintenance checkpoints WALs and incrementally vacuums already
free cache pages in 64-page turns. It then retires global LRU media refs/files,
followed by 20-message oldest prefixes in least recently opened channels across
accounts. It measures affected database allocation between bounded turns and
rescans the profile every 64 eviction turns and at the end. A held reader stops
further deletion in that cache and reports blocked reclamation; a later trim
reclaims already free pages even when there are no remaining message victims.
The durable state/outgoing floor and 24-hour orphan recovery grace can prevent
meeting a tiny ceiling; `pressure` remains true in that case.

`media-performance-baseline.json` records local observations, source context and
advisory durations. These establish deterministic physical/row gates, not p95
latency, SDK-only RSS, real-device performance or a complete process-memory cap.
Unit/integration regressions additionally cover allocated WAL/SHM accounting,
reader-blocked recovery, account-removal fences, cold retention, pinned entries,
tombstones and exact pagination after eviction.


## Concurrent encoded cache hits

`cargo run --locked -p papo-sdk --features test-support --example media_cost`
measures eight rounds of four warm 16 MiB objects across two accounts, sequentially
and concurrently. Both modes include reference validation, file reading, SHA-256
and LRU work; network population precedes measurement. JSON contains counts and
advisory durations only. This fixture does not measure native decoding or assert
a latency threshold.

The recorded local comparison in `media-performance-baseline.json` observed
sequential/concurrent totals of 308/308 ms before and 302/160 ms after narrowing
read exclusion. These are individual warm-cache observations, not p95 evidence.
Two read slots bound admitted read allocations; digest pins protect files while
reading outside global exclusion. Deterministic tests prove that a held read
allows another account's hit, survives eviction/collection, and cannot deliver
after its reference is retired. Queued stale denial callbacks cannot revoke a
new-generation grant. Under-budget orphan cleanup and resumable 100-entry turns
are separately checked, including explicit collection starting a full pass.
