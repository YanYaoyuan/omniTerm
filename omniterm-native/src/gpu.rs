// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Draws terminal grids on the GPU inside egui's render pass: one instanced quad per background
//! run, glyph piece, cursor or underline, sampled from the shared glyph atlas.

use crate::glyphs::{ATLAS_SIZE, Upload};
use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const SOLID: u32 = 0;
pub const MASK: u32 = 1;
pub const COLOR: u32 = 2;

/// One quad. Positions are physical pixels inside the pane; colors are straight sRGB.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable, PartialEq)]
pub struct Instance {
    pub rect: [f32; 4],
    pub atlas: [f32; 4],
    pub color: [f32; 4],
    pub kind: u32,
    pub _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct View {
    size: [f32; 2],
    atlas: [f32; 2],
    /// 1 when the window's surface is sRGB, so colors must be converted to linear first.
    srgb: f32,
    _pad: [f32; 3],
}

const SHADER: &str = r#"
struct View { size: vec2<f32>, atlas: vec2<f32>, srgb: f32 }
@group(0) @binding(0) var atlas_texture: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;
@group(1) @binding(0) var<uniform> view: View;

struct Quad {
    @location(0) rect: vec4<f32>,
    @location(1) atlas: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) kind: u32,
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) kind: u32,
}

@vertex
fn vs(@builtin(vertex_index) corner_index: u32, quad: Quad) -> Varyings {
    let corner = vec2<f32>(f32(corner_index & 1u), f32(corner_index >> 1u));
    let pixel = quad.rect.xy + corner * quad.rect.zw;
    var out: Varyings;
    out.position = vec4<f32>(pixel.x / view.size.x * 2.0 - 1.0, 1.0 - pixel.y / view.size.y * 2.0, 0.0, 1.0);
    out.uv = (quad.atlas.xy + corner * quad.atlas.zw) / view.atlas;
    out.color = quad.color;
    out.kind = quad.kind;
    return out;
}

fn linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs(in: Varyings) -> @location(0) vec4<f32> {
    var rgb = in.color.rgb;
    if (view.srgb > 0.5) { rgb = linear(rgb); }
    let texel = textureSampleLevel(atlas_texture, atlas_sampler, in.uv, 0.0);
    if (in.kind == 0u) {
        return vec4<f32>(rgb * in.color.a, in.color.a);
    }
    if (in.kind == 1u) {
        let alpha = texel.a * in.color.a;
        return vec4<f32>(rgb * alpha, alpha);
    }
    var glyph = texel.rgb;
    if (view.srgb > 0.5 && texel.a > 0.0) { glyph = linear(glyph / texel.a) * texel.a; }
    return vec4<f32>(glyph, texel.a) * in.color.a;
}
"#;

struct PaneBuffers {
    instances: wgpu::Buffer,
    capacity: u64,
    view: wgpu::Buffer,
    view_group: wgpu::BindGroup,
    count: u32,
    version: u64,
    size: [f32; 2],
}

pub struct Resources {
    pipeline: wgpu::RenderPipeline,
    atlas: wgpu::Texture,
    atlas_group: wgpu::BindGroup,
    view_layout: wgpu::BindGroupLayout,
    srgb: bool,
    panes: HashMap<u64, PaneBuffers>,
}

/// Builds the pipeline once and stores it where egui hands it back to every callback.
pub fn install(render_state: &egui_wgpu::RenderState) {
    let device = &render_state.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("omniterm-grid"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("omniterm-atlas"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("omniterm-view"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("omniterm-grid"),
        bind_group_layouts: &[Some(&atlas_layout), Some(&view_layout)],
        immediate_size: 0,
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Uint32];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("omniterm-grid"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Instance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: render_state.target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let atlas = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("omniterm-atlas"),
        size: wgpu::Extent3d { width: ATLAS_SIZE, height: ATLAS_SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("omniterm-atlas"),
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
    let atlas_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("omniterm-atlas"),
        layout: &atlas_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&atlas_view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
        ],
    });
    let resources = Resources {
        pipeline,
        atlas,
        atlas_group,
        view_layout,
        srgb: render_state.target_format.is_srgb(),
        panes: HashMap::new(),
    };
    render_state.renderer.write().callback_resources.insert(resources);
}

/// One pane's draw for one frame.
pub struct GridPaint {
    pub pane: u64,
    pub size: [f32; 2],
    pub instances: Arc<Vec<Instance>>,
    /// Changes when the quads do; unchanged quads are not uploaded again.
    pub version: u64,
    /// Atlas pixels rasterized this frame, shared by every pane: the first to prepare uploads them.
    pub uploads: Arc<Mutex<Vec<Upload>>>,
}

impl CallbackTrait for GridPaint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = resources.get_mut::<Resources>() else { return Vec::new() };
        for upload in self.uploads.lock().unwrap_or_else(|p| p.into_inner()).drain(..) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &resources.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: upload.x, y: upload.y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &upload.rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(upload.width * 4), rows_per_image: Some(upload.height) },
                wgpu::Extent3d { width: upload.width, height: upload.height, depth_or_array_layers: 1 },
            );
        }
        let bytes = bytemuck::cast_slice::<Instance, u8>(&self.instances);
        let needed = (bytes.len() as u64).max(256);
        let view_layout = &resources.view_layout;
        let buffers = resources.panes.entry(self.pane).or_insert_with(|| {
            let view = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("omniterm-view"),
                size: std::mem::size_of::<View>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let view_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("omniterm-view"),
                layout: view_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: view.as_entire_binding() }],
            });
            PaneBuffers { instances: instance_buffer(device, needed), capacity: needed, view, view_group, count: 0, version: u64::MAX, size: [0.0; 2] }
        });
        if buffers.version != self.version {
            if buffers.capacity < needed {
                let capacity = needed.next_power_of_two();
                buffers.instances = instance_buffer(device, capacity);
                buffers.capacity = capacity;
            }
            queue.write_buffer(&buffers.instances, 0, bytes);
            buffers.count = self.instances.len() as u32;
            buffers.version = self.version;
        }
        if buffers.size != self.size {
            let view = View { size: self.size, atlas: [ATLAS_SIZE as f32; 2], srgb: if resources.srgb { 1.0 } else { 0.0 }, _pad: [0.0; 3] };
            queue.write_buffer(&buffers.view, 0, bytemuck::bytes_of(&view));
            buffers.size = self.size;
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        let Some(resources) = resources.get::<Resources>() else { return };
        let Some(buffers) = resources.panes.get(&self.pane) else { return };
        if buffers.count == 0 {
            return;
        }
        pass.set_pipeline(&resources.pipeline);
        pass.set_bind_group(0, &resources.atlas_group, &[]);
        pass.set_bind_group(1, &buffers.view_group, &[]);
        pass.set_vertex_buffer(0, buffers.instances.slice(..));
        pass.draw(0..4, 0..buffers.count);
    }
}

use eframe::egui;

fn instance_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("omniterm-instances"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Frees the buffers of panes that are gone.
pub fn forget(render_state: &egui_wgpu::RenderState, live: &[u64]) {
    if let Some(resources) = render_state.renderer.write().callback_resources.get_mut::<Resources>() {
        resources.panes.retain(|pane, _| live.contains(pane));
    }
}
