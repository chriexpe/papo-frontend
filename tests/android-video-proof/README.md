# Android native Surface proof

This debug-only app is independent of the product client, Compose and UniFFI.
Its fixed synthetic participant starts as the **Fixture Red** avatar. The label
changes to **Fenced VP8 · native Surface** only after a valid frame has been
packed and successfully posted to Android. No credentials, address, pixel
arrays, or frame handles cross JNI. The native client connects only to loopback.

## Build

Install Rust's `x86_64-linux-android` and `aarch64-linux-android` targets,
Cargo NDK, SDK 37 and NDK 30.0.16248370. Extract the GStreamer **Android**
1.28.7 universal development archive so that `GSTREAMER_ROOT_ANDROID` contains
both `x86_64/lib/pkgconfig` and `arm64/lib/pkgconfig` (defaults to
`~/Android/gst`). Do not use the host Linux GStreamer packages.

```sh
./android/gradlew -p android :video-proof:assembleDebug
```

The resulting APK is at `android/video-proof/build/outputs/apk/debug/video-proof-debug.apk`.
Gradle invokes the scoped build script, which links the required static
GStreamer plugins into the native library for both ABIs.

## Exercise the debug APK

On a Linux host with an Android emulator, start the existing loopback SFU:

```sh
tests/call-sfu/run.sh -advertise 10.0.2.2
```

For a selected test emulator, forward the fixture's TCP signalling port with
`adb reverse tcp:18765 tcp:18765`. This does not forward ICE or media: those
are UDP and use the emulator's host alias published by the fixture. Install
and open the debug APK. Verify the named avatar before frames arrive, then the
red picture and moving white counter with the live label. Background and
reopen to exercise detachment, reattachment and a new fenced subscription.
Stop the fixture and verify the avatar returns. Remove the TCP reverse rule
when finished. A physical handset additionally needs UDP reachability to the
fixture host, so do not advertise the emulator-only alias on that path.

### Dual-tile slot reuse & stalled-writer controls

Pass `--ez dual true` when launching `ProofActivity` alongside `tests/call-sfu/run.sh -slot-reuse-proof`
to display two tiles (`Fixture Red` + `Fixture Green`) sharing `papo-video-0`, controlled via
the SFU HTTP endpoints `/control/reuse/stage-new-rtp` and `/control/reuse/advance-route`.

Debug broadcast actions (`io.github.chriexpe.papo.videoproof.*`) exercise recovery paths without
UI automation races: `ACTION_ARM_STALL` (holds `ANativeWindow_lock` -> write boundary),
`ACTION_RELEASE_STALL`, `ACTION_RECREATE` (replaces active `TextureView`s while retiring the old
session independently), `ACTION_TRY_OVERFLOW_ATTACH` (verifies `-2` typed refusal when both live
and retiring slots are occupied), and `ACTION_DUMP_STATE` (logs native + Java JSON telemetry).
