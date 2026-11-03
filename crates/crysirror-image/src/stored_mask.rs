//! Three-input stored-byte interpolation with threshold/lobed coverage tables.
use crate::ImageSurface;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredMaskBlend {
    /// Source and mask positions corresponding to destination pixel (0,0).
    pub source_origin: [i32; 2],
    pub mask_origin: [i32; 2],
    pub parameter: i32,
    pub transparency: u16,
    pub control: u16,
    pub mask_channel: u8,
}
impl StoredMaskBlend {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source_origin
                .iter()
                .chain(&self.mask_origin)
                .all(|v| v.unsigned_abs() <= 1_000_000)
                && self.transparency <= 256
                && self.control <= 256
                && self.mask_channel < 4,
            "stored mask blend parameters"
        );
        Ok(())
    }
    pub fn apply(
        &self,
        destination: &ImageSurface,
        source: &ImageSurface,
        mask: &ImageSurface,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        self.validate()?;
        destination.validate()?;
        source.validate()?;
        mask.validate()?;
        ensure!(
            u64::from(destination.width) * u64::from(destination.height)
                <= crate::effects::MAX_EFFECT_PIXELS,
            "stored mask blend pixel budget"
        );
        let mut out = destination.clone();
        for y in 0..out.height {
            check()?;
            for x in 0..out.width {
                let sx = i64::from(x) + i64::from(self.source_origin[0]);
                let sy = i64::from(y) + i64::from(self.source_origin[1]);
                let mx = i64::from(x) + i64::from(self.mask_origin[0]);
                let my = i64::from(y) + i64::from(self.mask_origin[1]);
                if sx < 0
                    || sy < 0
                    || sx >= i64::from(source.width)
                    || sy >= i64::from(source.height)
                    || mx < 0
                    || my < 0
                    || mx >= i64::from(mask.width)
                    || my >= i64::from(mask.height)
                {
                    continue;
                }
                let si = ((sy * i64::from(source.width) + sx) * 4) as usize;
                let mi = ((my * i64::from(mask.width) + mx) * 4) as usize;
                let di = ((y * out.width + x) * 4) as usize;
                let w = threshold_weight(
                    mask.rgba[mi + usize::from(self.mask_channel)],
                    self.parameter,
                    i32::from(self.transparency),
                    i32::from(self.control),
                );
                for c in 0..4 {
                    let d = i32::from(out.rgba[di + c]);
                    let s = i32::from(source.rgba[si + c]);
                    out.rgba[di + c] = if w >= 256 {
                        s as u8
                    } else {
                        (d + (((s - d) * i32::from(w >> 1)) as i16 as i32 >> 7)).clamp(0, 255) as u8
                    };
                }
            }
        }
        Ok(out)
    }
}

pub fn threshold_weight(
    mask_byte: u8,
    mask_parameter: i32,
    transparency: i32,
    extra_control: i32,
) -> u16 {
    let transparency = transparency.clamp(0, 256);
    let parameter = mask_parameter as u32;

    if parameter < 8 {
        let shift = parameter;
        let shifted_unit = 1_i32 << shift;
        let raw = 256_i32
            .saturating_sub(transparency.saturating_mul(shifted_unit + 1))
            .saturating_add(i32::from(mask_byte) << shift);
        if raw <= 0 {
            return 0;
        }
        if extra_control == 0 && raw >= 256 {
            return 256;
        }

        // The target indexes a 129-entry 0..128 table with raw>>1, then
        // performs a signed fixed-point /128 interpolation. Preserve the
        // table's one-bit quantization rather than treating raw itself as the
        // final weight.
        let table_index = raw.min(256) >> 1;
        if extra_control == 0 {
            return (table_index * 2) as u16;
        }

        // sub_411F10 rebuilds the same table with slope
        // (256-extra_control)/256. Values reaching this path through
        // CDspObjBackF::sub_41D540 are in 0..=256.
        let extra_control = extra_control.clamp(0, 256);
        let table_value = table_index * (256 - extra_control) / 256;
        return (table_value * 2) as u16;
    }

    // For parameters >= 8 the low three bits select the number of triangular
    // lobes. x86 compares the original parameter unsigned and masks it only
    // after entering this branch, so negative i32 parameters intentionally
    // arrive here too.
    let q = (parameter & 7) as i32;
    let index = 256 - 2 * transparency + i32::from(mask_byte);
    if index <= 0 {
        return 0;
    }
    if index >= 256 {
        return 256;
    }

    // sub_411D10/sub_412070 build the triangular lookup table through x87,
    // but for an integer table index its phase can be written exactly as a
    // rational with denominator 256. This avoids host-float rounding drift:
    //   lobe = floor(index * (2*q+1) / 256)
    //   phase/span = remainder/256 (reversed on odd lobes).
    let lobes = 2 * q + 1;
    let product = index * lobes;
    let lobe = product >> 8;
    let remainder = product & 0xff;
    let phase_units = if lobe & 1 != 0 {
        256 - remainder
    } else {
        remainder
    };
    if extra_control == 0 {
        return phase_units as u16;
    }
    let amplitude = extra_control.clamp(0, 256);
    ((phase_units * amplitude) >> 8).clamp(0, 256) as u16
}
