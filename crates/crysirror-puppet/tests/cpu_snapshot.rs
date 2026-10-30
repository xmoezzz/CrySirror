//! Public engine capture APIs, without a VM, renderer, font, window or device.
use crysirror_puppet::{
    cpu_snapshot::{MeshSnapshot, SnapshotTexture},
    FramePart, PartBlend, PuppetFrame,
};
use std::sync::Arc;
fn quad(color: [f32; 4], blend: PartBlend) -> FramePart {
    FramePart {
        id: "authored".into(),
        texture: 0,
        positions: vec![[0., 0.], [4., 0.], [0., 2.], [4., 2.]],
        uvs: vec![[0., 0.], [1., 0.], [0., 1.], [1., 1.]],
        indices: vec![0, 1, 2, 2, 1, 3],
        colors: vec![color; 4],
        blend,
        masks: vec![],
        invert_mask: false,
        visible: true,
    }
}
fn frame(parts: Vec<FramePart>) -> PuppetFrame {
    PuppetFrame {
        size: [4., 2.],
        parts,
        textures: vec!["authored".into()],
    }
}
#[test]
fn bilinear_srgb_sampling_and_resource_replacement_do_not_change_old_captures() {
    let mut pixels: Arc<[u8]> = Arc::from([255, 0, 0, 255, 0, 255, 0, 255]);
    let declaration = frame(vec![quad([1.; 4], PartBlend::Normal)]);
    let old = MeshSnapshot::new(
        declaration.clone(),
        vec![SnapshotTexture {
            size: [2, 1],
            rgba: pixels.clone(),
        }],
    )
    .unwrap();
    Arc::make_mut(&mut pixels)[..4].copy_from_slice(&[0, 0, 255, 255]);
    let old_image = old.render([4, 2], &|| Ok(())).unwrap();
    assert_eq!(
        old_image.rgba,
        [255, 0, 0, 255, 225, 137, 0, 255, 137, 225, 0, 255, 0, 255, 0, 255].repeat(2)
    );
    let new = MeshSnapshot::new(
        declaration,
        vec![SnapshotTexture {
            size: [2, 1],
            rgba: pixels,
        }],
    )
    .unwrap();
    assert_eq!(
        &new.render([4, 2], &|| Ok(())).unwrap().rgba[..4],
        &[0, 0, 255, 255]
    );
}
#[test]
fn hdr_blend_modes_keep_destination_coverage_and_use_premultiplied_linear_rgb() {
    for (blend, expected) in [
        (PartBlend::Add, [207, 225, 240]),
        (PartBlend::Subtract, [165, 137, 99]),
        (PartBlend::Multiply, [152, 165, 177]),
        (PartBlend::Screen, [198, 207, 216]),
    ] {
        let snapshot = MeshSnapshot::new(
            frame(vec![
                quad([0.5, 0.5, 0.5, 1.], PartBlend::Normal),
                quad([0.25, 0.5, 0.75, 0.5], blend),
            ]),
            vec![SnapshotTexture {
                size: [1, 1],
                rgba: Arc::from([255; 4]),
            }],
        )
        .unwrap();
        for pixel in snapshot
            .render([4, 2], &|| Ok(()))
            .unwrap()
            .rgba
            .chunks_exact(4)
        {
            assert_eq!(pixel[3], 255);
            for i in 0..3 {
                assert!(
                    (i16::from(pixel[i]) - expected[i]).abs() <= 1,
                    "{blend:?}: {pixel:?}"
                );
            }
        }
    }
}
