// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "arc-formats")]
//! End-to-end CPU image/resource tests. Every native byte stream is generated here.
use crysirror_core::{state::ClientId, vfs::VfsProvider, Error};
use crysirror_engine::{App, Value};
use crysirror_resource::MemoryKeyProvider;
use crysirror_runtime::{assets::Surface, Runtime};
use serde_json::json;
use std::sync::Arc;

const QNT_PIXELS: [u8; 16] = [
    20, 40, 60, 255, 40, 60, 80, 255, 60, 80, 100, 255, 80, 100, 120, 255,
];
fn app(root: &std::path::Path) -> App {
    std::fs::create_dir(root.join("assets")).unwrap();
    std::fs::write(root.join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"native image integration","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    App::new(
        Runtime::load(&root.join("project.json"), &root.join("state")).unwrap(),
        "a",
    )
    .unwrap()
}
fn call(
    app: &mut App,
    class: &'static str,
    method: &str,
    args: serde_json::Value,
) -> anyhow::Result<Value> {
    let cx = app.context();
    let object = cx.singleton(class)?;
    let args: Vec<_> = args
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(Value::from_json)
        .collect();
    cx.call(object, method, &args)
}
fn client(app: &App, name: &str) -> ClientId {
    app.runtime().unwrap().client_id(name).unwrap()
}
fn surface(app: &App, client: ClientId, slot: u32) -> Surface {
    app.runtime()
        .unwrap()
        .assets
        .surface(client, &format!("__surface/{slot}"))
        .unwrap()
        .clone()
}
fn unchanged(app: &App, client: ClientId, slot: u32, before: &Surface) {
    let now = surface(app, client, slot);
    assert_eq!(now.revision, before.revision);
    assert_eq!(now.rgba, before.rgba);
    assert_eq!((now.width, now.height), (before.width, before.height));
}
/// One final stored DEFLATE block inside RFC1950, avoiding a test-only codec dependency.
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    assert!(raw.len() < 65536);
    let len = raw.len() as u16;
    let mut out = vec![0x78, 1, 1];
    out.extend(len.to_le_bytes());
    out.extend((!len).to_le_bytes());
    out.extend(raw);
    let (mut a, mut b) = (1u32, 0u32);
    for &v in raw {
        a = (a + u32::from(v)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend(((b << 16) | a).to_be_bytes());
    out
}
fn qnt() -> Vec<u8> {
    let mut residual = Vec::new();
    for channel in [2, 1, 0] {
        let tl = QNT_PIXELS[channel];
        let tr = QNT_PIXELS[4 + channel];
        let bl = QNT_PIXELS[8 + channel];
        let br = QNT_PIXELS[12 + channel];
        residual.extend([
            tl,
            tl.wrapping_sub(bl),
            tl.wrapping_sub(tr),
            (((u16::from(bl) + u16::from(tr)) / 2) as u8).wrapping_sub(br),
        ]);
    }
    let compressed = zlib_stored(&residual);
    let mut out = b"QNT\0".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend([0; 12]);
    for v in [2u32, 2, 24, 0, compressed.len() as u32, 0] {
        out.extend(v.to_le_bytes());
    }
    out.extend([0; 24]);
    out.extend(compressed);
    out
}
fn teyl(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = b"TEYL".to_vec();
    out.extend(width.to_le_bytes());
    out.extend(height.to_le_bytes());
    for p in rgba.chunks_exact(4) {
        out.extend([p[2], p[1], p[0], p[3]]);
    }
    out
}
fn sar(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let start = 6 + files.iter().map(|(n, _)| n.len() + 1 + 8).sum::<usize>();
    let mut out = (files.len() as u16).to_be_bytes().to_vec();
    out.extend((start as u32).to_be_bytes());
    let mut offset = 0;
    for (name, data) in files {
        out.extend(name.as_bytes());
        out.push(0);
        out.extend((offset as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        offset += data.len();
    }
    for (_, data) in files {
        out.extend(data);
    }
    out
}
fn mount(app: &mut App, archive: &str, point: &str) {
    call(
        app,
        "Resources",
        "mount",
        json!([archive,point,{"format":"nscripter/sar"}]),
    )
    .unwrap();
}

#[test]
fn mounted_native_images_feed_bitmap_resources_and_gltf_material_mips() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let a = client(&app, "a");
    let b = client(&app, "b");
    let formats = call(&mut app, "Resources", "imageFormats", json!([]))
        .unwrap()
        .to_json();
    assert_eq!(formats["native"].as_array().unwrap().len(), 96);
    for id in [
        "alice-soft/qnt",
        "almond-collective/teyl",
        "kirikiri/tlg",
        "entis/eri",
    ] {
        assert!(formats["native"].as_array().unwrap().contains(&json!(id)));
    }
    let other = [
        1u8, 2, 3, 255, 5, 6, 7, 255, 9, 10, 11, 255, 13, 14, 15, 255,
    ];
    let mut geometry = Vec::new();
    for point in [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
        for v in point {
            geometry.extend(v.to_le_bytes());
        }
    }
    let gltf=serde_json::to_vec(&json!({"asset":{"version":"2.0"},"buffers":[{"uri":"triangle.bin","byteLength":36}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}],"images":[{"uri":"color.qnt"},{"uri":"rough.teyl"}],"textures":[{"source":0},{"source":1}],"materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicRoughnessTexture":{"index":1}}}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"material":0}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0})).unwrap();
    let package = sar(&[
        ("models/color.qnt", qnt()),
        ("models/rough.teyl", teyl(2, 2, &other)),
        ("models/triangle.bin", geometry),
        ("models/triangle.gltf", gltf),
    ]);
    std::fs::write(tmp.path().join("assets/textures.sar"), package).unwrap();
    mount(&mut app, "textures.sar", "pack");
    let probe = call(
        &mut app,
        "Resources",
        "probeImage",
        json!(["pack/models/color.qnt"]),
    )
    .unwrap()
    .to_json();
    assert!(probe.as_array().unwrap().contains(&json!("alice-soft/qnt")));
    call(
        &mut app,
        "Bitmap",
        "load",
        json!([1, "pack/models/color.qnt"]),
    )
    .unwrap();
    assert_eq!(surface(&app, a, 1).rgba.as_ref(), QNT_PIXELS);
    call(
        &mut app,
        "Resources",
        "decodeImage",
        json!([2, "pack/models/rough.teyl"]),
    )
    .unwrap();
    assert_eq!(surface(&app, a, 2).rgba.as_ref(), other);
    let runtime = app.runtime_mut().unwrap();
    let document = runtime
        .assets
        .read_for(a, "pack/models/triangle.gltf")
        .unwrap();
    let assets = std::cell::RefCell::new(&mut runtime.assets);
    let model = crysirror_asset::pbr::load_pbr_with_decoder(
        &document,
        |name| {
            Ok(assets
                .borrow_mut()
                .read_for(a, &format!("pack/models/{name}"))?
                .to_vec())
        },
        |name, bytes| {
            assets
                .borrow()
                .decode_image(a, &format!("pack/models/{name}"), bytes)
        },
    )
    .unwrap();
    assert_eq!(model.meshes[0][0].vertices.len(), 3);
    assert_eq!(model.meshes[0][0].indices, [0, 1, 2]);
    assert_eq!(model.materials[0].base_color_texture, Some(0));
    assert_eq!(model.materials[0].metallic_roughness_texture, Some(1));
    assert_eq!(model.images.len(), 2);
    assert_eq!(model.images[0].levels[0], QNT_PIXELS);
    assert!(model.images[0].srgb);
    assert_eq!(model.images[0].levels.len(), 2);
    assert_eq!(model.images[0].levels[1], [55, 74, 93, 255]);
    assert_eq!(model.images[1].levels[0], other);
    assert!(!model.images[1].srgb);
    assert_eq!(model.images[1].levels[1], [7, 8, 9, 255]);
    drop(assets);
    assert!(app
        .runtime_mut()
        .unwrap()
        .assets
        .read_for(b, "pack/models/color.qnt")
        .is_err());
    app.context().state().unwrap().client = b;
    assert!(call(
        &mut app,
        "Bitmap",
        "load",
        json!([1, "pack/models/color.qnt"])
    )
    .is_err());
    assert!(call(&mut app, "Bitmap", "info", json!([1])).is_err());
}

/// Native FC01 1.01 stream with literal LZSS data; test-local key, no title database.
fn encrypted_mcg(key: u8) -> Vec<u8> {
    let mut out = vec![0; 64];
    out[..8].copy_from_slice(b"MCG 1.01");
    for (at, value) in [(16, 64u32), (28, 1), (32, 1), (36, 24), (40, 4)] {
        out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let plain = [15u8, 30, 20, 10, 0];
    let mut k = key;
    for (i, v) in plain.iter().enumerate() {
        out.push((v ^ k).rotate_right(1));
        k = k.wrapping_add((plain.len() - 1 - i) as u8);
    }
    out
}
#[test]
fn native_cipher_gets_only_the_callers_host_key_and_failed_decode_preserves_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let a = client(&app, "a");
    let b = client(&app, "b");
    let keys = Arc::new(MemoryKeyProvider::default());
    keys.insert(a, "fc01/mcg", vec![0x53]).unwrap();
    app.context().state().unwrap().resources.keys = keys.clone();
    std::fs::write(tmp.path().join("assets/encrypted.mcg"), encrypted_mcg(0x53)).unwrap();
    call(
        &mut app,
        "Resources",
        "decodeImage",
        json!([5, "encrypted.mcg"]),
    )
    .unwrap();
    let before = surface(&app, a, 5);
    assert_eq!(before.rgba.as_ref(), [10, 20, 30, 255]);
    app.context().state().unwrap().client = b;
    call(
        &mut app,
        "Bitmap",
        "create",
        json!([5, 1, 1, [7, 8, 9, 10]]),
    )
    .unwrap();
    let b_before = surface(&app, b, 5);
    let error = call(
        &mut app,
        "Resources",
        "decodeImage",
        json!([5, "encrypted.mcg"]),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("key unavailable"));
    unchanged(&app, b, 5, &b_before);
    unchanged(&app, a, 5, &before);
    app.context().state().unwrap().client = a;
    keys.remove(a, "fc01/mcg").unwrap();
    assert!(call(
        &mut app,
        "Resources",
        "decodeImage",
        json!([5, "encrypted.mcg"])
    )
    .is_err());
    unchanged(&app, a, 5, &before);
}

fn rct_reference() -> Vec<u8> {
    let mut out = b"\x98\x5a\x92\x9aTC01".to_vec();
    out.extend(1u32.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(3u32.to_le_bytes());
    let name = b"base.teyl";
    out.extend((name.len() as u16).to_le_bytes());
    out.extend(name);
    out.extend([88, 0, 255]);
    out
}
fn rc8_mask(value: u8) -> Vec<u8> {
    let mut out = b"\x98\x5a\x92\x9a8_00".to_vec();
    out.extend(1u32.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    for i in 0..256u16 {
        out.extend([i as u8; 3]);
    }
    out.push(value);
    out
}
struct DependencyFault {
    corrupt: bool,
}
impl VfsProvider for DependencyFault {
    fn kind(&self) -> &'static str {
        "image-test-fault"
    }
    fn read(&self, path: &str) -> crysirror_core::Result<Vec<u8>> {
        if path == "base.teyl" {
            if self.corrupt {
                Ok(b"TEYL corrupt".to_vec())
            } else {
                Err(Error::Io("access denied test dependency".into()))
            }
        } else {
            Err(Error::NotFound)
        }
    }
}
#[test]
fn references_and_masks_resolve_in_the_mounted_client_vfs_and_fail_atomically() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path());
    let a = client(&app, "a");
    let b = client(&app, "b");
    let package = sar(&[
        ("overlay.rct", rct_reference()),
        ("base.teyl", teyl(1, 1, &[10, 20, 30, 100])),
        ("overlay_.rc8", rc8_mask(55)),
    ]);
    std::fs::write(tmp.path().join("assets/references.sar"), package).unwrap();
    mount(&mut app, "references.sar", "refs");
    call(
        &mut app,
        "Resources",
        "decodeImage",
        json!([9, "refs/overlay.rct"]),
    )
    .unwrap();
    let before = surface(&app, a, 9);
    assert_eq!(before.rgba.as_ref(), [10, 20, 30, 200]);
    app.context().state().unwrap().client = b;
    assert!(call(
        &mut app,
        "Resources",
        "decodeImage",
        json!([9, "refs/overlay.rct"])
    )
    .is_err());
    app.context().state().unwrap().client = a;
    let absent = sar(&[("overlay.rct", rct_reference())]);
    std::fs::write(tmp.path().join("assets/optional.sar"), absent).unwrap();
    mount(&mut app, "optional.sar", "optional");
    call(
        &mut app,
        "Resources",
        "decodeImage",
        json!([10, "optional/overlay.rct"]),
    )
    .unwrap();
    assert_eq!(surface(&app, a, 10).rgba.as_ref(), [0, 0, 0, 0]);
    for corrupt in [false, true] {
        app.runtime_mut()
            .unwrap()
            .host
            .fs
            .mount_provider(a, "refs", Arc::new(DependencyFault { corrupt }))
            .unwrap();
        let error = call(
            &mut app,
            "Resources",
            "decodeImage",
            json!([9, "refs/overlay.rct"]),
        )
        .unwrap_err();
        if !corrupt {
            assert!(format!("{error:#}").contains("access denied test dependency"));
        }
        unchanged(&app, a, 9, &before);
    }
}
