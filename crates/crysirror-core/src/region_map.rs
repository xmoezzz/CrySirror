// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Client-owned integer interaction maps, independent of visible colour pixels.
//! Resident immutable label planes and bounded overlays share the same copy,
//! resize and reflection policy. Transforming a plane moves views, not pixels.
use crate::{Error, Result};
use alloc::{sync::Arc, vec::Vec};
/// A resident immutable integer-label field. Sampling must neither decode nor
/// read a device/file. Providers retain their allocation charge until drop.
pub trait RegionPlane: Send + Sync + core::fmt::Debug {
    fn client(&self) -> u32;
    fn size(&self) -> [u32; 2];
    fn value_at(&self, point: [u32; 2]) -> Result<Option<u32>>;
}
#[derive(Clone, Debug)]
enum Paint {
    Fill(u32),
    Plane {
        resource: Arc<dyn RegionPlane>,
        origin: [i64; 2],
        step: [i64; 2],
    },
}
#[derive(Clone, Debug)]
struct Operation {
    rect: [i64; 4],
    paint: Paint,
}
impl Operation {
    fn crop_move(&self, rect: [i64; 4], at: [i64; 2]) -> Self {
        let [l, t, r, b] = rect;
        let mut paint = self.paint.clone();
        if let Paint::Plane { origin, step, .. } = &mut paint {
            origin[0] += (l - self.rect[0]) * step[0];
            origin[1] += (t - self.rect[1]) * step[1];
        }
        Self {
            rect: [at[0], at[1], at[0] + r - l, at[1] + b - t],
            paint,
        }
    }
}
/// Half-open rectangle with overflow-safe sums; empty extents cover nothing.
pub fn contains_rect(rect: [i64; 4], point: [i64; 2]) -> bool {
    let [x, y, w, h] = rect.map(i128::from);
    let [px, py] = point.map(i128::from);
    w > 0 && h > 0 && px >= x && py >= y && px < x + w && py < y + h
}
#[derive(Clone, Debug)]
pub struct RegionMap {
    client: u32,
    size: [u32; 2],
    operations: Vec<Operation>,
}

// A sparse interaction map may span a wide sprite strip. Preserve the
// previous maximum area as dense label reads can materialize the whole map.
fn valid_map_size([w, h]: [u32; 2]) -> bool {
    (1..=16_384).contains(&w) && (1..=16_384).contains(&h)
        && u64::from(w) * u64::from(h) <= 8192 * 8192
}

impl RegionMap {
    pub fn new(client: u32, size: [u32; 2]) -> Result<Self> {
        if client == 0 || !valid_map_size(size) {
            return Err(Error::Invalid("region map identity/dimensions"));
        }
        Ok(Self {
            client,
            size,
            operations: Vec::new(),
        })
    }
    pub fn client(&self) -> u32 {
        self.client
    }
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn from_plane(client: u32, plane: Arc<dyn RegionPlane>) -> Result<Self> {
        if plane.client() != client {
            return Err(Error::Invalid("foreign region plane"));
        }
        let size = plane.size();
        let mut map = Self::new(client, size)?;
        if u64::from(size[0]) * u64::from(size[1]) > 16 * 1024 * 1024 {
            return Err(Error::Limit("region plane pixel budget"));
        }
        map.operations.push(Operation {
            rect: [0, 0, size[0].into(), size[1].into()],
            paint: Paint::Plane {
                resource: plane,
                origin: [0, 0],
                step: [1, 1],
            },
        });
        Ok(map)
    }
    /// Row-major labels of a rectangle inside the map.
    pub fn labels(&self, client: u32, rect: [u32; 4]) -> Result<Vec<u32>> {
        let [x, y, w, h] = rect;
        if x.checked_add(w).is_none_or(|r| r > self.size[0]) || y.checked_add(h).is_none_or(|b| b > self.size[1]) {
            return Err(Error::Invalid("region label rectangle"));
        }
        let mut out = Vec::with_capacity(w as usize * h as usize);
        for py in y..y + h {
            for px in x..x + w {
                out.push(self.get(client, [px.into(), py.into()])?);
            }
        }
        Ok(out)
    }
    /// Paint a whole plane with its top-left at `at` (clipped to the map).
    pub fn paint_plane(&mut self, client: u32, at: [i64; 2], plane: Arc<dyn RegionPlane>) -> Result<()> {
        if client != self.client || plane.client() != client {
            return Err(Error::Invalid("foreign region plane"));
        }
        let size = plane.size();
        let [l, t] = [at[0].max(0), at[1].max(0)];
        let r = (at[0] + i64::from(size[0])).min(self.size[0].into());
        let b = (at[1] + i64::from(size[1])).min(self.size[1].into());
        if r <= l || b <= t {
            return Ok(());
        }
        if self.operations.len() >= 4096 {
            // Flatten so long sequences of plane paints stay within budget.
            let all = self.labels(client, [0, 0, self.size[0], self.size[1]])?;
            let flat = LabelPlane::new(client, self.size, all.iter().map(|v| *v as u8).collect())?;
            *self = Self::from_plane(client, Arc::new(flat))?;
        }
        self.push(Operation {
            rect: [l, t, r, b],
            paint: Paint::Plane { resource: plane, origin: [l - at[0], t - at[1]], step: [1, 1] },
        })
    }
    fn push(&mut self, operation: Operation) -> Result<()> {
        if self.operations.len() >= 4096 {
            return Err(Error::Limit("region map fill budget"));
        }
        self.operations.push(operation);
        Ok(())
    }
    pub fn fill(&mut self, client: u32, rect: [i64; 4], region: u32) -> Result<()> {
        if client != self.client {
            return Err(Error::Invalid("foreign region map"));
        }
        let [x, y, w, h] = rect;
        if rect.iter().any(|v| v.unsigned_abs() > 1_000_000) || w < 0 || h < 0 {
            return Err(Error::Invalid("region map rectangle"));
        }
        let [l, t, r, b] = [
            x.max(0),
            y.max(0),
            (x + w).min(self.size[0].into()),
            (y + h).min(self.size[1].into()),
        ];
        if r <= l || b <= t {
            return Ok(());
        }
        let full = [l, t, r, b] == [0, 0, self.size[0].into(), self.size[1].into()];
        if full {
            self.operations.clear();
            if region == 0 {
                return Ok(());
            }
        }
        self.push(Operation {
            rect: [l, t, r, b],
            paint: Paint::Fill(region),
        })
    }
    pub fn flip(&mut self, client: u32, horizontal: bool, vertical: bool) -> Result<()> {
        if client != self.client {
            return Err(Error::Invalid("foreign region map"));
        }
        for op in &mut self.operations {
            let [l, t, r, b] = op.rect;
            if let Paint::Plane { origin, step, .. } = &mut op.paint {
                if horizontal {
                    origin[0] += (r - l - 1) * step[0];
                    step[0] = -step[0];
                }
                if vertical {
                    origin[1] += (b - t - 1) * step[1];
                    step[1] = -step[1];
                }
            }
            if horizontal {
                op.rect[0] = i64::from(self.size[0]) - r;
                op.rect[2] = i64::from(self.size[0]) - l;
            }
            if vertical {
                op.rect[1] = i64::from(self.size[1]) - b;
                op.rect[3] = i64::from(self.size[1]) - t;
            }
        }
        Ok(())
    }
    pub fn get(&self, client: u32, point: [i64; 2]) -> Result<u32> {
        if client != self.client {
            return Err(Error::Invalid("foreign region map"));
        }
        let [x, y] = point;
        if x < 0 || y < 0 || x >= i64::from(self.size[0]) || y >= i64::from(self.size[1]) {
            return Ok(0);
        }
        for op in self.operations.iter().rev() {
            let [l, t, r, b] = op.rect;
            if x < l || x >= r || y < t || y >= b {
                continue;
            }
            return match &op.paint {
                Paint::Fill(value) => Ok(*value),
                Paint::Plane {
                    resource,
                    origin,
                    step,
                } => {
                    let sx = origin[0] + (x - l) * step[0];
                    let sy = origin[1] + (y - t) * step[1];
                    let sx = u32::try_from(sx)
                        .map_err(|_| Error::Invalid("region plane view bounds"))?;
                    let sy = u32::try_from(sy)
                        .map_err(|_| Error::Invalid("region plane view bounds"))?;
                    resource
                        .value_at([sx, sy])?
                        .ok_or(Error::Invalid("region plane view bounds"))
                }
            };
        }
        Ok(0)
    }
    pub fn resize(&mut self, client: u32, size: [u32; 2]) -> Result<()> {
        if client != self.client {
            return Err(Error::Invalid("foreign region map"));
        }
        let mut next = Self::new(client, size)?;
        for op in &self.operations {
            let [l, t, r, b] = op.rect;
            let r = r.min(size[0].into());
            let b = b.min(size[1].into());
            if r > l && b > t {
                next.push(op.crop_move([l, t, r, b], [l, t]))?;
            }
        }
        *self = next;
        Ok(())
    }
    /// Snapshot copy includes zero labels; overlap does not alias later writes.
    /// Scope/clip/operation budget errors leave the destination unchanged.
    pub fn copy_from(
        &mut self,
        client: u32,
        source: &Self,
        rect: [i64; 4],
        at: [i64; 2],
        clip: Option<[i64; 4]>,
    ) -> Result<bool> {
        if client != self.client || client != source.client {
            return Err(Error::Invalid("foreign region copy"));
        }
        let Some(([sx, sy, w, h], [dx, dy])) =
            clip_region_copy(Some(source.size), self.size, rect, at, clip)?
        else {
            return Ok(false);
        };
        let mut next = self.clone();
        next.fill(client, [dx, dy, w, h], 0)?;
        for op in &source.operations {
            let [l, t, r, b] = op.rect;
            let l = l.max(sx);
            let t = t.max(sy);
            let r = r.min(sx + w);
            let b = b.min(sy + h);
            if r > l && b > t {
                next.push(op.crop_move([l, t, r, b], [dx + l - sx, dy + t - sy]))?;
            }
        }
        *self = next;
        Ok(true)
    }
}
/// Clip a copy while preserving source-to-destination coordinate correspondence.
pub fn clip_region_copy(
    source_size: Option<[u32; 2]>,
    destination_size: [u32; 2],
    source: [i64; 4],
    destination: [i64; 2],
    clip: Option<[i64; 4]>,
) -> Result<Option<([i64; 4], [i64; 2])>> {
    if !valid_map_size(destination_size)
        || source_size.is_some_and(|size| !valid_map_size(size))
        || source
            .iter()
            .chain(destination.iter())
            .any(|v| v.unsigned_abs() > 1_000_000)
        || source[2] < 0
        || source[3] < 0
    {
        return Err(Error::Invalid("region copy bounds"));
    }
    let [sx, sy, w, h] = source;
    let [dx, dy] = destination;
    let mut l = dx.max(0);
    let mut t = dy.max(0);
    let mut r = (dx + w).min(destination_size[0].into());
    let mut b = (dy + h).min(destination_size[1].into());
    if let Some(c) = clip {
        if c.iter().any(|v| v.unsigned_abs() > 1_000_000) || c[2] < 0 || c[3] < 0 {
            return Err(Error::Invalid("region copy clip"));
        }
        l = l.max(c[0]);
        t = t.max(c[1]);
        r = r.min(c[0] + c[2]);
        b = b.min(c[1] + c[3]);
    }
    if let Some([sw, sh]) = source_size {
        l = l.max(dx - sx);
        t = t.max(dy - sy);
        r = r.min(dx - sx + i64::from(sw));
        b = b.min(dy - sy + i64::from(sh));
    }
    if r <= l || b <= t {
        return Ok(None);
    }
    Ok(Some(([sx + l - dx, sy + t - dy, r - l, b - t], [l, t])))
}
/// Worker result for encoded label images. Ready retains decoded memory charges.
#[derive(Clone, Debug)]
pub enum RegionMapState {
    Pending,
    Ready(RegionMap),
    Failed(Error),
}
pub trait RegionMapQuery: Send {
    fn client(&self) -> u32;
    fn poll(&mut self) -> Result<RegionMapState>;
}

#[cfg(test)]
mod copy_tests {
    use super::*;
    #[test]
    fn narrow_strips_accept_wide_coordinates_and_retain_the_area_limit() {
        let mut source = RegionMap::new(1, [9984, 100]).unwrap();
        source.fill(1, [9983, 0, 1, 1], 47).unwrap();
        let mut cell = RegionMap::new(1, [1, 1]).unwrap();
        cell.copy_from(1, &source, [9983, 0, 1, 1], [0, 0], None).unwrap();
        assert_eq!(cell.get(1, [0, 0]).unwrap(), 47);
        assert!(source.resize(1, [16_384, 8192]).is_err());
        assert_eq!(source.size(), [9984, 100]);
    }

    #[test]
    fn scoped_region_copy_clips_preserves_zero_holes_and_snapshots_overlap() {
        let mut source = RegionMap::new(1, [4, 2]).unwrap();
        source.fill(1, [0, 0, 4, 2], 7).unwrap();
        source.fill(1, [1, 0, 1, 2], 0).unwrap();
        let mut dst = RegionMap::new(1, [6, 2]).unwrap();
        dst.fill(1, [0, 0, 6, 2], 9).unwrap();
        dst.copy_from(1, &source, [0, 0, 4, 2], [1, 0], Some([2, 0, 3, 2]))
            .unwrap();
        let got: Vec<_> = (0..6).map(|x| dst.get(1, [x, 0]).unwrap()).collect();
        assert_eq!(got, alloc::vec![9, 9, 0, 7, 7, 9]);
        let old = dst.clone();
        dst.copy_from(1, &old, [0, 0, 5, 2], [1, 0], None).unwrap();
        assert_eq!(
            (0..6)
                .map(|x| dst.get(1, [x, 0]).unwrap())
                .collect::<Vec<_>>(),
            alloc::vec![9, 9, 9, 0, 7, 7]
        );
        assert!(
            dst.copy_from(2, &source, [0, 0, 1, 1], [0, 0], None)
                .is_err()
        );
        assert!(
            dst.copy_from(
                1,
                &RegionMap::new(2, [4, 2]).unwrap(),
                [0, 0, 1, 1],
                [0, 0],
                None
            )
            .is_err()
        );
        dst.resize(1, [8, 3]).unwrap();
        assert_eq!(dst.get(1, [7, 2]).unwrap(), 0);
        assert_eq!(dst.get(1, [4, 0]).unwrap(), 7);
    }
    #[test]
    fn region_copy_budget_and_bad_coordinates_do_not_mutate_destination() {
        let mut dst = RegionMap::new(1, [4, 2]).unwrap();
        for _ in 0..4096 {
            dst.fill(1, [0, 0, 1, 1], 9).unwrap();
        }
        let source = RegionMap::new(1, [4, 2]).unwrap();
        assert!(
            dst.copy_from(1, &source, [0, 0, 1, 1], [2, 0], None)
                .is_err()
        );
        assert_eq!(dst.get(1, [0, 0]).unwrap(), 9);
        assert_eq!(dst.get(1, [2, 0]).unwrap(), 0);
        assert!(clip_region_copy(None, [4, 2], [i64::MAX, 0, 1, 1], [0, 0], None).is_err());
        assert!(clip_region_copy(None, [4, 2], [0, 0, 1, 1], [0, 0], Some([0, 0, -1, 1])).is_err());
    }
}

/// Owned row-major byte labels (KiriKiri province images are 8-bit).
#[derive(Debug)]
pub struct LabelPlane {
    client: u32,
    size: [u32; 2],
    labels: Vec<u8>,
}
impl LabelPlane {
    pub fn new(client: u32, size: [u32; 2], labels: Vec<u8>) -> Result<Self> {
        if size[0] == 0 || size[1] == 0 || labels.len() as u64 != u64::from(size[0]) * u64::from(size[1]) {
            return Err(Error::Invalid("label plane layout"));
        }
        Ok(Self { client, size, labels })
    }
}
impl RegionPlane for LabelPlane {
    fn client(&self) -> u32 {
        self.client
    }
    fn size(&self) -> [u32; 2] {
        self.size
    }
    fn value_at(&self, point: [u32; 2]) -> Result<Option<u32>> {
        let [x, y] = point;
        Ok((x < self.size[0] && y < self.size[1]).then(|| u32::from(self.labels[(y * self.size[0] + x) as usize])))
    }
}
/// layerExBTOA `copyAlphaToProvince(threshold, matched, otherwise)` over one
/// clip rectangle: `alpha` and `old` are row-major, the result replaces
/// `old`. `None` means the call changes nothing.
pub fn alpha_to_labels(alpha: &[u8], old: &[u32], threshold: Option<i64>, matched: i64, otherwise: i64) -> Option<Vec<u8>> {
    let byte = |v: i64| (0..256).contains(&v);
    let (th, on, off) = (threshold.unwrap_or(-1) as u8, matched as u8, otherwise as u8);
    let mode = match threshold {
        Some(t) if (0..256).contains(&t) => match (byte(matched), byte(otherwise)) {
            (false, false) => return None,
            (true, true) => 1,
            (true, false) => 3,
            (false, true) => 4,
        },
        Some(t) if t >= 256 => {
            if byte(otherwise) { 2 } else { return None }
        }
        _ => 0,
    };
    Some(
        alpha
            .iter()
            .zip(old)
            .map(|(a, o)| match mode {
                0 => *a,
                1 => if *a >= th { on } else { off },
                2 => off,
                3 => if *a >= th { on } else { *o as u8 },
                _ => if *a < th { off } else { *o as u8 },
            })
            .collect(),
    )
}
