// SPDX-License-Identifier: MPL-2.0
// Exact generated-pixel comparisons against the complete pinned native source.
use crysirror_image::{
    ImageSurface,
    bitmap::{BitmapRect, BlitOptions, PixelMath, Sampling, StretchOptions},
};
use reallive::surface::{Blend, Rect, Surface};
fn image(w: u32, h: u32, seed: u32) -> ImageSurface {
    let mut state = seed;
    let rgba = (0..w * h * 4)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 24) as u8
        })
        .collect();
    ImageSurface {
        width: w,
        height: h,
        rgba,
    }
}
fn main() {
    let source = image(4, 3, 71);
    let original = image(5, 4, 99);
    let native_source = Surface {
        width: 4,
        height: 3,
        rgba: source.rgba.clone(),
    };
    let mut straight_cases = 0;
    for (mode, blend) in [
        (PixelMath::Copy, Blend::Copy),
        (PixelMath::Mask255, Blend::Mask),
    ] {
        for opacity in [0, 1, 127, 128, 254, 255] {
            for sx in [-4, -1, 0, 2, 4] {
                for sy in [-2, 0, 2, 3] {
                    for w in [0, 1, 3, 6] {
                        for h in [0, 1, 2, 5] {
                            for (dx, dy) in [(-3, -2), (-1, 0), (0, 0), (3, 2), (5, 4)] {
                                let mut shared = original.clone();
                                let mut native = Surface {
                                    width: 5,
                                    height: 4,
                                    rgba: original.rgba.clone(),
                                };
                                shared
                                    .blit(
                                        &source,
                                        BlitOptions {
                                            source_rect: BitmapRect::new(sx, sy, w, h),
                                            destination: [dx, dy],
                                            opacity,
                                            mode,
                                            color_key: None,
                                        },
                                    )
                                    .unwrap();
                                native.blit(
                                    &native_source,
                                    Rect::new(sx, sy, w as i32, h as i32),
                                    dx,
                                    dy,
                                    opacity,
                                    blend,
                                    None,
                                );
                                assert_eq!(
                                    shared.rgba, native.rgba,
                                    "mode={mode:?} opacity={opacity} source={sx},{sy},{w},{h} dest={dx},{dy}"
                                );
                                straight_cases += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut premultiplied_cases = 0;
    for mask in 0..=255u8 {
        for s in [0, 1, 64, 128, 254, 255] {
            for d in [0, 1, 64, 128, 254, 255] {
                let source = ImageSurface::filled(1, 1, [s, 255 - s, s / 2, mask]).unwrap();
                let mut target = ImageSurface::filled(1, 1, [d, 255 - d, d / 2, 71]).unwrap();
                let mut expected = [d, 255 - d, d / 2];
                avg32::buffer::blend_pixel([s, 255 - s, s / 2], &mut expected, mask);
                target
                    .blit(
                        &source,
                        BlitOptions {
                            source_rect: BitmapRect::new(0, 0, 1, 1),
                            destination: [0, 0],
                            opacity: 255,
                            mode: PixelMath::PremultipliedMask256,
                            color_key: None,
                        },
                    )
                    .unwrap();
                assert_eq!(&target.rgba[..3], &expected);
                assert_eq!(target.rgba[3], 71);
                premultiplied_cases += 1;
            }
        }
    }
    let mut nearest_cases = 0;
    for (sx, sy, sw, sh) in [(-2, -1, 7, 5), (0, 0, 4, 3), (1, 1, 2, 2)] {
        for (dx, dy, dw, dh) in [(-3, -2, 8, 7), (0, 0, 4, 3), (1, 1, 3, 2), (8, 8, 2, 2)] {
            let mut native_source = avg32::buffer::PdtBuffer::new(4, 3);
            for (i, p) in source.rgba.chunks_exact(4).enumerate() {
                native_source.set_pixel(i, [p[0], p[1], p[2]]);
                native_source.mask[i] = p[3];
            }
            let mut manager = avg32::pdtmgr::PdtManager::new();
            let mut target = avg32::buffer::PdtBuffer::new(5, 4);
            for (i, p) in original.rgba.chunks_exact(4).enumerate() {
                target.set_pixel(i, [p[0], p[1], p[2]]);
                target.mask[i] = p[3];
            }
            manager.set(2, Some(target));
            manager.stretch_from(
                &native_source,
                avg32::pdtmgr::Rect::new(sx, sy, sx + sw as i32 - 1, sy + sh as i32 - 1),
                avg32::pdtmgr::Rect::new(dx, dy, dx + dw as i32 - 1, dy + dh as i32 - 1),
                2,
            );
            let mut shared = original.clone();
            shared
                .stretch_blit(
                    &source,
                    StretchOptions {
                        source_rect: BitmapRect::new(sx, sy, sw, sh),
                        destination: BitmapRect::new(dx, dy, dw, dh),
                        opacity: 255,
                        mode: PixelMath::CopyRgb,
                        sampling: Sampling::Nearest,
                        color_key: None,
                    },
                )
                .unwrap();
            let expected = manager.get(2).unwrap();
            for (i, p) in shared.rgba.chunks_exact(4).enumerate() {
                assert_eq!(&p[..3], &expected.rgb[i * 3..i * 3 + 3]);
                assert_eq!(p[3], expected.mask[i]);
            }
            nearest_cases += 1;
        }
    }
    println!(
        "PASS: RealLive Copy/Mask255={straight_cases}, AVG32 premultiplied mask={premultiplied_cases}, AVG32 nearest RGB/mask={nearest_cases}"
    );
}
