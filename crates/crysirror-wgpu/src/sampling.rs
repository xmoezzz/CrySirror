//! Per-image-layer sampler aliases. An alias owns a bind group and cheap GPU
//! texture/view clones; pixel storage and its accounting remain on the base key.
use super::{Renderer, Texture};
use anyhow::{ensure, Context, Result};
use crysirror_runtime::assets::Assets;
use crysirror_scene::layer::{ImageSampling, TextureAddressMode};

fn address(mode: TextureAddressMode) -> wgpu::AddressMode {
    match mode {
        TextureAddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        TextureAddressMode::Repeat => wgpu::AddressMode::Repeat,
        TextureAddressMode::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
    }
}
fn mode_key(mode: TextureAddressMode) -> char {
    match mode {
        TextureAddressMode::ClampToEdge => 'c',
        TextureAddressMode::Repeat => 'r',
        TextureAddressMode::MirrorRepeat => 'm',
    }
}
impl Renderer {
    /// Preserve the ordinary revision-bearing key for the default sampler.
    pub(super) fn sampled_image_key(
        assets: &Assets,
        owner: u32,
        asset: &str,
        sampling: ImageSampling,
    ) -> Result<String> {
        let base = Self::image_key(assets, owner, asset)?;
        Ok(if sampling == ImageSampling::default() {
            base
        } else {
            format!(
                "sampled:{}{}:{base}",
                mode_key(sampling.address_u),
                mode_key(sampling.address_v)
            )
        })
    }
    /// Prepare pixels through the ordinary engine asset cache, then create an
    /// independent sampler bind. Caller retains both alias and base used keys.
    pub(super) fn ensure_sampled_image(
        &mut self,
        assets: &mut Assets,
        owner: u32,
        asset: &str,
        key: &str,
        sampling: ImageSampling,
    ) -> Result<bool> {
        ensure!(
            key == Self::sampled_image_key(assets, owner, asset, sampling)?,
            "image sampler cache key differs from its asset/revision/state"
        );
        if sampling == ImageSampling::default() {
            return self.ensure_image(assets, owner, asset, key);
        }
        if self.textures.contains_key(key) {
            return Ok(true);
        }
        let base = Self::image_key(assets, owner, asset)?;
        if !self.ensure_image(assets, owner, asset, &base)? {
            return Ok(false);
        }
        let texture = self
            .textures
            .get(&base)
            .context("sampled image base vanished")?;
        let gpu_texture = texture.texture.clone();
        let view = texture.view.clone();
        let sampler = self.gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("retained image addressing"),
            address_mode_u: address(sampling.address_u),
            address_mode_v: address(sampling.address_v),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("retained image sampler alias"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
        self.textures.insert(
            key.into(),
            Texture {
                bind,
                view,
                texture: gpu_texture,
                bytes: 0,
            },
        );
        Ok(true)
    }
}
