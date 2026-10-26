// SPDX-License-Identifier: MPL-2.0
//! Authored images/particle frames only. No UE implementation or game assets.
use crysirror_core::state::ClientId;
use crysirror_runtime::assets::Assets;
use crysirror_vfx::{
    BlendMode, EmitterFrame, FacingMode, FrameSnapshot, MeshShape, Particle, RenderSpec,
    SimulationSpace,
};
use crysirror_wgpu::{
    vfx_gpu::GpuParticle,
    vfx_render::{GpuEmitterDraw, VfxRenderer, VfxView},
    Gpu,
};
use glam::{Mat4, Vec3};
use std::{
    path::PathBuf,
    sync::{mpsc, Arc},
    time::Duration,
};
use wgpu::util::DeviceExt;

#[test]
fn vfx_render_and_gpu_sort_sources_are_valid_wgsl() {
    for (name, text) in [
        ("render", include_str!("../src/shaders/vfx_render.wgsl")),
        ("sort", include_str!("../src/shaders/vfx_sort.wgsl")),
        ("cull", include_str!("../src/shaders/vfx_cull.wgsl")),
    ] {
        let module = naga::front::wgsl::parse_str(text)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(text)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    }
}
fn gpu() -> Arc<Gpu> {
    Arc::new(
        pollster::block_on(Gpu::request(&wgpu::Instance::default(), None))
            .expect("explicit VFX GPU test needs an adapter"),
    )
}
fn frame_view() -> VfxView {
    let camera = Vec3::new(0., 0., 3.);
    let view = Mat4::look_at_rh(camera, Vec3::ZERO, Vec3::Y);
    VfxView {
        view_projection: Mat4::orthographic_rh(-1., 1., -1., 1., 0.1, 10.) * view,
        view,
        camera,
        viewport: [128, 128],
        ..Default::default()
    }
}
fn particle(id: u64, position: [f32; 3], color: [f32; 4]) -> Particle {
    Particle {
        id,
        position,
        previous_position: position,
        velocity: [0.; 3],
        age: 0.75,
        lifetime: 1.,
        color,
        size: [0.5; 2],
        rotation: 0.,
        ribbon_id: 0,
    }
}
fn sprite(
    texture: Option<String>,
    columns: u32,
    frames: u32,
    soft_distance: f32,
    blend: BlendMode,
) -> RenderSpec {
    RenderSpec::Sprite {
        texture,
        columns,
        rows: 1,
        frames,
        fps: 0.,
        soft_distance,
        blend,
        facing: FacingMode::Camera,
    }
}
fn emitter(id: u64, render: RenderSpec, particles: Vec<Particle>) -> EmitterFrame {
    EmitterFrame {
        system_key: 1,
        key: id,
        client: 1,
        emitter_id: id.to_string(),
        transform: Mat4::IDENTITY.to_cols_array_2d(),
        space: SimulationSpace::World,
        render,
        particles: Arc::from(particles),
    }
}
fn texture(
    g: &Gpu,
    label: &str,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    g.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: 128,
            height: 128,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
fn depth_clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, value: f32) {
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VFX test opaque depth"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(value),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}
fn read(g: &Gpu, mut encoder: wgpu::CommandEncoder, target: &wgpu::Texture) -> image::RgbaImage {
    let buffer = g.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VFX explicit test readback"),
        size: 128 * 128 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(128 * 4),
                rows_per_image: Some(128),
            },
        },
        wgpu::Extent3d {
            width: 128,
            height: 128,
            depth_or_array_layers: 1,
        },
    );
    let submission = g.queue.submit(Some(encoder.finish()));
    let (tx, rx) = mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    g.device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(20)),
        })
        .expect("bounded GPU test completion");
    rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
    let pixels = buffer.slice(..).get_mapped_range().to_vec();
    buffer.unmap();
    image::RgbaImage::from_raw(128, 128, pixels).unwrap()
}
fn draw(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &VfxRenderer,
    target: &wgpu::TextureView,
    depth: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VFX test draw"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    renderer.draw(&mut pass);
}
fn artifacts() -> PathBuf {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/verification/vfx-render");
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
#[ignore = "requires a real GPU; actual texture/mesh/ribbon pixels"]
fn texture_flipbook_mesh_and_ribbon_are_distinct_renderers() {
    let g = gpu();
    let temp = tempfile::tempdir().unwrap();
    let mut image = image::RgbaImage::new(8, 4);
    for (x, _, p) in image.enumerate_pixels_mut() {
        *p = image::Rgba(if x < 4 {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 255]
        });
    }
    image.save(temp.path().join("sheet.png")).unwrap();
    let mut assets = Assets::new(temp.path()).unwrap();
    assets.register(ClientId(1), "").unwrap();
    let frame = FrameSnapshot {
        emitters: vec![
            emitter(
                1,
                sprite(Some("sheet.png".into()), 2, 2, 0., BlendMode::Alpha),
                vec![particle(1, [-0.5, 0.3, 0.], [1.; 4])],
            ),
            emitter(
                2,
                RenderSpec::Mesh {
                    mesh: MeshShape::Cube,
                    texture: None,
                    blend: BlendMode::Alpha,
                },
                vec![particle(2, [0.5, 0.3, 0.], [1., 0.2, 0., 1.])],
            ),
            emitter(
                3,
                RenderSpec::Ribbon {
                    width: 0.14,
                    texture: None,
                    uv_scale: 1.,
                    max_segments: 16,
                    blend: BlendMode::Alpha,
                },
                vec![
                    particle(3, [-0.65, -0.5, 0.], [0., 0., 1., 1.]),
                    particle(4, [0.65, -0.5, 0.], [0., 0., 1., 1.]),
                ],
            ),
        ],
        ..Default::default()
    };
    let target = texture(
        &g,
        "VFX color",
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        &g,
        "VFX output depth",
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let opaque = texture(
        &g,
        "VFX scene depth",
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let mut renderer = VfxRenderer::new(&g.device, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap();
    renderer.begin_frame(&frame_view(), 1).unwrap();
    renderer
        .prepare_cpu(
            &g,
            &frame,
            &mut assets,
            &opaque.create_view(&Default::default()),
        )
        .unwrap();
    renderer.finish_frame();
    assert_eq!(renderer.stats().sprite_batches, 1);
    assert_eq!(renderer.stats().mesh_batches, 1);
    assert_eq!(renderer.stats().ribbon_segments, 1);
    let mut encoder = g.device.create_command_encoder(&Default::default());
    depth_clear(&mut encoder, &opaque.create_view(&Default::default()), 1.);
    draw(
        &mut encoder,
        &renderer,
        &target.create_view(&Default::default()),
        &depth.create_view(&Default::default()),
    );
    let image = read(&g, encoder, &target);
    image
        .save(artifacts().join("sprite-mesh-ribbon.png"))
        .unwrap();
    let green = image
        .pixels()
        .filter(|p| p[1] > 180 && p[0] < 60 && p[2] < 60)
        .count();
    let orange = image
        .pixels()
        .filter(|p| p[0] > 180 && p[1] > 20 && p[1] < 100 && p[2] < 30)
        .count();
    let blue = image
        .pixels()
        .filter(|p| p[2] > 180 && p[0] < 30 && p[1] < 30)
        .count();
    assert!(green>300&&orange>300&&blue>400,"real flipbook, mesh and continuous ribbon pixels: green={green} orange={orange} blue={blue}");
}

#[test]
#[ignore = "requires a real GPU; verifies depth-intersection fade"]
fn soft_particle_uses_prepass_depth_in_world_units() {
    let g = gpu();
    let temp = tempfile::tempdir().unwrap();
    let mut assets = Assets::new(temp.path()).unwrap();
    assets.register(ClientId(1), "").unwrap();
    let view = frame_view();
    let world_depth = view.view_projection * Vec3::ZERO.extend(1.);
    let depth_value = world_depth.z / world_depth.w;
    let mut values = vec![];
    for soft in [0., 0.5] {
        let frame = FrameSnapshot {
            emitters: vec![emitter(
                1,
                sprite(None, 1, 1, soft, BlendMode::Alpha),
                vec![particle(1, [0., 0., 0.02], [1., 0., 0., 1.])],
            )],
            ..Default::default()
        };
        let target = texture(
            &g,
            "soft particle color",
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let output_depth = texture(
            &g,
            "soft particle render depth",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let pre = texture(
            &g,
            "soft particle predepth",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let mut renderer = VfxRenderer::new(&g.device, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap();
        renderer.begin_frame(&view, 1).unwrap();
        renderer
            .prepare_cpu(
                &g,
                &frame,
                &mut assets,
                &pre.create_view(&Default::default()),
            )
            .unwrap();
        renderer.finish_frame();
        let mut encoder = g.device.create_command_encoder(&Default::default());
        depth_clear(
            &mut encoder,
            &pre.create_view(&Default::default()),
            depth_value,
        );
        draw(
            &mut encoder,
            &renderer,
            &target.create_view(&Default::default()),
            &output_depth.create_view(&Default::default()),
        );
        let image = read(&g, encoder, &target);
        image
            .save(artifacts().join(format!("soft-{soft}.png")))
            .unwrap();
        values.push(image.get_pixel(64, 64)[0]);
    }
    assert!(
        values[0] > 220 && values[1] < values[0] / 4,
        "intersection fade must affect real coverage: {values:?}"
    );
}

#[test]
#[ignore = "requires real GPU compute/indirect rendering; no CPU live-count readback"]
fn gpu_resident_alpha_particles_sort_per_view_before_indirect_draw() {
    let g = gpu();
    let temp = tempfile::tempdir().unwrap();
    let mut assets = Assets::new(temp.path()).unwrap();
    assets.register(ClientId(1), "").unwrap();
    let p = |z: f32, color: [f32; 4], id: u32| GpuParticle {
        position_age: [0., 0., z, 0.25],
        velocity_lifetime: [0., 0., 0., 1.],
        previous_position_rotation: [0., 0., z, 0.],
        color,
        size: [0.8, 0.8, 0., 0.],
        metadata: [id, 0, 0, 1],
    };
    // Near blue intentionally precedes far red in source order.
    let particles = g
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("resident test particle state"),
            contents: bytemuck::cast_slice(&[
                p(0.2, [0., 0., 1., 0.5], 1),
                p(0., [1., 0., 0., 0.5], 2),
            ]),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let indices = g
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("resident test alive list"),
            contents: bytemuck::cast_slice(&[0u32, 1]),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let indirect = g
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("resident test draw args"),
            contents: bytemuck::cast_slice(&[6u32, 2, 0, 0]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT,
        });
    let metadata = emitter(1, sprite(None, 1, 1, 0., BlendMode::Alpha), vec![]);
    let input = GpuEmitterDraw {
        emitter: &metadata,
        particles: &particles,
        indices: &indices,
        indirect: &indirect,
        capacity: 2,
    };
    let target = texture(
        &g,
        "GPU alpha color",
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(
        &g,
        "GPU alpha output depth",
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let pre = texture(
        &g,
        "GPU alpha predepth",
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let mut renderer = VfxRenderer::new(&g.device, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap();
    renderer.begin_frame(&frame_view(), 1).unwrap();
    let mut encoder = g.device.create_command_encoder(&Default::default());
    depth_clear(&mut encoder, &pre.create_view(&Default::default()), 1.);
    renderer
        .prepare_gpu(
            &g,
            &mut encoder,
            &[input],
            &mut assets,
            &pre.create_view(&Default::default()),
        )
        .unwrap();
    renderer.finish_frame();
    assert_eq!(renderer.stats().gpu_live_particles, None);
    assert_eq!(renderer.stats().gpu_sorted_emitters, 1);
    draw(
        &mut encoder,
        &renderer,
        &target.create_view(&Default::default()),
        &depth.create_view(&Default::default()),
    );
    let image = read(&g, encoder, &target);
    image
        .save(artifacts().join("gpu-alpha-sorted.png"))
        .unwrap();
    let center = image.get_pixel(64, 64).0;
    assert!(
        center[2] > 110 && u32::from(center[2]) > u32::from(center[0]) * 17 / 10,
        "near blue must composite over far red after GPU sorting: {center:?}"
    );
}
