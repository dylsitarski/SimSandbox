//! Meshes drawn by wgpu in the window (ADR 0086 point 7): a view's
//! triangles (already projected, in the view's normalised device
//! coordinates, depth in [0, 1]) and outlines are rendered into a texture
//! of its own with a depth buffer, in the paint callback's `prepare`, and
//! that texture is put on screen in `paint`. Installed once when the app
//! starts ([`install`]); without it views draw with egui's painter, as
//! headless renders do.

use egui_wgpu::wgpu;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use wgpu::util::DeviceExt;

static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Whether the pipelines are installed (in the app's window).
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

/// One corner: where (normalised device coordinates, depth 0 nearest to
/// 1 furthest) and its colour (sRGB-encoded, as egui's).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub colour: [f32; 4],
}

impl Vertex {
    pub fn new(pos: [f32; 3], c: egui::Color32) -> Self {
        Self {
            pos,
            colour: [c.r(), c.g(), c.b(), c.a()].map(|v| v as f32 / 255.0),
        }
    }
}

const DRAW: &str = r#"
struct Out { @builtin(position) pos: vec4<f32>, @location(0) colour: vec4<f32> };
@vertex fn vs(@location(0) p: vec3<f32>, @location(1) c: vec4<f32>) -> Out {
    var o: Out;
    o.pos = vec4<f32>(p, 1.0);
    o.colour = c;
    return o;
}
@fragment fn fs(i: Out) -> @location(0) vec4<f32> { return i.colour; }
"#;

const BLIT: &str = r#"
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    return o;
}
@fragment fn fs(i: Out) -> @location(0) vec4<f32> { return textureSample(t, s, i.uv); }
"#;

/// A view's own picture on the GPU.
struct Target {
    size: [u32; 2],
    colour: wgpu::TextureView,
    depth: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

struct Resources {
    format: wgpu::TextureFormat,
    triangles: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    targets: HashMap<u64, Target>,
}

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Create the pipelines in the window's renderer.
pub fn install(rs: &egui_wgpu::RenderState) {
    let device = &rs.device;
    let format = rs.target_format;
    let draw = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("simsandbox mesh"),
        source: wgpu::ShaderSource::Wgsl(DRAW.into()),
    });
    let blit = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("simsandbox mesh blit"),
        source: wgpu::ShaderSource::Wgsl(BLIT.into()),
    });
    let empty = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("simsandbox mesh"),
        bind_group_layouts: &[],
        immediate_size: 0,
    });
    let vertex = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
    };
    let pipeline = |topology| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("simsandbox mesh"),
            layout: Some(&empty),
            vertex: wgpu::VertexState {
                module: &draw,
                entry_point: Some("vs"),
                buffers: &[Some(vertex.clone())],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &draw,
                entry_point: Some("fs"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    let triangles = pipeline(wgpu::PrimitiveTopology::TriangleList);
    let lines = pipeline(wgpu::PrimitiveTopology::LineList);
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("simsandbox mesh blit"),
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
    let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("simsandbox mesh blit"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("simsandbox mesh blit"),
        layout: Some(&blit_layout),
        vertex: wgpu::VertexState {
            module: &blit,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &blit,
            entry_point: Some("fs"),
            targets: &[Some(format.into())],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("simsandbox mesh"),
        ..Default::default()
    });
    rs.renderer.write().callback_resources.insert(Resources {
        format,
        triangles,
        lines,
        blit,
        layout,
        sampler,
        targets: HashMap::new(),
    });
    INSTALLED.store(true, Ordering::Relaxed);
}

/// One view's mesh to draw this frame.
pub struct MeshCallback {
    /// Which view (its targets are kept between frames).
    pub id: u64,
    /// The picture's size in pixels.
    pub size: [u32; 2],
    pub background: egui::Color32,
    pub triangles: Vec<Vertex>,
    pub lines: Vec<Vertex>,
}

/// An sRGB-encoded colour as a target of `format` takes it: linear for
/// sRGB formats, which encode it again.
fn for_format(c: [f32; 4], format: wgpu::TextureFormat) -> [f32; 4] {
    if !format.is_srgb() {
        return c;
    }
    let lin = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    [lin(c[0]), lin(c[1]), lin(c[2]), c[3]]
}

impl egui_wgpu::CallbackTrait for MeshCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = resources.get_mut::<Resources>() else {
            return Vec::new();
        };
        let size = [self.size[0].max(1), self.size[1].max(1)];
        // Views closed long ago: their pictures go.
        if r.targets.len() > 32 {
            r.targets.clear();
        }
        if r.targets.get(&self.id).is_none_or(|t| t.size != size) {
            let texture = |format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("simsandbox mesh"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let colour = texture(
                r.format,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            )
            .create_view(&Default::default());
            let depth = texture(DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT)
                .create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("simsandbox mesh"),
                layout: &r.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&colour),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&r.sampler),
                    },
                ],
            });
            r.targets.insert(
                self.id,
                Target {
                    size,
                    colour,
                    depth,
                    bind,
                },
            );
        }
        let target = &r.targets[&self.id];
        let upload = |v: &[Vertex]| {
            let v: Vec<Vertex> = v
                .iter()
                .map(|p| Vertex {
                    pos: p.pos,
                    colour: for_format(p.colour, r.format),
                })
                .collect();
            (!v.is_empty()).then(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("simsandbox mesh"),
                    contents: bytemuck::cast_slice(&v),
                    usage: wgpu::BufferUsages::VERTEX,
                })
            })
        };
        let (tris, lines) = (upload(&self.triangles), upload(&self.lines));
        let bg = for_format(
            [
                self.background.r(),
                self.background.g(),
                self.background.b(),
                255,
            ]
            .map(|v| v as f32 / 255.0),
            r.format,
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("simsandbox mesh"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.colour,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: bg[0] as f64,
                        g: bg[1] as f64,
                        b: bg[2] as f64,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(b) = &tris {
            pass.set_pipeline(&r.triangles);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..self.triangles.len() as u32, 0..1);
        }
        if let Some(b) = &lines {
            pass.set_pipeline(&r.lines);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..self.lines.len() as u32, 0..1);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(r) = resources.get::<Resources>() else {
            return;
        };
        let Some(target) = r.targets.get(&self.id) else {
            return;
        };
        pass.set_pipeline(&r.blit);
        pass.set_bind_group(0, &target.bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Draw `callback` over `rect` (ADR 0086).
pub fn paint(painter: &egui::Painter, rect: egui::Rect, callback: MeshCallback) {
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, callback));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_reach_srgb_and_plain_targets_alike() {
        let v = Vertex::new([0.0, 0.0, 0.5], egui::Color32::from_rgb(255, 128, 0));
        assert_eq!(v.colour, [1.0, 128.0 / 255.0, 0.0, 1.0]);
        // A plain target takes egui's encoded colour as it is; an sRGB one
        // the linear value it encodes back to the same.
        let plain = wgpu::TextureFormat::Bgra8Unorm;
        let srgb = wgpu::TextureFormat::Bgra8UnormSrgb;
        assert_eq!(for_format(v.colour, plain), v.colour);
        let lin = for_format(v.colour, srgb);
        assert_eq!((lin[0], lin[2], lin[3]), (1.0, 0.0, 1.0));
        assert!((lin[1] - 0.2158605).abs() < 1e-6);
    }
}
