//! Fixed-point affine sprite projection and authoritative raster bounds.
use anyhow::{Result, ensure};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedSpriteProjection {
    pub position_x: i32,
    pub position_y: i32,
    pub position_z: i32,
    pub anchor_x: i32,
    pub anchor_y: i32,
    pub rotation: i32,
    pub perspective: i32,
    /// CDspObjSprite+0x27C.  sub_429AF0 uses this as the gate for
    /// perspective-projecting the resolved X/Y position by the Z scale.
    pub project_position: bool,
    /// CDspObjSprite+0x280. sub_4258D0 passes this to sub_417730: zero
    /// selects the nearest-neighbour rasterizer (sub_418280), nonzero selects
    /// the bilinear rasterizer (sub_417C50).
    pub interpolation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedSpriteDynamics {
    /// CDspObj+0xB8. CProcCtrlDspObj writes 8.24 progress here when the
    /// fixed-parameter selector is 1.
    pub progress_8_24: i32,
    /// Sprite properties 0x80/0x81/0x82/0x8F (sub_4285A0).
    pub offset_delta_x_16_16: i32,
    pub offset_delta_y_16_16: i32,
    pub rotation_delta_16_16: i32,
    pub scale_delta_x_16_16: i32,
    pub scale_delta_y_16_16: i32,
    pub uniform_scale_delta_16_16: i32,
    pub curve: crate::progress_curve::FixedProgressShape,
    /// Sprite property 0x42 / sub_428130. Defaults are 1.0, 1.0.
    pub base_scale_x_16_16: i32,
    pub base_scale_y_16_16: i32,
    pub coupled_scale: bool,
    /// Sprite+0x308 width-adjust parameter used by sub_429220. Property 0x60
    /// supplies it while its packed high-word gate is non-zero.
    pub width_adjust_16_16: i32,
}

impl Default for FixedSpriteDynamics {
    fn default() -> Self {
        Self {
            progress_8_24: 0,
            offset_delta_x_16_16: 0,
            offset_delta_y_16_16: 0,
            rotation_delta_16_16: 0,
            scale_delta_x_16_16: 0,
            scale_delta_y_16_16: 0,
            uniform_scale_delta_16_16: 0,
            curve: crate::progress_curve::FixedProgressShape::Linear,
            base_scale_x_16_16: 0x10000,
            base_scale_y_16_16: 0x10000,
            coupled_scale: false,
            width_adjust_16_16: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedSpriteGeometry {
    /// Raster top-left returned by sub_4281E0 after the ordinary CDspObj
    /// position (including its integer offset banks) has the mode-5 raster
    /// origin removed. The bottom-up convention belongs to the affine source
    /// transform; it does not invert the display object's screen Y axis.
    pub x: f32,
    pub y: f32,
    /// Ordinary CDspObj pixel position written by sub_429AF0 through
    /// vtable+40 before the raster-origin offset is applied.
    pub object_x: f32,
    pub object_y: f32,
    pub width: f32,
    pub height: f32,
    /// CDspObjSprite+0x2DC, subtracted from resolved object X by sub_4281E0.
    pub origin_offset_x: f32,
    /// CDspObjSprite+0x2E0, subtracted from resolved object Y by sub_4281E0.
    pub origin_offset_y: f32,
    pub scale: f32,
    pub rotation_degrees: f32,
    /// Forward affine image footprint, relative to the raster top-left. The
    /// points are source-edge TL, BL, BR, TR. The target does not stretch the
    /// bitmap to `width`/`height`: those values are only the inclusive raster
    /// bounding rectangle used for clipping. sub_416750 performs the inverse
    /// mapping for each destination pixel; this quad is the algebraic inverse
    /// of that mapping.
    pub local_affine_quad: [[f32; 2]; 4],
}

impl FixedSpriteProjection {
    pub fn perspective_scale_fixed(self) -> u32 {
        let perspective = self.perspective.max(0) as u64;
        if perspective == 0 || self.position_z == 0 {
            return 0x1_0000;
        }
        if self.position_z < 0 {
            (((perspective << 16).saturating_add(self.position_z.unsigned_abs() as u64))
                / perspective)
                .min(u32::MAX as u64) as u32
        } else {
            ((perspective << 32) / ((self.position_z as u64).saturating_add(perspective << 16)))
                .min(u32::MAX as u64) as u32
        }
    }

    /// The five values sub_4296C0 derives from the sprite's mode-5 state:
    /// base offset X/Y (16.16) plus the 0x80 deltas, rotation with the 0x81
    /// delta through the 0x8F curve, and the two 16.16 scales. Graph91:38
    /// parameter 0x10000000 returns exactly these.
    pub fn mode5_transform_parameters(self, dynamic: FixedSpriteDynamics) -> Result<[i32; 5]> {
        #[inline]
        fn mul_progress(delta: i32, progress: i32) -> i32 {
            (((i128::from(delta) * i128::from(progress as u32)) >> 24) as i64) as i32
        }
        #[inline]
        fn mul_fixed_signed(value: i32, scale: u32) -> i32 {
            (((i128::from(value) * i128::from(scale)) >> 16) as i64) as i32
        }
        let perspective_scale_fixed = self.perspective_scale_fixed();
        let progress = dynamic.progress_8_24.clamp(0, 0x0100_0000);

        // sub_4296C0.  The base offset pair is stored at Sprite+0x248/0x24C
        // after sub_427AA0 shifts the script's integer values by 16.
        let mut offset_x_16_16 = self.anchor_x.wrapping_shl(16);
        let mut offset_y_16_16 = self.anchor_y.wrapping_shl(16);
        offset_x_16_16 =
            offset_x_16_16.wrapping_add(mul_progress(dynamic.offset_delta_x_16_16, progress));
        offset_y_16_16 =
            offset_y_16_16.wrapping_add(mul_progress(dynamic.offset_delta_y_16_16, progress));

        let curve_weight = crate::progress_curve::sample_fixed_progress(dynamic.curve, progress)?;
        let rotation = self.rotation.wrapping_add(
            (((i128::from(dynamic.rotation_delta_16_16) * i128::from(curve_weight)) >> 16) as i64)
                as i32,
        );

        let scale_x_delta = mul_progress(dynamic.scale_delta_x_16_16, progress);
        let scale_y_delta = mul_progress(dynamic.scale_delta_y_16_16, progress);
        let uniform_delta = mul_progress(dynamic.uniform_scale_delta_16_16, progress);
        let (scale_x_fixed, scale_y_fixed) = if dynamic.coupled_scale {
            let sx = dynamic.base_scale_x_16_16.wrapping_add(mul_progress(
                dynamic
                    .uniform_scale_delta_16_16
                    .wrapping_add(dynamic.scale_delta_x_16_16),
                progress,
            ));
            let sy = dynamic.base_scale_y_16_16.wrapping_add(mul_progress(
                dynamic
                    .uniform_scale_delta_16_16
                    .wrapping_add(dynamic.scale_delta_y_16_16),
                progress,
            ));
            (
                mul_fixed_signed(sx, perspective_scale_fixed) as u32,
                mul_fixed_signed(sy, perspective_scale_fixed) as u32,
            )
        } else {
            let px = (perspective_scale_fixed as i64 + i64::from(scale_x_delta)) as i32;
            let py = (perspective_scale_fixed as i64 + i64::from(scale_y_delta)) as i32;
            let bx = dynamic.base_scale_x_16_16.wrapping_add(uniform_delta);
            let by = dynamic.base_scale_y_16_16.wrapping_add(uniform_delta);
            (
                mul_fixed_signed(px, bx as u32) as u32,
                mul_fixed_signed(py, by as u32) as u32,
            )
        };

        Ok([
            offset_x_16_16,
            offset_y_16_16,
            rotation,
            scale_x_fixed as i32,
            scale_y_fixed as i32,
        ])
    }

    /// Exact mode-5 projection recovered from
    /// sub_4296C0 -> sub_429220 -> sub_429AF0.  The constructor arguments at
    /// indices 8/9 are the bitmap-origin coordinates consumed by the native
    /// bottom-up DIB transform (stored as 16.16 by sub_427AA0).
    pub fn screen_geometry(
        self,
        image_width: f32,
        image_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        graph_center: (i32, i32),
        use_graph_center: bool,
        dynamic: FixedSpriteDynamics,
    ) -> Result<FixedSpriteGeometry> {
        ensure!(
            [image_width, image_height, viewport_width, viewport_height]
                .iter()
                .all(|v| v.is_finite() && *v > 0. && *v <= 16384.),
            "fixed sprite projection dimensions"
        );
        #[inline]
        fn mul_progress(delta: i32, progress: i32) -> i32 {
            (((i128::from(delta) * i128::from(progress as u32)) >> 24) as i64) as i32
        }
        #[inline]
        fn mul_fixed_signed(value: i32, scale: u32) -> i32 {
            (((i128::from(value) * i128::from(scale)) >> 16) as i64) as i32
        }
        #[inline]
        fn scaled_fraction_16(value: i32, scale: u32) -> f64 {
            // 0x429535/0x429575 call the target's signed 32x32 -> 64 helper,
            // SHRD by 16, then AND 0xffff.  Only the fractional word affects
            // the raster bound rounding.
            let product = i128::from(value) * i128::from(scale);
            let shifted = product >> 16;
            (shifted as i64 as u64 & 0xffff) as f64 / 65_536.0
        }

        let perspective_scale_fixed = self.perspective_scale_fixed();
        let perspective_scale = perspective_scale_fixed as f64 / 65_536.0;
        let [
            offset_x_16_16,
            offset_y_16_16,
            rotation,
            scale_x_fixed,
            scale_y_fixed,
        ] = self.mode5_transform_parameters(dynamic)?;
        let (scale_x_fixed, scale_y_fixed) = (scale_x_fixed as u32, scale_y_fixed as u32);

        // sub_429220.  BGI uses bottom-up DIB coordinates and deliberately
        // mixes floor for X with ceil for Y. Preserve that asymmetry.
        let width = image_width as f64;
        let height = image_height as f64;
        let adjusted_width =
            width * (f64::from(dynamic.width_adjust_16_16.wrapping_add(0x10000))) / 65_536.0;
        let offset_x = (adjusted_width - width) * 0.5 + f64::from(offset_x_16_16) / 65_536.0;
        let offset_y = f64::from(offset_y_16_16) / 65_536.0;
        let radians = f64::from(rotation) * std::f64::consts::PI / 11_796_480.0;
        let (sin, cos) = radians.sin_cos();
        let sx = scale_x_fixed as f64 / 65_536.0;
        let sy = scale_y_fixed as f64 / 65_536.0;
        let right = adjusted_width - offset_x - 1.0;
        let left = -offset_x;
        let bottom = offset_y * sy;
        let top = offset_y - height + 1.0;
        let corners = [
            (
                left * sx * cos - bottom * sin,
                left * sx * sin + bottom * cos,
            ),
            (
                right * sx * cos - bottom * sin,
                right * sx * sin + bottom * cos,
            ),
            (
                left * sx * cos - top * sy * sin,
                left * sx * sin + top * sy * cos,
            ),
            (
                right * sx * cos - top * sy * sin,
                right * sx * sin + top * sy * cos,
            ),
        ];
        let mut min_x = 1_000_000_000.0_f64;
        let mut max_x = -1_000_000_000.0_f64;
        let mut min_y = 1_000_000_000.0_f64;
        let mut max_y = -1_000_000_000.0_f64;
        for (x, y) in corners {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        let max_x_adjusted = max_x + scaled_fraction_16(self.position_x, scale_x_fixed);
        let min_y_adjusted = min_y - scaled_fraction_16(self.position_y, scale_y_fixed);
        let min_x_floor = min_x.floor();
        let max_x_floor = max_x_adjusted.floor();
        let max_y_ceil = max_y.ceil();
        let min_y_ceil = min_y_adjusted.ceil();
        let raster_width = max_x_floor - min_x_floor + 1.0;
        let raster_height = max_y_ceil - min_y_ceil + 1.0;
        let origin_offset_x = -min_x_floor;
        let origin_offset_y = max_y_ceil;

        // sub_429AF0. The ordinary object position is centered in the display
        // (or Graph90:06 centre when enabled/valid) and only then has the
        // transformed-raster origin removed by sub_4281E0.
        let viewport_width_i32 = viewport_width.max(0.0).floor().min(i32::MAX as f32) as i32;
        let viewport_height_i32 = viewport_height.max(0.0).floor().min(i32::MAX as f32) as i32;
        let valid_graph_center = use_graph_center
            && graph_center.0 >= 0
            && graph_center.1 >= 0
            && graph_center.0 < viewport_width_i32
            && graph_center.1 < viewport_height_i32;
        let (center_x, center_y) = if valid_graph_center {
            (graph_center.0, graph_center.1)
        } else {
            (viewport_width_i32 >> 1, viewport_height_i32 >> 1)
        };
        let projected_x = if self.project_position {
            mul_fixed_signed(self.position_x, perspective_scale_fixed)
        } else {
            self.position_x
        };
        let projected_y = if self.project_position {
            mul_fixed_signed(self.position_y, perspective_scale_fixed)
        } else {
            self.position_y
        };
        let object_x_fixed = (center_x << 16).wrapping_add(projected_x);
        let object_y_fixed = (center_y << 16).wrapping_add(projected_y);
        let object_x = object_x_fixed >> 16;
        let object_y = object_y_fixed >> 16;

        // sub_4258D0 case 5 passes
        //   frac + ((raster_origin - local_clip) << 16)
        // to sub_417730. sub_416750 then inverse-maps every destination pixel
        // back into source coordinates. Algebraically inverting that mapping
        // gives the source-pixel-centre -> raster-local transform below. Keep
        // the 16-bit ordinary-position remainder: sub_429AF0 stores it at
        // Sprite+0x2A4/+0x2A8 and the case-5 rasterizer consumes it.
        let fractional_x = f64::from((object_x_fixed as u32 & 0xffff) as u16) / 65_536.0;
        let fractional_y = f64::from((object_y_fixed as u32 & 0xffff) as u16) / 65_536.0;
        let anchor_x = f64::from(offset_x_16_16) / 65_536.0;
        let anchor_y = f64::from(offset_y_16_16) / 65_536.0;
        let map_source_edge = |source_x: f64, source_y: f64| -> [f32; 2] {
            let source_dx = (source_x - anchor_x) * sx;
            let source_dy = (source_y - anchor_y) * sy;
            [
                (origin_offset_x + fractional_x + source_dx * cos + source_dy * sin + 0.5) as f32,
                (origin_offset_y + fractional_y - source_dx * sin + source_dy * cos + 0.5) as f32,
            ]
        };
        // The native rasterizer addresses destination pixels by integer pixel
        // centre (local 0 means the centre of the first pixel), whereas GPU
        // vertex coordinates address pixel edges (the first fragment centre is
        // at 0.5). The +0.5 above is only that coordinate-system bridge; it is
        // not a target half-pixel correction. Texture UV 0/1 are texel edges,
        // while native source integer coordinates denote texel centres, so the
        // source footprint expands by half a texel. The native raster bbox is
        // still applied as a scissor and remains authoritative.
        let local_affine_quad = [
            map_source_edge(-0.5, -0.5),
            map_source_edge(-0.5, height - 0.5),
            map_source_edge(width - 0.5, height - 0.5),
            map_source_edge(width - 0.5, -0.5),
        ];

        // sub_4281E0 calls sub_41B260 to resolve the ordinary screen
        // position, then subtracts Sprite+0x2DC/+0x2E0.  Do not flip this Y:
        // the target's bottom-up DIB convention is consumed by the affine
        // rasterizer (sub_417730/sub_416750), not by CDspObj screen placement.
        Ok(FixedSpriteGeometry {
            x: object_x as f32 - origin_offset_x as f32,
            y: object_y as f32 - origin_offset_y as f32,
            object_x: object_x as f32,
            object_y: object_y as f32,
            width: raster_width as f32,
            height: raster_height as f32,
            origin_offset_x: origin_offset_x as f32,
            origin_offset_y: origin_offset_y as f32,
            scale: perspective_scale as f32,
            rotation_degrees: (f64::from(rotation) / 65_536.0) as f32,
            local_affine_quad,
        })
    }
}
