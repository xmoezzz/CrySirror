// SPDX-License-Identifier: MPL-2.0
//! FBX and PMX interchange importers feed the ordinary glTF pipeline.
use crysirror_asset::{
    anim::{self, Layer},
    interchange::{self, Format},
    pbr::{self, PbrModel},
};
use glam::{Mat4, Vec3};

fn no_files(path: &str) -> anyhow::Result<Vec<u8>> {
    anyhow::bail!("unexpected external read `{path}`")
}

/// World-space skinned (or rigid) vertex positions under a pose.
fn posed_points(model: &PbrModel, layers: &[Layer]) -> Vec<Vec3> {
    posed(model, layers, false)
}

fn posed(model: &PbrModel, layers: &[Layer], skinned_only: bool) -> Vec<Vec3> {
    let global = anim::pose(model, layers);
    let mut out = Vec::new();
    for (i, n) in model.nodes.iter().enumerate() {
        let Some(mesh) = n.mesh else { continue };
        for prim in &model.meshes[mesh] {
            match (n.skin, &prim.skin_attrs) {
                (Some(s), Some(attrs)) => {
                    let skin = &model.skins[s];
                    let joints: Vec<Mat4> = skin
                        .joints
                        .iter()
                        .zip(&skin.inverse_bind)
                        .map(|(j, ib)| global[*j] * *ib)
                        .collect();
                    for (v, (j, w)) in prim.vertices.iter().zip(attrs) {
                        let p = Vec3::from(v.position);
                        out.push(
                            (0..4)
                                .map(|k| joints[j[k] as usize].transform_point3(p) * w[k])
                                .sum(),
                        );
                    }
                }
                _ if skinned_only => {}
                _ => out.extend(
                    prim.vertices
                        .iter()
                        .map(|v| global[i].transform_point3(Vec3::from(v.position))),
                ),
            }
        }
    }
    out
}

fn bounds(points: &[Vec3]) -> (Vec3, Vec3) {
    points.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(a, b), p| (a.min(*p), b.max(*p)),
    )
}

#[test]
fn blender_fbx_skin_animation_and_embedded_texture() {
    let bytes = include_bytes!("data/bar.fbx");
    assert_eq!(interchange::sniff(bytes), Format::FbxBinary);
    let model = pbr::load_pbr(bytes, no_files).unwrap();
    assert_eq!(model.skins.len(), 1);
    assert_eq!(model.skins[0].joints.len(), 2);
    assert_eq!(model.images.len(), 1, "embedded checker texture");
    assert_eq!([model.images[0].width, model.images[0].height], [4, 4]);
    assert_eq!(model.animations.len(), 1);
    assert!(
        (model.animations[0].duration - 1.0).abs() < 0.01,
        "{}",
        model.animations[0].duration
    );

    // Blender Z-up → glTF Y-up, metres: a bar from 0.5 m to 1.5 m along +Y,
    // 0.2 m wide.
    let (lo, hi) = bounds(&posed_points(&model, &[]));
    assert!(
        (lo - Vec3::new(-0.1, 0.5, -0.1)).abs().max_element() < 1e-3,
        "{lo}"
    );
    assert!(
        (hi - Vec3::new(0.1, 1.5, 0.1)).abs().max_element() < 1e-3,
        "{hi}"
    );

    // At the end of the clip the upper bone (head at 1 m) is bent 90° about
    // X: the top half lies horizontal near 1 m, reaching 0.5 m along ±Z.
    let end = posed_points(
        &model,
        &[Layer {
            clip: 0,
            time: 0.999,
            weight: 1.,
        }],
    );
    let (lo, hi) = bounds(&end);
    assert!(hi.y < 1.2, "bar not bent: {hi}");
    assert!(
        lo.z.abs().max(hi.z.abs()) > 0.4,
        "bend direction: {lo} {hi}"
    );

    // The entity importer reports nothing silently.
    let imported = crysirror_asset::import_gltf(bytes, &Default::default(), no_files).unwrap();
    assert!(
        imported
            .report
            .dropped
            .iter()
            .any(|d| d.contains("sampled")),
        "{:?}",
        imported.report.dropped
    );
}

#[test]
fn fbx_rejects_ascii_and_truncation() {
    let e =
        pbr::load_pbr(b"; FBX 7.4.0 project file\nFBXHeaderExtension: {", no_files).unwrap_err();
    assert!(format!("{e:#}").contains("ASCII FBX"));
    let bytes = include_bytes!("data/bar.fbx");
    assert!(pbr::load_pbr(&bytes[..bytes.len() / 2], no_files).is_err());
}

/// glTF Fox → Blender FBX → CrySirror must land where the glTF does, in rest
/// pose and mid-animation (skinned meshes only: Blender's glTF importer adds
/// an icosphere bone-display mesh that its FBX export keeps). Runs when the public demo asset and a converted
/// copy exist locally (tools/blender/fox_fbx.sh); never part of the repository.
#[test]
fn fox_fbx_matches_its_gltf_source_when_present() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (Ok(glb), Ok(fbx)) = (
        std::fs::read(root.join("examples/aaa-fox-sponza/assets/Fox.glb")),
        std::fs::read(root.join("target/verification/fox.fbx")),
    ) else {
        eprintln!("skipped: Fox.glb or target/verification/fox.fbx absent");
        return;
    };
    let a = pbr::load_pbr(&glb, no_files).unwrap();
    let b = pbr::load_pbr(&fbx, no_files).unwrap();
    assert_eq!(a.animations.len(), b.animations.len());
    assert_eq!(b.images.len(), a.images.len(), "embedded texture");
    let colour = |m: &PbrModel| {
        m.materials
            .iter()
            .find(|x| x.base_color_texture.is_some())
            .map(|x| (x.base_color_factor, x.emissive_factor))
    };
    assert_eq!(colour(&b).map(|c| c.1), Some([0.; 3]), "{:?}", b.materials);
    // Node rest transforms are not comparable: Blender writes the pose of the
    // current frame as each node's local transform. Animated poses are.
    for (c, t) in [(0usize, 0.5f32), (1, 0.37), (2, 0.21)] {
        let name = &a.animations[c].name;
        let j = b
            .animations
            .iter()
            .position(|x| x.name.ends_with(name.as_str()))
            .unwrap_or_else(|| {
                panic!(
                    "clip {name} missing: {:?}",
                    b.animations.iter().map(|x| &x.name).collect::<Vec<_>>()
                )
            });
        let clip = Some(name);
        let layer = |m: &PbrModel| {
            vec![Layer {
                clip: if std::ptr::eq(m, &a) { c } else { j },
                time: t,
                weight: 1.,
            }]
        };
        let (pa, pb) = (posed(&a, &layer(&a), true), posed(&b, &layer(&b), true));
        let ((la, ha), (lb, hb)) = (bounds(&pa), bounds(&pb));
        let size = (ha - la).max_element();
        let err = (la - lb).abs().max((ha - hb).abs()).max_element() / size;
        eprintln!("clip {clip:?} t={t}: relative bounds error {err:.5}");
        assert!(
            err < 1e-3,
            "clip {clip:?}: glTF {la}..{ha} vs FBX {lb}..{hb}"
        );
    }
}

/// Minimal PMX 2.0 writer for tests: UTF-16LE text, 4-byte indices.
fn pmx_fixture() -> Vec<u8> {
    let mut b = b"PMX ".to_vec();
    let f = |b: &mut Vec<u8>, v: f32| b.extend_from_slice(&v.to_le_bytes());
    let i = |b: &mut Vec<u8>, v: i32| b.extend_from_slice(&v.to_le_bytes());
    let text = |b: &mut Vec<u8>, s: &str| {
        let u: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        b.extend_from_slice(&(u.len() as i32).to_le_bytes());
        b.extend_from_slice(&u);
    };
    f(&mut b, 2.0);
    b.push(8);
    b.extend_from_slice(&[0, 0, 4, 4, 4, 4, 4, 4]);
    for s in ["テスト", "test", "", ""] {
        text(&mut b, s);
    }
    // A 2x20-unit quad in the XY plane facing -Z (MMD front), two bones.
    let verts = [
        ([-1., 0., 0.], 0.),
        ([1., 0., 0.], 0.),
        ([1., 20., 0.], 1.),
        ([-1., 20., 0.], 1.),
    ];
    i(&mut b, 4);
    for (p, w_upper) in verts {
        for v in p {
            f(&mut b, v);
        }
        for v in [0., 0., -1.] {
            f(&mut b, v);
        }
        f(&mut b, 0.);
        f(&mut b, 0.);
        b.push(1); // BDEF2
        i(&mut b, 0);
        i(&mut b, 1);
        f(&mut b, 1. - w_upper);
        f(&mut b, 1.); // edge
    }
    // Front faces are clockwise seen from the viewer in MMD (left-handed).
    i(&mut b, 6);
    for v in [0, 3, 2, 0, 2, 1] {
        i(&mut b, v);
    }
    i(&mut b, 1);
    text(&mut b, "skin.png");
    i(&mut b, 1);
    text(&mut b, "体");
    text(&mut b, "body");
    for v in [1., 1., 1., 1., 0., 0., 0., 5., 0.5, 0.5, 0.5] {
        f(&mut b, v);
    }
    b.push(0x01); // double-sided
    for v in [0., 0., 0., 1., 1.] {
        f(&mut b, v);
    }
    i(&mut b, 0); // texture
    i(&mut b, -1); // sphere
    b.push(0);
    b.push(1); // shared toon
    b.push(0);
    text(&mut b, "");
    i(&mut b, 6);
    i(&mut b, 2);
    for (name, pos, parent) in [("センター", [0., 0., 0.], -1), ("上半身", [0., 10., 0.], 0)]
    {
        text(&mut b, name);
        text(&mut b, "");
        for v in pos {
            f(&mut b, v);
        }
        i(&mut b, parent);
        i(&mut b, 0);
        b.extend_from_slice(&0x0002u16.to_le_bytes()); // rotatable, offset tail
        for v in [0., 1., 0.] {
            f(&mut b, v);
        }
    }
    i(&mut b, 3);
    // Vertex morph: lift the top edge 1 unit towards MMD front (-Z).
    text(&mut b, "まばたき");
    text(&mut b, "blink");
    b.push(1);
    b.push(1);
    i(&mut b, 2);
    for v in [2, 3] {
        i(&mut b, v);
        for d in [0., 0., -1.] {
            f(&mut b, d);
        }
    }
    // Group morph: half of the vertex morph.
    text(&mut b, "半分");
    text(&mut b, "half");
    b.push(1);
    b.push(0);
    i(&mut b, 1);
    i(&mut b, 0);
    f(&mut b, 0.5);
    // Bone morph (reported).
    text(&mut b, "ボーン");
    text(&mut b, "bone");
    b.push(1);
    b.push(2);
    i(&mut b, 1);
    i(&mut b, 1);
    for v in [0., 1., 0., 0., 0., 0., 1.] {
        f(&mut b, v);
    }
    b
}

#[test]
fn pmx_converts_to_metres_with_skin_and_texture() {
    let bytes = pmx_fixture();
    assert_eq!(interchange::sniff(&bytes), Format::Pmx);
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2,
        2,
        image::Rgba([255, 255, 255, 128]),
    ))
    .write_to(
        &mut std::io::Cursor::new(&mut png),
        image::ImageOutputFormat::Png,
    )
    .unwrap();
    let files = |p: &str| {
        if p == "skin.png" {
            Ok(png.clone())
        } else {
            anyhow::bail!("{p}")
        }
    };
    let model = pbr::load_pbr(&bytes, files).unwrap();
    assert_eq!(model.skins[0].joints.len(), 2);
    assert_eq!(model.images.len(), 1);
    assert_eq!(
        model.materials[0].alpha_mode,
        pbr::AlphaMode::Mask,
        "texture alpha → cutout"
    );
    assert!(model.materials[0].double_sided);
    let (lo, hi) = bounds(&posed_points(&model, &[]));
    assert!(
        (hi.y - 1.6).abs() < 1e-4 && lo.y.abs() < 1e-4 && (hi.x - 0.08).abs() < 1e-4,
        "{lo} {hi}"
    );
    // MMD's front (-Z, left-handed) becomes glTF +Z; winding still faces it.
    let prim = &model.meshes[0][0];
    for tri in prim.indices.chunks_exact(3) {
        let p = |k: usize| Vec3::from(prim.vertices[tri[k] as usize].position);
        let geometric = (p(1) - p(0)).cross(p(2) - p(0)).normalize();
        assert!(geometric.dot(Vec3::from(prim.vertices[tri[0] as usize].normal)) > 0.99);
        assert!(geometric.z > 0.99, "{geometric}");
    }
    // Morph targets survive conversion with names, metres and handedness.
    let names: Vec<&str> = prim.morphs.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["まばたき", "半分"]);
    let top_z = |w: f32, name: &str| {
        let v = prim.morphed(|n| if n == name { w } else { 0. }).unwrap();
        v.iter().filter(|v| v.position[1] > 1.).map(|v| v.position[2]).fold(f32::MIN, f32::max)
    };
    assert!((top_z(1., "まばたき") - 0.08).abs() < 1e-5);
    assert!((top_z(1., "半分") - 0.04).abs() < 1e-5);
    assert!(prim.morphed(|_| 0.).is_none());
    let imported = crysirror_asset::import_gltf(&bytes, &Default::default(), files).unwrap();
    let dropped = imported.report.dropped.join("; ");
    assert!(dropped.contains("1 bone/UV/material") && dropped.contains("toon"), "{dropped}");
}

#[test]
fn pmx_rejects_bad_indices() {
    let mut bytes = pmx_fixture();
    // Corrupt the first face index (after the 4 vertices) to point past them.
    let faces: Vec<u8> = [6i32, 0, 3].iter().flat_map(|v| v.to_le_bytes()).collect();
    let at = bytes
        .windows(12)
        .position(|w| w == faces.as_slice())
        .unwrap()
        + 4;
    bytes[at..at + 4].copy_from_slice(&99i32.to_le_bytes());
    let e = pbr::load_pbr(&bytes, no_files).unwrap_err();
    assert!(format!("{e:#}").contains("outside"), "{e:#}");
}
