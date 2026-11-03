use crate::{native_background::NativeBackgroundState, native_display::CDspObjLayout32};
use std::collections::BTreeMap;
/// CDspObj+0x12C owns this immutable bit plane, independently of its bitmap.
#[derive(Debug, Clone)]
pub(crate) struct NativeHitMask {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bits: Vec<u8>,
}
impl NativeHitMask {
    pub(crate) fn hit(&self, x: i32, y: i32) -> i32 {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return 0;
        }
        let stride = (self.width as usize + 7) / 8;
        i32::from(self.bits[y as usize * stride + x as usize / 8] & (1 << (x & 7)))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeGraphObjectProperties {
    /// Exact target base field image.  The semantic fields below are portable
    /// mirrors until their concrete `CDspObj` member offsets are closed.
    pub(crate) native: CDspObjLayout32,
    /// Exact dynamic `CDspObjBack` subclass selected by `sub_43E190`.
    pub(crate) background: Option<NativeBackgroundState>,
    pub(crate) blend_mode: i32,
    pub(crate) mask_alpha: i32,
    pub(crate) alpha_multiplier: i32,
    /// Primary display bitmap for Sprite/Background subclasses. Keep this
    /// separate from CDspObj hit masks and CDspObjSprite auxiliary bitmaps.
    pub(crate) format_resource: Option<i32>,
    /// CDspObjSprite +0x13C auxiliary bitmap installed by Graph90:55 /
    /// sub_427F80. This modifies Sprite rendering but is not its primary.
    pub(crate) aux_resource: Option<i32>,
    /// Base CDspObj generated hit-mask source installed by Graph90:3C /
    /// sub_443A40 -> sub_41BC70. It controls pointer acceptance only.
    pub(crate) hit_mask_resource: Option<i32>,
    pub(crate) hit_mask: Option<NativeHitMask>,
    pub(crate) properties: BTreeMap<u32, (i32, i32)>,
    pub(crate) named_properties: BTreeMap<String, i32>,
}

impl Default for RuntimeGraphObjectProperties {
    fn default() -> Self {
        Self {
            native: CDspObjLayout32::default(),
            background: None,
            // CDspObj::CDspObj (sub_41A400) installs these exact defaults.
            blend_mode: 128,
            mask_alpha: 0,
            alpha_multiplier: 256,
            format_resource: None,
            aux_resource: None,
            hit_mask_resource: None,
            hit_mask: None,
            properties: BTreeMap::new(),
            named_properties: BTreeMap::new(),
        }
    }
}

impl RuntimeGraphObjectProperties {
    /// CDspObjSprite's GetAlpha virtual returns the crossfade coefficient
    /// only for mode1 selector1; base blend mode1 is unrelated.
    pub(crate) fn alpha_virtual(&self) -> i32 {
        if self.named_properties.get("target-object-mode") == Some(&1)
            && self.named_properties.get("mode1-selector") == Some(&1)
        {
            self.named_properties
                .get("mode1-amount")
                .copied()
                .unwrap_or_else(|| self.alpha_parameter())
                .clamp(0, 256)
        } else {
            self.alpha_parameter()
        }
    }

    pub(crate) fn set_property(&mut self, property: u32, value: i32, extra: i32) {
        match property {
            1 => self.blend_mode = value,
            2 => self.set_alpha_parameter(value),
            // CDspObj::SetProperty selector 196 reaches sub_41ADF0 and writes
            // CDspObj+0x48. sub_41C0E0 consults this gate before adding the
            // global display offset configured by Graph91:06.
            196 => self.native.global_display_offset_enabled = value as u32,
            // CDspObj::SetProperty 0x8000 dispatches to sub_41BEB0(value,
            // extra), which writes +0x80/+0x84. sub_41B370 then uses the pair
            // to decide whether X/Y fixed coordinates are rounded to whole
            // 16.16 pixels before they are stored and propagated.
            0x8000 => {
                self.native.fixed_position_rounding_enabled = value as u32;
                self.native.fixed_position_rounding_mode = extra as u32;
            }
            // Property 0x8100 reaches sub_41BF10 and writes CDspObj+0x24,
            // an additive component of the vtable+0x1C manager sort key.
            0x8100 => self.native.sort_bias = value,
            _ => {
                self.properties.insert(property, (value, extra));
            }
        }
    }

    pub(crate) fn alpha_parameter(&self) -> i32 {
        self.native.alpha_parameter
    }

    /// CDspObj::SetAlpha (`sub_41B620`) stores the supplied DWORD without
    /// range validation. Callers whose native wrapper validates 0..=256 do
    /// that before reaching this setter.
    pub(crate) fn set_alpha_parameter(&mut self, value: i32) {
        self.native.alpha_parameter = value;
    }

    /// `CDspObj::GetMaskAlpha` (`sub_41B770`) in target transparency units.
    /// `0` means fully source-visible and `256` means fully transparent for
    /// the BackF mode-1 compositor.
    pub(crate) fn transparency_parameter(&self) -> i32 {
        // CDspObj's drawable predicate (sub_41AE30) rejects a fully masked
        // object before asking GetMaskAlpha for its blend-specific value.
        if self.mask_alpha >= 256 {
            return 256;
        }
        // CDspObj::GetMaskAlpha (sub_41B770) returns transparency in 1/256
        // units. Preserve its integer arithmetic exactly here so native
        // compositors can consume the integer result before any f32
        // conversion performed by the renderer.
        match self.blend_mode {
            1 | 0x20..=0x24 => 256i32.wrapping_sub(
                self.alpha_multiplier
                    .wrapping_mul(256i32.wrapping_sub(self.alpha_parameter()))
                    .wrapping_mul(256i32.wrapping_sub(self.mask_alpha))
                    >> 16,
            ),
            2 | 3 | 4 | 0xc0 | 0xc1 => {
                self.alpha_parameter()
                    .wrapping_mul(self.alpha_multiplier)
                    .wrapping_mul(256i32.wrapping_sub(self.mask_alpha))
                    >> 16
            }
            _ => self.alpha_parameter(),
        }
        .clamp(0, 256)
    }

    pub(crate) fn opacity(&self) -> f32 {
        // BackRPL and Sprite mode 4 use SetAlpha to scale their wave
        // samples (409F30), independently of source coverage. Sprite's
        // vector blitter reads only mask alpha and its multiplier.
        if self.background.is_some_and(|b| {
            matches!(
                b.class,
                crate::native_background::NativeBackgroundClass::BackD
                    | crate::native_background::NativeBackgroundClass::BackDst
                    | crate::native_background::NativeBackgroundClass::BackGrd
                    | crate::native_background::NativeBackgroundClass::BackRpl
                    | crate::native_background::NativeBackgroundClass::BackStr
                    | crate::native_background::NativeBackgroundClass::BackRtt
                    | crate::native_background::NativeBackgroundClass::BackMsc
            )
        }) {
            return 1.0;
        }
        if self.background.is_none() && self.named_properties.get("target-object-mode") == Some(&4)
        {
            return ((self
                .alpha_multiplier
                .wrapping_mul(256i32.wrapping_sub(self.mask_alpha))
                >> 8) as f32
                / 256.0)
                .clamp(0.0, 1.0);
        }
        let transparency = self.transparency_parameter();
        (1.0 - transparency as f32 / 256.0).clamp(0.0, 1.0)
    }
}
