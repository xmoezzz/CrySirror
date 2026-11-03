//! Bounded channel-plane extraction with optional wrapping16bit interpolation.
use crate::ImageSurface;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlphaPlanePair {
    pub source_origin: [i32; 2],
    pub channel: u8,
    pub paired: bool,
    pub amount: u16,
}
impl AlphaPlanePair {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.channel < 4
                && self
                    .source_origin
                    .iter()
                    .all(|v| v.unsigned_abs() <= 1_000_000),
            "channel plane parameters"
        );
        Ok(())
    }
    pub fn apply(
        &self,
        destination: &ImageSurface,
        first: &ImageSurface,
        second: &ImageSurface,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<ImageSurface> {
        self.validate()?;
        destination.validate()?;
        first.validate()?;
        second.validate()?;
        ensure!(
            u64::from(destination.width) * u64::from(destination.height)
                <= crate::effects::MAX_EFFECT_PIXELS,
            "channel plane pixel budget"
        );
        let mut out = ImageSurface {
            width: destination.width,
            height: destination.height,
            rgba: vec![0; crate::rgba_len(destination.width, destination.height)?],
        };
        for y in 0..out.height {
            check()?;
            for x in 0..out.width {
                let at = ((y * out.width + x) * 4) as usize;
                out.rgba[at + 3] = 255;
                let sx = i64::from(x) + i64::from(self.source_origin[0]);
                let sy = i64::from(y) + i64::from(self.source_origin[1]);
                if sx < 0 || sy < 0 || sx >= i64::from(first.width) || sy >= i64::from(first.height)
                {
                    continue;
                }
                if self.paired && (sx >= i64::from(second.width) || sy >= i64::from(second.height))
                {
                    continue;
                }
                let value = first.rgba[((sy as u32 * first.width + sx as u32) * 4) as usize
                    + usize::from(self.channel)];
                let value = if self.paired {
                    let b = second.rgba[((sy as u32 * second.width + sx as u32) * 4) as usize
                        + usize::from(self.channel)];
                    let product = self
                        .amount
                        .wrapping_mul((i32::from(b) - i32::from(value)) as u16);
                    value.wrapping_add((product >> 8) as u8)
                } else {
                    value
                };
                out.rgba[at..at + 3].fill(value);
            }
        }
        Ok(out)
    }
}
