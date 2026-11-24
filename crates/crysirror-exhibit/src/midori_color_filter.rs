// SPDX-License-Identifier: MPL-2.0
//! Midori character palette state; independent of viewport filter effects.
use anyhow::{Result, ensure};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ColorFilter {
    bgr: u32,
    strength: u16,
}

impl ColorFilter {
    pub fn deferred(raw: &[u8]) -> Result<Self> {
        let fields: Vec<_> = raw.splitn(4, |&b| b == b',').collect();
        ensure!(
            fields.len() == 3 && fields[0] == b"0",
            "unsupported Midori immediate character filter mode"
        );
        let decimal = |field: &[u8]| -> Result<u32> {
            ensure!(
                !field.is_empty() && field.iter().all(u8::is_ascii_digit),
                "unsupported Midori character filter operand"
            );
            Ok(std::str::from_utf8(field)?.parse()?)
        };
        let bgr = decimal(fields[1])?;
        let strength = decimal(fields[2])?;
        ensure!(
            strength <= 256 && bgr <= 0xff_ffff,
            "unsupported Midori character filter range"
        );
        Ok(if strength == 0 {
            Self::default() // the native command clears both manager fields
        } else {
            Self {
                bgr,
                strength: strength as u16,
            }
        })
    }

    // 0x52820/0x3e9d0: skip an already empty register, clear a stale one
    // explicitly, or emit the native unsigned target and signed strength.
    pub fn capture(self, previous: &[u8]) -> Option<(u32, String)> {
        if self.strength == 0 {
            (!previous.is_empty()).then(|| (4, String::new()))
        } else {
            Some((4, format!("0,{},{}", self.bgr, self.strength)))
        }
    }

    // 0x157260 interpolates each BGR palette component with an unsigned
    // 16-bit product and wrapping byte addition. GYU opacity is separate
    // and is not modified. Only the 256-entry indexed path uses this.
    pub fn apply_gyu_palette(self, palette: &mut [u8]) {
        if self.strength == 0 {
            return;
        }
        let target = self.bgr.to_le_bytes();
        for entry in palette.chunks_exact_mut(4) {
            for channel in 0..3 {
                let from = entry[channel];
                let delta = i32::from(target[channel]) - i32::from(from);
                let high = (delta * i32::from(self.strength)) as u16 >> 8;
                entry[channel] = from.wrapping_add(high as u8);
            }
            entry[3] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_keeps_active_filter_or_drops_only_stale_text() -> Result<()> {
        assert_eq!(ColorFilter::default().capture(b""), None);
        let f = ColorFilter::deferred(b"0,16416,48")?;
        assert_eq!(f.capture(b""), Some((4, "0,16416,48".into())));
        let empty = ColorFilter::deferred(b"0,255,0")?;
        assert_eq!(empty.capture(b"old"), Some((4, String::new())));
        let mut p = [255, 33, 0, 111, 0, 0, 0, 222];
        f.apply_gyu_palette(&mut p);
        assert_eq!(p, [213, 38, 0, 0, 6, 12, 0, 0]);
        assert!(ColorFilter::deferred(b"1,3145728,50").is_err());
        assert!(ColorFilter::deferred(b"0,R1,48").is_err());
        assert!(ColorFilter::deferred(b"0,16416,257").is_err());
        assert!(ColorFilter::deferred(b"0,16416,48,0").is_err());
        Ok(())
    }
}
