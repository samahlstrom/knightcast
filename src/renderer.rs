use crate::{
    assets::{Assets, Part, Vertex},
    config::Config,
    tracking::Pose,
};

use anyhow::{Context, Result};

use glam::{Mat4, Quat, Vec3};

use wgpu::util::DeviceExt;

struct Draw {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    count: u32,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    part: Part,
    double_sided: bool,
    center: Vec3,
    y_bounds: [f32; 2],
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    depth: wgpu::TextureView,
    multisample: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    single: wgpu::RenderPipeline,
    draws: Vec<Draw>,
    pub width: u32,
    pub height: u32,
    pub backend: String,
    pub report: Vec<String>,
}

impl Renderer {
    pub async fn headless(assets: Assets, width: u32, height: u32) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .context("no headless GPU adapter")?;

        let info = adapter.get_info();

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await?;

        Ok(Self::new(
            device,
            queue,
            assets,
            width,
            height,
            format!("{} / {:?}", info.name, info.backend),
        ))
    }

    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        assets: Assets,
        width: u32,
        height: u32,
        backend: String,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("KnightCast offscreen output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
        });

        let view = texture.create_view(&Default::default());

        let multisample = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("four sample artwork target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 4,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());

        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 4,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("unlit art"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let make_pipeline = |cull_mode| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("unlit rigid art"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: 4,
                    ..Default::default()
                },
                multiview: None,
                cache: None,
            })
        };

        let pipeline = make_pipeline(None);

        let single = make_pipeline(Some(wgpu::Face::Back));

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            ..Default::default()
        });

        let draws = assets
            .meshes
            .into_iter()
            .map(|mesh| {
                let vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                let index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });

                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&Mat4::IDENTITY.to_cols_array()),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });

                let tex = device.create_texture_with_data(
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: None,
                        size: wgpu::Extent3d {
                            width: mesh.texture.width(),
                            height: mesh.texture.height(),
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    mesh.texture.as_raw(),
                );

                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(
                                &tex.create_view(&Default::default()),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });

                Draw {
                    vertex,
                    index,
                    count: mesh.indices.len() as u32,
                    uniform,
                    bind,
                    part: mesh.part,
                    double_sided: mesh.double_sided,
                    center: mesh.center,
                    y_bounds: mesh
                        .vertices
                        .iter()
                        .fold([f32::INFINITY, f32::NEG_INFINITY], |b, v| {
                            [b[0].min(v.position[1]), b[1].max(v.position[1])]
                        }),
                }
            })
            .collect();

        Self {
            device,
            queue,
            texture,
            view,
            depth,
            multisample,
            pipeline,
            single,
            draws,
            width,
            height,
            backend,
            report: assets.report,
        }
    }

    /// egui's image shader consumes gamma-encoded color values. The scene is
    /// already encoded by its sRGB render attachment, so sample those bytes
    /// through a non-sRGB view rather than decoding them a second time.
    pub fn preview_view(&self) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("egui gamma-encoded preview"),
            format: Some(wgpu::TextureFormat::Rgba8Unorm),
            ..Default::default()
        })
    }

    pub fn render(&self, c: &Config, p: &Pose) {
        let camera = Mat4::orthographic_rh(
            0.23 - 6.27 / c.zoom,
            0.23 + 6.27 / c.zoom,
            0.5 - 6.5 / c.zoom,
            0.5 + 6.5 / c.zoom,
            0.1,
            100.,
        ) * Mat4::look_at_rh(Vec3::new(0., 0., 20.), Vec3::ZERO, Vec3::Y);

        let (yaw, pitch, roll) = p.current.to_euler(glam::EulerRot::YXZ);

        let q = Quat::from_euler(
            glam::EulerRot::YXZ,
            yaw.clamp(-45f32.to_radians(), 45f32.to_radians()),
            pitch.clamp(-25f32.to_radians(), 25f32.to_radians()),
            roll.clamp(-24f32.to_radians(), 24f32.to_radians()),
        );

        let base = Quat::from_euler(
            glam::EulerRot::YXZ,
            c.base_degrees[0].to_radians(),
            c.base_degrees[1].to_radians(),
            c.base_degrees[2].to_radians(),
        );

        let pivot = Vec3::from_array(c.pivot);

        let root = Mat4::from_translation(Vec3::new(c.offset[0], c.offset[1], 0.))
            * Mat4::from_scale(Vec3::splat(c.model_scale));

        let head = root
            * Mat4::from_translation(pivot + Vec3::new(0., 0.25, 0.))
            * Mat4::from_quat(base * q)
            * Mat4::from_translation(-pivot);

        for draw in &self.draws {
            let model = match draw.part {
                Part::Body => Mat4::from_scale(Vec3::splat(c.body_scale)),
                Part::Neck => {
                    root * Mat4::from_translation(Vec3::new(0., -0.25, -0.66))
                        * Mat4::from_scale(Vec3::new(1., 2., 1.))
                }

                Part::Jaw => head * Mat4::from_translation(Vec3::new(0., -0.54 * p.mouth, 0.)),
                Part::EyeLeft | Part::EyeRight => {
                    let blink = p.blink[usize::from(draw.part == Part::EyeRight)];

                    head * Mat4::from_translation(draw.center)
                        * Mat4::from_scale(Vec3::new(1., (1. - blink).max(0.03), 1.))
                        * Mat4::from_translation(-draw.center)
                }

                Part::Interior => {
                    let stretch = 0.54 * p.mouth / (draw.y_bounds[1] - draw.y_bounds[0]).max(0.001);
                    head * Mat4::from_translation(Vec3::new(0., -draw.y_bounds[1] * stretch, 0.))
                        * Mat4::from_scale(Vec3::new(1., 1. + stretch, 1.))
                }
                Part::Head => head,
            };

            self.queue.write_buffer(
                &draw.uniform,
                0,
                bytemuck::cast_slice(&(camera * model).to_cols_array()),
            );
        }

        let mut encoder = self.device.create_command_encoder(&Default::default());

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("KnightCast native render"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.multisample,
                    resolve_target: Some(&self.view),
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: (c.background[0] * c.background[3]) as f64,
                            g: (c.background[1] * c.background[3]) as f64,
                            b: (c.background[2] * c.background[3]) as f64,
                            a: c.background[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            for draw in &self.draws {
                pass.set_pipeline(if draw.double_sided {
                    &self.pipeline
                } else {
                    &self.single
                });

                pass.set_bind_group(0, &draw.bind, &[]);

                pass.set_vertex_buffer(0, draw.vertex.slice(..));

                pass.set_index_buffer(draw.index.slice(..), wgpu::IndexFormat::Uint32);

                pass.draw_indexed(0..draw.count, 0, 0..1);
            }
        }

        self.queue.submit([encoder.finish()]);
    }

    pub fn screenshot(&self, path: &std::path::Path) -> Result<()> {
        let row = (self.width * 4).div_ceil(256) * 256;

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * self.height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self.device.create_command_encoder(&Default::default());

        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit([encoder.finish()]);

        let (tx, rx) = std::sync::mpsc::channel();

        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });

        self.device.poll(wgpu::PollType::wait_indefinitely())?;

        rx.recv()??;

        let mapped = buffer.slice(..).get_mapped_range();

        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);

        for line in mapped.chunks(row as usize) {
            rgba.extend_from_slice(&line[..self.width as usize * 4]);
        }

        // GPU alpha blending stores premultiplied linear RGB; PNG requires straight alpha.
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let alpha = pixel[3] as f32 / 255.;
            if alpha == 0. {
                pixel[..3].fill(0);
            } else if alpha < 1. {
                for channel in &mut pixel[..3] {
                    let encoded = *channel as f32 / 255.;
                    let linear = if encoded <= 0.04045 {
                        encoded / 12.92
                    } else {
                        ((encoded + 0.055) / 1.055).powf(2.4)
                    };
                    let straight = (linear / alpha).clamp(0., 1.);
                    let encoded = if straight <= 0.0031308 {
                        straight * 12.92
                    } else {
                        1.055 * straight.powf(1. / 2.4) - 0.055
                    };
                    *channel = (encoded * 255.).round() as u8;
                }
            }
        }
        image::save_buffer(
            path,
            &rgba,
            self.width,
            self.height,
            image::ColorType::Rgba8,
        )?;

        drop(mapped);

        buffer.unmap();

        Ok(())
    }
}
