# Rendering architecture

Papo's UI remains **egui**. The desktop GPU backend converges on **WGPU** so
the application does not maintain separate Direct3D, Vulkan, Metal and OpenGL
implementations.

## Target shape

```text
egui (layout/input/paint primitives)
        |
        v
Papo compositor
  - scene texture
  - backdrop glass
  - future GPU effects
        |
        v
egui-wgpu / wgpu
  Windows -> D3D12
  Linux   -> Vulkan
  macOS   -> Metal
```

Android remains on the existing eframe/Glow path until its rendering/lifecycle
work is intentionally migrated.

## KWin Wayland close-to-tray compatibility

Closing Papo from Plasma's window overview reproduced a compositor SIGSEGV on
2026-10-27 with KWin 6.8.0 and Mesa RADV 26.2.4. The recorded stack was
`SurfaceInterface::requestedTimingOfNextCommit` →
`SurfaceItemWayland::handleFramePainted`. Papo subsequently exited because its
Wayland connection broke; there was no GPU-reset message in the captured kernel
log. The capture is local and contains no committed account logs.

Inspection of KWin's `handleFramePainted` and `clearFifoBarrier` suggests that
applying a queued unmap can freeze the item and clear its surface before the
next dereference. Mesa uses that FIFO barrier path for Vulkan FIFO presentation.
This is the working explanation, not proof that every compositor crash has the
same cause.

The vendored egui-wgpu painter therefore prefers supported **Mailbox**
presentation for native Wayland windows in a KDE session when the requested mode
is AutoVsync. Mailbox remains tear-free without Mesa's FIFO barriers. Other
window backends, explicit presentation choices, and surfaces without Mailbox
support keep their existing mode. The choice is retained when a surface is
recreated. The debug log reports when the workaround is selected.

Upstream sources used for the diagnosis:

- [KWin surface painting](https://invent.kde.org/plasma/kwin/-/blob/Plasma/6.8/src/scene/surfaceitem_wayland.cpp)
- [KWin FIFO barrier processing](https://invent.kde.org/plasma/kwin/-/blob/Plasma/6.8/src/wayland/surface.cpp)
- [Mesa Wayland presentation](https://gitlab.freedesktop.org/mesa/mesa/-/blob/main/src/vulkan/wsi/wsi_common_wayland.c)

Linux termination monitoring also uses the session portal, logind when
accessible, and SIGTERM/SIGINT independently. Portal QueryEnd is acknowledged
without inhibiting logout; Papo exits on Ending, allowing cancellation by other
applications. Native close commands wake the event loop even while hidden so
normal draft/settings persistence can run. Flatpak no longer depends solely on
an inaccessible system bus for session-end notification.

## Why glass cannot be a normal WGPU PaintCallback

The current glass renderer uses `egui_glow` to copy pixels that have already
been drawn underneath a pill, blur them, and composite them back. A direct
translation to an `egui_wgpu::Callback` is not correct: WGPU/WebGPU does not
allow the active render attachment to be sampled as a texture at the same
time.

Therefore exact backdrop blur requires Papo to own an **offscreen scene
texture**:

1. render normal egui/content into the scene texture;
2. copy/downsample the regions requested by glass surfaces;
3. run horizontal/vertical WGSL blur passes;
4. composite the blurred regions with saturation/rounded masks;
5. present the composed frame.

This is deliberately a renderer-level change rather than another
platform-specific blur workaround.

## Migration rules

- Desktop selects `eframe::Renderer::Wgpu` through `render::desktop_renderer`.
- Papo code must not branch on D3D12/Vulkan/Metal for visual effects.
- New shaders are WGSL and live under the rendering layer.
- The existing Glow glass path is temporary migration code and disappears once
  the WGPU compositor is wired in.
- Visual effects are defined once at the Papo/WGPU layer. Windows, Linux and
  macOS consume the same implementation; native API differences are WGPU's job.
