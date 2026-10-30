// SPDX-License-Identifier: MPL-2.0
//! Device-free captures of immutable engine mesh declarations and RGBA resources.
//! This is a bounded CPU reference for the ordinary PuppetFrame blend contract,
//! independent of native VMs. No legacy rasterizer is embedded here.
use crate::{PartBlend, PuppetFrame};
use anyhow::{Result, ensure};
use crysirror_image::{ImageSurface, rgba_len};
use std::sync::Arc;
mod quad;

#[derive(Clone)]
pub struct SnapshotTexture {
    pub size: [u32; 2],
    /// Straight RGBA8, with sRGB colour channels and linear coverage.
    pub rgba: Arc<[u8]>,
}
impl SnapshotTexture {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            rgba_len(self.size[0], self.size[1])? == self.rgba.len(),
            "snapshot texture byte length"
        );
        Ok(())
    }
}
/// Immutable, worker-safe input. Captures remain valid after the owner replaces
/// a texture: resources are immutable Arc allocations, not GPU/device pointers.
#[derive(Clone)]
pub struct MeshSnapshot {
    frame: Arc<PuppetFrame>,
    textures: Arc<[SnapshotTexture]>,
    cached: Option<([u32; 2], Arc<ImageSurface>)>,
}
impl MeshSnapshot {
    pub fn new(frame: PuppetFrame, textures: Vec<SnapshotTexture>) -> Result<Self> {
        frame.validate()?;
        ensure!(
            frame.textures.len() == textures.len(),
            "snapshot texture table mismatch"
        );
        let mut bytes = 0usize;
        for texture in &textures {
            texture.validate()?;
            bytes = bytes
                .checked_add(texture.rgba.len())
                .ok_or_else(|| anyhow::anyhow!("snapshot texture overflow"))?;
        }
        ensure!(bytes <= 256 * 1024 * 1024, "snapshot texture budget");
        for part in &frame.parts {
            ensure!(
                part.masks.is_empty() && !part.invert_mask,
                "CPU mesh captures do not support stencil masks"
            );
            ensure!(
                part.colors
                    .iter()
                    .all(|c| c[..3].iter().all(|v| (0.0..=16.0).contains(v))
                        && (0.0..=1.0).contains(&c[3])),
                "snapshot colour range"
            );
        }
        Ok(Self {
            frame: Arc::new(frame),
            textures: textures.into(),
            cached: None,
        })
    }
    /// Blocking offline convenience. Live hosts should execute `render` on a
    /// bounded worker and publish that product at their dependency boundary.
    pub fn capture(&mut self, size: [u32; 2]) -> Result<Arc<ImageSurface>> {
        if let Some((old, image)) = &self.cached {
            if *old == size {
                return Ok(image.clone());
            }
        }
        let image = Arc::new(self.render(size, &|| Ok(()))?);
        self.cached = Some((size, image.clone()));
        Ok(image)
    }
    /// Pure worker execution; cancellation is checked between rows. A caller
    /// owns this immutable input and publishes only its matching completion.
    pub fn render(&self, size: [u32; 2], check: &dyn Fn() -> Result<()>) -> Result<ImageSurface> {
        self.render_inner(size, check, true)
    }
    fn render_inner(
        &self,
        size: [u32; 2],
        check: &dyn Fn() -> Result<()>,
        rectangles: bool,
    ) -> Result<ImageSurface> {
        let bytes = rgba_len(size[0], size[1])?;
        ensure!(
            bytes <= 48 * 1024 * 1024,
            "snapshot target working-memory budget"
        );
        let mut target = vec![[0f32; 4]; bytes / 4];
        let transfer = crysirror_image::color::srgb8_transfer();
        let table: [f32; 256] = std::array::from_fn(|i| transfer.decode(i as u8));
        let scale = [
            size[0] as f32 / self.frame.size[0],
            size[1] as f32 / self.frame.size[1],
        ];
        let mut work = 0u64;
        for part in &self.frame.parts {
            check()?;
            if !part.visible || part.colors.iter().all(|c| c[3] == 0.) {
                continue;
            }
            let texture = &self.textures[part.texture as usize];
            if rectangles
                && quad::render(
                    part,
                    texture,
                    scale,
                    size,
                    &table,
                    &mut target,
                    &mut work,
                    check,
                )?
            {
                continue;
            }
            for triangle in part.indices.chunks_exact(3) {
                let mut ids = [
                    triangle[0] as usize,
                    triangle[1] as usize,
                    triangle[2] as usize,
                ];
                let mut p = ids.map(|i| {
                    [
                        part.positions[i][0] * scale[0],
                        part.positions[i][1] * scale[1],
                    ]
                });
                let mut area = edge(p[0], p[1], p[2]);
                ensure!(
                    p.iter().flatten().all(|v| v.is_finite()) && area.is_finite(),
                    "snapshot transformed coordinate range"
                );
                if area.abs() < 1e-8 {
                    continue;
                }
                if area < 0. {
                    ids.swap(1, 2);
                    p.swap(1, 2);
                    area = -area;
                }
                let lo = [
                    p.iter()
                        .map(|p| p[0])
                        .fold(f32::INFINITY, f32::min)
                        .floor()
                        .max(0.)
                        .min(size[0] as f32) as u32,
                    p.iter()
                        .map(|p| p[1])
                        .fold(f32::INFINITY, f32::min)
                        .floor()
                        .max(0.)
                        .min(size[1] as f32) as u32,
                ];
                let hi = [
                    p.iter()
                        .map(|p| p[0])
                        .fold(f32::NEG_INFINITY, f32::max)
                        .ceil()
                        .max(0.)
                        .min(size[0] as f32) as u32,
                    p.iter()
                        .map(|p| p[1])
                        .fold(f32::NEG_INFINITY, f32::max)
                        .ceil()
                        .max(0.)
                        .min(size[1] as f32) as u32,
                ];
                work = work.saturating_add(u64::from(hi[0] - lo[0]) * u64::from(hi[1] - lo[1]));
                ensure!(
                    work <= 128 * 1024 * 1024,
                    "snapshot triangle coverage budget"
                );
                let include = [
                    top_left(p[1], p[2]),
                    top_left(p[2], p[0]),
                    top_left(p[0], p[1]),
                ];
                for y in lo[1]..hi[1] {
                    check()?;
                    for x in lo[0]..hi[0] {
                        let point = [x as f32 + 0.5, y as f32 + 0.5];
                        let e = [
                            edge(p[1], p[2], point),
                            edge(p[2], p[0], point),
                            edge(p[0], p[1], point),
                        ];
                        if (0..3).any(|i| e[i] < 0. || (e[i] == 0. && !include[i])) {
                            continue;
                        }
                        let w = e.map(|e| e / area);
                        let uv = std::array::from_fn(|k| {
                            (0..3).map(|i| part.uvs[ids[i]][k] * w[i]).sum()
                        });
                        let color: [f32; 4] = std::array::from_fn(|k| {
                            (0..3).map(|i| part.colors[ids[i]][k] * w[i]).sum()
                        });
                        let sample = sample(texture, uv, &table);
                        let alpha = (sample[3] * color[3]).clamp(0., 1.);
                        let src = [
                            sample[0] * color[0] * alpha,
                            sample[1] * color[1] * alpha,
                            sample[2] * color[2] * alpha,
                            alpha,
                        ];
                        blend(&mut target[(y * size[0] + x) as usize], src, part.blend);
                    }
                }
            }
        }
        let mut rgba = Vec::with_capacity(bytes);
        for (i, p) in target.iter().enumerate() {
            if i % 4096 == 0 {
                check()?;
            }
            if p[3] <= 1e-5 {
                rgba.extend_from_slice(&[0; 4]);
                continue;
            }
            let encode = |v: f32| transfer.encode(v / p[3]);
            rgba.extend_from_slice(&[
                encode(p[0]),
                encode(p[1]),
                encode(p[2]),
                (p[3] * 255.).round().clamp(0., 255.) as u8,
            ]);
        }
        Ok(ImageSurface {
            width: size[0],
            height: size[1],
            rgba,
        })
    }
}
fn edge(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
fn top_left(a: [f32; 2], b: [f32; 2]) -> bool {
    b[1] < a[1] || (b[1] == a[1] && b[0] > a[0])
}
fn sample(t: &SnapshotTexture, uv: [f32; 2], table: &[f32; 256]) -> [f32; 4] {
    let x = uv[0].clamp(0., 1.) * t.size[0] as f32 - 0.5;
    let y = uv[1].clamp(0., 1.) * t.size[1] as f32 - 0.5;
    sample_xy(t, x, y, table)
}
#[inline]
fn sample_xy(t: &SnapshotTexture, x: f32, y: f32, table: &[f32; 256]) -> [f32; 4] {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let get = |x: i32, y: i32| {
        let at = ((y.clamp(0, t.size[1] as i32 - 1) as u32 * t.size[0]
            + x.clamp(0, t.size[0] as i32 - 1) as u32)
            * 4) as usize;
        [
            table[t.rgba[at] as usize],
            table[t.rgba[at + 1] as usize],
            table[t.rgba[at + 2] as usize],
            t.rgba[at + 3] as f32 / 255.,
        ]
    };
    let a = get(ix, iy);
    if fx == 0. && fy == 0. {
        return a;
    }
    if fy == 0. {
        let b = get(ix + 1, iy);
        return std::array::from_fn(|k| a[k] * (1. - fx) + b[k] * fx);
    }
    if fx == 0. {
        let c = get(ix, iy + 1);
        return std::array::from_fn(|k| a[k] * (1. - fy) + c[k] * fy);
    }
    let (b, c, d) = (get(ix + 1, iy), get(ix, iy + 1), get(ix + 1, iy + 1));
    std::array::from_fn(|k| {
        (a[k] * (1. - fx) + b[k] * fx) * (1. - fy) + (c[k] * (1. - fx) + d[k] * fx) * fy
    })
}
#[inline]
fn blend(dst: &mut [f32; 4], src: [f32; 4], mode: PartBlend) {
    let a = src[3];
    for i in 0..3 {
        dst[i] = match mode {
            PartBlend::Normal => src[i] + dst[i] * (1. - a),
            PartBlend::Add => src[i] + dst[i],
            PartBlend::Subtract => dst[i] - src[i],
            PartBlend::Multiply => src[i] * dst[i] + dst[i] * (1. - a),
            PartBlend::Screen => src[i] * (1. - dst[i]) + dst[i],
        };
    }
    if mode == PartBlend::Normal {
        dst[3] = a + dst[3] * (1. - a);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FramePart;
    fn quad(color: [f32; 4]) -> FramePart {
        FramePart {
            id: "quad".into(),
            texture: 0,
            positions: vec![[0., 0.], [4., 0.], [0., 4.], [4., 4.]],
            uvs: vec![[0., 0.], [1., 0.], [0., 1.], [1., 1.]],
            indices: vec![0, 1, 2, 2, 1, 3],
            colors: vec![color; 4],
            blend: PartBlend::Normal,
            masks: vec![],
            invert_mask: false,
            visible: true,
        }
    }
    fn snapshot(parts: Vec<FramePart>) -> MeshSnapshot {
        MeshSnapshot::new(
            PuppetFrame {
                size: [4., 4.],
                parts,
                textures: vec!["white".into()],
            },
            vec![SnapshotTexture {
                size: [1, 1],
                rgba: Arc::from([255u8; 4]),
            }],
        )
        .unwrap()
    }
    #[test]
    fn adjacent_triangles_have_no_double_blended_seam_and_inputs_are_immutable() {
        let mut s = snapshot(vec![quad([1., 0., 0., 0.5])]);
        let out = s.capture([4, 4]).unwrap();
        assert!(out.rgba.chunks_exact(4).all(|p| p == [255, 0, 0, 128]));
        assert!(Arc::ptr_eq(&out, &s.capture([4, 4]).unwrap()));
        let a = Arc::new(s);
        let b = a.clone();
        let job = std::thread::spawn(move || b.render([2, 2], &|| Ok(())).unwrap());
        assert_eq!(job.join().unwrap().rgba, [255, 0, 0, 128].repeat(4));
    }
    #[test]
    fn linear_source_over_blends_and_reverse_winding_match() {
        let mut front = quad([1., 0., 0., 0.5]);
        front.indices = vec![2, 1, 0, 3, 1, 2];
        let result = snapshot(vec![quad([0., 1., 0., 1.]), front])
            .render([4, 4], &|| Ok(()))
            .unwrap();
        assert!(result.rgba.chunks_exact(4).all(|p| p == [188, 188, 0, 255]));
    }
    #[test]
    fn malformed_masks_limits_and_cancellation_are_explicit() {
        let mut q = quad([1.; 4]);
        q.invert_mask = true;
        assert!(
            MeshSnapshot::new(
                PuppetFrame {
                    size: [4., 4.],
                    parts: vec![q],
                    textures: vec!["x".into()]
                },
                vec![SnapshotTexture {
                    size: [1, 1],
                    rgba: Arc::from([255u8; 4])
                }]
            )
            .is_err()
        );
        let s = snapshot(vec![quad([1.; 4])]);
        assert!(s.render([0, 4], &|| Ok(())).is_err());
        assert!(s.render([4, 4], &|| anyhow::bail!("cancelled")).is_err());
    }

    #[test]
    fn rectangular_capture_matches_triangle_reference_for_clipping_filtering_and_blends() {
        let pixels: Vec<_> = (0..35u16)
            .flat_map(|i| {
                [
                    (i * 7) as u8,
                    (255 - i * 5) as u8,
                    (i * 3) as u8,
                    (40 + i * 6) as u8,
                ]
            })
            .collect();
        for blend_mode in [
            PartBlend::Normal,
            PartBlend::Add,
            PartBlend::Subtract,
            PartBlend::Multiply,
            PartBlend::Screen,
        ] {
            for rect in [
                [-1.25, -0.5, 6.5, 5.75],
                [0.5, 0.5, 3.5, 3.5],
                [0., 0., 4., 4.],
            ] {
                for uv in [[-0.2, -0.1, 1.2, 1.1], [1., 1., 0., 0.], [0., 0., 1., 1.]] {
                    let mut q = quad([0.8, 0.65, 0.9, 0.6]);
                    q.texture = 1;
                    q.positions = vec![
                        [rect[0], rect[1]],
                        [rect[2], rect[1]],
                        [rect[0], rect[3]],
                        [rect[2], rect[3]],
                    ];
                    q.uvs = vec![
                        [uv[0], uv[1]],
                        [uv[2], uv[1]],
                        [uv[0], uv[3]],
                        [uv[2], uv[3]],
                    ];
                    q.blend = blend_mode;
                    let s = MeshSnapshot::new(
                        PuppetFrame {
                            size: [4., 4.],
                            textures: vec!["white".into(), "pattern".into()],
                            parts: vec![quad([0.2, 0.4, 0.6, 0.75]), q],
                        },
                        vec![
                            SnapshotTexture {
                                size: [1, 1],
                                rgba: Arc::from([255u8; 4]),
                            },
                            SnapshotTexture {
                                size: [7, 5],
                                rgba: pixels.clone().into(),
                            },
                        ],
                    )
                    .unwrap();
                    let fast = s.render_inner([9, 7], &|| Ok(()), true).unwrap();
                    let reference = s.render_inner([9, 7], &|| Ok(()), false).unwrap();
                    assert!(
                        fast.rgba
                            .iter()
                            .zip(&reference.rgba)
                            .all(|(a, b)| a.abs_diff(*b) <= 1),
                        "{blend_mode:?} {rect:?} {uv:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn rectangle_topology_guard_preserves_nonrectangular_and_gradient_draws() {
        for scenario in 0..4 {
            let mut q = quad([0.7, 0.4, 0.2, 0.65]);
            match scenario {
                0 => {
                    q.positions[2][0] = 1.;
                }
                1 => {
                    q.colors[1][0] = 0.9;
                }
                2 => {
                    q.uvs[3][0] = 0.7;
                }
                _ => {
                    q.indices = vec![0, 1, 2, 0, 1, 2];
                }
            }
            let s = snapshot(vec![q]);
            assert_eq!(
                s.render_inner([8, 8], &|| Ok(()), true).unwrap().rgba,
                s.render_inner([8, 8], &|| Ok(()), false).unwrap().rgba
            );
        }
    }

    #[test]
    fn large_feedback_quad_has_reference_pixels() {
        let size = [1024u32, 640];
        let pixels: Vec<_> = (0..size[0] * size[1])
            .flat_map(|i| {
                [
                    (i % 251) as u8,
                    ((i / size[0]) % 253) as u8,
                    (i % 193) as u8,
                    255,
                ]
            })
            .collect();
        let mut back = quad([1.; 4]);
        back.positions = vec![[0., 0.], [1024., 0.], [0., 640.], [1024., 640.]];
        let mut feedback = back.clone();
        feedback.colors = vec![[1., 1., 1., 200. / 255.]; 4];
        let snapshot = MeshSnapshot::new(
            PuppetFrame {
                size: [1024., 640.],
                parts: vec![back, feedback],
                textures: vec!["feedback".into()],
            },
            vec![SnapshotTexture {
                size,
                rgba: pixels.into(),
            }],
        )
        .unwrap();
        let clock = std::time::Instant::now();
        let fast = snapshot.render_inner(size, &|| Ok(()), true).unwrap();
        let fast_us = clock.elapsed().as_micros();
        let clock = std::time::Instant::now();
        let reference = snapshot.render_inner(size, &|| Ok(()), false).unwrap();
        eprintln!(
            "feedback capture fast_us={fast_us} triangle_us={}",
            clock.elapsed().as_micros()
        );
        assert!(
            fast.rgba
                .iter()
                .zip(&reference.rgba)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
    }
}
