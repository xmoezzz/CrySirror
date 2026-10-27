// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Device-free integer region identifiers for interaction maps, separate from
//! visible pixels. Last fill wins; zero is the empty region. No private VM painter.
use crate::{Error, Result};
use alloc::vec::Vec;
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
