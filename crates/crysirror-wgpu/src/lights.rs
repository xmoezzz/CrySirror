// SPDX-License-Identifier: MPL-2.0
//! Clustered local lights (ADR 0018). The view frustum is split into
//! `CLUSTER_X × CLUSTER_Y` screen tiles and `CLUSTER_Z` logarithmic depth
//! slices; each light's range sphere is assigned conservatively on the CPU
//! and the forward shader walks only its cluster's list.
use crysirror_scene::components::LightShape;
use crysirror_scene::view::PbrLight;
use glam::{Mat4, Vec3};

pub const CLUSTER_X: u32 = 16;
pub const CLUSTER_Y: u32 = 9;
pub const CLUSTER_Z: u32 = 24;
pub const CLUSTERS: usize = (CLUSTER_X * CLUSTER_Y * CLUSTER_Z) as usize;
/// Lights uploaded per frame; the nearest are kept when exceeded.
pub const MAX_LIGHTS: usize = 4096;
/// Lights shaded per cluster; extra lights are dropped and counted.
pub const MAX_PER_CLUSTER: usize = 256;

pub const KIND_POINT: f32 = 0.;
pub const KIND_SPOT: f32 = 1.;
pub const KIND_RECT: f32 = 2.;
pub const KIND_DIRECTIONAL: f32 = 3.;
pub const KIND_AMBIENT: f32 = 4.;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuLight {
    /// xyz world position, w range.
    pub position_range: [f32; 4],
    /// xyz emission axis, w kind.
    pub direction_kind: [f32; 4],
    /// rgb color × candela, w shadow slot (-1 none).
    pub intensity: [f32; 4],
    /// x cos(outer), y 1/(cos(inner) − cos(outer)), z falloff exponent
    /// (0 = inverse square).
    pub cone: [f32; 4],
    /// Rect half-extent axes.
    pub right: [f32; 4],
    pub up: [f32; 4],
}

/// Frustum parameters of the camera the clusters are built for.
#[derive(Clone, Copy, Debug)]
pub struct ClusterView {
    pub view: Mat4,
    pub fov_y_radians: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusterStats {
    pub submitted: usize,
    pub uploaded: usize,
    pub visible: usize,
    pub assignments: usize,
    /// Light/cluster pairs dropped by [`MAX_PER_CLUSTER`].
    pub overflow: usize,
}

pub struct ClusterData {
    pub lights: Vec<GpuLight>,
    /// Per cluster: (offset, count) into `indices`.
    pub grid: Vec<[u32; 2]>,
    pub indices: Vec<u32>,
    pub stats: ClusterStats,
}

pub fn gpu_light(l: &PbrLight, shadow_slot: i32) -> GpuLight {
    let (kind, cone) = match l.shape {
        LightShape::Point => (KIND_POINT, [-2., 0., 0., 0.]),
        LightShape::Spot { inner_degrees, outer_degrees } => {
            let co = outer_degrees.to_radians().cos();
            let ci = inner_degrees.to_radians().cos();
            (KIND_SPOT, [co, 1. / (ci - co).max(1e-4), 0., 0.])
        }
        LightShape::Rect { .. } => (KIND_RECT, [0.; 4]),
        LightShape::Directional => (KIND_DIRECTIONAL, [0.; 4]),
        LightShape::Ambient => (KIND_AMBIENT, [0.; 4]),
    };
    let mut cone = cone;
    cone[2] = match l.falloff {
        crysirror_scene::components::Falloff::InverseSquare => 0.,
        crysirror_scene::components::Falloff::Exponent { exponent } => exponent,
    };
    let v4 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
    GpuLight {
        position_range: v4(l.position, l.range),
        direction_kind: v4(l.direction, kind),
        intensity: v4(l.intensity, shadow_slot as f32),
        cone,
        right: v4(l.right, 0.),
        up: v4(l.up, 0.),
    }
}

/// Depth slice of a positive view depth (shared with the shader).
pub fn slice_of(depth: f32, near: f32, far: f32) -> u32 {
    let t = (depth.max(near) / near).ln() / (far / near).ln();
    ((t * CLUSTER_Z as f32).floor().max(0.) as u32).min(CLUSTER_Z - 1)
}

pub fn build(lights: &[PbrLight], cv: &ClusterView, shadow_slot: impl Fn(usize) -> i32) -> ClusterData {
    let mut stats = ClusterStats { submitted: lights.len(), ..Default::default() };
    let eye = cv.view.inverse().transform_point3(Vec3::ZERO);
    let mut order: Vec<usize> = (0..lights.len()).collect();
    if order.len() > MAX_LIGHTS {
        let dist = |i: usize| (Vec3::from_array(lights[i].position) - eye).length() - lights[i].range;
        order.sort_by(|a, b| dist(*a).total_cmp(&dist(*b)));
        order.truncate(MAX_LIGHTS);
    }
    stats.uploaded = order.len();
    let gpu: Vec<GpuLight> = order.iter().map(|i| gpu_light(&lights[*i], shadow_slot(*i))).collect();
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); CLUSTERS];
    let sy = 1. / (cv.fov_y_radians * 0.5).tan();
    let sx = sy / cv.aspect;
    for (gi, &li) in order.iter().enumerate() {
        let l = &lights[li];
        let c = cv.view.transform_point3(Vec3::from_array(l.position));
        let r = l.range;
        let depth = -c.z;
        if depth + r < cv.near || depth - r > cv.far {
            continue;
        }
        let z0 = (depth - r).max(cv.near);
        let z1 = (depth + r).min(cv.far);
        // Conservative NDC bounds: for fixed x, x/d is monotonic in d.
        let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for d in [z0, z1] {
            for x in [c.x - r, c.x + r] {
                let v = x * sx / d;
                x0 = x0.min(v);
                x1 = x1.max(v);
            }
            for y in [c.y - r, c.y + r] {
                let v = y * sy / d;
                y0 = y0.min(v);
                y1 = y1.max(v);
            }
        }
        if x1 < -1. || x0 > 1. || y1 < -1. || y0 > 1. {
            continue;
        }
        stats.visible += 1;
        let tile = |v: f32, n: u32| (((v * 0.5 + 0.5) * n as f32).floor().max(0.) as u32).min(n - 1);
        // Screen y grows downward: NDC +1 is tile row 0.
        let (tx0, tx1) = (tile(x0, CLUSTER_X), tile(x1, CLUSTER_X));
        let (ty0, ty1) = (tile(-y1, CLUSTER_Y), tile(-y0, CLUSTER_Y));
        let (s0, s1) = (slice_of(z0, cv.near, cv.far), slice_of(z1, cv.near, cv.far));
        for z in s0..=s1 {
            for y in ty0..=ty1 {
                for x in tx0..=tx1 {
                    let list = &mut lists[cluster_index(x, y, z)];
                    if list.len() < MAX_PER_CLUSTER {
                        list.push(gi as u32);
                        stats.assignments += 1;
                    } else {
                        stats.overflow += 1;
                    }
                }
            }
        }
    }
    let mut grid = Vec::with_capacity(CLUSTERS);
    let mut indices = Vec::with_capacity(stats.assignments);
    for list in &lists {
        grid.push([indices.len() as u32, list.len() as u32]);
        indices.extend_from_slice(list);
    }
    ClusterData { lights: gpu, grid, indices, stats }
}

pub fn cluster_index(x: u32, y: u32, z: u32) -> usize {
    ((z * CLUSTER_Y + y) * CLUSTER_X + x) as usize
}

/// GPU buffers for the cluster data, grown on demand.
pub struct LightBuffers {
    pub lights: wgpu::Buffer,
    pub grid: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub stats: ClusterStats,
}

fn storage(d: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    d.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl LightBuffers {
    pub fn new(d: &wgpu::Device) -> Self {
        Self {
            lights: storage(d, "lights", std::mem::size_of::<GpuLight>() as u64 * 64),
            grid: storage(d, "light grid", 8 * CLUSTERS as u64),
            indices: storage(d, "light indices", 4 * 4096),
            stats: ClusterStats::default(),
        }
    }
    pub fn upload(&mut self, d: &wgpu::Device, q: &wgpu::Queue, data: &ClusterData) {
        let lb: &[u8] = bytemuck::cast_slice(&data.lights);
        if lb.len() as u64 > self.lights.size() {
            self.lights = storage(d, "lights", (lb.len() as u64).next_power_of_two());
        }
        let ib: &[u8] = bytemuck::cast_slice(&data.indices);
        if ib.len() as u64 > self.indices.size() {
            self.indices = storage(d, "light indices", (ib.len() as u64).next_power_of_two());
        }
        if !lb.is_empty() {
            q.write_buffer(&self.lights, 0, lb);
        }
        if !ib.is_empty() {
            q.write_buffer(&self.indices, 0, ib);
        }
        q.write_buffer(&self.grid, 0, bytemuck::cast_slice(&data.grid));
        self.stats = data.stats;
    }
}

/// Local-light shadow atlas: square tiles, a spot light uses one and a
/// point light six (cube faces +X, −X, +Y, −Y, +Z, −Z in that order).
pub const SHADOW_ATLAS: u32 = 4096;
/// The shared depth atlas is twice as wide: virtual shadow pages use the right half (vsm.rs).
pub const SHADOW_ATLAS_WIDTH: u32 = 2 * SHADOW_ATLAS;
pub const SHADOW_TILE: u32 = 512;
pub const SHADOW_TILES_PER_ROW: u32 = SHADOW_ATLAS / SHADOW_TILE;
pub const MAX_SHADOW_TILES: usize = (SHADOW_TILES_PER_ROW * SHADOW_TILES_PER_ROW) as usize;

#[derive(Clone, Debug)]
pub struct ShadowTile {
    pub view_proj: Mat4,
    pub light: usize,
    pub position: Vec3,
    pub range: f32,
}
#[derive(Clone, Debug, Default)]
pub struct ShadowAlloc {
    /// First tile per input light, -1 when it has no shadow this frame.
    pub slots: Vec<i32>,
    pub tiles: Vec<ShadowTile>,
    /// Shadow-casting lights requested / granted (budget is visible, not silent).
    pub requested: usize,
    pub granted: usize,
}

pub const CUBE_FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::NEG_Y),
    (Vec3::NEG_X, Vec3::NEG_Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::Z, Vec3::NEG_Y),
    (Vec3::NEG_Z, Vec3::NEG_Y),
];

/// Grant atlas tiles to shadow-casting point/spot lights, nearest first.
pub fn allocate_shadows(lights: &[PbrLight], eye: Vec3) -> ShadowAlloc {
    let mut out = ShadowAlloc { slots: vec![-1; lights.len()], ..Default::default() };
    let mut order: Vec<usize> = (0..lights.len())
        .filter(|i| lights[*i].shadows && matches!(lights[*i].shape, LightShape::Point | LightShape::Spot { .. }))
        .collect();
    out.requested = order.len();
    let dist = |i: usize| ((Vec3::from_array(lights[i].position) - eye).length() - lights[i].range).max(0.);
    order.sort_by(|a, b| dist(*a).total_cmp(&dist(*b)));
    for i in order {
        let l = &lights[i];
        let pos = Vec3::from_array(l.position);
        let near = (l.range * 0.005).max(0.01);
        let (faces, fov) = match l.shape {
            LightShape::Spot { outer_degrees, .. } => {
                let dir = Vec3::from_array(l.direction);
                let up = if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
                (vec![(dir, up)], (2. * outer_degrees + 4.).min(178.).to_radians())
            }
            _ => (CUBE_FACES.to_vec(), std::f32::consts::FRAC_PI_2),
        };
        if out.tiles.len() + faces.len() > MAX_SHADOW_TILES {
            continue;
        }
        out.slots[i] = out.tiles.len() as i32;
        let proj = Mat4::perspective_rh(fov, 1., near, l.range);
        for (dir, up) in faces {
            out.tiles.push(ShadowTile {
                view_proj: proj * Mat4::look_at_rh(pos, pos + dir, up),
                light: i,
                position: pos,
                range: l.range,
            });
        }
        out.granted += 1;
    }
    out
}

/// Pixel rectangle of an atlas tile (x, y, size).
pub fn tile_rect(tile: usize) -> (u32, u32, u32) {
    let t = tile as u32;
    ((t % SHADOW_TILES_PER_ROW) * SHADOW_TILE, (t / SHADOW_TILES_PER_ROW) * SHADOW_TILE, SHADOW_TILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn light(p: [f32; 3], range: f32) -> PbrLight {
        PbrLight {
            key: 0,
            position: p,
            direction: [0., -1., 0.],
            right: [0.; 3],
            up: [0.; 3],
            intensity: [1.; 3],
            range,
            shape: LightShape::Point,
            falloff: Default::default(),
            shadows: false,
        }
    }
    fn view() -> ClusterView {
        ClusterView {
            view: Mat4::look_at_rh(Vec3::ZERO, -Vec3::Z, Vec3::Y),
            fov_y_radians: 60f32.to_radians(),
            aspect: 16. / 9.,
            near: 0.1,
            far: 100.,
        }
    }
    #[test]
    fn lights_land_in_their_clusters_and_culled_ones_do_not() {
        let cv = view();
        // Centered small light 10 m ahead, a light behind the camera, one far left.
        let data = build(&[light([0., 0., -10.], 1.), light([0., 0., 20.], 2.), light([-500., 0., -10.], 1.)], &cv, |_| -1);
        assert_eq!(data.stats.visible, 1);
        let z = slice_of(10., cv.near, cv.far);
        let center = data.grid[cluster_index(CLUSTER_X / 2, CLUSTER_Y / 2, z)];
        assert_eq!(center[1], 1);
        assert_eq!(data.indices[center[0] as usize], 0);
        // Corner tile at the same depth does not see it.
        assert_eq!(data.grid[cluster_index(0, 0, z)][1], 0);
        // Slices far from the light's depth range do not see it either.
        assert_eq!(data.grid[cluster_index(CLUSTER_X / 2, CLUSTER_Y / 2, 0)][1], 0);
    }
    #[test]
    fn shadow_budget_prefers_near_lights_and_point_lights_take_six_tiles() {
        let mut ls: Vec<PbrLight> = (0..20).map(|i| PbrLight { shadows: true, ..light([0., 0., -(i as f32) * 10.], 2.) }).collect();
        ls[0].shape = LightShape::Spot { inner_degrees: 10., outer_degrees: 30. };
        ls[1].shadows = false;
        let a = allocate_shadows(&ls, Vec3::ZERO);
        assert_eq!(a.requested, 19);
        // 1 spot tile + 10 point lights × 6 = 61 ≤ 64; the 11th point light does not fit.
        assert_eq!((a.granted, a.tiles.len()), (11, 61));
        assert_eq!(a.slots[0], 0);
        assert_eq!(a.slots[1], -1);
        assert_eq!(a.slots[2], 1);
        assert!(a.slots[19] == -1 && a.slots[11] == 55);
        // Every cube face sees the point straight along its axis at the far plane.
        for (k, (dir, _)) in CUBE_FACES.iter().enumerate() {
            let t = &a.tiles[(a.slots[2] as usize) + k];
            let c = t.view_proj * (t.position + *dir * 1.).extend(1.);
            assert!((c.x / c.w).abs() < 1e-4 && (c.y / c.w).abs() < 1e-4 && (0. ..1.).contains(&(c.z / c.w)), "face {k}");
        }
        assert_eq!(tile_rect(9), (512, 512, 512));
    }
    #[test]
    fn upper_light_maps_to_top_rows_and_budget_is_counted() {
        let cv = view();
        let data = build(&[light([0., 4., -10.], 0.5)], &cv, |_| -1);
        let z = slice_of(10., cv.near, cv.far);
        let rows: Vec<u32> = (0..CLUSTER_Y).filter(|y| data.grid[cluster_index(CLUSTER_X / 2, *y, z)][1] > 0).collect();
        assert!(!rows.is_empty() && rows.iter().all(|y| *y < CLUSTER_Y / 2), "{rows:?}");
        let many: Vec<_> = (0..300).map(|_| light([0., 0., -10.], 1.)).collect();
        let data = build(&many, &cv, |_| -1);
        assert!(data.stats.overflow > 0);
        assert!(data.grid.iter().all(|g| g[1] as usize <= MAX_PER_CLUSTER));
    }
}
