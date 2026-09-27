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
- The existing Glow glass path is legacy during this PR and is removed only
  after the WGPU compositor reaches visual parity.
- No Linux regression is acceptable at merge time: this branch stays draft
  until frosted glass works through the WGPU path as well.
