# Media cache audit (PR #98)

## Architecture and contract

Papo has three independent lifetimes:
1. Network-backed server state (the server is authoritative).
2. Persistent local resources (Turso for normalized metadata and a disk media cache for larger authenticated/public binaries).
3. Volatile presentation resources (decoded egui/GPU textures and GStreamer players).

Evicting a GPU texture must not be treated as invalidating the source asset. Offline hydration must be speculative: a later successful server response always wins. Failure to contact the server must not erase previously cached data.

## Inventory

| Surface | Existing source | Current persistence | Notes |
| --- | --- | --- | --- |
| Message attachments | authenticated /attachments/:id | files on disk | cached by server key; bounded/aged |
| Attachment thumbnails | authenticated thumbnail | files on disk | server-scoped; bounded/aged |
| Profile banners | /media/:sha | disk | content-addressed |
| Rich-preview external images | remote HTTP | disk | canonical URL identity; no cross-account session dependency |
| Link preview metadata | preview coordinator | Turso | distinct from image bytes |
| Custom server stickers (emojis) | /emojis base64 list | **Turso BLOB snapshot, added here** | asynchronous restore; authoritative empty list; per-server isolation |
| Custom sticker/avatar/icon GPU images | base64 attached to state | volatile textures | texture keys versioned by source content in this PR |
| User profile avatars | /users/profiles base64 | **volatile only** | future: use versioned, bounded original asset cache with account/server ownership |
| Server icon | /server base64 | **volatile only** | future: persist alongside versioned server metadata, not as an unbounded list |
| Giphy picker previews | Giphy provider | volatile only | intentionally ephemeral; never pin decoded animation indefinitely |
| GIFs in chat | remote provider URLs | provider-dependent | preserve animation limits and eviction priority |
| Audio/video players | authenticated assets | file cache + live player | player lifetime bounded independently |

## Correctness cases covered by this PR

- Sticker list initially absent vs an authoritative empty list are different states.
- A new server list must supersede any race with old local hydration.
- Changing sticker/avatar/icon bytes without changing the logical ID must invalidate stale textures.
- Removing a server clears its persistent emoji rows.
- Authentication/account reset clears reconstructible emoji rows alongside cached messages.
- Media remains server-scoped and encrypted transport credentials are never written to the image store.

## Remaining cross-cutting work before calling media persistence fully unified

- Move other repeatedly-used base64 media (profile avatars, server icon) behind a versioned, bounded disk asset API rather than copying base64 into many UI state maps.
- Add global disk-budget enforcement for those new buckets, quota instrumentation, and version-aware orphan cleanup.
- Preserve client responsiveness: no synchronous asset DB/disk reads in the render frame.
- Consider conditional requests (ETag/If-None-Match or revision cursor) at the backend for /emojis and profile/server endpoints to avoid repeat wire transfer. The current contract returns full base64.
- Test stale/empty/offline/owner-switch/multi-server and GPU pressure scenarios on desktop and Android.

This audit intentionally does **not** propose persisting decoded GPU frames or Giphy picker previews; that previously caused avoidable memory pressure.
