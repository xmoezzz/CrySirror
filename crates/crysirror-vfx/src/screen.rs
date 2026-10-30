//! CPU particle snapshots projected into ordinary engine mesh layers. This
//! reuses the VFX simulator's immutable frames and PuppetFrame rendering path;
//! it does not simulate particles or render a private composed screen.
use crate::{BlendMode, EmitterFrame, FacingMode, RenderSpec, SimulationSpace};
use anyhow::{ensure, Context, Result};
use crysirror_puppet::{FramePart, PartBlend, PuppetFrame, MAX_PARTS};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

/// Pixel-plane projection of a published CPU sprite emitter.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenSpriteOptions {
    /// Full logical canvas; the owning mesh layer supplies the canvas clip.
    pub canvas: [u32; 2],
    /// Position anchor as a fraction of particle width/height. [0,0] is
    /// top-left, [0.5,0.5] is centered. Rotation uses this same origin.
    pub anchor: [f32; 2],
}
/// Client-owned retained mesh layer for a CPU sprite emitter's publication.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenSpriteTarget {
    /// Full logical pixel canvas.
    pub canvas: [u32; 2],
    /// Unique layer id in the emitter's client namespace, owned until removed.
    pub layer: u32,
    /// Optional existing layer parent in the same client namespace.
    #[serde(default)]
    pub parent: Option<u32>,
    /// Ordinary painter order, independent of world-space VFX draws.
    #[serde(default)]
    pub order: [i32; 2],
    /// Position anchor in fractions of each particle's full size.
    pub anchor: [f32; 2],
}
impl ScreenSpriteTarget {
    /// Validate a declaration before simulation allocations/publication.
    pub fn validate(&self) -> Result<()> {
        ScreenSpriteOptions {
            canvas: self.canvas,
            anchor: self.anchor,
        }
        .validate()?;
        ensure!(
            self.layer != 0 && self.parent != Some(self.layer),
            "VFX screen target layer identity"
        );
        Ok(())
    }
}
impl ScreenSpriteOptions {
    /// Validate bounded logical pixel geometry independently of any VM/device.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.canvas.iter().all(|&n| (1..=16384).contains(&n))
                && self
                    .anchor
                    .iter()
                    .all(|n| n.is_finite() && (0.0..=1.0).contains(n)),
            "VFX screen canvas/anchor"
        );
        Ok(())
    }
}
/// Pack owned CPU particles, in their snapshot order, into immutable quad
/// parts. Positions are logical pixels in the XY plane; Z is orthographically
/// discarded. Local-space particles use the emitter's affine transform.
/// Optional UV rectangles are parallel to particles, allowing arbitrary atlas
/// layouts. Without them the existing sprite grid/fps contract is retained.
/// Camera-facing sprites are supported; other facing and premultiplied inputs
/// reject until they can be represented exactly by the common mesh contract.
pub fn pack_screen_emitter(
    owner: u32,
    frame: &EmitterFrame,
    options: &ScreenSpriteOptions,
    uvs: Option<&[[f32; 4]]>,
) -> Result<PuppetFrame> {
    options.validate()?;
    ensure!(owner == frame.client, "VFX screen emitter owner mismatch");
    frame.render.validate()?;
    let RenderSpec::Sprite {
        texture,
        columns,
        rows,
        frames,
        fps,
        blend,
        facing,
        ..
    } = &frame.render
    else {
        anyhow::bail!("VFX screen projection requires a sprite emitter")
    };
    ensure!(
        matches!(facing, FacingMode::Camera),
        "VFX screen facing profile unsupported"
    );
    let texture = texture
        .as_ref()
        .context("VFX screen texture asset required")?;
    let blend = match blend {
        BlendMode::Alpha => PartBlend::Normal,
        BlendMode::Additive => PartBlend::Add,
        BlendMode::Multiply => PartBlend::Multiply,
        BlendMode::Premultiplied => {
            anyhow::bail!("VFX premultiplied screen sprite profile unsupported")
        }
    };
    ensure!(frame.particles.len() <= MAX_PARTS, "VFX screen part limit");
    if let Some(uvs) = uvs {
        ensure!(
            uvs.len() == frame.particles.len(),
            "VFX screen UV/particle length mismatch"
        );
    }
    let matrix = if frame.space == SimulationSpace::Local {
        Mat4::from_cols_array_2d(&frame.transform)
    } else {
        Mat4::IDENTITY
    };
    ensure!(
        matrix.to_cols_array().iter().all(|n| n.is_finite())
            && matrix.x_axis.w == 0.
            && matrix.y_axis.w == 0.
            && matrix.z_axis.w == 0.
            && matrix.w_axis.w == 1.,
        "VFX screen projection requires finite affine transform"
    );
    let mut output = PuppetFrame {
        size: [options.canvas[0] as f32, options.canvas[1] as f32],
        textures: vec![texture.clone()],
        parts: Vec::with_capacity(frame.particles.len()),
    };
    for (index, particle) in frame.particles.iter().enumerate() {
        ensure!(
            particle
                .position
                .iter()
                .chain(&particle.size)
                .chain(&particle.color)
                .all(|n| n.is_finite())
                && particle.rotation.is_finite()
                && particle.age.is_finite()
                && particle.age >= 0.
                && particle.size.iter().all(|&n| (0.0..=16384.).contains(&n)),
            "VFX screen particle attributes"
        );
        if particle.size.contains(&0.) || particle.color[3] == 0. {
            continue;
        }
        let uv = if let Some(uvs) = uvs {
            uvs[index]
        } else {
            let cell = ((particle.age * *fps) as u64 % u64::from(*frames)) as u32;
            let col = cell % columns;
            let row = cell / columns;
            [
                col as f32 / *columns as f32,
                row as f32 / *rows as f32,
                (col + 1) as f32 / *columns as f32,
                (row + 1) as f32 / *rows as f32,
            ]
        };
        ensure!(
            uv.iter().all(|n| n.is_finite() && (0.0..=1.0).contains(n))
                && uv[0] < uv[2]
                && uv[1] < uv[3],
            "VFX screen source UV rectangle"
        );
        let [width, height] = particle.size;
        let left = -options.anchor[0] * width;
        let top = -options.anchor[1] * height;
        let (sin, cos) = particle.rotation.sin_cos();
        let positions = [
            [left, top],
            [left + width, top],
            [left, top + height],
            [left + width, top + height],
        ]
        .into_iter()
        .map(|[x, y]| {
            let p = matrix.transform_point3(Vec3::new(
                particle.position[0] + cos * x - sin * y,
                particle.position[1] + sin * x + cos * y,
                particle.position[2],
            ));
            [p.x, p.y]
        })
        .collect::<Vec<_>>();
        ensure!(
            positions
                .iter()
                .flatten()
                .all(|p| p.is_finite() && p.abs() <= 1_000_000.),
            "VFX screen projected position budget"
        );
        output.parts.push(FramePart {
            id: format!("vfx/{}/{}/{}", owner, frame.key, particle.id),
            texture: 0,
            positions,
            uvs: vec![
                [uv[0], uv[1]],
                [uv[2], uv[1]],
                [uv[0], uv[3]],
                [uv[2], uv[3]],
            ],
            indices: vec![0, 1, 2, 2, 1, 3],
            colors: vec![particle.color; 4],
            blend,
            masks: vec![],
            invert_mask: false,
            visible: true,
        });
    }
    output.validate()?;
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Particle, RenderSpec};
    use std::sync::Arc;
    fn fixture() -> EmitterFrame {
        EmitterFrame {
            system_key: 1,
            key: 2,
            client: 3,
            emitter_id: "authored".into(),
            transform: Mat4::IDENTITY.to_cols_array_2d(),
            space: SimulationSpace::Local,
            render: RenderSpec::Sprite {
                texture: Some("authored.png".into()),
                columns: 2,
                rows: 1,
                frames: 2,
                fps: 1.,
                soft_distance: 0.,
                blend: BlendMode::Alpha,
                facing: FacingMode::Camera,
            },
            screen: None,
            screen_uvs: None,
            particles: Arc::from([Particle {
                id: 7,
                position: [2., 3., 0.],
                previous_position: [2., 3., 0.],
                velocity: [0.; 3],
                age: 1.,
                lifetime: 10.,
                color: [1., 0.5, 0.25, 0.75],
                size: [4., 2.],
                rotation: 0.,
                ribbon_id: 0,
            }]),
        }
    }
    #[test]
    fn owned_pixel_frame_preserves_uv_rgba_blend_order_and_local_transform() {
        let mut frame = fixture();
        frame.transform = Mat4::from_translation(Vec3::new(10., 20., 0.)).to_cols_array_2d();
        let packed = pack_screen_emitter(
            3,
            &frame,
            &ScreenSpriteOptions {
                canvas: [64, 64],
                anchor: [0., 0.],
            },
            Some(&[[0.125, 0.25, 0.5, 0.75]]),
        )
        .unwrap();
        assert_eq!(
            packed.parts[0].positions,
            [[12., 23.], [16., 23.], [12., 25.], [16., 25.]]
        );
        assert_eq!(
            packed.parts[0].uvs,
            [[0.125, 0.25], [0.5, 0.25], [0.125, 0.75], [0.5, 0.75]]
        );
        assert_eq!(packed.parts[0].colors, vec![[1., 0.5, 0.25, 0.75]; 4]);
        assert_eq!(packed.parts[0].blend, PartBlend::Normal);
        assert_eq!(packed.textures, ["authored.png"]);
    }
    #[test]
    fn existing_grid_animation_and_common_mesh_cpu_capture_work_without_a_vm() {
        let frame = fixture();
        let packed = pack_screen_emitter(
            3,
            &frame,
            &ScreenSpriteOptions {
                canvas: [8, 8],
                anchor: [0., 0.],
            },
            None,
        )
        .unwrap();
        assert_eq!(packed.parts[0].uvs[0], [0.5, 0.]);
        let snapshot = crysirror_puppet::cpu_snapshot::MeshSnapshot::new(
            packed,
            vec![crysirror_puppet::cpu_snapshot::SnapshotTexture {
                size: [2, 1],
                rgba: Arc::from([255, 0, 0, 255, 0, 255, 0, 255]),
            }],
        )
        .unwrap();
        let pixels = snapshot.render([8, 8], &|| Ok(())).unwrap();
        assert!(pixels.rgba.chunks_exact(4).any(|p| p[1] > 0 && p[3] > 0));
    }
    #[test]
    fn foreign_owner_bad_uv_and_unsupported_facing_fail() {
        let mut frame = fixture();
        let options = ScreenSpriteOptions {
            canvas: [64, 64],
            anchor: [0.5, 0.5],
        };
        assert!(pack_screen_emitter(4, &frame, &options, None).is_err());
        assert!(pack_screen_emitter(3, &frame, &options, Some(&[])).is_err());
        assert!(pack_screen_emitter(3, &frame, &options, Some(&[[0., 0., 1.1, 1.]])).is_err());
        if let RenderSpec::Sprite { facing, .. } = &mut frame.render {
            *facing = FacingMode::Velocity;
        }
        assert!(pack_screen_emitter(3, &frame, &options, None).is_err());
    }
}
