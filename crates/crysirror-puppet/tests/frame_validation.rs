use crysirror_puppet::{FramePart, PartBlend, PuppetFrame};
fn triangle() -> FramePart {
    FramePart {
        id: "triangle".into(),
        texture: 0,
        positions: vec![[0., 0.], [1., 0.], [0., 1.]],
        uvs: vec![[0., 0.], [1., 0.], [0., 1.]],
        indices: vec![0, 1, 2],
        colors: vec![[1.; 4]; 3],
        blend: PartBlend::Normal,
        masks: vec![],
        invert_mask: false,
        visible: true,
    }
}
#[test]
fn worker_frames_reject_invalid_attributes_indices_and_mask_cycles() {
    let valid = PuppetFrame {
        size: [16., 16.],
        textures: vec!["texture.png".into()],
        parts: vec![triangle(), triangle()],
    };
    valid.validate().unwrap();
    let mut bad = valid.clone();
    bad.parts[0].uvs.pop();
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.parts[0].indices[0] = 3;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.parts[0].positions[0][0] = f32::NAN;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.parts[0].texture = 1;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.parts[0].masks.push(1);
    bad.parts[1].masks.push(0);
    assert!(bad.validate().is_err());
    let mut bad = valid;
    bad.size[0] = 0.;
    assert!(bad.validate().is_err());
}
#[test]
fn mesh_clip_preserves_uv_and_color_interpolation() {
    let mut part = triangle();
    part.colors = vec![[0., 0., 0., 1.], [1., 0., 0., 1.], [0., 1., 0., 1.]];
    crysirror_puppet::clip_part_rect(&mut part, [0.25, 0.25, 0.75, 0.75]).unwrap();
    assert!(!part.indices.is_empty());
    for ((p, uv), color) in part.positions.iter().zip(&part.uvs).zip(&part.colors) {
        assert!(p[0] >= 0.25 && p[1] >= 0.25 && p[0] <= 0.75 && p[1] <= 0.75);
        for i in 0..2 {
            assert!((p[i] - uv[i]).abs() < 1e-6);
            assert!((p[i] - color[i]).abs() < 1e-6);
        }
    }
    crysirror_puppet::clip_part_rect(&mut part, [2., 2., 3., 3.]).unwrap();
    assert!(part.indices.is_empty());
}
