// SPDX-License-Identifier: MPL-2.0
//! Immutable CPU geometry snapshots consumed by the native AS service.
use crate::{geometry, hardware_rt::RtTriangle};
use anyhow::{Result, ensure};
pub(crate) fn simple(scene: &crysirror_runtime::model::Scene3d) -> Result<Vec<RtTriangle>> {
    let vertices = geometry::triangles(scene)?;
    let mut out = Vec::with_capacity(vertices.len() / 3);
    for v in vertices.chunks_exact(3) {
        ensure!(
            v[0].color[3] >= 1.,
            "hardware primary path supports opaque geometry only"
        );
        out.push(RtTriangle {
            a: v[0].position,
            b: v[1].position,
            c: v[2].position,
            normal: v[0].normal,
            color: v[0].color,
        });
    }
    Ok(out)
}
