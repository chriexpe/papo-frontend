# Android voice/video calls — PR #125

This follow-up starts from merged PR #124. The first review gate is an interactive
**Demo-only UI**. Live-server call integration and media-engine extraction follow
approval of this presentation. No call transport, microphone/camera capture,
permissions or Android background-call behavior is enabled by this first slice.

## UI review build

Open **Servers → Explore demo**, then **Lounge** or **Studio** in Chats.
Opening a voice channel shows its lobby without joining it. Join preview starts
muted with the camera off. Lounge exercises a group; Studio exercises a two-person
call. The participant roster and any sample camera/screen content are explicitly
part of Demo, not real users or a connection to a server.

- Back and the downward chevron minimize the call. Only Leave ends its session.
- The current conversation header keeps its name, pin and people buttons. All
  call identity, participant avatars, microphone/camera and Leave controls live
  in a separate pill immediately below it, matching the call pill while browsing
  Chats or Servers. The room/server label sits on the left; the space before the
  controls holds as many participant squircles as fit, prioritizing speakers.
- The room label reopens the call. Its server label and account-scoped avatars
  continue identifying the call while another server's conversation is open.
- Chats and Servers also keep a compact call header available while browsing.
- The Chat/Call views share the voice channel's Demo messages and saved composer.
  Opening Chat doesn't join, mute, leave or replace a call.
- Joining another room asks to leave the current one. Cancel leaves it intact.
- The call screen's overflow offers voice, sample video and sample shared-screen
  layouts. Tap a participant to enlarge; a filled Show everyone button or Back
  restores the grid before minimizing.
- When video is available, the browsing call pill includes an expand button and
  a 0–4 preview-count picker (default 2). Zero hides video previews without
  switching off the camera or leaving. Fewer available videos leave no empty
  tiles. Speaker priority chooses videos when capped; retained tiles keep their
  relative order. Preview height adapts to the viewport and keyboard.
- Opening video slightly separates the call pill from the channel header and
  expands the controls and previews into one glass surface; hiding reverses it.
  A short spring/opacity transition respects app and Android motion settings.
  Speaker changes do not replay this transition.
- The video-count sheet is hosted once above navigation and bound to the active
  session token, so changing chat/account cannot create competing pickers. Like
  the full-screen call, it uses the existing covered-conversation visibility guard.
- Audio output choices preview selection only. They do not change Android routing.
- Speaker changes are a five-second Demo cycle. Tiles retain their order. Studio
  has one remote participant and therefore retains that participant as its speaker.

The preview controller belongs to the ViewModel, separately from selected server,
conversation and full-screen presentation. Activity recreation preserves a preview;
process death does not restore a fictional connection. Session tokens reject stale
controls after leaving/rejoining the same channel. Removing the Demo account ends
its preview. Foreground call presentation uses the existing overlay visibility
mechanism so the covered conversation is not reported visible/read.

UI copy is in English/Portuguese Android string/plural resources. Presentation uses
the existing theme, UI-density preferences and 48dp minimum control hit regions.
The screen and grid, controls, header and preview state are separated into files.

## Subsequent stages (after UI acceptance)

1. Separate signaling/media ownership from egui, preserving core session/attempt
   guards and authoritative room membership. Verify the current backend contract
   before exposing any direct-call/ringing or screen-publishing features.
2. Connect actual microphone/audio reception, routing, permission denial, mute,
   disconnect and room switching to the approved UI.
3. Connect video reception/publication, camera switching and shared-screen viewing.
4. Integrate Android call lifetime, notification controls and picture-in-picture;
   verify activity/process/network transitions and supported background behavior.
5. Enable voice-channel message history/cache for live servers. The existing core
   deliberately skips it; the backend supports voice-channel messages, but that
   production change is deferred with the rest of the wiring.

The previously deferred hourglass investigation remains separate. This UI slice
changes no production read-state/navigation algorithm, core media transport or
live-server voice-channel visibility. Device instrumentation follows the user's
UI review; it has not been run for this slice.
