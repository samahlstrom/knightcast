//! Exercise the actual egui GPU shader, not just the scene's PNG output.
use crate::{assets::Assets, config::Config, renderer::Renderer, tracking::Pose};
use eframe::{egui, egui_wgpu};

fn pixel(gpu: &Renderer, texture: &wgpu::Texture) -> [u8; 4] {
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 4, y: 4, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(1),
            },
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).unwrap();
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    rx.recv().unwrap().unwrap();
    buffer.slice(..).get_mapped_range()[..4].try_into().unwrap()
}

fn through_egui(gpu: &Renderer, view: &wgpu::TextureView, format: wgpu::TextureFormat) -> [u8; 4] {
    let output = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preview regression output"),
        size: wgpu::Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut ui = egui_wgpu::Renderer::new(
        &gpu.device,
        format,
        egui_wgpu::RendererOptions {
            dithering: false,
            ..Default::default()
        },
    );
    let id = ui.register_native_texture(&gpu.device, view, wgpu::FilterMode::Nearest);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(8., 8.));
    let mut mesh = egui::Mesh::with_texture(id);
    mesh.add_rect_with_uv(
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
        egui::Color32::WHITE,
    );
    let jobs = vec![egui::ClippedPrimitive {
        clip_rect: rect,
        primitive: egui::epaint::Primitive::Mesh(mesh),
    }];
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [8, 8],
        pixels_per_point: 1.,
    };
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let commands = ui.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
    {
        let view = output.create_view(&Default::default());
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        ui.render(&mut pass.forget_lifetime(), &jobs, &screen);
    }
    gpu.queue
        .submit(commands.into_iter().chain([encoder.finish()]));
    pixel(gpu, &output)
}

#[test]
#[ignore = "requires a native GPU; run explicitly without opening a window"]
fn preview_preserves_rendered_srgb_colors() {
    let gpu = pollster::block_on(Renderer::headless(
        Assets {
            meshes: vec![],
            report: vec![],
        },
        8,
        8,
    ))
    .unwrap();
    for background in [
        [0.18, 0.18, 0.18, 1.],
        [0.04, 0.35, 0.12, 1.],
        [0.7, 0.06, 0.02, 1.],
        [0.3, 0.4, 0.55, 1.],
    ] {
        gpu.render(
            &Config {
                background,
                ..Default::default()
            },
            &Pose::default(),
        );
        let expected = pixel(&gpu, &gpu.texture);
        for format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            let actual = through_egui(&gpu, &gpu.preview_view(), format);
            for channel in 0..4 {
                assert!(
                    actual[channel].abs_diff(expected[channel]) <= 1,
                    "{format:?}: {actual:?} differs from scene {expected:?}"
                );
            }
        }
        // Control: using the scene's sRGB view reproduces the old darkening.
        let wrong = through_egui(&gpu, &gpu.view, wgpu::TextureFormat::Rgba8Unorm);
        assert!((0..3).any(|i| wrong[i].abs_diff(expected[i]) > 20));
    }
}
