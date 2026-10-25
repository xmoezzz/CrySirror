// SPDX-License-Identifier: MPL-2.0
#![allow(dead_code)]
use ab_glyph::{Font, FontArc, FontVec, PxScale};
use crysirror_image::{
    ImageSurface as DecodedImage,
    composition::{FloatBlend, FloatDraw, FloatSampling},
};
use crysirror_text::{TextSystem, glyph::GlyphRaster};
mod original_bitmap;
mod original_core;
mod original_snapshot;
mod graph {
    #[derive(Clone, Copy)]
    pub struct RuntimeClipRect {
        pub x: f32,
        pub y: f32,
        pub width: f32,
        pub height: f32,
    }
}
fn image(width: u32, height: u32, mut state: u32) -> DecodedImage {
    let rgba = (0..width * height * 4)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 24) as u8
        })
        .collect();
    DecodedImage {
        width,
        height,
        rgba,
    }
}
fn main() {
    let mut pixels = 0;
    for (mode, profile) in [
        (1, FloatBlend::Coverage),
        (0x20, FloatBlend::Coverage),
        (128, FloatBlend::Coverage),
        (2, FloatBlend::Add),
        (3, FloatBlend::Subtract),
        (240, FloatBlend::ConstantInterpolation256),
    ] {
        for a in 0..=255u8 {
            for s in [0, 1, 64, 128, 254, 255] {
                for d in [0, 1, 64, 128, 254, 255] {
                    for opacity in [0., 1. / 256., 0.5, 255. / 256., 1.] {
                        let mut a0 = [d, 255 - d, d / 2, 71];
                        let mut b = a0;
                        let src = [s, 255 - s, s / 2, a];
                        original_core::composite_native_rgba(&mut a0, src, opacity, mode);
                        crysirror_image::composition::blend_pixel(&mut b, src, opacity, profile);
                        assert_eq!(a0, b);
                        pixels += 1;
                    }
                }
            }
        }
    }
    let source = image(7, 5, 91);
    let target = image(13, 11, 47);
    let mut draws = 0;
    for (mode, profile) in [
        (1, FloatBlend::Coverage),
        (0x20, FloatBlend::Coverage),
        (128, FloatBlend::Coverage),
        (2, FloatBlend::Add),
        (3, FloatBlend::Subtract),
        (240, FloatBlend::ConstantInterpolation256),
    ] {
        for opacity in [0., 1. / 256., 0.5, 255. / 256., 1.] {
            for ignore in [false, true] {
                for src in [
                    [-1.25, -0.5, 8., 6.],
                    [0., 0., 7., 5.],
                    [1.5, 0.75, 3.5, 2.5],
                ] {
                    for dest in [
                        [-2.25, -1.5, 12., 8.],
                        [0., 0., 7., 5.],
                        [2.25, 3.5, 5., 6.],
                    ] {
                        for clip in [None, Some([1.25, 0.75, 8.5, 6.25])] {
                            for quad in [
                                None,
                                Some([[0.25, -0.5], [-1.5, 7.], [10.5, 10.], [12.25, 2.5]]),
                            ] {
                                for linear in [false, true] {
                                    let mut a = target.clone();
                                    let mut b = a.clone();
                                    let nc = clip.map(|c| graph::RuntimeClipRect {
                                        x: c[0],
                                        y: c[1],
                                        width: c[2],
                                        height: c[3],
                                    });
                                    original_snapshot::image(
                                        &mut a, &source, dest, src, opacity, mode, nc, ignore,
                                        quad, linear,
                                    );
                                    b.draw_image(
                                        (&source).into(),
                                        FloatDraw {
                                            destination: dest,
                                            source: src,
                                            opacity,
                                            ignore_source_alpha: ignore,
                                            blend: profile,
                                            quad,
                                            sampling: if quad.is_none() {
                                                FloatSampling::NearestClamp
                                            } else if linear {
                                                FloatSampling::Bilinear16Transparent
                                            } else {
                                                FloatSampling::NearestTransparent
                                            },
                                            clip,
                                        },
                                    )
                                    .unwrap();
                                    assert_eq!(
                                        a.rgba, b.rgba,
                                        "mode {mode} opacity {opacity} dest {dest:?} src {src:?} quad {quad:?} linear {linear} clip {clip:?}"
                                    );
                                    draws += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut packed = 0;
    for mode in [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x40, 0x41,
        0x80, 0xc0, 0xc1, 0xf0, 0xff,
    ] {
        for sf in [1, 2] {
            for df in [1, 2] {
                for parameter in [0, 1, 63, 128, 255, 256] {
                    for (x, y) in [(-3, -2), (-1, 0), (0, 0), (3, 2)] {
                        let mut a = target.clone();
                        let mut b = a.clone();
                        let expected = original_bitmap::blit_mode(
                            &mut a, df, &source, sf, x, y, mode, parameter,
                        );
                        let actual = crysirror_image::packed::blit(
                            &mut b, df, &source, sf, x, y, mode, parameter,
                        )
                        .unwrap();
                        assert_eq!(actual, expected);
                        assert_eq!(a, b, "packed mode {mode} formats {sf}->{df} p {parameter}");
                        packed += 1;
                    }
                }
            }
        }
    }
    let mut native_bitmap_routes = 0;
    for sf in [1, 2] {
        for df in [1, 2] {
            for parameter in [0, 1, 63, 128, 254, 255, 256] {
                for (x, y) in [(-3, -2), (-1, 0), (0, 0), (3, 2)] {
                    let mut a = target.clone();
                    let mut b = a.clone();
                    let expected = original_bitmap::blit_alpha_over_parameter(
                        &mut a, df, &source, sf, x, y, parameter,
                    );
                    let actual = crysirror_image::packed::alpha_over_parameter(
                        &mut b, df, &source, sf, x, y, parameter,
                    )
                    .unwrap();
                    assert_eq!(expected, actual);
                    assert_eq!(a, b);
                    native_bitmap_routes += 1;
                }
            }
        }
    }
    for format in [1, 2] {
        for transition in 0..=256 {
            for sa in [0, 1, 127, 254, 255] {
                for da in [0, 1, 127, 254, 255] {
                    let p = [15, 80, 250, da];
                    let s = [240, 90, 3, sa];
                    assert_eq!(
                        original_bitmap::crossfade_pixel(p, s, format, transition),
                        crysirror_image::packed::mix_pixel(p, s, format, transition).unwrap()
                    );
                    native_bitmap_routes += 1;
                }
            }
        }
    }
    for format in [1, 2] {
        for transition in [0, 1, 63, 128, 254, 255, 256] {
            assert_eq!(
                original_bitmap::crossfade_cache(&target, &source, format, transition).unwrap(),
                crysirror_image::packed::crossfade(&target, &source, format, transition).unwrap()
            );
            native_bitmap_routes += 1;
        }
    }
    for (width, height) in [(0, 5), (7, 0), (0, 0)] {
        let empty = DecodedImage {
            width,
            height,
            rgba: Vec::new(),
        };
        for sf in [1, 2] {
            for df in [1, 2] {
                for mode in [0, 1, 0x20, 0x80, 0xf0] {
                    let mut a = target.clone();
                    let mut b = a.clone();
                    let expected =
                        original_bitmap::blit_mode(&mut a, df, &empty, sf, 0, 0, mode, 0);
                    assert_eq!(
                        expected,
                        crysirror_image::packed::blit(&mut b, df, &empty, sf, 0, 0, mode, 0)
                            .unwrap()
                    );
                    assert_eq!(a, b);
                    native_bitmap_routes += 1;
                }
            }
        }
    }
    let args: Vec<_> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).unwrap();
    let face = args[2].parse().unwrap();
    let font = FontArc::new(FontVec::try_from_vec_and_index(bytes.clone(), face).unwrap());
    let mut text = TextSystem::default();
    text.add_font_face("reference", bytes, face).unwrap();
    let mut glyphs = 0;
    for ch in ['A', 'g', '日', '本', '語'] {
        for size in [12., 23.5, 32.] {
            for baseline in [[0.25, 24.5], [3., 35.], [-3.25, 18.75]] {
                for bold in [false, true] {
                    for italic in [false, true] {
                        for opacity in [0.25, 0.5, 1.] {
                            for clip in [None, Some([1.5, 2.5, 35., 31.])] {
                                let mut a = image(48, 48, 73);
                                let mut b = a.clone();
                                let id = font.glyph_id(ch);
                                assert_ne!(id.0, 0);
                                let color = [0.2, 0.6, 0.9, opacity];
                                original_snapshot::glyph(
                                    &mut a,
                                    &font,
                                    id.with_scale_and_position(
                                        PxScale::from(size),
                                        ab_glyph::point(baseline[0], baseline[1]),
                                    ),
                                    color,
                                    bold,
                                    italic,
                                    clip.map(|c| graph::RuntimeClipRect {
                                        x: c[0],
                                        y: c[1],
                                        width: c[2],
                                        height: c[3],
                                    }),
                                );
                                let cell = text
                                    .rasterize_glyph_cell(
                                        "reference",
                                        GlyphRaster {
                                            glyph_id: u32::from(id.0),
                                            size,
                                            baseline,
                                        },
                                    )
                                    .unwrap();
                                b.draw_coverage(
                                    cell.origin,
                                    cell.width,
                                    cell.height,
                                    &cell.coverage,
                                    color,
                                    bold,
                                    italic,
                                    clip,
                                )
                                .unwrap();
                                assert_eq!(
                                    a, b,
                                    "glyph {ch} size {size} baseline {baseline:?} bold {bold} italic {italic}"
                                );
                                glyphs += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    println!(
        "PASS: BGI float blends={pixels}, rect/quad/clip/sampling={draws}, packed kernels={packed}, native bitmap routes={native_bitmap_routes}, exact positioned glyphs={glyphs}"
    );
}
