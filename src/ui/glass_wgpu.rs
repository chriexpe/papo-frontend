//! Backend-neutral frosted glass for desktop.
//!
//! The callback marks a paint-order barrier. Papo's narrow egui-wgpu patch
//! ends the active egui render pass before calling `composite`, so this code
//! can copy the pixels already drawn beneath the glass, blur them, composite
//! them back, and let egui resume drawing the tint/border/content on top.

use std::sync::Arc;

use eframe::egui_wgpu::{self, wgpu};

const DOWNSCALE: u32 = 4;
const PASSES: usize = 3;
const SATURATION: f32 = 1.35;

pub type SharedGlass = Arc<GlassRenderer>;

#[derive(Default)]
pub struct GlassRenderer;

impl GlassRenderer {
    pub fn new(state: &egui_wgpu::RenderState) -> Option<SharedGlass> {
        let resources = GlassGpu::new(&state.device, state.target_format);
        state
            .renderer
            .write()
            .callback_resources
            .insert(resources);
        Some(Arc::new(Self))
    }
}

#[derive(Clone, Copy)]
pub struct GlassCallback {
    pub corner_points: f32,
}

impl egui_wgpu::CallbackTrait for GlassCallback {
    fn is_compositor(&self) -> bool {
        true
    }

    fn composite(
        &self,
        info: egui::PaintCallbackInfo,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::Texture,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(gpu) = resources.get::<GlassGpu>() else {
            return;
        };
        gpu.composite(device, encoder, target, info, self.corner_points);
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        _render_pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        // Compositor callbacks are consumed between render-pass segments by
        // the patched egui-wgpu renderer, so normal paint is intentionally empty.
    }
}

struct GlassGpu {
    format: wgpu::TextureFormat,
    sampler: wgpu::Sampler,
    blur_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    blur_h: wgpu::RenderPipeline,
    blur_v: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
}

impl GlassGpu {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("papo_glass"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glass.wgsl").into()),
        });

        let texture_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };

        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("papo_glass_blur_layout"),
            entries: &[texture_entry, sampler_entry],
        });
        let composite_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("papo_glass_composite_layout"),
                entries: &[
                    texture_entry,
                    sampler_entry,
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let blur_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("papo_glass_blur_pipeline_layout"),
                bind_group_layouts: &[Some(&blur_layout)],
                immediate_size: 0,
            });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("papo_glass_composite_pipeline_layout"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            });

        let pipeline = |label: &'static str,
                        layout: &wgpu::PipelineLayout,
                        fragment: &'static str,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let blur_h = pipeline(
            "papo_glass_blur_h",
            &blur_pipeline_layout,
            "fs_blur_h",
            None,
        );
        let blur_v = pipeline(
            "papo_glass_blur_v",
            &blur_pipeline_layout,
            "fs_blur_v",
            None,
        );
        let composite = pipeline(
            "papo_glass_composite",
            &composite_pipeline_layout,
            "fs_composite",
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("papo_glass_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        Self {
            format,
            sampler,
            blur_layout,
            composite_layout,
            blur_h,
            blur_v,
            composite,
        }
    }

    fn texture(
        &self,
        device: &wgpu::Device,
        label: &'static str,
        width: u32,
        height: u32,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage,
            view_formats: &[self.format],
        })
    }

    fn sampled_group(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("papo_glass_sampled"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    fn blur_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        source: &wgpu::BindGroup,
        horizontal: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("papo_glass_blur"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(if horizontal {
            &self.blur_h
        } else {
            &self.blur_v
        });
        pass.set_bind_group(0, source, &[]);
        pass.draw(0..3, 0..1);
    }

    fn composite(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::Texture,
        info: egui::PaintCallbackInfo,
        corner_points: f32,
    ) {
        use wgpu::util::DeviceExt as _;

        let viewport = info.viewport_in_pixels();
        let x = viewport.left_px.max(0) as u32;
        let y = viewport.top_px.max(0) as u32;
        let width = viewport
            .width_px
            .max(0)
            .min(target.width().saturating_sub(x) as i32) as u32;
        let height = viewport
            .height_px
            .max(0)
            .min(target.height().saturating_sub(y) as i32) as u32;
        if width <= 1 || height <= 1 {
            return;
        }

        let captured = self.texture(
            device,
            "papo_glass_capture",
            width,
            height,
            wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &captured,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let down_width = (width / DOWNSCALE).max(1);
        let down_height = (height / DOWNSCALE).max(1);
        let ping_usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING;
        let ping0 = self.texture(
            device,
            "papo_glass_ping_0",
            down_width,
            down_height,
            ping_usage,
        );
        let ping1 = self.texture(
            device,
            "papo_glass_ping_1",
            down_width,
            down_height,
            ping_usage,
        );
        let captured_view = captured.create_view(&wgpu::TextureViewDescriptor::default());
        let ping0_view = ping0.create_view(&wgpu::TextureViewDescriptor::default());
        let ping1_view = ping1.create_view(&wgpu::TextureViewDescriptor::default());

        let mut source = self.sampled_group(device, &self.blur_layout, &captured_view);
        for pass_index in 0..PASSES * 2 {
            let horizontal = pass_index % 2 == 0;
            let target_view = if pass_index % 2 == 0 {
                &ping0_view
            } else {
                &ping1_view
            };
            self.blur_pass(encoder, target_view, &source, horizontal);
            source = self.sampled_group(device, &self.blur_layout, target_view);
        }

        let final_view = if (PASSES * 2 - 1) % 2 == 0 {
            &ping0_view
        } else {
            &ping1_view
        };

        let corner_px = corner_points * info.pixels_per_point;
        let params = [width as f32, height as f32, corner_px, SATURATION];
        let mut bytes = [0_u8; 16];
        for (chunk, value) in bytes.chunks_exact_mut(4).zip(params) {
            chunk.copy_from_slice(&value.to_ne_bytes());
        }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("papo_glass_params"),
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("papo_glass_composite_group"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(final_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });

        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("papo_glass_composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            x as f32,
            y as f32,
            width as f32,
            height as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(x, y, width, height);
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
