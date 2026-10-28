// Portions adapted from CxImage (xImaDsp.cpp, v7.0.2, Davide Pizzolato) as
// carried by wamsoft/layerExImage 3aff625dba5c545ba722bb43b0a5ca1ad4f4336d
// (LayerExImage.cpp). CxImage license: see target notice reproduced in
// docs/third-party/cximage.txt. Modifications CrySirror contributors (MPL-2.0).
//! Colour adjustments with the exact integer/float steps of the CxImage DSP
//! routines: brightness/contrast LUT, HSL colorize and HSL modulate. RGB only;
//! alpha is preserved. Pixels are straight RGBA bytes.

/// `light(brightness, contrast)`: brightness -255..255, contrast -100..100.
pub fn brightness_contrast_lut(brightness: i32, contrast: i32) -> [u8; 256] {
    let c = (100 + contrast) as f32 / 100.0;
    let b = brightness + 128;
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = (((i as i32 - 128) as f32 * c + b as f32) as i32).clamp(0, 255) as u8;
    }
    lut
}

const HSLMAX: u32 = 255;
const RGBMAX: u32 = 255;
const HSLUNDEFINED: u8 = (HSLMAX * 2 / 3) as u8;

/// CxImage RGBtoHSL on bytes; returns (h, s, l).
fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r32, g32, b32) = (u32::from(r), u32::from(g), u32::from(b));
    let cmax = r32.max(g32).max(b32);
    let cmin = r32.min(g32).min(b32);
    let l = ((((cmax + cmin) * HSLMAX) + RGBMAX) / (2 * RGBMAX)) as u8;
    if cmax == cmin {
        return (HSLUNDEFINED, 0, l);
    }
    let s = if u32::from(l) <= HSLMAX / 2 {
        ((((cmax - cmin) * HSLMAX) + ((cmax + cmin) / 2)) / (cmax + cmin)) as u8
    } else {
        ((((cmax - cmin) * HSLMAX) + ((2 * RGBMAX - cmax - cmin) / 2))
            / (2 * RGBMAX - cmax - cmin)) as u8
    };
    let d = cmax - cmin;
    let delta = |c: u32| ((((cmax - c) * (HSLMAX / 6)) + (d / 2)) / d) as u16;
    let (rd, gd, bd) = (delta(r32), delta(g32), delta(b32));
    // BYTE arithmetic wraps exactly as the original casts do.
    let mut h = if r32 == cmax {
        bd.wrapping_sub(gd) as u8
    } else if g32 == cmax {
        ((HSLMAX / 3) as u16)
            .wrapping_add(rd)
            .wrapping_sub(bd) as u8
    } else {
        (((2 * HSLMAX) / 3) as u16)
            .wrapping_add(gd)
            .wrapping_sub(rd) as u8
    };
    if u32::from(h) > HSLMAX {
        h = (u32::from(h) - HSLMAX) as u8;
    }
    (h, s, l)
}

fn hue_to_rgb(n1: f32, n2: f32, mut hue: f32) -> f32 {
    if hue > 360.0 {
        hue -= 360.0;
    } else if hue < 0.0 {
        hue += 360.0;
    }
    if hue < 60.0 {
        n1 + (n2 - n1) * hue / 60.0
    } else if hue < 180.0 {
        n2
    } else if hue < 240.0 {
        n1 + (n2 - n1) * (240.0 - hue) / 60.0
    } else {
        n1
    }
}

fn hsl_to_rgb(h: u8, s: u8, l: u8) -> (u8, u8, u8) {
    let h = f32::from(h) * 360.0 / 255.0;
    let s = f32::from(s) / 255.0;
    let l = f32::from(l) / 255.0;
    let m2 = if l <= 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let m1 = 2.0 * l - m2;
    if s == 0.0 {
        let v = (l * 255.0) as u8;
        return (v, v, v);
    }
    (
        (hue_to_rgb(m1, m2, h + 120.0) * 255.0) as u8,
        (hue_to_rgb(m1, m2, h) * 255.0) as u8,
        (hue_to_rgb(m1, m2, h - 120.0) * 255.0) as u8,
    )
}

/// `colorize(hue, sat, blend)` for one RGB triple.
pub fn colorize(rgb: [u8; 3], hue: u8, sat: u8, blend: f64) -> [u8; 3] {
    let blend = blend.clamp(0.0, 1.0);
    let (_, _, l) = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
    let (r, g, b) = hsl_to_rgb(hue, sat, l);
    if blend > 0.999 {
        return [r, g, b];
    }
    let a0 = (256.0 * blend) as u32;
    let a1 = 256 - a0;
    let mix = |n: u8, o: u8| ((u32::from(n) * a0 + u32::from(o) * a1) >> 8) as u8;
    [mix(r, rgb[0]), mix(g, rgb[1]), mix(b, rgb[2])]
}

fn hue2rgb(n1: f64, n2: f64, mut hue: f64) -> i32 {
    if hue < 0.0 {
        hue += 1.0;
    } else if hue > 1.0 {
        hue -= 1.0;
    }
    let color = if hue < 1.0 / 6.0 {
        n1 + (n2 - n1) * hue * 6.0
    } else if hue < 1.0 / 2.0 {
        n2
    } else if hue < 2.0 / 3.0 {
        n1 + (n2 - n1) * (2.0 / 3.0 - hue) * 6.0
    } else {
        n1
    };
    (color * 255.0) as i32
}

/// `modulate(hue°, saturation%, luminance%)` for one RGB triple.
pub fn modulate(rgb: [u8; 3], hue: i32, saturation: i32, luminance: i32) -> [u8; 3] {
    let (h, s, l) = (
        f64::from(hue as f32 / 360.0),
        f64::from(saturation as f32 / 100.0),
        f64::from(luminance as f32 / 100.0),
    );
    let red = f64::from(rgb[0]) / 255.0;
    let green = f64::from(rgb[1]) / 255.0;
    let blue = f64::from(rgb[2]) / 255.0;
    let cmax = red.max(green).max(blue);
    let cmin = red.min(green).min(blue);
    let delta = cmax - cmin;
    let add = cmax + cmin;
    let mut lum = add / 2.0;
    let (mut hh, mut sat);
    if delta == 0.0 {
        sat = 0.0;
        hh = 0.0;
    } else {
        sat = if lum < 0.5 { delta / add } else { delta / (2.0 - add) };
        hh = if red == cmax {
            (green - blue) / delta
        } else if green == cmax {
            2.0 + (blue - red) / delta
        } else {
            4.0 + (red - green) / delta
        };
        hh /= 6.0;
    }
    hh += h;
    while hh < 0.0 {
        hh += 1.0;
    }
    while hh > 1.0 {
        hh -= 1.0;
    }
    if s > 0.0 {
        sat += (1.0 - sat) * s;
    } else {
        sat += sat * s;
    }
    if l > 0.0 {
        lum += (1.0 - lum) * l;
    } else {
        lum += lum * l;
    }
    // Native stores int results into BYTE channels (truncating cast).
    let (r, g, b) = if sat == 0.0 {
        let v = (lum * 255.0) as i32;
        (v, v, v)
    } else {
        let m2 = if lum <= 0.5 {
            lum * (1.0 + sat)
        } else {
            lum + sat - lum * sat
        };
        let m1 = 2.0 * lum - m2;
        (
            hue2rgb(m1, m2, hh + 1.0 / 3.0),
            hue2rgb(m1, m2, hh),
            hue2rgb(m1, m2, hh - 1.0 / 3.0),
        )
    };
    [r as u8, g as u8, b as u8]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identities_and_reference_points() {
        assert_eq!(brightness_contrast_lut(0, 0)[77], 77);
        assert_eq!(brightness_contrast_lut(255, 0)[0], 255);
        assert_eq!(brightness_contrast_lut(0, -100)[10], 128);
        for c in [[0, 0, 0], [255, 255, 255], [200, 30, 60], [12, 240, 99]] {
            let m = modulate(c, 0, 0, 0);
            for i in 0..3 {
                assert!((i32::from(m[i]) - i32::from(c[i])).abs() <= 1, "{c:?} -> {m:?}");
            }
            assert_eq!(colorize(c, 0, 0, 0.0), c);
        }
        assert_eq!(modulate([200, 30, 60], 0, -100, 0)[0], modulate([200, 30, 60], 0, -100, 0)[1]);
        assert_eq!(modulate([10, 20, 30], 0, 0, 100), [255, 255, 255]);
        assert_eq!(modulate([10, 20, 30], 0, 0, -100), [0, 0, 0]);
    }
}
