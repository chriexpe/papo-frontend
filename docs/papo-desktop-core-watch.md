# Core watch list from the desktop session slice

Recorded 2026-11-06 while implementing phase 1, slice 1. These are follow-up
investigations, not permission to expand that PR or change shared policy.
No egui or Android implementation was copied. Fixtures use loopback-only,
synthetic accounts and messages. Keep this list updated when a later PR
confirms, fixes, or rejects a finding.

## Findings

| Finding | Evidence | Classification and follow-up |
| --- | --- | --- |
| There is no general timeline content revision or incremental projection API. | `crates/papo-core/src/state/mod.rs`: `messages_in`, `message`, `record_timeline_mutation`. The mutation journal advances only while a refresh/history request is active. `sync_generation` and diagnostic `journal_revision` are **not** content revisions. | API/performance hazard, not a confirmed reducer bug. Desktop currently compares borrowed presentation fields after bounded drains. Measure a shared change-set/revision API before adding one; Android must benefit too. Never use journal revision to skip normal live changes. |
| Live upserts do not populate an unverified timeline. | `Store::apply_mutation` excludes live message upserts unless `timeline_status(channel) == Fresh`; `history_server_cursor` also excludes uncovered older history. Core's refresh-race journal preserves arrivals during an active request. | Intentional continuity guard that easily invalidates a benchmark or frontend. Seed through a valid `RefreshTicket` or load history first. Count actual applied rows/events, not socket writes. Investigate only if production loses an event after valid reconciliation. |
| A 10k-message server channel does not imply a 10k-message disk cache. | `crates/papo-core/src/cache/types.rs`: `MESSAGE_RETENTION = 500`, `CACHE_PAGE_SIZE = 100`, separate pin retention; `cache/store.rs` enforces pruning. | Product/performance policy to evaluate separately. Do not disable retention in a benchmark or assume it is a bug. Measure offline history usefulness, disk/write cost and paging behavior before deciding whether 500 is appropriate. |
| The receiver-based cache API does not wake consumers. | `ClientDb::load_channel_page_async` returns a `std::sync::mpsc::Receiver`; `ServerRuntime::open_with_store` restores metadata/outbox, not timeline pages. | Addressed for desktop during #143 review: additive `load_channel_page_with_callback` completes on the existing cache worker; the actor uses a one-read/one-completion mailbox with identity/epoch checks. Existing receiver APIs are unchanged. No waiter thread or actor receipt timeout remains. Other clients may adopt the callback when useful. |
| Disabling the cache also disables durable outbox restoration. | `ClientDb::open(None)` is disabled storage, not an in-memory database; authenticated bootstrap reports unavailable outbox storage. Observed against the synthetic loopback backend. | Contract trap, not necessarily a bug. Use an isolated temporary production cache for authenticated lifecycle tests. Do not mask unrelated runtime errors or advertise disabled storage as a durable session. |
| Dropping `Net` does not prove its thread was joined. | `crates/papo-core/src/api/net.rs`: explicit `Net::shutdown` signals shutdown and joins its interactive thread; ordinary drop is not that lifecycle barrier. | Lifecycle API hazard. Desktop wraps runtime ownership with explicit shutdown and join, including unwinding. Evaluate whether a shell-neutral RAII lifecycle contract would improve all clients in a separate PR. |
| Thread resource exhaustion is a panic in `Net::spawn`. | `api/net.rs`: the OS thread builder ends in `expect("thread de rede")`, unlike the fallible desktop session-thread boundary. | Confirmed error-path hazard, not server-input-triggered. A separate shared API change should return a typed error and update every caller. The desktop owner reports its own unwind rather than panicking GPUI. |
| `ClientDb::flush()` is not an acknowledged durability result. | `cache/mod.rs`: it returns `()`, ignores a ten-second receipt timeout and cannot report success/failure to callers. | Confirmed observability/lifecycle hazard. Desktop calls the existing barrier before join completion, but must not describe that return as proof of disk durability. Investigate a fallible barrier in a shared-crate PR; durable outbox writes already have their separate acknowledged path. |
| Real REST admission pacing can dominate fixture timings. | `crates/papo-core/src/api/client.rs`: shared per-server admission budget (250 ms). Twenty 100-row pages take seconds; 10k paging is not an isolated reducer benchmark. | Measurement hazard, not a request to remove rate limiting. Separate setup, transport/reconnect elapsed time, cache I/O, actor work and foreground application costs. |

## Desktop prerequisites from #143 review

- Runtime effects are currently discarded by the screen-free actor. Before
  enabling authenticated outgoing/account/call commands, deliver effects
  reliably and in order, with session identity checks. Immutable projection
  deltas are not a substitute for acknowledgement/effect delivery.
- Arc retention is incremental allocation, not incremental change detection:
  the diagnostic builder still scans the selected timeline and rebuilds its ID
  set. Revisit the core change-set/revision API before the full timeline.
- Rapid replacements now coalesce on one persistent server thread with one
  pending factory; they no longer create a chain of waiting OS threads.

## Desktop framework watch

GPUI Fast 0.1.13 defaults to automatic last-window quit on non-macOS and
allows only 200 ms for returned native quit futures. Its synchronous quit
callback runs before that grace period; AppKit termination need not return
through Rust's application runner. The session slice must use explicit quit
mode, retain shutdown completion across repeated requests and fence native
termination before returning. This is a desktop lifecycle concern, not a
reason to add GPUI concepts to shared crates.

## Not established

No data-loss bug, unsafe credential fallback, or policy regression in
`papo-core` is claimed by this list. The cache limit and admission budget are
existing decisions whose suitability needs its own evidence and PR.
