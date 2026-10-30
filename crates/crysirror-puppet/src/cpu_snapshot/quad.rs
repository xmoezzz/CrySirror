// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Exact rectangular topology specialization of the shared linear mesh capture.
use super::*;

pub(super) fn render(
    part: &crate::FramePart,
    texture: &SnapshotTexture,
    scale: [f32; 2],
    size: [u32; 2],
    table: &[f32; 256],
    target: &mut [[f32; 4]],
    work: &mut u64,
    check: &dyn Fn() -> Result<()>,
) -> Result<bool> {
    if part.positions.len() != 4
        || part.indices.len() != 6
        || !part.colors.iter().all(|c| *c == part.colors[0])
    {
        return Ok(false);
    }
    let lo = [0, 1].map(|axis| {
        part.positions
            .iter()
            .map(|p| p[axis])
            .fold(f32::INFINITY, f32::min)
    });
    let hi = [0, 1].map(|axis| {
        part.positions
            .iter()
            .map(|p| p[axis])
            .fold(f32::NEG_INFINITY, f32::max)
    });
    if lo[0] >= hi[0] || lo[1] >= hi[1] {
        return Ok(false);
    }
    let mut corners = [usize::MAX; 4];
    let mut kinds = [0u32; 4];
    for (i, p) in part.positions.iter().enumerate() {
        if !(p[0] == lo[0] || p[0] == hi[0]) || !(p[1] == lo[1] || p[1] == hi[1]) {
            return Ok(false);
        }
        let kind = usize::from(p[0] == hi[0]) + 2 * usize::from(p[1] == hi[1]);
        if corners[kind] != usize::MAX {
            return Ok(false);
        }
        corners[kind] = i;
        kinds[i] = kind as u32;
    }
    let mut triangles = [[0u32; 3]; 2];
    for (tri, input) in triangles.iter_mut().zip(part.indices.chunks_exact(3)) {
        *tri = [
            kinds[input[0] as usize],
            kinds[input[1] as usize],
            kinds[input[2] as usize],
        ];
        tri.sort_unstable();
    }
    triangles.sort_unstable();
    if triangles != [[0, 1, 2], [1, 2, 3]] && triangles != [[0, 1, 3], [0, 2, 3]] {
        return Ok(false);
    }
    let uv = corners.map(|i| part.uvs[i]);
    if uv[0][1] != uv[1][1] || uv[2][1] != uv[3][1] || uv[0][0] != uv[2][0] || uv[1][0] != uv[3][0]
    {
        return Ok(false);
    }
    let lo = [lo[0] * scale[0], lo[1] * scale[1]];
    let hi = [hi[0] * scale[0], hi[1] * scale[1]];
    if !lo.into_iter().chain(hi).all(f32::is_finite) {
        return Ok(false);
    }
    let area = (hi[0] - lo[0]) * (hi[1] - lo[1]);
    if !area.is_finite() || area <= 1e-8 {
        return Ok(false);
    }
    // Charge the same bounding rectangles as the two reference triangles.
    let bound = [0, 1].map(|i| {
        (
            lo[i].floor().clamp(0., size[i] as f32) as u32,
            hi[i].ceil().clamp(0., size[i] as f32) as u32,
        )
    });
    *work = work.saturating_add(
        2 * u64::from(bound[0].1 - bound[0].0) * u64::from(bound[1].1 - bound[1].0),
    );
    ensure!(
        *work <= 128 * 1024 * 1024,
        "snapshot triangle coverage budget"
    );
    // Pixel centres on the top/left edge are included, bottom/right excluded.
    let range = [0, 1].map(|i| {
        (
            (lo[i] - 0.5).ceil().clamp(0., size[i] as f32) as u32,
            (hi[i] - 0.5).ceil().clamp(0., size[i] as f32) as u32,
        )
    });
    let xs: Vec<_> = (range[0].0..range[0].1)
        .map(|x| {
            let u = (x as f32 + 0.5 - lo[0])
                * ((uv[1][0] - uv[0][0]) * texture.size[0] as f32 / (hi[0] - lo[0]))
                + uv[0][0] * texture.size[0] as f32;
            u.clamp(0., texture.size[0] as f32) - 0.5
        })
        .collect();
    let color = part.colors[0];
    for y in range[1].0..range[1].1 {
        check()?;
        let v = (y as f32 + 0.5 - lo[1])
            * ((uv[2][1] - uv[0][1]) * texture.size[1] as f32 / (hi[1] - lo[1]))
            + uv[0][1] * texture.size[1] as f32;
        let sy = v.clamp(0., texture.size[1] as f32) - 0.5;
        let row =
            &mut target[(y * size[0] + range[0].0) as usize..(y * size[0] + range[0].1) as usize];
        for (dst, &sx) in row.iter_mut().zip(&xs) {
            let sample = sample_xy(texture, sx, sy, table);
            let alpha = (sample[3] * color[3]).clamp(0., 1.);
            let src = [
                sample[0] * color[0] * alpha,
                sample[1] * color[1] * alpha,
                sample[2] * color[2] * alpha,
                alpha,
            ];
            blend(dst, src, part.blend);
        }
    }
    Ok(true)
}
