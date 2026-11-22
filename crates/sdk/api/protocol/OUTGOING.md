# Outgoing protocol inventory

Reviewed against Papo-Chat/papo-backend at the revision in `backend-commit`.
The OpenAPI and gateway drift checks enforce that same revision. This pin
updates the older fork revision; it does not include unmerged plugin/app work.

| Concern | Production contract | SDK consequence |
| --- | --- | --- |
| Message creation | `POST /messages`, multipart; required `channel_id`, optional `content`, `reply_to`, JSON string `embeds`, repeated file parts `attachments`; success 201 with `Message` | Preserve each filename and file's bytes; never convert the upload to JSON |
| Attachment-backed embed | `thumbnail`/`image` accepts exactly one of HTTPS `url` or `attachment_name`; the latter must identify exactly one sanitized multipart filename in the same request | Keep names immutable with the durable payload; referenced files become embed media, not standalone message attachments |
| Embed video | HTTPS URL and supported `mime_type` required | Attachment references do not replace the video URL contract |
| Editing | `PUT /messages/{message_id}` with JSON `content` and optional embeds | Editing does not upload new multipart files |
| Deleting | `DELETE /messages/{message_id}` | Dependencies need the acknowledged backend message ID; a 404 means the message is already gone, so the deletion is done, not refused |
| Reactions | Channel/message-scoped POST/DELETE; server totals arrive on the gateway | Do not infer message identity from content or time |
| Message identity | Human creation has no nonce or idempotency key; identical POSTs create different IDs | No automatic replay of an ambiguous dispatch, including a transport failure, lost success body, or process death after committing `Sending` |
| Response additions | Nullable attachment video/thumbnail dimensions; search/notification external authors | Preserve metadata; unknown dimensions remain unknown |
| Channel additions | `parent_id` in REST and create/update events; atomic `access_roles` on creation and initial role `users` | Regenerate the complete contract; fixture emits category-only changes too |
| Voice peer identity and codec | `voice_joined` carries opaque `peer_id` and `video_codec`; owner-scoped events (`voice_answer`, `voice_offer`, `voice_ice_candidate`, `voice_audio_routes`, `voice_video_routes`, async `error`) and `voice_leave` carry `peer_id`; `error` also carries optional `channel_id` | Drop owner-scoped events and self-`voice_leave` whose `peer_id` differs from the current peer; leave and rejoin back-to-back on the same socket without a barrier timer |
| Voice video routes | `voice_video_routes` mirrors `voice_audio_routes` with `peer_id`, per-peer `revision` and `routes: { track_id, user_id, kind, first_seq?, rtp_route_id?, rtp_route_ext_id? }[]`; `revision` is compared per event type (audio vs video) | Drive video slot bindings directly from `voice_video_routes` without client-side slot prediction; drop stale revisions per route event kind. Route fences require both a nonzero `u64` assignment ID and an extension ID in `1..=14`; missing or invalid pairs leave the route without a video destination. `first_seq` is legacy metadata only |
| Voice leave audience | `voice_leave` reaches users with `connect_voice` and always reaches the departing owner connection even after permission revocation | Cold `presence_sync.user_voice` is still unfiltered on the backend and is filtered by admitted voice permission in the SDK |

A video slot keeps its assignment ID until reused or rebound. The negotiated
`urn:papo:rtp-hdrext:video-route-v1` extension carries the ID as eight big-endian
bytes in an RFC 8285 one-byte header extension.

The transport performs one attempt and disables redirects and request retries.
A decoded successful message response supplies the authoritative message ID;
transport and decoding errors do not establish that a message was not created. Server 5xx responses
also require conservative outcome handling in the outgoing queue: an error can
occur after a write. This inventory does not establish retry safety for every
mutation route.

`message_upload` proves multipart wire preservation on the loopback fixture.
The isolated PostgreSQL proof also uses production handlers to test uploaded
embed image retrieval, missing/ambiguous filenames, mutually exclusive URL and
filename inputs, and distinct IDs for two identical successful human POSTs.
These tests prove the pin's behavior, not the deployment revision of a user server.
