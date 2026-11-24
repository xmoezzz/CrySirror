// SPDX-License-Identifier: MPL-2.0
//! Compressed triangle indices of background mesh fragments (PAK 0xa79).
//!
//! Layout, all big-endian, bit streams most-significant bit first:
//! - header: index bit width, explicit reference count `E`, emission count
//!   `T`, zero. `T − E` new vertices are numbered from zero; most fragments
//!   have `T = E + vertex count`, while some reach nearly every vertex
//!   through explicit references (which may then name later vertices);
//! - `E` explicit vertex references of `width` bits, padded to 16 bits;
//! - one 2-bit code per triangle, padded to 16 bits. Code 3 starts a new
//!   triangle from three emissions. Code 0 forms `(p0, p2, v)` and code 1
//!   forms `(p2, p1, v)` from the previous triangle `p` and one emission.
//!   Code 2 is not used;
//! - `T` emission flags: 0 takes the next new vertex, 1 the next explicit
//!   reference.
//!
//! The rules were derived from the installed menu-temple background and
//! checked against every fragment's counts and reference order.
use anyhow::{Context, Result, bail, ensure};

struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn get(&mut self, n: u32) -> Result<u32> {
        let mut v = 0;
        for _ in 0..n {
            let byte = *self.data.get(self.at >> 3).context("short ND index stream")?;
            v = v << 1 | u32::from(byte >> (7 - (self.at & 7)) & 1);
            self.at += 1;
        }
        Ok(v)
    }
    fn align16(&mut self) {
        self.at = self.at.div_ceil(16) * 16;
    }
}

/// Decode `triangles` triangles referring to `vertices` vertices.
pub fn decode_indices(blob: &[u8], triangles: usize, vertices: usize) -> Result<Vec<[u16; 3]>> {
    ensure!(blob.len() >= 8 && triangles <= 0x10000 && vertices <= 0x10000, "ND index header");
    let word = |i: usize| usize::from(u16::from_be_bytes([blob[2 * i], blob[2 * i + 1]]));
    let (width, explicit, emissions) = (word(0) as u32, word(1), word(2));
    ensure!((1..=16).contains(&width) && emissions >= explicit && emissions - explicit <= vertices, "ND index counts");
    let mut bits = Bits { data: blob, at: 64 };
    let refs = (0..explicit).map(|_| bits.get(width)).collect::<Result<Vec<_>>>()?;
    bits.align16();
    let codes = (0..triangles).map(|_| bits.get(2)).collect::<Result<Vec<_>>>()?;
    bits.align16();
    let (mut next_new, mut next_ref, mut used) = (0u32, 0, 0);
    let mut emit = |bits: &mut Bits| -> Result<u16> {
        used += 1;
        ensure!(used <= emissions, "ND index emissions exceed header");
        if bits.get(1)? == 0 {
            next_new += 1;
            ensure!(next_new as usize <= emissions - explicit, "ND index new vertex beyond count");
            return Ok((next_new - 1) as u16);
        }
        let v = *refs.get(next_ref).context("ND index explicit references exhausted")?;
        next_ref += 1;
        ensure!((v as usize) < vertices, "ND index reference beyond the vertex count");
        Ok(v as u16)
    };
    let mut out: Vec<[u16; 3]> = Vec::with_capacity(triangles);
    for code in codes {
        let t = match (code, out.last()) {
            (3, _) => [emit(&mut bits)?, emit(&mut bits)?, emit(&mut bits)?],
            (0, Some(p)) => [p[0], p[2], emit(&mut bits)?],
            (1, Some(p)) => [p[2], p[1], emit(&mut bits)?],
            (c, _) => bail!("ND index code {c} without a previous triangle"),
        };
        out.push(t);
    }
    ensure!(used == emissions && next_ref == explicit, "ND index stream not fully consumed");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pack MSB-first fields.
    fn pack(fields: &[(u32, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0usize;
        for &(v, n) in fields {
            for i in (0..n).rev() {
                if at % 8 == 0 {
                    out.push(0);
                }
                *out.last_mut().unwrap() |= ((v >> i & 1) as u8) << (7 - at % 8);
                at += 1;
            }
        }
        out
    }

    #[test]
    fn restart_and_shared_edges() {
        // 4 vertices, 3 triangles: restart (0,1,2), code 0 → (0,2,3),
        // code 1 → (3,2,ref 1). One explicit reference to vertex 1.
        let mut f = vec![(2, 16), (1, 16), (5, 16), (0, 16), (1, 2)];
        f.push((0, 14)); // pad explicit to 16 bits
        f.extend([(3, 2), (0, 2), (1, 2), (0, 10)]);
        f.extend([(0, 1), (0, 1), (0, 1), (0, 1), (1, 1)]);
        let blob = pack(&f);
        assert_eq!(decode_indices(&blob, 3, 4).unwrap(), [[0, 1, 2], [0, 2, 3], [3, 2, 1]]);
        // A reference beyond the vertex count is rejected.
        let mut bad = f.clone();
        bad[4] = (3, 2);
        assert!(decode_indices(&pack(&bad), 3, 3).is_err());
        // A shared-edge code cannot start the stream.
        let mut first = f.clone();
        first[6] = (0, 2);
        assert!(decode_indices(&pack(&first), 3, 4).is_err());
    }
}
