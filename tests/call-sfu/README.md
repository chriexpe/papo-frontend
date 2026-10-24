# Android call fixture

An isolated REST/WebSocket server and Pion SFU for the Compose call tests. It uses
synthetic VP8 video, Opus silence and the emulator's synthetic cameras. No real
server, account, microphone on the host, or external participant is contacted.

Requires Go (the version in `go.mod`), ffmpeg with libvpx, and a phone emulator
with host access through `10.0.2.2`. From the repository root:

```sh
tests/call-sfu/run.sh
```

The listener defaults to `127.0.0.1:18765`; ICE advertises `10.0.2.2`. Override
`-listen` and `-advertise` when using a different test environment. Keep it on a
trusted local interface: its accounts and control endpoints are test-only.

Build/install the debug app and test APK with the normal Compose Gradle tasks.
Grant the test app CAMERA and RECORD_AUDIO; Android 17 also requires
ACCESS_LOCAL_NETWORK. Select the **phone** serial explicitly, for example:

```sh
adb -s emulator-5554 shell pm grant io.github.chriexpe.papo.compose android.permission.RECORD_AUDIO
adb -s emulator-5554 shell pm grant io.github.chriexpe.papo.compose android.permission.CAMERA
adb -s emulator-5554 shell pm grant io.github.chriexpe.papo.compose android.permission.ACCESS_LOCAL_NETWORK
adb -s emulator-5554 shell am instrument -w -r \
  -e class io.github.chriexpe.papo.compose.CallTransportTest,io.github.chriexpe.papo.compose.CallLifecycleTest \
  io.github.chriexpe.papo.compose.test/androidx.test.runner.AndroidJUnitRunner
```

`CallTransportTest` uses an independent native handle, in-memory credentials and
a temporary database. It checks SDP/ICE/DTLS/SRTP, remote camera **and** screen
frames, local camera capture/switching, audio playback settings, cancellation
during ICE discovery, delayed leave/rejoin ordering, stale controls and socket
loss without automatic rejoining.

`CallLifecycleTest` temporarily adds **only this fixture** to the process-owned
client, verifies service/audio focus, browsing, Activity recreation, PiP and
notification controls, then forgets that fixture and restores the previous
server selection. It skips when a real call is already active or the fixture is
already saved. Run these tests sequentially against a single fixture.

`/stats` reports received outgoing audio/video packet counts and joins/leaves.
`/control/ice-delay?ms=200` delays ICE discovery; leave acknowledgements are always
delayed 250 ms. `/control/disconnect` closes the fixture socket. A successful test
proves actual transport and decoded frames, not audible speech quality or
physical Bluetooth hardware behavior.
