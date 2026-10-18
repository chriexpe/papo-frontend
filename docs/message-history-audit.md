# Message history, cache and navigation audit

This audit covers the frontend and `papo-core`. No changes to papo-backend are required. The existing `GET /channels/:channel_id/messages` endpoint supports directional pagination with the exact `(since, last_id)` cursor and `order=asc|desc`.

## Data flow

```mermaid
flowchart TD
    Open[Select a channel] --> Local[Restore cached metadata and recent page on DB worker]
    Open --> Head[Fetch current server head on network scheduler]
    Open --> Pins[Fetch full pins independently]
    Local --> Display[Speculative display]
    Head --> Reconcile[Validate request generation and reconcile covered range]
    Live[WebSocket and local mutations] --> Journal[Record mutations while requests are in flight]
    Journal --> Replay[Replay after snapshot]
    Reconcile --> Replay
    Replay --> Display
    Replay --> Write[Queue transactional cache effects]
    Scroll[Scroll toward older messages] --> Online{Online?}
    Online -->|yes| Verified[Fetch from last server-verified cursor]
    Online -->|no| Disk[Page through proven local history]
    Verified --> Reconcile
    Disk --> Display
    Jump[Click a pin, search hit or reply] --> Context[Fetch bounded surrounding window]
    Context --> Separate[Display separate context; retain live head]
```

Metadata restores at runtime startup. Message pages restore asynchronously for the selected channel. Neither a cache hit nor selecting a channel makes it fresh or marks its messages as read. The server snapshot establishes freshness only for its connection generation. Runtime/session ownership protects server and account boundaries; request IDs additionally protect individual page, context and pin loads.

## Findings and corrections

| Finding | Consequence | Correction |
| --- | --- | --- |
| Jump only scrolled to rows already in memory; an unresolved jump also disabled pagination and disappeared after ten seconds | Clicking an ancient pin appeared to do nothing | Explicit, cancellable context fetch; visible loading and errors; loaded replies use the same navigation path |
| Cached full pins shared the chronological message list | An isolated ancient pin could become the pagination cursor and skip the intervening history | Separate pin snapshots and timeline membership, both in memory and in persistence |
| Pagination used the oldest displayed/cached message as the server boundary | Cache hits skipped revalidation of old server edits/deletions | Separate exact server cursor; online pages revalidate cached rows in their covered range; offline pagination uses the local cursor |
| Cache restoration could overwrite already projected messages | A late DB read could undo live edits or fresh context data | Cache hydration fills missing chronological rows only; it cannot update the separate fresh context |
| History requests ran inside the network command loop | A slow GET delayed processing WebSocket events and other commands | History, context and pin reads use the bounded scheduler |
| Head loading awaited the pin query | Slow pins delayed displaying an otherwise available message head | Fetch pins independently; keep cached pins visible during refresh |
| Older pages and pin snapshots lacked request ownership and mutation replay | A late response could resurrect a deletion, reverse a pin, or merge into a different connection/window | Generation/request tickets and mutation journals; cancellation makes obsolete completions inert |
| Head reconciliation deleted stale rows only if loaded in memory | Deleted messages still on disk could reappear later | Delete absent rows across the entire authoritative head range, including disk-only rows |
| Accepted pages were stored as unrelated individual upserts | A dropped earlier write or retention could leave a disconnected segment looking continuous | Atomic checked append: a page extends cached coverage only when its boundary remains a proven timeline row in the database |
| Generic message/pin/send upserts implied continuity | One standalone message could falsely bridge a history gap | Standalone upserts preserve metadata without creating timeline membership; only snapshots and checked appends establish coverage |
| Cached message times rounded to milliseconds | Equal-millisecond page boundaries could omit or reorder messages | Persist the full fractional second and use timestamp precision plus ID in local and server cursors |
| Pin retention could delete rows inside chronological history | Retention could punch holes in a supposedly continuous cache | Bound the chronological suffix independently; auxiliary metadata/pin eviction only removes rows outside that suffix |
| Older history loaded only within 360 points of the top and required overflow | Slow requests arrived too late; a short first page could strand large windows | Fetch within the shared upper runway, including when the first page fits the viewport |

## Existing API used for context navigation

For a known target timestamp `T` and ID `M`:

1. `GET /channels/C/messages?since=T&last_id=M&order=desc&limit=30` returns preceding messages.
2. Find the immediate predecessor `P`. Request `GET /channels/C/messages?since=P.created_at&last_id=P.id&order=asc&limit=31`. This returns the target itself, freshly hydrated, followed by newer messages. If there is no predecessor, request the first ascending page without a cursor.

The combined window contains up to 30 preceding messages, the target, and 30 following messages. `has_more` from each direction controls further pagination. Timestamp precision and ID ordering are preserved even when many messages share exactly the same timestamp. Missing/deleted targets produce an error instead of displaying stale pin/search content.

Pins and search hits already supply timestamps. In the local backend implementation, `last_id` without `since` does not locate a message: the query ignores that ID as a cursor. An unloaded reply or notification with only an opaque ID therefore needs a descending seek to locate its exact cursor first. This seek is cancellable, uses bounded pages, and does not inject intermediate pages into the chat or its cache. An ID-only lookup on the existing backend endpoint would make that particular case faster; it is not required for pin/search jumps.

A context window is a separate view, not a disconnected insertion into the live timeline. New messages continue into the live head. Edits, deletions, pins, reactions and moderation updates affect loaded copies. Scrolling can extend the context in either direction. The down-arrow/Escape returns to the live head; sending also returns there so the optimistic echo remains visible. Merely viewing a distant context does not mark the unseen interval as read.

## Persistence contract

Schema 19 adds explicit `timeline` membership and `created_at_nanos` to cached messages. Initial/older pages contain chronological rows only; initial hydration also returns a separate bounded pin snapshot. A pin that is genuinely part of a fetched chronological page remains a normal chronological row as well.

A successful server head replaces only the range it covers. Older proven history can survive when there is a real overlapping timeline message, including an own message. A standalone send or pin is not overlap evidence. Without overlap, the old chronological region is discarded and must be refetched. Checked historical appends additionally reconcile edits/deletions within their covered range. A failed cache write never turns a later disconnected page into usable chronological coverage. If an append loses its boundary, those rows remain snapshots until a later head reconciliation re-establishes continuous coverage.

Chronological rows and detached metadata have independent bounded retention, so pressure from standalone acknowledgements cannot remove interior rows of a proven timeline.

The DB worker batches accepted writes transactionally and runs retention at the end of the batch. Chronological cache pages and older pages in the live-history path sort once per batch rather than repeatedly sorting all loaded messages after every row. Message rows, attachment metadata, reactions, embeds and external-author metadata travel together. Decoded media/player resources retain their separate lifetimes.

The migration rebuilds previously unproven chronological cache once. It preserves retained pins, read state, drafts, credentials and queued sends. Ordinary history becomes available again from the server; until refetched it is unavailable offline. The cache cannot infer continuity retroactively from arbitrary legacy rows.

Context windows remain session-local and are not persisted as extra timeline segments. Restart restores the recent proven history and retained pins. This intentionally avoids treating the unrequested interval between a distant context and the live head as cached history. Historical pages in the continuous head path do persist normally.

## Scrolling and media

The media runway is shared by message previews and attachment loading: 1,800 points above and 480 below the readable area, increased from 1,200/320. History pagination begins in the upper runway too. Existing visible-message anchoring remains responsible for compensating row-height changes; earlier fetching reduces the amount of layout work arriving at the visible edge. Context jumps use an immediate scroll, followed by the existing highlight.

Frame-by-frame inspection of `Replay_2026-10-18_00-24-18.mp4` from 57 seconds through the end (294 consecutive frames) found a one-frame history flash at 58.483 seconds. The channel opens around 58.250 seconds; recent messages are visible before and after the flash, but the flash shows substantially older messages. A headless regression reproduced this when history is prepended while the viewport is stuck to the bottom: egui updates the final scroll offset after laying out the rows, leaving that frame's geometry at the old offset. The frontend now compares the layout viewport with the returned offset when content resizes and repeats layout before presenting the frame. Ordinary wheel movement alone does not trigger this extra pass. Tests cover prepending at the bottom and preview expansion/contraction there, alongside the existing scrolled-away anchoring tests.

Follow-up recordings (`00-53-30` around 45–53 seconds and `00-56-37` around 49–55 seconds) showed repeated movement and a social image alternately appearing/disappearing. Runtime logs confirmed continuous texture eviction/reload and repeated failures for the same remote image. Resource-class priority kept old stickers/avatars ahead of recently used post images, while budget trimming also forgot zero-byte failed results and retried them on the next paint. Persistent chat textures now share an LRU, transient picker media still goes first, and failed results remain cached. Known embed-image dimensions survive texture reloads so a GPU miss does not collapse the row. Twitter profile/default-avatar URLs and images matching the post author avatar are excluded from main covers, including for previously cached metadata; direct image links remain supported. The newest-message arrow always uses return-to-latest rather than the separate message-highlight path. Regressions cover both budget faults, cover filtering, stable image geometry and a real down-arrow click followed by repeated stable frames at the newest message.

A further regression reproduced the arrow stopping before the newest message when the tail expands between its hidden measurement and its first visible layout. The navigation intent previously ended as soon as a pixel offset was calculated; that offset could already be stale. Return-to-latest now remains active until the rendered viewport reaches the current bottom. Anchor correction and pagination cannot override that intent while it is resolving. Tests cover a growing tail and a real arrow click with new unread messages, rather than only static rows.

## Embed authority

The local Go backend's embed service was audited read-only. Its Twitter path uses post photos, video thumbnails or external-card media; a text-only post can correctly have no image. The frontend nevertheless independently re-scraped backend links and preferred the scraped image. It could fill intentionally absent media, infer a player from the URL, and repeat the same lookup in search/pin panels. A cached texture could also outlive the backend metadata declaring that image absent.

“Prefer backend embeds” is enabled by default under Appearance → Chat and persists with the other settings. Backend cards are authoritative, including absent image/video fields. Their URLs are not submitted to the frontend unfurler, even when previously scraped results exist. Links without corresponding backend cards still use frontend discovery. Disabling the preference restores the existing enrichment behavior. Pins and loaded search hits pass their available backend metadata into panel cards instead of unconditionally re-scraping their links. Search hits without locally available embed metadata continue to use frontend discovery.

Current backend metadata is checked before returning cached inline image textures, so removed media cannot reappear from that cache. Changing the preference invalidates row measurements and retained embed geometry. The frontend fallback's broader discovery heuristics remain in place for links without backend embeds and when the preference is disabled; this change establishes authority rather than claiming that every fallback parser can determine post-media ownership. No backend files were changed.

The screenshot comparison also exposed two presentation faults. Multiline reply content was centered as a multiline galley in an 18-point single-line quote; flattening line breaks before mention binding and layout keeps the snippet inside that slot. GIF embeds were treated as ordinary titled link cards and limited to their thumbnail's natural pixel size. Declared GIF fetch methods and image/gif MIME types now select standalone animation presentation for any provider, with older GIPHY, Tenor and KLIPY page URLs recognized for compatibility. The same dimensions are reserved before decoding and used afterward, scaled proportionally to chat media bounds. Backend bytes remain authoritative: a legacy static thumbnail cannot regain animation without an updated backend asset.

## Verification

Regression tests exercise directional HTTP requests against a local mock of the existing endpoint, including the first/last message, exact timestamp ties, fresh target hydration and deleted targets. Store tests cover separate windows, both pagination directions, cancellation/supersession, reconnects, concurrent edits/deletes/pins, stale cache reads, all-own overlap, online historical revalidation, read-frontier isolation and malformed pages. Real DB tests cover detached pins, missing append anchors, range deletion, retention and fractional-timestamp round trips. Existing headless scrolling tests cover prepending history, simultaneous row growth above/below the viewport and wheel input during relayout.

Live scrolling with real media still needs a visual recheck on the user's desktop; passing state/layout tests is not a claim that all perceived jumps are gone.
