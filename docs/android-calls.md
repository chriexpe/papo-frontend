# Android voice/video calls — PR #125

The accepted Material 3 call presentation now uses live server state and the
shared GStreamer engine. Demo still uses labelled sample content and never opens
a microphone or camera.

## Ownership and integration

- `papo-core` owns protocol state, room presence and voice-channel message history.
- `papo-call` owns GStreamer/WebRTC workers, SFU camera/screen slots, immutable RGBA
  frames and per-publisher audio mixing. It has no egui or Activity dependency.
  egui retains a texture/presentation adapter over the same worker.
- `papo-android` exposes typed call snapshots, semantic errors, attempt-guarded
  actions and frame reads. Media/signaling continue without Compose collection.
- Android owns runtime permissions, audio focus/device routing, foreground
  service, notification actions, PiP and rendering. Connections belong to the
  Application's ViewModelStore, independently of Activity recreation.

Native media workers never invoke Java callbacks directly. GStreamer caches its
JNI environment per thread; foreign callback attachment/detachment can invalidate
that cache. Device permissions/rotation are sampled on the JVM-owned actor into
native state, and a dedicated notifier thread delivers wake callbacks. Frames use
an independent latest-frame path instead of publishing chat snapshots at video
rate. Android's renderer reuses a bitmap and pauses when its surface is stopped.

Android packaging uses the pinned, checksummed GStreamer 1.28.7 Android SDK,
including CA assets. CI caches its arm64 and x86_64 SDK directories. Both ABIs
build the native call runtime; ordinary chat media remains on Media3. Default
host bindings/tools builds do not require GStreamer; Android enables `calls`.

## Behavior

Opening a voice channel shows its lobby without joining. Joining starts muted
with the camera off. Back/minimize returns to browsing; Leave ends the call.
Chat/Call tabs share that voice channel's real messages, cache and draft.

The current conversation keeps its channel/pin/people navigation. A separate
call pill immediately below it contains room/server identity, speaker-prioritized
participant squircles and mic/camera/leave controls. It also remains available in
Chats and Servers, scoped to the call's original account and room.

When real camera or shared-screen sources exist, participant-strip taps first
reopen hidden previews, then open the call screen. The adjacent count control
cycles 1 → 2 → 3 → 4 → 1, only while previews are visible. A subtle × closes the
preview area without stopping publication. No sources means no extra video area;
remote video reception never depends on enabling the local camera. Retained
tiles preserve their order while speakers receive selection priority.

Roster/lobby rows and call avatar/name targets open the dedicated participant
action drawer. View profile and Message remain separate actions. Other joined
participants offer receiver-side 0–200% volume, reset to 100%, and Mute for me.
These controls preserve each other and never alter the sender's microphone
indicator. Overrides reset on leave/replacement. Your own drawer offers mic,
camera and camera switching. Tile backgrounds enlarge the participant; a filled
Show everyone button restores the grid.

Track-to-user ownership comes from `voice_audio_routes`. Older servers without
that event retain normal audio, but individual gain/mute controls explain their
unavailability rather than modifying unidentified audio. Cold presence only
identifies occupants; unknown microphone/camera status is not presented as a
confirmed device state.

## Lifecycle and ordering

The foreground call notification opens the call, toggles mute and hangs up.
Audio uses communication mode, exclusive transient focus and Android's explicit
communication device selection (speaker, earpiece, wired/USB or Bluetooth).
OpenSL ES capture/playback use communication/voice presets to match that routing.
Device changes update routing. Focus loss mutes the microphone; focus recovery
does not silently unmute it. Permission/initialization failures remain semantic
errors translated by Compose in English/Portuguese resources.

Active video/screen calls enter system PiP when leaving the app. PiP shows a
shared screen or the current/last speaker, with camera/avatar fallback and
attempt-scoped mute/leave actions. Audio-only calls continue through the service.
Call teardown releases communication mode, focus, service and scoped wake lock.
Process death does not restart a fictional call; reconnect does not autojoin.

Attempt IDs remain unique when native clients are recreated. Stale UI/frame/
notification actions cannot affect replacement calls. ICE discovery alone never
enters a room: Android validates its attempt before sending `voice_join`.
Because server leave events have no attempt ID, rapid rejoining waits for the
previous leave acknowledgement. A timeout reports failure and retains that
barrier until acknowledgement/disconnect. Cancelling during discovery discards
the eventual result without joining later.

## Scope and limitations

This implements voice-channel calls, camera publication/reception, camera
switching and incoming shared-screen viewing. It does not add Android screen
capture/publication or invent a direct-call invitation/ringing protocol. Those
need separate UI/platform work and backend support where applicable.

The hourglass/read-state investigation remains deferred. Voice channels now use
the existing history/cache path; its reading/navigation algorithms are unchanged.
Physical microphone/speaker quality, Bluetooth hardware and multi-device internet
TURN behavior still require testing on actual devices/servers. Emulator tests
prove transport, frames and lifecycle rather than acoustic quality.

## Validation and review

- 15 shared media host tests, including actual GStreamer volume properties and
  SFU slot reassignment; 486 core tests; 73 Android adapter host tests.
- Existing egui library still compiles against the shared engine.
- Universal arm64-v8a/x86_64 debug APK, Kotlin formatting check and Android lint.
- Two focused Android tests against the isolated Pion SFU: actual media/camera/
  screen frames, cancellation/rejoin/stale actions and socket-loss behavior;
  process-owned calls, browsing, Activity recreation, foreground service/audio
  mode, PiP and notification controls.

Fixture setup and exact device-test commands: [`tests/call-sfu/README.md`](../tests/call-sfu/README.md).
Use synthetic cameras and that local fixture for automated tests; never connect
an automated microphone/camera test to a user's saved real server.

For a UI-only review, open **Servers → Explore demo → Lounge/Studio**. Demo's
sample video/screen layouts remain in its overflow. Live calls expose only actual
sources/devices. Presentation follows existing density/theme/motion preferences
and keeps 48dp interactive targets. The latest real-call review APK is
`target/android-apks/papo-android-0.8.5-calls-live.apk`.
