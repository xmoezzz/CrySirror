// SPDX-License-Identifier: MPL-2.0
//! Virtual shadow maps for the sun (ADR 0027). Own implementation.
//!
//! - Clipmap levels `l = 0..levels`, each a virtual 8192² shadow map
//!   (64×64 pages of 128²) centred on the camera. Level `l` has a texel size
//!   of `texel · 2^l`.
//! - A pixel picks the finest level whose texel is at least its footprint.
//! - Pages are addressed by absolute light-space page coordinates. The
//!   per-level window is toroidal (slot = coordinate mod 64), so a page keeps
//!   its key while the camera moves and stays cached.
//! - A compute pass marks the pages that visible pixels need. The marks are
//!   read back and the CPU allocates physical pages in the right half of the
//!   shared shadow atlas (1024 pages, LRU). Only pages that are new or were
//!   invalidated are rendered.
//! - The sampling shader falls back to coarser levels when a page is missing.
use glam::{Mat4, Vec3};
use std::collections::HashMap;

pub const PAGE: u32 = 128;
pub const WINDOW: i32 = 64;
pub const MAX_LEVELS: u32 = 16;
/// Physical pages: the 4096² right half of the 8192×4096 shadow atlas.
pub const PHYS_ORIGIN_X: u32 = 4096;
pub const PHYS_PER_ROW: u32 = 4096 / PAGE;
pub const PHYS_PAGES: u32 = PHYS_PER_ROW * PHYS_PER_ROW;
pub const TABLE_ENTRIES: usize = (MAX_LEVELS as usize) * (WINDOW * WINDOW) as usize;

pub type PageKey = (u32, i32, i32);

/// Everything that, when changed, invalidates all cached pages. Geometry
/// changes invalidate only the pages they overlap (`invalidate`).
#[derive(Clone, Debug, PartialEq)]
pub struct CacheKey {
    pub light_view: [f32; 16],
    pub depth: [f32; 2],
    pub texel: f32,
    pub levels: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VsmStats {
    pub requested: usize,
    pub resident: usize,
    pub rendered: usize,
    pub dropped: usize,
    pub invalidations: u64,
    /// Resident pages invalidated this frame by moving/added/removed casters.
    pub dirtied: usize,
    /// Running totals since creation.
    pub total_rendered: u64,
    pub total_dirtied: u64,
}

/// Page world size of a level.
pub fn page_world(texel: f32, level: u32) -> f32 {
    texel * (1u32 << level) as f32 * PAGE as f32
}
/// Window centre (absolute page) of each level for a light-space camera position.
pub fn centers(light_view: &Mat4, camera: Vec3, texel: f32, levels: u32) -> Vec<[i32; 2]> {
    let c = light_view.transform_point3(camera);
    (0..levels)
        .map(|l| {
            let pw = page_world(texel, l);
            [(c.x / pw).floor() as i32, (c.y / pw).floor() as i32]
        })
        .collect()
}
pub fn slot_of(abs: i32) -> i32 {
    abs.rem_euclid(WINDOW)
}
/// The absolute page in the window of `center` occupying `slot`.
pub fn abs_of(slot: i32, center: i32) -> i32 {
    let lo = center - WINDOW / 2;
    lo + (slot - lo).rem_euclid(WINDOW)
}
pub fn in_window(abs: i32, center: i32) -> bool {
    (center - WINDOW / 2..center + WINDOW / 2).contains(&abs)
}
pub fn table_index(level: u32, sx: i32, sy: i32) -> usize {
    level as usize * (WINDOW * WINDOW) as usize + (sy * WINDOW + sx) as usize
}
/// Packed absolute page coordinates stored with each table entry.
pub fn pack(ax: i32, ay: i32) -> u32 {
    ((ax + 32768) as u32 & 0xFFFF) | (((ay + 32768) as u32 & 0xFFFF) << 16)
}
/// Pixel rectangle of a physical page in the shadow atlas.
pub fn phys_rect(phys: u32) -> (u32, u32) {
    (PHYS_ORIGIN_X + (phys % PHYS_PER_ROW) * PAGE, (phys / PHYS_PER_ROW) * PAGE)
}
/// Orthographic projection of one page (light view space → NDC).
pub fn page_projection(texel: f32, key: PageKey, depth: [f32; 2]) -> Mat4 {
    let pw = page_world(texel, key.0);
    let (x0, y0) = (key.1 as f32 * pw, key.2 as f32 * pw);
    Mat4::orthographic_rh(x0, x0 + pw, y0, y0 + pw, depth[0], depth[1])
}

/// CPU page cache: physical allocation with LRU eviction.
#[derive(Default)]
pub struct PageCache {
    pub key: Option<CacheKey>,
    resident: HashMap<PageKey, (u32, u64)>,
    free: Vec<u32>,
    frame: u64,
    pub stats: VsmStats,
}
impl PageCache {
    pub fn new() -> Self {
        Self { free: (0..PHYS_PAGES).rev().collect(), ..Default::default() }
    }
    /// Allocate `requested` pages for this frame; returns the pages to render.
    pub fn update(&mut self, key: CacheKey, requested: &[PageKey]) -> Vec<(PageKey, u32)> {
        self.frame += 1;
        if self.key.as_ref() != Some(&key) {
            if self.key.is_some() {
                self.stats.invalidations += 1;
            }
            self.resident.clear();
            self.free = (0..PHYS_PAGES).rev().collect();
            self.key = Some(key);
        }
        let mut render = Vec::new();
        let mut dropped = 0;
        let wanted: std::collections::HashSet<PageKey> = requested.iter().copied().collect();
        for &k in requested {
            if let Some(e) = self.resident.get_mut(&k) {
                e.1 = self.frame;
                continue;
            }
            let phys = match self.free.pop() {
                Some(p) => Some(p),
                None => {
                    // Evict the least recently used page not needed this frame.
                    let victim = self
                        .resident
                        .iter()
                        .filter(|(pk, _)| !wanted.contains(pk))
                        .min_by_key(|(pk, e)| (e.1, **pk))
                        .map(|(pk, e)| (*pk, e.0));
                    victim.map(|(pk, p)| {
                        self.resident.remove(&pk);
                        p
                    })
                }
            };
            match phys {
                Some(p) => {
                    self.resident.insert(k, (p, self.frame));
                    render.push((k, p));
                }
                None => dropped += 1,
            }
        }
        self.stats.requested = requested.len();
        self.stats.resident = self.resident.len();
        self.stats.rendered = render.len();
        self.stats.total_rendered += render.len() as u64;
        self.stats.dropped = dropped;
        render
    }
    /// Drop resident pages whose light-space square overlaps any circle
    /// `(x, y, radius)` (casters affect a page along the whole light ray, so
    /// depth is irrelevant). Requested pages among them re-render this frame.
    pub fn invalidate(&mut self, texel: f32, dirty: &[(f32, f32, f32)]) -> usize {
        if dirty.is_empty() {
            self.stats.dirtied = 0;
            return 0;
        }
        let hit: Vec<PageKey> = self
            .resident
            .keys()
            .copied()
            .filter(|&(l, ax, ay)| {
                let pw = page_world(texel, l);
                let (cx, cy) = ((ax as f32 + 0.5) * pw, (ay as f32 + 0.5) * pw);
                dirty.iter().any(|&(x, y, r)| (x - cx).abs() <= pw * 0.5 + r && (y - cy).abs() <= pw * 0.5 + r)
            })
            .collect();
        for k in &hit {
            if let Some((p, _)) = self.resident.remove(k) {
                self.free.push(p);
            }
        }
        self.stats.dirtied = hit.len();
        self.stats.total_dirtied += hit.len() as u64;
        hit.len()
    }
    /// Page table for the current windows: [packed abs coords, phys + 1].
    pub fn table(&self, centers: &[[i32; 2]]) -> Vec<[u32; 2]> {
        let mut t = vec![[0u32; 2]; TABLE_ENTRIES];
        for (&(l, ax, ay), &(p, _)) in &self.resident {
            let Some(c) = centers.get(l as usize) else { continue };
            if in_window(ax, c[0]) && in_window(ay, c[1]) {
                t[table_index(l, slot_of(ax), slot_of(ay))] = [pack(ax, ay), p + 1];
            }
        }
        t
    }
}

/// Decode a marked request buffer (one u32 per table entry) with the window
/// centres that were current when it was marked.
pub fn decode_requests(marks: &[u32], centers: &[[i32; 2]]) -> Vec<PageKey> {
    let mut out = Vec::new();
    for (l, c) in centers.iter().enumerate() {
        for sy in 0..WINDOW {
            for sx in 0..WINDOW {
                if marks.get(table_index(l as u32, sx, sy)).is_some_and(|&m| m != 0) {
                    out.push((l as u32, abs_of(sx, c[0]), abs_of(sy, c[1])));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(geometry: u64) -> CacheKey {
        CacheKey { light_view: [0.; 16], depth: [0., 1.], texel: 0.02 + geometry as f32 * 0.01, levels: 4 }
    }

    #[test]
    fn window_slots_round_trip_for_negative_and_positive_pages() {
        for center in [-1000, -33, -1, 0, 5, 31, 32, 977] {
            for abs in center - 32..center + 32 {
                assert!(in_window(abs, center));
                assert_eq!(abs_of(slot_of(abs), center), abs, "center {center} abs {abs}");
            }
            assert!(!in_window(center + 32, center) && !in_window(center - 33, center));
        }
    }

    #[test]
    fn cache_renders_only_new_pages_and_reuses_resident_ones() {
        let mut c = PageCache::new();
        let a = [(0, 1, 2), (0, -3, 4), (1, 0, 0)];
        assert_eq!(c.update(key(1), &a).len(), 3);
        // Same request: nothing to render.
        assert!(c.update(key(1), &a).is_empty());
        // One new page.
        let b = [(0, 1, 2), (2, 7, 7)];
        let r = c.update(key(1), &b);
        assert_eq!(r.iter().map(|x| x.0).collect::<Vec<_>>(), vec![(2, 7, 7)]);
        // A key change (light, range, texel, levels) invalidates everything.
        assert_eq!(c.update(key(2), &b).len(), 2);
        assert_eq!(c.stats.invalidations, 1);
    }

    #[test]
    fn moving_casters_invalidate_only_overlapping_pages() {
        let mut c = PageCache::new();
        let pw0 = page_world(0.02, 0);
        let pages = [(0, 0, 0), (0, 5, 0), (1, 0, 0), (0, -3, -3)];
        c.update(key(0), &pages);
        // A caster of radius 0.1 at the centre of page (0, 0, 0): touches
        // that page and the coarser page (1, 0, 0) covering the same area.
        let n = c.invalidate(0.02, &[(0.5 * pw0, 0.5 * pw0, 0.1)]);
        assert_eq!(n, 2);
        let r = c.update(key(0), &pages);
        let mut keys: Vec<PageKey> = r.iter().map(|x| x.0).collect();
        keys.sort();
        assert_eq!(keys, vec![(0, 0, 0), (1, 0, 0)]);
        assert_eq!(c.stats.resident, 4);
    }

    #[test]
    fn eviction_is_lru_and_never_evicts_pages_needed_now() {
        let mut c = PageCache::new();
        let all: Vec<PageKey> = (0..PHYS_PAGES as i32).map(|i| (0, i % 64, i / 64)).collect();
        assert_eq!(c.update(key(0), &all).len(), PHYS_PAGES as usize);
        // A frame needing every page plus one more drops the extra one.
        let mut more = all.clone();
        more.push((3, 0, 0));
        let r = c.update(key(0), &more);
        assert!(r.is_empty() && c.stats.dropped == 1);
        // Needing only page (3,0,0) evicts the least recently used page.
        let r = c.update(key(0), &[(3, 0, 0)]);
        assert_eq!(r.len(), 1);
        assert_eq!(c.stats.resident, PHYS_PAGES as usize);
    }

    #[test]
    fn table_and_requests_use_the_same_slots() {
        let mut c = PageCache::new();
        let centers = vec![[-5, 40], [0, 0]];
        let pages = [(0, -5 - 32, 40 + 31), (1, 3, -2)];
        c.update(key(0), &pages);
        let t = c.table(&centers);
        let mut marks = vec![0u32; TABLE_ENTRIES];
        for (i, e) in t.iter().enumerate() {
            if e[1] != 0 {
                marks[i] = 1;
            }
        }
        let mut back = decode_requests(&marks, &centers);
        back.sort();
        let mut want = pages.to_vec();
        want.sort();
        assert_eq!(back, want);
        // Out-of-window resident pages are absent from the table.
        let t = c.table(&[[100, 100], [0, 0]]);
        assert_eq!(t.iter().filter(|e| e[1] != 0).count(), 1);
    }

    #[test]
    fn page_projection_maps_the_page_square_to_ndc() {
        let m = page_projection(0.02, (2, -3, 5), [-10., 10.]);
        let pw = page_world(0.02, 2);
        // View z = -d where d is the distance along the light: near d → depth 0.
        let lo = m.project_point3(Vec3::new(-3. * pw, 5. * pw, 10.));
        let hi = m.project_point3(Vec3::new(-2. * pw, 6. * pw, -10.));
        assert!((lo - Vec3::new(-1., -1., 0.)).length() < 1e-4, "{lo}");
        assert!((hi - Vec3::new(1., 1., 1.)).length() < 1e-4, "{hi}");
    }
}
