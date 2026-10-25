// SPDX-License-Identifier: MPL-2.0
//! Device-free topology decoding. Preserve encoded winding; conversion is the
//! importer's responsibility. EDGE profile: 8-lane deltas, flags, 2-bit triangles.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexEncoding {
    U16Le,
    U16Be,
    Edge,
}
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Bits<'_> {
    fn get(&mut self, width: usize) -> Result<u32> {
        ensure!(
            width <= 16 && self.at + width <= self.bytes.len() * 8,
            "truncated index bitstream"
        );
        let mut n = 0;
        for _ in 0..width {
            n = (n << 1) | u32::from((self.bytes[self.at / 8] >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Ok(n)
    }
}
pub fn decode(
    data: &[u8],
    encoding: IndexEncoding,
    index_count: usize,
    vertex_count: usize,
) -> Result<Vec<u32>> {
    ensure!(
        index_count > 0
            && index_count <= 3_000_000
            && index_count % 3 == 0
            && vertex_count > 0
            && vertex_count <= 65535,
        "index count/vertex range"
    );
    let values = match encoding {
        IndexEncoding::U16Le | IndexEncoding::U16Be => {
            ensure!(data.len() == index_count * 2, "raw index buffer length");
            data.chunks_exact(2)
                .map(|b| {
                    u32::from(if matches!(encoding, IndexEncoding::U16Le) {
                        u16::from_le_bytes([b[0], b[1]])
                    } else {
                        u16::from_be_bytes([b[0], b[1]])
                    })
                })
                .collect()
        }
        IndexEncoding::Edge => edge(data, index_count, vertex_count)?,
    };
    ensure!(
        values.iter().all(|&i| (i as usize) < vertex_count),
        "decoded index outside vertex stream"
    );
    Ok(values)
}
fn edge(data: &[u8], index_count: usize, vertex_count: usize) -> Result<Vec<u32>> {
    ensure!(data.len() >= 8 && index_count <= 65535, "EDGE header/count");
    let u16at = |o| u16::from_be_bytes([data[o], data[o + 1]]) as usize;
    let reused = u16at(0);
    let bias = u16at(2) as i32;
    let masks = u16at(4);
    let width = data[6] as usize;
    ensure!(
        data[7] == 0 && width <= 16 && (width > 0 || reused == 0),
        "unsupported EDGE version/width"
    );
    let codes = (index_count / 3 + 3) / 4;
    let packed = (reused * width + 7) / 8;
    let used = 8 + masks + codes + packed;
    ensure!(
        used <= data.len() && data.len() <= (used + 15) & !15,
        "EDGE stream size/padding"
    );
    let mut flags = Bits {
        bytes: &data[8..8 + masks],
        at: 0,
    };
    let mut ops = Bits {
        bytes: &data[8 + masks..8 + masks + codes],
        at: 0,
    };
    let mut bits = Bits {
        bytes: &data[8 + masks + codes..used],
        at: 0,
    };
    let mut deltas: Vec<i32> = Vec::with_capacity(reused);
    for i in 0..reused {
        let n = bits.get(width)? as i32 - bias + if i >= 8 { deltas[i - 8] } else { 0 };
        ensure!(
            n >= 0 && (n as usize) < vertex_count,
            "EDGE delta out of range"
        );
        deltas.push(n);
    }
    let (mut cursor, mut sequential) = (0, 0u32);
    let mut vertex = || -> Result<u32> {
        if flags.get(1)? != 0 {
            ensure!(cursor < deltas.len(), "EDGE reuse flags exceed deltas");
            let v = deltas[cursor] as u32;
            cursor += 1;
            Ok(v)
        } else {
            let v = sequential;
            sequential += 1;
            ensure!(
                (v as usize) < vertex_count,
                "EDGE sequential index out of range"
            );
            Ok(v)
        }
    };
    let mut out = Vec::with_capacity(index_count);
    let mut previous = None;
    for _ in 0..index_count / 3 {
        let code = ops.get(2)?;
        let t = if code == 3 {
            [vertex()?, vertex()?, vertex()?]
        } else {
            let p: [u32; 3] =
                previous.ok_or_else(|| anyhow::anyhow!("EDGE edge reuse before first triangle"))?;
            let (a, b) = [(0, 2), (2, 1), (1, 0)][code as usize];
            [p[a], p[b], vertex()?]
        };
        out.extend(t);
        previous = Some(t);
    }
    ensure!(
        cursor == reused && sequential as usize == vertex_count && (flags.at + 7) / 8 == masks,
        "EDGE vertex/stream totals mismatch"
    );
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn topology_modes_deltas_and_endian_have_known_results() {
        // Four triangles, explicit [0,1,2], then each reversed previous edge + next.
        // flags: six sequential vertices. controls: 3,0,1,2 = 0xc6.
        let data = [0, 0, 0, 0, 0, 1, 0, 0, 0, 0xc6];
        assert_eq!(
            decode(&data, IndexEncoding::Edge, 12, 6).unwrap(),
            [0, 1, 2, 0, 2, 3, 3, 2, 4, 2, 3, 5]
        );
        // Reuse [0,2,1] after a first sequential triangle; 2-bit explicit values.
        let delta = [0, 3, 0, 0, 0, 1, 2, 0, 0x1c, 0xf0, 0x24];
        assert_eq!(
            decode(&delta, IndexEncoding::Edge, 6, 3).unwrap(),
            [0, 1, 2, 0, 2, 1]
        );
        // Ninth reuse delta predicts from the first (eight lanes), including a
        // negative difference. This exercises the recurrence, not just packed values.
        let values = [2u32, 1, 0, 2, 1, 0, 2, 1, 0];
        let mut packed = 0u32;
        for (i, v) in values.iter().enumerate() {
            let d = *v as i32 - if i >= 8 { values[i - 8] as i32 } else { 0 } + 2;
            packed |= (d as u32) << (29 - i * 3);
        }
        let mut lanes = vec![0, 9, 0, 2, 0, 2, 3, 0, 0x1f, 0xf0, 0xff];
        lanes.extend(packed.to_be_bytes());
        let mut expected = vec![0, 1, 2];
        expected.extend(values);
        assert_eq!(
            decode(&lanes, IndexEncoding::Edge, 12, 3).unwrap(),
            expected
        );
        assert_eq!(
            decode(&[0, 0, 0, 1, 0, 2], IndexEncoding::U16Be, 3, 3).unwrap(),
            [0, 1, 2]
        );
        assert_eq!(
            decode(&[0, 0, 1, 0, 2, 0], IndexEncoding::U16Le, 3, 3).unwrap(),
            [0, 1, 2]
        );
    }
    #[test]
    fn corrupt_unknown_and_out_of_bounds_streams_fail() {
        for b in [
            vec![],
            vec![0; 8],
            vec![0, 0, 0, 0, 0, 1, 0, 1, 0, 0xc0],
            vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0x00],
            vec![0, 1, 0, 0, 0, 1, 16, 0, 0xff, 0xc0, 0xff, 0xff],
        ] {
            assert!(decode(&b, IndexEncoding::Edge, 3, 3).is_err());
        }
        assert!(decode(&[0, 0, 0, 1, 255, 255], IndexEncoding::U16Be, 3, 3).is_err());
    }
}
