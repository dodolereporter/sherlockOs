//! Draws the mascot with wgpu, in two passes.
//!
//! 1. The scene: the skinned mesh in a texture of its own, multisampled ×4 with a depth
//!    buffer, on a transparent background, then resolved.
//! 2. The composite: the resolved texture laid over the target in premultiplied alpha, cut
//!    round (the bubble) or not, over an optional background, with a soft drop shadow of the
//!    mascot under him.
//!
//! iced draws a custom primitive into its own single-sampled pass without depth, hence the
//! scene of its own. The same code serves iced (`widget`, lot D-30b) and the off-screen
//! captures (`capture`).

use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::model::{Alpha, Model, Vertex};

/// Samples per pixel of the scene.
pub const SAMPLES: u32 = 4;
/// Joints the shader holds; the model has 30.
const MAX_JOINTS: usize = 32;
const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// The drop shadow: offset to the right and down, then blur radius, as shares of the smaller
/// side of the frame; then opacity. The light of the model's reference renders comes from the
/// top left.
const SHADOW_OFFSET: [f32; 2] = [0.025, 0.045];
const SHADOW_BLUR: f32 = 0.05;
const SHADOW_OPACITY: f32 = 0.28;

/// Where the camera stands. It looks at `target` along -Z (the mascot faces +Z), from a bit
/// above when `pitch` is positive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    /// Vertical field of view, in radians.
    pub fov_y: f32,
    /// Angle above the horizontal, in radians.
    pub pitch: f32,
}

impl Camera {
    /// The top of the body: head, cap and scarf, legs out of the frame, with a margin between
    /// the pompom and the top of the frame. Made for a round bubble.
    pub const UPPER_BODY: Self = Self {
        target: Vec3::new(0.0, 2.15, 0.0),
        distance: 10.4,
        fov_y: 0.30,
        pitch: 0.06,
    };

    /// The whole mascot.
    pub const FULL_BODY: Self = Self {
        target: Vec3::new(0.0, 1.5, 0.0),
        distance: 11.0,
        fov_y: 0.32,
        pitch: 0.05,
    };

    /// Projection and view for a frame `aspect` times wider than high.
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        let direction = Vec3::new(0.0, self.pitch.sin(), self.pitch.cos());
        let eye = self.target + direction * self.distance;
        let view = Mat4::look_at_rh(eye, self.target, Vec3::Y);
        let projection = Mat4::perspective_rh(self.fov_y, aspect.max(0.01), 0.1, 100.0);
        projection * view
    }

    /// The point of the plane `z = depth` (in the space of the model) seen at `ndc`, from -1 to
    /// 1, x to the right and y up: where a point of the screen lies in front of the mascot.
    pub fn unproject(&self, ndc: glam::Vec2, aspect: f32, depth: f32) -> Vec3 {
        let inverse = self.view_projection(aspect).inverse();
        let near = inverse.project_point3(ndc.extend(0.0));
        let far = inverse.project_point3(ndc.extend(1.0));
        let ray = far - near;
        if ray.z.abs() < 1e-6 {
            return near;
        }
        near + ray * ((depth - near.z) / ray.z)
    }
}

/// The shape the mascot is cut to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cut {
    /// The whole rectangle.
    None,
    /// A disc centered in the frame. `radius` is a share of half the smaller side: 1 touches
    /// the edges.
    Circle { radius: f32 },
}

/// What to draw in one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame<'a> {
    /// Size in physical pixels.
    pub size: [u32; 2],
    /// Skinning matrices (`Pose::joint_matrices`).
    pub joints: &'a [Mat4],
    pub camera: Camera,
    pub cut: Cut,
    /// Drawn behind the mascot, inside the cut: straight sRGB components and opacity.
    /// Transparent for none.
    pub background: [f32; 4],
}

/// Why a capture failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the GPU did not finish: {0}")]
    Poll(#[from] wgpu::PollError),
    #[error("cannot read the capture back: {0}")]
    Map(#[from] wgpu::BufferAsyncError),
    #[error("the capture was never mapped")]
    NotMapped,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_projection: [f32; 16],
    joints: [[f32; 16]; MAX_JOINTS],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialUniform {
    base: [f32; 4],
    alpha_cutoff: f32,
    padding: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    origin: [f32; 2],
    center: [f32; 2],
    radius: f32,
    encode_srgb: f32,
    padding: [f32; 2],
    background: [f32; 4],
    shadow_offset: [f32; 2],
    shadow_blur: f32,
    shadow_opacity: f32,
}

struct Draw {
    first_index: u32,
    index_count: u32,
    material: usize,
}

struct Targets {
    size: [u32; 2],
    multisampled: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// The GPU side of the mascot, for one target format.
pub struct Renderer {
    opaque: wgpu::RenderPipeline,
    blended: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    materials: Vec<(wgpu::BindGroup, bool)>,
    /// Opaque parts first, then the blended ones.
    draws: Vec<Draw>,
    params: wgpu::Buffer,
    composite_layout: wgpu::BindGroupLayout,
    /// Linear filtering, for the taps of the drop shadow.
    composite_sampler: wgpu::Sampler,
    targets: Option<Targets>,
    encode_srgb: bool,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("draws", &self.draws.len())
            .field("size", &self.targets.as_ref().map(|t| t.size))
            .finish_non_exhaustive()
    }
}

impl Renderer {
    /// Sends `model` to the GPU, for a target of `format`.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &Model,
        format: wgpu::TextureFormat,
    ) -> Self {
        let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sherlock-3d globals"),
            entries: &[uniform(0, wgpu::ShaderStages::VERTEX)],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sherlock-3d material"),
            entries: &[
                uniform(0, wgpu::ShaderStages::FRAGMENT),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sherlock-3d composite"),
            entries: &[
                uniform(0, wgpu::ShaderStages::FRAGMENT),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let scene = device.create_shader_module(wgpu::include_wgsl!("scene.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sherlock-3d scene"),
            bind_group_layouts: &[&globals_layout, &material_layout],
            push_constant_ranges: &[],
        });
        let scene_pipeline = |blend: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sherlock-3d scene"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &scene,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32x2,
                            2 => Float32x4,
                            3 => Uint16x4,
                            4 => Float32x4,
                        ],
                    }],
                },
                // The materials are double-sided: nothing is culled.
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: !blend,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &scene,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: SCENE_FORMAT,
                        blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            })
        };
        let opaque = scene_pipeline(false);
        let blended = scene_pipeline(true);

        let composite_shader = device.create_shader_module(wgpu::include_wgsl!("composite.wgsl"));
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("sherlock-3d composite"),
                bind_group_layouts: &[&composite_layout],
                push_constant_ranges: &[],
            });
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sherlock-3d composite"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sherlock-3d vertices"),
            contents: bytemuck::cast_slice(&model.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sherlock-3d indices"),
            contents: bytemuck::cast_slice(&model.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sherlock-3d globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sherlock-3d globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sherlock-3d texture"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let white = upload_texture(
            device,
            queue,
            &[image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4]))],
        );
        let textures: Vec<wgpu::TextureView> = model
            .textures
            .iter()
            .map(|t| upload_texture(device, queue, &t.levels))
            .collect();
        let materials = model
            .materials
            .iter()
            .map(|material| {
                let alpha_cutoff = match material.alpha {
                    Alpha::Mask(cutoff) => cutoff,
                    Alpha::Opaque | Alpha::Blend => 0.0,
                };
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("sherlock-3d material"),
                    contents: bytemuck::bytes_of(&MaterialUniform {
                        base: material.base_color,
                        alpha_cutoff,
                        padding: [0.0; 3],
                    }),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let view = material
                    .texture
                    .and_then(|t| textures.get(t))
                    .unwrap_or(&white);
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sherlock-3d material"),
                    layout: &material_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });
                (group, material.alpha == Alpha::Blend)
            })
            .collect::<Vec<_>>();

        let mut draws: Vec<Draw> = model
            .primitives
            .iter()
            .filter(|p| p.material < materials.len())
            .map(|p| Draw {
                first_index: p.first_index,
                index_count: p.index_count,
                material: p.material,
            })
            .collect();
        draws.sort_by_key(|d| materials[d.material].1);

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sherlock-3d composite"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sherlock-3d shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            opaque,
            blended,
            composite,
            vertices,
            indices,
            globals,
            globals_group,
            materials,
            draws,
            params,
            composite_layout,
            composite_sampler,
            targets: None,
            encode_srgb: !format.is_srgb(),
        }
    }

    /// Uploads what `frame` needs. `origin` is the top left corner of the area in the target,
    /// in physical pixels.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame<'_>,
        origin: [f32; 2],
    ) {
        let size = [frame.size[0].max(1), frame.size[1].max(1)];
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = Some(self.targets(device, size));
        }
        let mut globals = Globals {
            view_projection: frame
                .camera
                .view_projection(size[0] as f32 / size[1] as f32)
                .to_cols_array(),
            joints: [Mat4::IDENTITY.to_cols_array(); MAX_JOINTS],
        };
        for (slot, joint) in globals.joints.iter_mut().zip(frame.joints) {
            *slot = joint.to_cols_array();
        }
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let (w, h) = (size[0] as f32, size[1] as f32);
        let radius = match frame.cut {
            Cut::None => -1.0,
            Cut::Circle { radius } => radius.max(0.0) * w.min(h) / 2.0,
        };
        let params = Params {
            origin,
            center: [w / 2.0, h / 2.0],
            radius,
            encode_srgb: if self.encode_srgb { 1.0 } else { 0.0 },
            padding: [0.0; 2],
            background: linear_premultiplied(frame.background),
            shadow_offset: SHADOW_OFFSET.map(|share| share * w.min(h)),
            shadow_blur: SHADOW_BLUR * w.min(h),
            shadow_opacity: SHADOW_OPACITY,
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }

    /// Draws the mascot in its own texture. Call after `prepare`.
    pub fn draw_scene(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(targets) = &self.targets else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sherlock-3d scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &targets.multisampled,
                depth_slice: None,
                resolve_target: Some(&targets.resolved),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &self.globals_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        let mut blending = false;
        pass.set_pipeline(&self.opaque);
        for draw in &self.draws {
            let (group, blend) = &self.materials[draw.material];
            if *blend && !blending {
                pass.set_pipeline(&self.blended);
                blending = true;
            }
            pass.set_bind_group(1, group, &[]);
            pass.draw_indexed(
                draw.first_index..draw.first_index + draw.index_count,
                0,
                0..1,
            );
        }
    }

    /// Lays the scene over the target of `pass`, at the `origin` given to `prepare`. It covers
    /// the whole target: the caller limits it with a scissor rectangle.
    pub fn composite(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(targets) = &self.targets else {
            return;
        };
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &targets.group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Renders `frame` alone in a new texture over `clear` (straight sRGB and opacity), and
    /// reads it back as straight sRGB. The target is `Rgba8Unorm` holding sRGB values, as iced
    /// draws: the renderer must have been made for that format.
    pub fn capture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame<'_>,
        clear: [f32; 4],
    ) -> Result<image::RgbaImage, Error> {
        let [width, height] = [frame.size[0].max(1), frame.size[1].max(1)];
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sherlock-3d capture"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        self.prepare(device, queue, frame, [0.0, 0.0]);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sherlock-3d capture"),
        });
        self.draw_scene(&mut encoder);
        {
            let [r, g, b, a] = clear.map(f64::from);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sherlock-3d capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: r * a,
                            g: g * a,
                            b: b * a,
                            a,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.composite(&mut pass);
        }
        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sherlock-3d capture"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            extent,
        );
        queue.submit([encoder.finish()]);

        let (sender, receiver) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.recv().map_err(|_| Error::NotMapped)??;

        let data = readback.slice(..).get_mapped_range();
        let mut image = image::RgbaImage::new(width, height);
        for (y, line) in data.chunks(row as usize).take(height as usize).enumerate() {
            for (x, pixel) in line.chunks(4).take(width as usize).enumerate() {
                let a = pixel[3];
                let straight = |c: u8| {
                    if a == 0 {
                        0
                    } else {
                        ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
                    }
                };
                image.put_pixel(
                    x as u32,
                    y as u32,
                    image::Rgba([
                        straight(pixel[0]),
                        straight(pixel[1]),
                        straight(pixel[2]),
                        a,
                    ]),
                );
            }
        }
        drop(data);
        readback.unmap();
        Ok(image)
    }

    fn targets(&self, device: &wgpu::Device, size: [u32; 2]) -> Targets {
        let texture = |label, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let multisampled = texture("sherlock-3d samples", SCENE_FORMAT, SAMPLES, attachment);
        let depth = texture("sherlock-3d depth", DEPTH_FORMAT, SAMPLES, attachment);
        let resolved = texture(
            "sherlock-3d scene",
            SCENE_FORMAT,
            1,
            attachment | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sherlock-3d composite"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&resolved),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.composite_sampler),
                },
            ],
        });
        Targets {
            size,
            multisampled,
            depth,
            resolved,
            group,
        }
    }
}

/// An sRGB texture with every level of `levels`.
fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    levels: &[image::RgbaImage],
) -> wgpu::TextureView {
    let (width, height) = levels.first().map_or((1, 1), |l| l.dimensions());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sherlock-3d texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len().max(1) as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, image) in levels.iter().enumerate() {
        let (w, h) = image.dimensions();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Straight sRGB and opacity to linear premultiplied.
fn linear_premultiplied([r, g, b, a]: [f32; 4]) -> [f32; 4] {
    let decode = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [decode(r) * a, decode(g) * a, decode(b) * a, a]
}
