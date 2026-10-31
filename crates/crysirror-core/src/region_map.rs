// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Device-free integer region identifiers for interaction maps, separate from
//! visible pixels. Last fill wins; zero is the empty region. No private VM painter.
use crate::{Error, Result};
use alloc::vec::Vec;
/// Half-open integer rectangle query, with overflow-safe coordinate sums.
/// Negative/empty extents have no coverage. Usable without a VM or device.
pub fn contains_rect(rect: [i64; 4], point: [i64; 2]) -> bool {
    let [x,y,w,h]=rect.map(i128::from);
    let [px,py]=point.map(i128::from);
    w>0 && h>0 && px>=x && py>=y && px<x+w && py<y+h
}
#[derive(Clone, Debug)]
pub struct RegionMap {
    client: u32,
    size: [u32; 2],
    fills: Vec<([i64; 4], u32)>,
}
impl RegionMap {
    pub fn new(client: u32, size: [u32; 2]) -> Result<Self> {
        if client == 0 || size.iter().any(|v| *v == 0 || *v > 8192) {
            return Err(Error::Invalid("region map identity/dimensions"));
        }
        Ok(Self {
            client,
            size,
            fills: Vec::new(),
        })
    }
    pub fn fill(&mut self, client: u32, rect: [i64; 4], region: u32) -> Result<()> {
        if client != self.client {
            return Err(Error::Invalid("foreign region map"));
        }
        let [x, y, w, h] = rect;
        if rect.iter().any(|v| v.unsigned_abs() > 1_000_000) || w < 0 || h < 0 {
            return Err(Error::Invalid("region map rectangle"));
        }
        let left = x.max(0);
        let top = y.max(0);
        let right = (x + w).min(self.size[0] as i64);
        let bottom = (y + h).min(self.size[1] as i64);
        if right <= left || bottom <= top {
            return Ok(());
        }
        let full =
            left == 0 && top == 0 && right == self.size[0] as i64 && bottom == self.size[1] as i64;
        if !full && self.fills.len() >= 4096 {
            return Err(Error::Limit("region map fill budget"));
        }
        if full {
            self.fills.clear();
            if region == 0 {
                return Ok(());
            }
        }
        self.fills.push(([left, top, right, bottom], region));
        Ok(())
    }
    /// Reflect all region rectangles, preserving last-fill precedence.
    pub fn flip(&mut self, client: u32, horizontal: bool, vertical: bool) -> Result<()> {
        if client != self.client {
            return Err(Error::Invalid("foreign region map"));
        }
        for (rect, _) in &mut self.fills {
            let [l, t, r, b] = *rect;
            if horizontal {
                rect[0] = self.size[0] as i64 - r;
                rect[2] = self.size[0] as i64 - l;
            }
            if vertical {
                rect[1] = self.size[1] as i64 - b;
                rect[3] = self.size[1] as i64 - t;
            }
        }
        Ok(())
    }
    pub fn get(&self, client: u32, point: [i64; 2]) -> Result<u32> {
        if client != self.client {
            return Err(Error::Invalid("foreign region map"));
        }
        let [x, y] = point;
        Ok(self
            .fills
            .iter()
            .rev()
            .find(|([l, t, r, b], _)| x >= *l && x < *r && y >= *t && y < *b)
            .map_or(0, |(_, r)| *r))
    }
}


/// Clip a copy to destination bounds/clip, preserving source-to-destination
/// correspondence. Optional source bounds allow callers to represent an absent
/// source plane explicitly. All arithmetic is checked against a fixed domain.
pub fn clip_region_copy(source_size: Option<[u32;2]>, destination_size:[u32;2],
    source:[i64;4], destination:[i64;2], clip:Option<[i64;4]>)
    -> Result<Option<([i64;4],[i64;2])>> {
    if destination_size.iter().any(|v|*v==0||*v>8192)
        || source_size.is_some_and(|s|s.iter().any(|v|*v==0||*v>8192))
        || source.iter().chain(destination.iter()).any(|v|v.unsigned_abs()>1_000_000)
        || source[2]<0 || source[3]<0 {
        return Err(Error::Invalid("region copy bounds"));
    }
    let [sx,sy,w,h]=source;let [dx,dy]=destination;
    let mut l=dx.max(0);let mut t=dy.max(0);
    let mut r=(dx+w).min(destination_size[0].into());
    let mut b=(dy+h).min(destination_size[1].into());
    if let Some(c)=clip {
        if c.iter().any(|v|v.unsigned_abs()>1_000_000)||c[2]<0||c[3]<0 {
            return Err(Error::Invalid("region copy clip"));
        }
        l=l.max(c[0]);t=t.max(c[1]);r=r.min(c[0]+c[2]);b=b.min(c[1]+c[3]);
    }
    if let Some([sw,sh])=source_size {
        l=l.max(dx-sx);t=t.max(dy-sy);
        r=r.min(dx-sx+i64::from(sw));b=b.min(dy-sy+i64::from(sh));
    }
    if r<=l||b<=t {return Ok(None);}
    Ok(Some(([sx+l-dx,sy+t-dy,r-l,b-t],[l,t])))
}
impl RegionMap {
    /// Resize retains covered old coordinates, clips lost pixels and initializes
    /// newly exposed area to zero. Validation precedes all mutation.
    pub fn resize(&mut self,client:u32,size:[u32;2])->Result<()> {
        if client!=self.client {return Err(Error::Invalid("foreign region map"));}
        let mut next=Self::new(client,size)?;
        for ([l,t,r,b],value) in &self.fills {next.fill(client,[*l,*t,r-l,b-t],*value)?;}
        *self=next;Ok(())
    }
    /// Snapshot-copy integer identifiers; zero holes overwrite older labels.
    /// Source/destination scopes, clipping and total fill budget are validated
    /// before commit. Overlap is safe when the caller supplies a source snapshot.
    pub fn copy_from(&mut self,client:u32,source:&Self,rect:[i64;4],at:[i64;2],clip:Option<[i64;4]>)->Result<bool> {
        if client!=self.client||client!=source.client {return Err(Error::Invalid("foreign region copy"));}
        let Some(([sx,sy,w,h],[dx,dy]))=clip_region_copy(Some(source.size),self.size,rect,at,clip)? else{return Ok(false);};
        let mut next=self.clone();
        next.fill(client,[dx,dy,w,h],0)?;
        for ([l,t,r,b],value) in &source.fills {
            let l=(*l).max(sx);let t=(*t).max(sy);let r=(*r).min(sx+w);let b=(*b).min(sy+h);
            if r>l&&b>t {next.fill(client,[dx+l-sx,dy+t-sy,r-l,b-t],*value)?;}
        }
        *self=next;Ok(true)
    }
}
#[cfg(test)]
mod copy_tests {
    use super::*;
    #[test]
    fn scoped_region_copy_clips_preserves_zero_holes_and_snapshots_overlap() {
        let mut source=RegionMap::new(1,[4,2]).unwrap();
        source.fill(1,[0,0,4,2],7).unwrap();source.fill(1,[1,0,1,2],0).unwrap();
        let mut dst=RegionMap::new(1,[6,2]).unwrap();dst.fill(1,[0,0,6,2],9).unwrap();
        dst.copy_from(1,&source,[0,0,4,2],[1,0],Some([2,0,3,2])).unwrap();
        let got:Vec<_>=(0..6).map(|x|dst.get(1,[x,0]).unwrap()).collect();
        assert_eq!(got,alloc::vec![9,9,0,7,7,9]);
        let old=dst.clone();dst.copy_from(1,&old,[0,0,5,2],[1,0],None).unwrap();
        assert_eq!((0..6).map(|x|dst.get(1,[x,0]).unwrap()).collect::<Vec<_>>(),alloc::vec![9,9,9,0,7,7]);
        assert!(dst.copy_from(2,&source,[0,0,1,1],[0,0],None).is_err());
        assert!(dst.copy_from(1,&RegionMap::new(2,[4,2]).unwrap(),[0,0,1,1],[0,0],None).is_err());
        dst.resize(1,[8,3]).unwrap();assert_eq!(dst.get(1,[7,2]).unwrap(),0);
        assert_eq!(dst.get(1,[4,0]).unwrap(),7);
    }
    #[test]
    fn region_copy_budget_and_bad_coordinates_do_not_mutate_destination() {
        let mut dst=RegionMap::new(1,[4,2]).unwrap();
        for _ in 0..4096 {dst.fill(1,[0,0,1,1],9).unwrap();}
        let source=RegionMap::new(1,[4,2]).unwrap();
        assert!(dst.copy_from(1,&source,[0,0,1,1],[2,0],None).is_err());
        assert_eq!(dst.get(1,[0,0]).unwrap(),9);assert_eq!(dst.get(1,[2,0]).unwrap(),0);
        assert!(clip_region_copy(None,[4,2],[i64::MAX,0,1,1],[0,0],None).is_err());
        assert!(clip_region_copy(None,[4,2],[0,0,1,1],[0,0],Some([0,0,-1,1])).is_err());
    }
}
