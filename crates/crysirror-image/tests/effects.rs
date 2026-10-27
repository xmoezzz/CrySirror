// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_image::{
    effects::ImageEffect as E, transition::ImageTransition, vector::VectorPath, ImageSurface,
};
#[test]
fn gamma_gray_flip_and_raster_have_defined_pixels_and_preserve_alpha() {
    let image = ImageSurface {
        width: 3,
        height: 1,
        rgba: vec![128, 0, 0, 255, 0, 255, 0, 128, 11, 22, 33, 0],
    };
    let gamma = image
        .apply_effect(&E::Gamma {
            gamma: [2.; 3],
            floor: [0; 3],
            ceiling: [255; 3],
        })
        .unwrap();
    assert_eq!(gamma.rgba[0], 181);
    assert_eq!(&gamma.rgba[8..], &image.rgba[8..]);
    assert_eq!(gamma.rgba[7], 128);
    let gray = image.apply_effect(&E::Grayscale).unwrap();
    assert_eq!(&gray.rgba[..4], &[27, 27, 27, 255]);
    assert_eq!(&gray.rgba[4..8], &[182, 182, 182, 128]);
    let flip = E::Flip {
        horizontal: true,
        vertical: true,
    };
    let mirrored = image.apply_effect(&flip).unwrap();
    assert_eq!(&mirrored.rgba[..4], &image.rgba[8..]);
    assert_eq!(mirrored.apply_effect(&flip).unwrap(), image);
    assert_eq!(
        image
            .apply_effect(&E::Raster {
                amplitude: 0.,
                frequency: 1.,
                phase: 0.
            })
            .unwrap(),
        image
    );
    let shifted = image
        .apply_effect(&E::Raster {
            amplitude: 1.,
            frequency: 0.,
            phase: std::f32::consts::FRAC_PI_2,
        })
        .unwrap();
    assert_eq!(&shifted.rgba[..4], &image.rgba[4..8]);
}
#[test]
fn gaussian_uses_premultiplied_color_without_hidden_rgb_halos() {
    let image = ImageSurface {
        width: 3,
        height: 1,
        rgba: vec![255, 0, 0, 0, 0, 0, 255, 255, 255, 0, 0, 0],
    };
    let blurred = image.apply_effect(&E::GaussianBlur { sigma: 1. }).unwrap();
    for p in blurred.rgba.chunks_exact(4) {
        assert_eq!(&p[..3], &[0, 0, 255]);
        assert!(p[3] > 0 && p[3] < 255);
    }
    let flat = ImageSurface::filled(2, 3, [20, 40, 60, 128]).unwrap();
    assert_eq!(
        flat.apply_effect(&E::GaussianBlur { sigma: 16. }).unwrap(),
        flat
    );
}
#[test]
fn seeded_noise_and_cancellation_do_not_mutate_the_original() {
    let image = ImageSurface::filled(4, 4, [128, 128, 128, 70]).unwrap();
    let e = E::Noise {
        seed: 9,
        amplitude: 50,
        monochrome: true,
    };
    let a = image.apply_effect(&e).unwrap();
    assert_eq!(a, image.apply_effect(&e).unwrap());
    for p in a.rgba.chunks_exact(4) {
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
        assert_eq!(p[3], 70);
    }
    let count = std::cell::Cell::new(0);
    assert!(image
        .apply_effect_cancellable(&E::GaussianBlur { sigma: 2. }, &|| {
            count.set(count.get() + 1);
            anyhow::ensure!(count.get() < 3, "canceled");
            Ok(())
        })
        .is_err());
    assert!(image.rgba.chunks_exact(4).all(|p| p == [128, 128, 128, 70]));
}
#[test]
fn vector_paths_fill_and_stroke_real_pixels_and_reject_invalid_geometry() {
    let image = ImageSurface::filled(8, 8, [0; 4]).unwrap();
    let path:VectorPath=serde_json::from_value(serde_json::json!({"commands":[{"kind":"move","to":[1,1]},{"kind":"line","to":[7,1]},{"kind":"line","to":[7,7]},{"kind":"line","to":[1,7]},{"kind":"close"}],"fill":[255,0,0,255],"stroke":{"color":[0,255,0,255],"width":1},"transform":[1,0,0,1,0,0],"even_odd":false})).unwrap();
    let drawn = image.apply_effect(&E::Vector { path }).unwrap();
    assert_eq!(
        &drawn.rgba[(4 * 8 + 4) * 4..(4 * 8 + 4) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert!(drawn.rgba.chunks_exact(4).any(|p| p[1] > 0));
    assert_eq!(&drawn.rgba[..4], &[0; 4]);
}
#[test]
fn projective_mapping_has_exact_identity_and_visible_degenerate_errors() {
    let image = ImageSurface {
        width: 2,
        height: 2,
        rgba: vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    };
    let q = [[0., 0.], [2., 0.], [2., 2.], [0., 2.]];
    assert_eq!(
        image.apply_effect(&E::Perspective { quad: q }).unwrap(),
        image
    );
    let out = image
        .apply_effect(&E::Perspective {
            quad: [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
        })
        .unwrap();
    assert_eq!(&out.rgba[4..8], &[0; 4]);
    assert!(image
        .apply_effect(&E::Perspective {
            quad: [[0., 0.]; 4]
        })
        .is_err());
}
#[test]
fn transitions_have_exact_endpoints_mask_order_and_alpha_correct_midpoint() {
    let a = ImageSurface::filled(2, 1, [255, 0, 0, 0]).unwrap();
    let b = ImageSurface::filled(2, 1, [0, 0, 255, 255]).unwrap();
    let mut spec = ImageTransition {
        progress: 0.,
        softness: 0.,
        blur: 4.,
    };
    assert_eq!(a.transition_to(&b, None, &spec, &|| Ok(())).unwrap(), a);
    spec.progress = 1.;
    assert_eq!(a.transition_to(&b, None, &spec, &|| Ok(())).unwrap(), b);
    spec.progress = 0.5;
    spec.blur = 0.;
    let mid = a.transition_to(&b, None, &spec, &|| Ok(())).unwrap();
    assert_eq!(&mid.rgba[..4], &[0, 0, 255, 128]);
    let mask = ImageSurface {
        width: 2,
        height: 1,
        rgba: vec![255; 4].into_iter().chain([0, 0, 0, 255]).collect(),
    };
    let out = a.transition_to(&b, Some(&mask), &spec, &|| Ok(())).unwrap();
    assert_eq!(&out.rgba[..4], &b.rgba[..4]);
    assert_eq!(out.rgba[7], 0);
}

#[test]
fn sine_row_copy_retains_uncovered_pixels_and_uses_truncating_legacy_offsets() {
    let mut source = ImageSurface::filled(5, 2, [0, 0, 0, 255]).unwrap();
    for (i, p) in source.rgba.chunks_exact_mut(4).enumerate() {
        p[0] = (i as u8 + 1) * 10;
    }
    let dest = ImageSurface::filled(5, 2, [9, 9, 9, 77]).unwrap();
    let spec = crysirror_image::row_copy::SineRowCopy {
        amplitude: 3,
        lines: 4,
        cycle: 4,
        time: 1,
    };
    let out = dest.sine_copy_from(&source, &spec, &|| Ok(())).unwrap();
    assert_eq!(
        out.rgba.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>(),
        [20, 30, 40, 50, 9, 9, 9, 60, 70, 80]
    );
    assert_eq!(&out.rgba[16..20], &[9, 9, 9, 77]);
    let zero = crysirror_image::row_copy::SineRowCopy { cycle: 0, ..spec };
    assert_eq!(
        dest.sine_copy_from(&source, &zero, &|| Ok(())).unwrap(),
        dest
    );
}
