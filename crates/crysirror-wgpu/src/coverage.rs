//! Per-client transient ordered coverage planes. The renderer owns GPU storage;
//! logical commands are retained by the shared Layer service and replayed for
//! live presentation, immutable captures and nested snapshots alike.
use super::*;
pub(super) struct Plane {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub bind: wgpu::BindGroup,
}
impl Renderer {
    pub(super) fn ensure_coverage(&mut self, owners: &BTreeSet<u32>, size: [u32; 2]) -> Result<()> {
        ensure!(
            u64::from(size[0]) * u64::from(size[1]) * 4 * (owners.len() as u64)
                <= 256 * 1024 * 1024,
            "ordered coverage GPU byte budget"
        );
        self.coverage_planes
            .retain(|owner, _| owners.contains(owner));
        for owner in owners {
            if self
                .coverage_planes
                .get(owner)
                .is_some_and(|p| p.texture.width() == size[0] && p.texture.height() == size[1])
            {
                continue;
            }
            let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("ordered client coverage"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bind = self
                .gpu
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("client coverage binding"),
                    layout: &self.draw_pipelines.coverage_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    }],
                });
            self.coverage_planes.insert(
                *owner,
                Plane {
                    texture,
                    view,
                    bind,
                },
            );
        }
        Ok(())
    }
}
pub(super) fn configure(record: &mut FilterGpu, active: bool) {
    if active {
        record.c[3] = (record.c[3] as u32 | 4096) as f32;
    }
}
