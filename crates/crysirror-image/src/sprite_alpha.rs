// SPDX-License-Identifier: MPL-2.0
//! Generic sprite-sheet transparency extraction. Native tags stay in adapters.
use crate::ImageSurface;
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpriteAlpha {
    /// Keep decoded alpha unchanged.
    Preserve,
    /// Replace coverage with fully opaque alpha.
    Opaque,
    /// Exact RGB key selected from the top edge (right selects last texel).
    CornerKey { right: bool },
    /// Exact caller-specified RGB key.
    ColorKey { color: [u8; 3] },
    /// Each horizontal cell stores RGB then a same-sized coverage plane.
    /// Coverage is 255 minus the selected byte channel, preserving hidden RGB.
    SideMask { cells: u16, channel: u8 },
}
impl SpriteAlpha {
    pub fn validate(&self) -> Result<()> {
        if let Self::SideMask { cells, channel } = self {
            ensure!(
                (1..=1024).contains(cells) && *channel < 4,
                "side-mask cells/channel"
            );
        }
        Ok(())
    }
}
impl ImageSurface {
    pub fn sprite_alpha(&self, spec: &SpriteAlpha, check: &dyn Fn() -> Result<()>) -> Result<Self> {
        self.validate()?;
        spec.validate()?;
        check()?;
        let mut out = self.clone();
        let key = match spec {
            SpriteAlpha::CornerKey { right } => {
                let at = if *right {
                    (self.width as usize - 1) * 4
                } else {
                    0
                };
                Some(self.rgba[at..at + 3].try_into().unwrap())
            }
            SpriteAlpha::ColorKey { color } => Some(*color),
            _ => None,
        };
        if let SpriteAlpha::SideMask { cells, channel } = spec {
            let cells = u32::from(*cells);
            ensure!(
                self.width.is_multiple_of(cells * 2),
                "side-mask must contain paired equal cell planes"
            );
            let cell = self.width / cells / 2;
            out.width = self.width / 2;
            out.rgba = vec![0; crate::rgba_len(out.width, out.height)?];
            for y in 0..self.height {
                check()?;
                for c in 0..cells {
                    for x in 0..cell {
                        let src = ((y * self.width + c * cell * 2 + x) * 4) as usize;
                        let dst = ((y * out.width + c * cell + x) * 4) as usize;
                        out.rgba[dst..dst + 4].copy_from_slice(&self.rgba[src..src + 4]);
                        out.rgba[dst + 3] =
                            255 - self.rgba[src + cell as usize * 4 + *channel as usize];
                    }
                }
            }
        } else {
            for row in out.rgba.chunks_exact_mut(self.width as usize * 4) {
                check()?;
                for p in row.chunks_exact_mut(4) {
                    if let Some(key) = key {
                        p[3] = if p[..3] == key { 0 } else { 255 };
                    } else if matches!(spec, SpriteAlpha::Opaque) {
                        p[3] = 255;
                    }
                }
            }
        }
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_corner_key_and_paired_mask_preserve_rgb() {
        let image = ImageSurface {
            width: 4,
            height: 1,
            rgba: vec![10, 20, 30, 7, 1, 2, 3, 6, 255, 0, 0, 9, 0, 0, 255, 8],
        };
        let key = image
            .sprite_alpha(&SpriteAlpha::CornerKey { right: false }, &|| Ok(()))
            .unwrap();
        assert_eq!(key.rgba[3], 0);
        assert_eq!(key.rgba[7], 255);
        let mask = image
            .sprite_alpha(
                &SpriteAlpha::SideMask {
                    cells: 1,
                    channel: 2,
                },
                &|| Ok(()),
            )
            .unwrap();
        assert_eq!((mask.width, mask.height), (2, 1));
        assert_eq!(mask.rgba, vec![10, 20, 30, 255, 1, 2, 3, 0]);
        assert!(image
            .sprite_alpha(
                &SpriteAlpha::SideMask {
                    cells: 3,
                    channel: 2
                },
                &|| Ok(())
            )
            .is_err());
        assert!(image
            .sprite_alpha(&SpriteAlpha::Opaque, &|| anyhow::bail!("cancelled"))
            .is_err());
    }
}

/// Independent staggered stripe threshold image for the engine mask compositor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StripeReveal {
    /// False selects x-oriented bands; true selects y-oriented bands.
    pub vertical: bool,
    /// Band extent in logical pixels, 1 through 8192.
    pub period: u16,
    /// Reveal from the opposite edge.
    pub reverse: bool,
    /// Successive bands start later, producing a travelling curtain.
    pub stagger: bool,
}
impl StripeReveal {
    pub fn validate(&self) -> Result<()> {
        ensure!((1..=8192).contains(&self.period), "reveal stripe period");
        Ok(())
    }
}
impl ImageSurface {
    pub fn stripe_reveal(
        &self,
        spec: &StripeReveal,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        self.validate()?;
        spec.validate()?;
        let mut out = self.clone();
        let period = u64::from(spec.period);
        let extent = u64::from(if spec.vertical {
            self.height
        } else {
            self.width
        });
        for y in 0..self.height {
            check()?;
            for x in 0..self.width {
                let mut position = u64::from(if spec.vertical { y } else { x });
                if spec.reverse {
                    position = extent - 1 - position;
                }
                let within = position % period;
                let delay = if spec.stagger {
                    period * period * (position / period) / extent
                } else {
                    0
                };
                let span = if spec.stagger { period * 2 } else { period };
                let threshold = ((within + delay) * 255 / span).min(255) as u8;
                let at = ((y * self.width + x) * 4) as usize;
                out.rgba[at..at + 4].copy_from_slice(&[
                    255 - threshold,
                    255 - threshold,
                    255 - threshold,
                    255,
                ]);
            }
        }
        Ok(out)
    }
}
#[cfg(test)]
mod stripe_tests {
    use super::*;
    #[test]
    fn stripe_thresholds_repeat_reverse_and_stagger_without_native_ids() {
        let src = ImageSurface {
            width: 8,
            height: 1,
            rgba: vec![0; 32],
        };
        let mut spec = StripeReveal {
            vertical: false,
            period: 4,
            reverse: false,
            stagger: false,
        };
        let image = src.stripe_reveal(&spec, &|| Ok(())).unwrap();
        let a = image.rgba.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>();
        assert_eq!(a, vec![255, 192, 128, 64, 255, 192, 128, 64]);
        spec.reverse = true;
        let rev = src.stripe_reveal(&spec, &|| Ok(())).unwrap();
        assert_eq!(
            rev.rgba.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>(),
            a.into_iter().rev().collect::<Vec<_>>()
        );
        spec.reverse = false;
        spec.stagger = true;
        let curtain = src.stripe_reveal(&spec, &|| Ok(())).unwrap();
        assert!(curtain.rgba[0] > curtain.rgba[16]);
    }
}
