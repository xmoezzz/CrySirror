//! Native system.dat preference ABI (40EB40/40C1C0), distinct from save slots.
//! Buffers contain only game values; file/device pointers never enter this type.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
pub const SETTINGS_BYTES: usize = 0x4b80;
pub const SYSTEM_BYTES: usize = SETTINGS_BYTES + 0x7f8;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "Vec<u8>", into = "Vec<u8>")]
pub struct SystemData(Vec<u8>);
impl TryFrom<Vec<u8>> for SystemData {
    type Error = anyhow::Error;
    fn try_from(bytes: Vec<u8>) -> Result<Self> {
        ensure!(
            bytes.len() == SYSTEM_BYTES,
            "Circus system.dat requires exactly {SYSTEM_BYTES} bytes"
        );
        Ok(Self(bytes))
    }
}
impl From<SystemData> for Vec<u8> {
    fn from(data: SystemData) -> Self {
        data.0
    }
}
impl Default for SystemData {
    fn default() -> Self {
        // 40E9C0 copies the native numeric defaults (46 dwords) and two
        // CP932 name buffers. The remaining19KB are zero-initialized data.
        const DEFAULTS: [i32; 46] = [
            0, 3, 3, 3, 0, 1, 50, 7, 1, 1, 1, 6, 0, 1, 0, 60, 1, 70, 1, 100, 1, 0, 80, 0, 1, 1, 1,
            0, 0, 80, 0, 1, 100, 1, 1, 0, 1, 1, 5, 0, 1, 0, 3, 1, 1, 1,
        ];
        let mut result = Self(vec![0; SYSTEM_BYTES]);
        for (index, value) in DEFAULTS.into_iter().enumerate() {
            result.set_dword(index, value);
        }
        result
            .set_name(184, "常坂")
            .expect("bounded native default name");
        result
            .set_name(200, "一登")
            .expect("bounded native default name");
        for (index, value) in [(92, 0), (93, 5), (94, 2), (95, 4)] {
            result.set_dword(index, value);
        }
        // 40E1D0 initializes the retained audio-source entry50 before load.
        result.0[SETTINGS_BYTES + 2000..SETTINGS_BYTES + 2004].copy_from_slice(&1i32.to_le_bytes());
        result.0[SETTINGS_BYTES + 2004..SETTINGS_BYTES + 2008]
            .copy_from_slice(&100i32.to_le_bytes());
        result
    }
}
impl SystemData {
    /// 40E9C0(reset_all=1) replaces the19KB preference block. The separate
    /// 2040-byte source-entry block is retained when40EB40 cannot open a file.
    pub(crate) fn reset_preferences(&mut self) {
        let defaults = Self::default();
        self.0[..SETTINGS_BYTES].copy_from_slice(&defaults.0[..SETTINGS_BYTES]);
    }
    /// Merge a native read-label interval and encode it in the original four
    /// inline ranges / shared500-range overflow layout (4030F0/402430).
    pub(crate) fn mark_read(&mut self, scene: usize, start: u16, end: u16) -> Result<()> {
        ensure!(
            scene < 650 && start > 0 && end >= start,
            "Circus native read-label interval"
        );
        let mut records = Vec::with_capacity(650);
        for id in 0..650 {
            let record = 2636 + id * 20;
            let pointer = u32::from_le_bytes(self.0[record..record + 4].try_into()?) as usize;
            ensure!(pointer < 500, "Circus read-label overflow pointer");
            let (at, max) = if pointer == 0 {
                (record + 4, 4)
            } else {
                (15640 + pointer * 4, 500 - pointer)
            };
            let mut ranges = Vec::new();
            for n in 0..max {
                let lo = u16::from_le_bytes(self.0[at + n * 4..at + n * 4 + 2].try_into()?);
                let hi = u16::from_le_bytes(self.0[at + n * 4 + 2..at + n * 4 + 4].try_into()?);
                if lo == 0 {
                    break;
                }
                ensure!(hi >= lo, "Circus native read-label range");
                ranges.push((lo, hi));
            }
            records.push(ranges);
        }
        records[scene].push((start, end));
        records[scene].sort_unstable();
        let mut merged: Vec<(u16, u16)> = Vec::new();
        for (lo, hi) in records[scene].drain(..) {
            if let Some(last) = merged
                .last_mut()
                .filter(|last| u32::from(lo) <= u32::from(last.1) + 1)
            {
                last.1 = last.1.max(hi)
            } else {
                merged.push((lo, hi));
            }
        }
        records[scene] = merged;
        let required: usize = records
            .iter()
            .filter(|r| r.len() > 4)
            .map(|r| r.len() + 1)
            .sum();
        ensure!(
            required < 500,
            "Circus native read-label overflow exhausted"
        );
        // Repack only the known interval directory/pool. Other preference
        // bytes, including imported unknown fields, remain byte-for-byte.
        self.0[2636..17640].fill(0);
        let mut next = 1;
        for (id, ranges) in records.into_iter().enumerate() {
            let record = 2636 + id * 20;
            let at = if ranges.len() <= 4 {
                record + 4
            } else {
                self.0[record..record + 4].copy_from_slice(&(next as u32).to_le_bytes());
                let at = 15640 + next * 4;
                next += ranges.len() + 1;
                at
            };
            for (n, (lo, hi)) in ranges.into_iter().enumerate() {
                self.0[at + n * 4..at + n * 4 + 2].copy_from_slice(&lo.to_le_bytes());
                self.0[at + n * 4 + 2..at + n * 4 + 4].copy_from_slice(&hi.to_le_bytes());
            }
        }
        //402430 maintains the first free pool index in the preceding dword.
        self.0[15636..15640]
            .copy_from_slice(&(if next == 1 { 0u32 } else { next as u32 }).to_le_bytes());
        Ok(())
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
    pub(crate) fn dword(&self, index: usize) -> i32 {
        i32::from_le_bytes(
            self.0[index * 4..index * 4 + 4]
                .try_into()
                .expect("native settings dword bounds"),
        )
    }
    fn set_dword(&mut self, index: usize, value: i32) {
        self.0[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    pub(crate) fn names(&self) -> Result<[String; 2]> {
        Ok([self.name(184)?, self.name(200)?])
    }
    fn name(&self, offset: usize) -> Result<String> {
        let raw = &self.0[offset..offset + 16];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let (value, error) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(&raw[..end]);
        ensure!(!error, "Circus native preference name encoding");
        Ok(value.into_owned())
    }
    fn set_name(&mut self, offset: usize, value: &str) -> Result<()> {
        let (bytes, _, error) = encoding_rs::SHIFT_JIS.encode(value);
        ensure!(
            !error && bytes.len() < 16 && !value.contains('\0'),
            "Circus native preference name extent/encoding"
        );
        self.0[offset..offset + 16].fill(0);
        self.0[offset..offset + bytes.len()].copy_from_slice(&bytes);
        Ok(())
    }
    pub(crate) fn set_names(&mut self, names: &[String; 2]) -> Result<()> {
        self.set_name(184, &names[0])?;
        self.set_name(200, &names[1])
    }
    pub(crate) fn byte(&self, offset: usize) -> u8 {
        self.0[offset]
    }
    pub(crate) fn set_byte(&mut self, offset: usize, value: u8) {
        self.0[offset] = value;
    }
    pub(crate) fn value(&self, selector: u8) -> Result<Option<i32>> {
        if selector == 11 {
            return Ok(Some(99i32.wrapping_sub(self.dword(6))));
        }
        if selector == 217 {
            let index = self.dword(4831);
            ensure!(
                (0..20).contains(&index),
                "Circus native preference indexed selector"
            );
            return Ok(Some(self.dword(4410 + index as usize)));
        }
        Ok(prefix_index(selector).map(|i| self.dword(i)))
    }
    pub(crate) fn assign(&mut self, selector: u8, value: i32) -> Result<bool> {
        if selector == 11 {
            self.set_dword(6, 99i32.wrapping_sub(value));
            return Ok(true);
        }
        if selector == 217 {
            let index = self.dword(4831);
            ensure!(
                (0..20).contains(&index),
                "Circus native preference indexed setter"
            );
            self.set_dword(4410 + index as usize, value);
            return Ok(true);
        }
        if let Some(index) = prefix_index(selector) {
            self.set_dword(index, value);
            return Ok(true);
        }
        Ok(false)
    }
}
// Original40CC50 getter and40D240 setter offsets, both in native dwords.
fn prefix_index(selector: u8) -> Option<usize> {
    Some(match selector {
        5 => 18,
        7 => 15,
        8 => 17,
        9 => 19,
        10 => 9,
        12 => 11,
        13 => 4,
        14 => 13,
        15 => 16,
        16 => 7,
        17 => 12,
        18 => 10,
        26 => 1,
        27 => 2,
        28 => 5,
        29 => 3,
        30 => 8,
        180 => 45,
        181 => 44,
        182 => 43,
        184 => 42,
        185 => 41,
        188 => 40,
        190 => 30,
        191 => 39,
        194 => 38,
        196 => 95,
        197 => 94,
        198 => 93,
        199 => 92,
        200 => 37,
        201 => 36,
        202 => 35,
        203 => 31,
        205 => 33,
        206 => 29,
        207 => 22,
        208 => 23,
        209 => 28,
        210 => 27,
        211 => 26,
        212 => 25,
        213 => 24,
        214 => 21,
        218 => 4831,
        221 => 4830,
        225 => 32,
        227 => 34,
        237 => 20,
        _ => return None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_defaults_and_aliases_are_exact() {
        let data = SystemData::default();
        assert_eq!(data.bytes().len(), SYSTEM_BYTES);
        assert_eq!(data.value(11).unwrap(), Some(49));
        assert_eq!(data.value(16).unwrap(), Some(7));
        assert_eq!(data.value(27).unwrap(), Some(3));
        assert_eq!(data.byte(265), 0);
        assert_eq!(data.names().unwrap(), ["常坂", "一登"]);
    }
    #[test]
    fn native_roundtrip_preserves_unknown_bytes_and_rejects_truncation() {
        let mut data = SystemData::default();
        data.0[18000] = 73;
        data.0[SYSTEM_BYTES - 1] = 19;
        data.assign(11, 37).unwrap();
        let bytes = data.bytes().to_vec();
        let restored = SystemData::try_from(bytes.clone()).unwrap();
        assert_eq!(restored.bytes(), bytes);
        assert_eq!(restored.value(11).unwrap(), Some(37));
        assert!(SystemData::try_from(bytes[..SYSTEM_BYTES - 1].to_vec()).is_err());
    }
    #[test]
    fn native_read_ranges_merge_and_retain_original_overflow_allocator_header() {
        let mut data = SystemData::default();
        for label in [1, 3, 5, 7, 9, 11] {
            data.mark_read(12, label, label).unwrap();
        }
        assert!(data.dword(3909) > 1); //15636 free index, used by native402430.
        let restored = SystemData::try_from(data.bytes().to_vec()).unwrap();
        data.mark_read(12, 2, 10).unwrap();
        assert_eq!(data.dword(3909), 0);
        let at = 2636 + 12 * 20;
        assert_eq!(&data.bytes()[at + 4..at + 8], &[1, 0, 11, 0]);
        assert!(restored.dword(3909) > 1);
        assert!(data.mark_read(650, 1, 1).is_err());
    }
}
