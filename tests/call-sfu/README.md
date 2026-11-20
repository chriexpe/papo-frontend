# GStreamer SFU proof

The Go fixture runs on loopback. It generates VP8 colour and frame-counter
patterns and 440 Hz and 880 Hz Opus tones. The Rust proof client owns typed
REST and gateway signalling outside the GStreamer engine. Both capture
sources and the speaker output are synthetic.

Requires Go, FFmpeg with libvpx and libopus, Rust, and the GStreamer WebRTC,
Opus and VP8 plugins.

```sh
tests/call-sfu/run.sh
```

In another shell, from the repository root:

```sh
PAPO_CALL_PROOF_ADDRESS=http://127.0.0.1:18765 cargo run --locked -p papo-call-proof
curl -fsS http://127.0.0.1:18765/stats | python3 -c '
import json, sys
stats = json.load(sys.stdin)
assert stats["audio"] > 0 and stats["video"] > 0
print("Upstream RTP verified for audio and camera")
'
```

The proof checks offer layout and RFC 6464, duplex RTP, several cameras and
a distinct screen, and PCM tone energy after the audio slot assignment changes.
The fixture rejects the first subscription to the replacement camera, then
reuses the old slot with a late packet whose sequence precedes `first_seq`.

## Pinned backend

Supply a disposable, empty loopback PostgreSQL database via `DATABASE_URL`
and a policy-compliant generated password via `PAPO_PROOF_PASSWORD`. Use a
fresh database for each codec. The runner verifies the backend checkout against
the SDK's protocol pin.

```sh
python3 scripts/papo-backend-proof.py --backend .papo/backend-voice --media-codec vp8
python3 scripts/papo-backend-proof.py --backend .papo/backend-voice --media-codec h264
```

The client checks negotiated WebRTC, duplex PCM, camera publication and
decoding, publisher churn, subscription rejection, reused slot attribution,
and drain. Its diagnostics show the encoder and decoder factory names.

## Lease pressure and resource counts

```sh
cargo test --locked -p papo-call
cargo test --locked -p papo-call --test lease resources_plateau -- --ignored --nocapture
```

The Linux lease test measures threads and file descriptors after 100 warmed-up
cycles. Run the negotiated variant against the synthetic Go fixture to include
ICE, camera and microphone capture, and repeated leave/rejoin on the same socket.

```sh
PAPO_CALL_PROOF_ADDRESS=http://127.0.0.1:18765 PAPO_CALL_PROOF_PLATEAU=1 \
    cargo run --locked -p papo-call-proof
```
