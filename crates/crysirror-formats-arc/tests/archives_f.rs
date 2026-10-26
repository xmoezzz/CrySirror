// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-authored native-layout fixtures; no title data, fonts, or key catalogue.
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use crysirror_resource::{read_all, DecodeContext, FormatRegistry, MemoryKeyProvider};
use std::{
    io::{Read, Write},
    sync::Arc,
};
fn le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
fn sh(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn fixed(s: &[u8], n: usize) -> Vec<u8> {
    let mut s = s.to_vec();
    s.resize(n, 0);
    s
}
fn zlib(b: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(b).unwrap();
    e.finish().unwrap()
}
fn literals(b: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    for b in b.chunks(8) {
        v.push(255);
        v.extend(b)
    }
    v
}
fn memory(v: Vec<u8>) -> Arc<dyn AssetReader> {
    Arc::new(MemoryAssetReader(v.into()))
}
fn extract(cx: &DecodeContext, id: &str, b: Vec<u8>) -> Vec<(String, Vec<u8>)> {
    let mut r = FormatRegistry::default();
    crysirror_formats_arc::register(&mut r).unwrap();
    let (_, a) = r
        .open(memory(b), Some(id), cx)
        .unwrap_or_else(|e| panic!("{id}: {e:#}"));
    a.entries()
        .iter()
        .map(|e| {
            (
                e.path.clone(),
                read_all(
                    a.open(e.id, cx)
                        .unwrap_or_else(|e| panic!("{id}: {e:#}"))
                        .as_ref(),
                    cx.limits.max_entry_size,
                )
                .unwrap(),
            )
        })
        .collect()
}
fn one(id: &str, b: Vec<u8>) -> Vec<u8> {
    let cx = DecodeContext::new(ClientId(77));
    let out = extract(&cx, id, b);
    assert_eq!(out.len(), 1);
    assert_eq!(cx.resident_bytes(), 0);
    out[0].1.clone()
}
fn pixel(id: &str, b: Vec<u8>, expected: &[u8]) {
    let out = one(id, b);
    let image = crysirror_image::decode_standard(&out).unwrap();
    assert_eq!(image.rgba, expected, "{id}");
}
#[test]
fn kaguya_link_indexes_and_native_lzss() {
    let mut v = b"LINK".to_vec();
    v.extend(le(1));
    v.extend(le(6));
    v.extend(b"a.txt\0");
    v.extend(le(26));
    v.extend(le(3));
    v.extend(b"abc");
    assert_eq!(one("kaguya/link", v), b"abc");
    for compressed in [false, true] {
        let mut v = b"LIN2".to_vec();
        v.extend(le(1));
        v.extend(sh(5));
        v.extend(b"a.txt".iter().map(|b| b ^ 255));
        v.extend(le(25));
        v.extend(le(if compressed { 8 } else { 3 }));
        v.extend(sh(u16::from(compressed)));
        if compressed {
            v.extend(le(3));
            v.extend([0xe0, b'a', b'b', b'c']);
        } else {
            v.extend(b"abc")
        }
        assert_eq!(one("kaguya/link2", v), b"abc");
    }
    for version in [3, 4, 5, 6] {
        let name = if version == 6 {
            b"a\0.\0t\0x\0t\0".to_vec()
        } else {
            b"a.txt".to_vec()
        };
        let mut v = format!("LINK{version}").into_bytes();
        if version == 3 {
            v.resize(8, 0)
        } else if version == 6 {
            v.extend([0, 0, 0])
        } else {
            v.resize(10, 0)
        }
        let hs = if version == 6 { 15 } else { 16 };
        v.extend(le((hs + name.len() + 3) as u32));
        v.extend(sh(0));
        v.extend([0; 7]);
        if version == 6 {
            v.extend(sh(name.len() as u16))
        } else {
            v.push(name.len() as u8);
            v.extend([0; 2]);
        }
        v.extend(name);
        v.extend(b"abc");
        v.extend(le(0));
        assert_eq!(one(&format!("kaguya/link{version}"), v), b"abc");
    }
    let mut v = b"WFL1".to_vec();
    v.extend(le(5));
    v.extend(b"a.txt".iter().map(|b| b ^ 255));
    v.extend(sh(0));
    v.extend(le(3));
    v.extend(b"abc");
    v.extend(le(0));
    assert_eq!(one("kaguya/wfl", v), b"abc");
}
fn animation_fixture(id: &str, delta: bool) -> Vec<u8> {
    let mut v = id.as_bytes().to_vec();
    if id.starts_with("AN0") || id == "AN10" {
        v.extend([0; 16]);
        v.extend(sh(0));
        v.extend(sh(0));
    } else if id.starts_with("AN") {
        v.extend(sh(0));
        v.extend(sh(0));
        v.extend(sh(0));
        if id == "AN21" {
            v.extend(b"[PIC]10")
        }
    }
    v.extend(sh(if delta { 2 } else { 1 }));
    if id.starts_with("PL") || id == "AN20" || id == "AN21" {
        v.extend([0; 16]);
    }
    v.extend(le(0));
    v.extend(le(0));
    v.extend(le(1));
    v.extend(le(1));
    if id != "AN00" {
        v.extend(le(4))
    }
    v.extend([3, 2, 1, 255]);
    if delta {
        v.push(4);
        v.extend(le(4));
        v.extend([1, 2, 3, 0]);
    }
    v
}
#[test]
fn all_seven_kaguya_picture_archives_decode_and_delta() {
    for id in ["AN00", "AN10", "AN20", "AN21", "PL00", "PL10"] {
        let delta = id == "AN21" || id == "PL10";
        let cx = DecodeContext::new(ClientId(77));
        let out = extract(
            &cx,
            &format!("kaguya/{}", id.to_ascii_lowercase()),
            animation_fixture(id, delta),
        );
        assert_eq!(out.len(), if delta { 2 } else { 1 });
        assert_eq!(
            crysirror_image::decode_standard(&out[0].1).unwrap().rgba,
            [1, 2, 3, 255]
        );
        if delta {
            assert_eq!(
                crysirror_image::decode_standard(&out[1].1).unwrap().rgba,
                [4, 4, 4, 255]
            );
        }
    }
}
#[test]
fn leaf_plain_compressed_and_embedded_key_archives() {
    for compressed in [false, true] {
        let mut v = b"\x1e\xaf".to_vec();
        v.extend(sh(1));
        v.extend(fixed(b"a.txt", 23));
        v.push(u8::from(compressed));
        let data = if compressed {
            let mut b = le(3).to_vec();
            b.extend(literals(b"abc"));
            b
        } else {
            b"abc".to_vec()
        };
        v.extend(le(data.len() as u32));
        v.extend(le(0));
        v.extend(data);
        assert_eq!(one("leaf/a", v), b"abc");
    }
    let mut v = b"ar10".to_vec();
    v.extend(le(1));
    v.extend(le(23));
    v.push(0x55);
    v.extend(b"a.txt\0");
    v.extend(le(0));
    v.extend(le(3));
    v.push(1 ^ 0x55);
    v.push(0x19);
    v.extend(b"abc".iter().map(|b| b ^ 0x19));
    assert_eq!(one("leaf/ar10", v), b"abc");
    let mut v = b"LAC\0".to_vec();
    v.extend(le(1));
    let mut name = b"a.txt".iter().map(|b| b ^ 255).collect::<Vec<_>>();
    name.resize(32, 0);
    v.extend(name);
    v.extend(le(3));
    v.extend(le(48));
    v.extend(b"abc");
    assert_eq!(one("leaf/lac", v), b"abc");
    for version in [1, 2] {
        let mut v = b"KCAP".to_vec();
        v.extend(le(1));
        if version == 2 {
            v.extend(le(1))
        }
        v.extend(fixed(b"a.txt", 24));
        let at = if version == 1 { 40 } else { 44 };
        v.extend(le(at));
        v.extend(le(12));
        v.extend(le(12));
        v.extend(le(3));
        v.extend(literals(b"abc"));
        assert_eq!(one("leaf/kcap", v), b"abc");
    }
    let mut v = le(1).to_vec();
    v.extend(fixed(b"a.txt", 16));
    v.extend(le(12));
    v.extend(le(1));
    v.extend(le(32));
    v.extend(le(12));
    v.extend(le(3));
    v.extend(literals(b"abc"));
    let mut cx = DecodeContext::new(ClientId(77));
    cx.parameters
        .insert("leaf/pak1".into(), serde_json::json!({"version":2}));
    assert_eq!(extract(&cx, "leaf/pak1", v)[0].1, b"abc");
    let mut v = vec![0; 32];
    v[28..30].copy_from_slice(&sh(1));
    v.extend([0; 2]);
    v.extend(fixed(
        &b"a.txt"
            .iter()
            .map(|b| b.rotate_left(4))
            .collect::<Vec<_>>(),
        12,
    ));
    v.extend([0; 2]);
    v.extend(le(0));
    v.extend([0; 4]);
    v.extend(le(3));
    v.extend([0; 4]);
    v.extend(b"abc");
    assert_eq!(one("leaf/pak2", v), b"abc");
}
#[test]
fn leafpack_uses_only_client_key_alias() {
    let mut cx = DecodeContext::new(ClientId(77));
    let keys = MemoryKeyProvider::default();
    keys.insert(cx.client, "fixture", vec![11, 22]).unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert(
        "leaf/leafpack".into(),
        serde_json::json!({"key_id":"fixture"}),
    );
    let mut v = b"LEAFPACK".to_vec();
    v.extend(sh(1));
    v.extend(
        b"abc"
            .iter()
            .enumerate()
            .map(|(i, b)| b.wrapping_add([11, 22][i % 2])),
    );
    let mut t = fixed(b"a       txt", 12);
    t.extend(le(10));
    t.extend(le(3));
    t.extend([0; 4]);
    for (i, b) in t.iter_mut().enumerate() {
        *b = b.wrapping_add([11, 22][i % 2]);
    }
    v.extend(t);
    let out = extract(&cx, "leaf/leafpack", v.clone());
    assert_eq!(out[0], ("a.txt".into(), b"abc".to_vec()));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| one("leaf/leafpack", v))).is_err()
    );
}
#[test]
fn leaf_cz10_scanline_planes_and_pak_images() {
    let mut v = b"cz10".to_vec();
    v.extend(le(1));
    v.extend(sh(1));
    v.extend(sh(1));
    v.extend([0; 4]);
    v.extend(le(5));
    v.extend(le(4));
    v.extend([0, 1, 2, 3, 255]);
    pixel("leaf/cz10", v, &[3, 2, 1, 255]);
    let mut v = le(0).to_vec();
    v.extend([0x5f, 0xf8, 0x6d, 0x75]);
    v.extend([0; 18]);
    v.extend(sh(16));
    v.extend([0; 8]);
    v.extend(sh(1));
    v.extend(sh(1));
    v.extend(sh(0x7c00));
    pixel("leaf/pak2-image", v, &[248, 0, 0, 255]);
    let mut v = vec![0; 24];
    v[4..8].copy_from_slice(&[0x88, 0x33, 0x67, 0x82]);
    v.extend(sh(1));
    v.extend(sh(1));
    v.extend([0; 4]);
    v.extend(sh(0));
    v.extend(sh(1));
    v.extend([0; 8]);
    v.extend(sh(0));
    v.extend(sh(0));
    v.extend([0; 4]);
    v.extend(sh(1));
    v.extend(sh(1));
    v.extend([0; 4]);
    v.extend(le(0));
    v.extend([0; 8]);
    v.extend([0; 36]);
    v.extend(sh(0x03e0));
    pixel("leaf/pak2-texture", v, &[0, 248, 0, 255]);
    let mut v = le(1).to_vec();
    v.extend(le(1));
    v.extend(le(0));
    v.extend(le(0));
    v.extend(sh(1));
    v.extend(sh(32));
    v.extend(le(1));
    v.extend(le(1));
    v.extend(le(0));
    v.extend([3, 2, 1, 0]);
    pixel("leaf/px", v, &[1, 2, 3, 255]);
}
#[test]
fn nekopack_gamedat_gpda_mgr_gxt() {
    let mut packed = zlib(b"abc");
    let size = packed.len() + 4;
    let mut k = ((size >> 3) + 0x22) as u8;
    for b in &mut packed {
        *b ^= k;
        k = k.wrapping_shl(3);
        if k == 0 {
            break;
        }
    }
    let mut v = b"NEKOPACK4A".to_vec();
    v.extend(le(22));
    v.extend(le(6));
    v.extend(b"a.txt\0");
    let key = b"a.txt".iter().map(|b| u32::from(*b)).sum::<u32>();
    v.extend(le(36 ^ key));
    v.extend(le(size as u32 ^ key));
    v.extend(le(0));
    v.extend(packed);
    v.extend(le(3));
    assert_eq!(one("nekopack/nekopack4", v), b"abc");
    for (ver, ns) in [(b'K', 16), (b'2', 32)] {
        let mut v = b"GAMEDAT PAC".to_vec();
        v.push(ver);
        v.extend(le(1));
        v.extend(fixed(b"a.txt", ns));
        v.extend(le(0));
        v.extend(le(3));
        v.extend(b"abc");
        assert_eq!(one("pajamas/gamedat", v), b"abc");
    }
    let mut v = b"GPDA".to_vec();
    v.extend(le(44));
    v.extend(le(0));
    v.extend(le(1));
    v.extend(le(41));
    v.extend(le(0));
    v.extend(le(3));
    v.extend(le(32));
    v.extend(le(5));
    v.extend(b"a.txtabc");
    assert_eq!(one("playstation/gpda", v), b"abc");
    let mut v = sh(1).to_vec();
    v.extend(le(3));
    v.extend(le(4));
    v.extend([2, b'a', b'b', b'c']);
    assert_eq!(one("propeller/mgr", v), b"abc");
    let mut v = b"GXT\0".to_vec();
    v.extend(le(0x10000003));
    v.extend(le(1));
    v.resize(32, 0);
    v.extend(le(64));
    v.extend(le(1));
    v.extend(le(u32::MAX));
    v.extend(le(0));
    v.extend(le(0x60000000));
    v.extend(le(0));
    v.extend(sh(1));
    v.extend(sh(1));
    v.extend(le(0));
    v.push(42);
    pixel("playstation/gxt", v, &[42, 42, 42, 255]);
}
#[test]
fn qlie_nested_abmp_sections() {
    let mut v = b"ABMP7".to_vec();
    v.resize(12, 0);
    v.extend(le(0));
    v.extend(le(3));
    v.extend(b"abc");
    assert_eq!(one("qlie/abmp7", v), b"abc");
    for ver in [10, 11, 12] {
        let mut v = fixed(format!("abmp{ver}").as_bytes(), 16);
        v.extend(fixed(b"abdata10", 16));
        v.extend(le(3));
        v.extend(b"abc");
        assert_eq!(one("qlie/abmp10", v), b"abc");
    }
}
#[test]
#[ignore = "requires pinned ARC_UNPACKER_SOURCE; reads external corpus at runtime, distributes no binary fixtures"]
fn pinned_external_corpus_matches_declared_pixels_and_bytes() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ARC_UNPACKER_SOURCE").expect("ARC_UNPACKER_SOURCE"),
    )
    .canonicalize()
    .unwrap();
    let rev = std::process::Command::new("git")
        .args(["-C", root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert_eq!(
        std::str::from_utf8(&rev.stdout).unwrap().trim(),
        "456834ecf2e5686813802c37efd829310485c57d"
    );
    let clean = std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap(),
            "diff",
            "--quiet",
            "HEAD",
            "--",
            "src",
            "tests",
        ])
        .status()
        .unwrap();
    assert!(clean.success());
    let read = |p: &str| std::fs::read(root.join("tests/dec").join(p)).unwrap();
    let mut cx = DecodeContext::new(ClientId(77));
    cx.limits.max_index_size = 128 << 20;
    cx.limits.max_entry_size = 128 << 20;
    cx.parameters
        .insert("leaf/pak1".into(), serde_json::json!({"version":1}));
    for (id, path) in [
        ("leaf/a", "leaf/files/a/plain.a"),
        ("leaf/a", "leaf/files/a/compressed.a"),
        ("leaf/ar10", "leaf/files/ar10/test.ar1"),
        ("leaf/kcap", "leaf/files/kcap/v1.pak"),
        ("leaf/kcap", "leaf/files/kcap/v2-uncompressed.pak"),
        ("leaf/kcap", "leaf/files/kcap/v2-compressed.pak"),
        ("leaf/lac", "leaf/files/lac/test.lac"),
        ("leaf/pak2", "leaf/files/pak2/test.pak"),
        ("nekopack/nekopack4", "nekopack/files/pak/nekopack4a.pak"),
    ] {
        let out = extract(&cx, id, read(path));
        assert_eq!(out[0].1, b"1234567890", "{path}");
        assert_eq!(out[1].1, b"abcdefghijklmnopqrstuvwxyz", "{path}");
    }
    for (version, path, expected) in [
        (
            1,
            "leaf/files/pak1/LEAFLOGO.PAK",
            [
                "leaf/files/pak1/leaflogo-out.c16",
                "leaf/files/pak1/leaflogo-out.grp",
            ],
        ),
        (
            2,
            "leaf/files/pak1/LEAFLOGO2.PAK",
            [
                "leaf/files/pak1/leaf-out.c16",
                "leaf/files/pak1/leaf-out.grp",
            ],
        ),
    ] {
        cx.parameters
            .insert("leaf/pak1".into(), serde_json::json!({"version":version}));
        let out = extract(&cx, "leaf/pak1", read(path));
        assert_eq!(out.len(), 2);
        for ((_, b), path) in out.iter().zip(expected) {
            assert_eq!(*b, read(path));
        }
    }
    let out = extract(&cx, "qlie/abmp7", read("qlie/files/abmp7/button.b"));
    for ((_, b), path) in out.iter().zip([
        "qlie/files/abmp7/out/base.png",
        "qlie/files/abmp7/out/button-sound-effect-1.ogg",
        "qlie/files/abmp7/out/button-sound-effect-2.ogg",
    ]) {
        assert_eq!(*b, read(path));
    }
    let cases: &[(&str, &str, bool, &[&str])] = &[
        (
            "leaf/cz10",
            "leaf/files/cz10/cbg01_0.cz1",
            false,
            &["leaf/files/cz10/cbg01_0-out.png"],
        ),
        (
            "leaf/cz10",
            "leaf/files/cz10/chi06_2.cz1",
            false,
            &[
                "leaf/files/cz10/chi06_2_000-out.png",
                "leaf/files/cz10/chi06_2_001-out.png",
                "leaf/files/cz10/chi06_2_002-out.png",
                "leaf/files/cz10/chi06_2_003-out.png",
                "leaf/files/cz10/chi06_2_004-out.png",
            ],
        ),
        (
            "leaf/pak2-image",
            "leaf/files/pak2-image/SpMoji-zlib",
            true,
            &["leaf/files/pak2-image/SpMoji-out.png"],
        ),
        (
            "leaf/pak2-image",
            "leaf/files/pak2-image/c010101b-zlib",
            true,
            &["leaf/files/pak2-image/c010101b-out.png"],
        ),
        (
            "leaf/pak2-texture",
            "leaf/files/pak2-texture/title3-zlib",
            true,
            &[
                "leaf/files/pak2-texture/title3_000-out.png",
                "leaf/files/pak2-texture/title3_001-out.png",
                "leaf/files/pak2-texture/title3_002-out.png",
                "leaf/files/pak2-texture/title3_003-out.png",
            ],
        ),
        (
            "leaf/px",
            "leaf/files/px/fonts-zlib.px",
            true,
            &[
                "leaf/files/px/fonts_000-out.png",
                "leaf/files/px/fonts_001-out.png",
            ],
        ),
        (
            "leaf/px",
            "leaf/files/px/bg097-zlib.px",
            true,
            &["leaf/files/px/bg097-out.png"],
        ),
        (
            "leaf/px",
            "leaf/files/px/inputName-zlib.px",
            true,
            &[
                "leaf/files/px/inputName_000-out.png",
                "leaf/files/px/inputName_001-out.png",
                "leaf/files/px/inputName_002-out.png",
            ],
        ),
        (
            "leaf/px",
            "leaf/files/px/poi102-zlib.px",
            true,
            &["leaf/files/px/poi102-out.png"],
        ),
        (
            "leaf/px",
            "leaf/files/px/common_031-zlib.px",
            true,
            &["leaf/files/px/common_031-out.png"],
        ),
        (
            "leaf/px",
            "leaf/files/px/thumbnail0_001-zlib.px",
            true,
            &["leaf/files/px/thumbnail0_001-out.png"],
        ),
        (
            "playstation/gxt",
            "playstation/files/gxt/mask15-zlib.gxt",
            true,
            &["playstation/files/gxt/mask15-out.png"],
        ),
    ];
    for &(id, path, compressed, expected) in cases {
        let mut b = read(path);
        if compressed {
            let mut raw = Vec::new();
            flate2::read::ZlibDecoder::new(b.as_slice())
                .read_to_end(&mut raw)
                .unwrap();
            b = raw;
        }
        let out = extract(&cx, id, b);
        assert_eq!(out.len(), expected.len(), "{path}");
        for ((_, bytes), expected) in out.iter().zip(expected) {
            let actual = crysirror_image::decode_standard(bytes).unwrap();
            let expected = crysirror_image::decode_standard(&read(expected)).unwrap();
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height),
                "{path}"
            );
            let mismatches: Vec<_> = actual
                .rgba
                .chunks_exact(4)
                .zip(expected.rgba.chunks_exact(4))
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .take(5)
                .collect();
            assert!(
                mismatches.is_empty(),
                "pixel mismatch {path}: {mismatches:?}"
            );
        }
    }
}
#[test]
fn nitroplus_client_profiles_preserve_names_and_cipher_bytes() {
    let mut cx = DecodeContext::new(ClientId(77));
    let keys = MemoryKeyProvider::default();
    keys.insert(cx.client, "npa-word", vec![0; 4]).unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert(
        "nitroplus/npa".into(),
        serde_json::json!({"file_name_key_id":"npa-word"}),
    );
    let mut t = le(5).to_vec();
    t.extend(
        b"a.txt"
            .iter()
            .enumerate()
            .map(|(i, b)| b.wrapping_sub((i as u8).wrapping_mul(0xfc))),
    );
    t.push(2);
    t.extend(le(0));
    t.extend(le(0));
    t.extend(le(3));
    t.extend(le(3));
    let mut v = b"NPA\x01\0\0\0".to_vec();
    v.extend(le(0));
    v.extend(le(0));
    v.extend([0, 0]);
    v.extend(le(1));
    v.extend(le(0));
    v.extend(le(1));
    v.extend([0; 8]);
    v.extend(le(t.len() as u32));
    v.extend(t);
    v.extend(b"abc");
    let out = extract(&cx, "nitroplus/npa", v);
    assert_eq!(out[0], ("a.txt".into(), b"abc".to_vec()));
    let keys = MemoryKeyProvider::default();
    keys.insert(cx.client, "sg-fixture", vec![1, 2, 3]).unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert(
        "nitroplus/npa-sg".into(),
        serde_json::json!({"key_id":"sg-fixture"}),
    );
    let mut t = le(1).to_vec();
    t.extend(le(10));
    t.extend(b"a\0.\0t\0x\0t\0");
    t.extend(le(3));
    t.extend(le(34));
    t.extend(le(0));
    let mut v = le(t.len() as u32).to_vec();
    v.extend(t.iter().enumerate().map(|(i, b)| b ^ [1, 2, 3][i % 3]));
    v.extend(b"abc".iter().enumerate().map(|(i, b)| b ^ [1, 2, 3][i % 3]));
    assert_eq!(extract(&cx, "nitroplus/npa-sg", v)[0].1, b"abc");
}
#[test]
fn qlie_pack_table_names_and_dictionary_compression() {
    // Identity byte-pair dictionary with a short literal run.
    let mut packed = b"1PC\xff".to_vec();
    packed.extend(le(0));
    packed.extend(le(3));
    packed.extend([255, 128, 254]);
    packed.extend(le(3));
    packed.extend(b"abc");
    let mut t = sh(5).to_vec();
    let x = (0x3e + 5) as u8;
    t.extend(b"a.txt".iter().enumerate().map(|(i, b)| {
        let i = (i + 1) as u8;
        b ^ (i ^ x).wrapping_add(i)
    }));
    t.extend((0u64).to_le_bytes());
    t.extend(le(packed.len() as u32));
    t.extend(le(3));
    t.extend(le(1));
    t.extend(le(0));
    t.extend(le(0));
    t.extend([0; 28]);
    t.extend(le(0));
    t.extend([0; 36]);
    t.extend([0; 256]);
    // A zero seed block has nonzero derive_seed; invert its words so every
    // xor term is zero and the archive seed is explicitly zero.
    let at = t.len() - 256;
    for i in 0..32 {
        let k = (i as u16 + 1).wrapping_mul(0x0307);
        for j in 0..4 {
            t[at + i * 8 + j * 2..at + i * 8 + j * 2 + 2].copy_from_slice(&k.to_le_bytes());
        }
    }
    let table_at = packed.len() as u64;
    let mut v = packed;
    v.extend(t);
    v.extend(b"FilePackVer3.0\0\0");
    v.extend(le(1));
    v.extend(table_at.to_le_bytes());
    assert_eq!(one("qlie/pack", v), b"abc");
}

#[test]
fn npk2_aes_cbc_client_key_and_segment_fixture() {
    let mut cx = DecodeContext::new(ClientId(77));
    let keys = MemoryKeyProvider::default();
    keys.insert(cx.client, "npk-fixture", (0..32).collect())
        .unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert(
        "nitroplus/npk2".into(),
        serde_json::json!({"key_id":"npk-fixture"}),
    );
    let data = vec![
        78, 80, 75, 50, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 80,
        0, 0, 0, 36, 148, 181, 230, 39, 141, 74, 214, 11, 63, 169, 121, 197, 123, 29, 72, 253, 21,
        174, 96, 67, 131, 104, 142, 138, 60, 235, 120, 141, 52, 35, 225, 224, 184, 120, 101, 92,
        154, 196, 133, 192, 36, 233, 168, 58, 91, 152, 35, 231, 154, 165, 231, 114, 73, 190, 103,
        205, 255, 233, 15, 27, 84, 105, 157, 147, 5, 205, 222, 83, 61, 30, 238, 79, 82, 155, 235,
        9, 83, 46, 116, 209, 208, 0, 136, 66, 34, 128, 57, 47, 14, 37, 104, 173, 168, 100, 54,
    ];
    assert_eq!(extract(&cx, "nitroplus/npk2", data)[0].1, b"abc");
}

#[test]
fn cpz5_client_plugin_fixture() {
    let mut cx = DecodeContext::new(ClientId(77));
    let keys = MemoryKeyProvider::default();
    keys.insert(cx.client, "cpz-fixture", (0..96).collect())
        .unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert("purple-software/cpz5".into(),serde_json::json!({"key_id":"cpz-fixture","hash_iv":[0x67452301u32,0xefcdab89u32,0x98badcfeu32,0x10325476u32],"hash_permutation":[0,1,2,3],"hash_xor":[0,0,0,0],"hash_add":[0,0,0,0],"crypt_23_mul":1,"crypt_1a":{"add1":0,"add2":0,"tail_sub":0},"crypt_1c":{"addends":[0,0,0,0],"init_key":0},"crypt_3":{"start_pos":0,"tail_xor":0}}));
    let data = vec![
        67, 80, 90, 53, 216, 83, 58, 254, 242, 152, 242, 55, 50, 58, 111, 122, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 25, 124, 222, 67, 21, 244, 101, 204, 60, 169, 22, 208, 154, 186,
        163, 151, 199, 111, 73, 188, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 90, 11, 102, 156, 5, 238,
        169, 248, 105, 62, 57, 44, 163, 144, 88, 207, 148, 239, 152, 193, 211, 56, 186, 45, 255,
        247, 194, 188, 192, 126, 203, 209, 132, 65, 218, 26, 25, 53, 12, 135, 182, 224, 22, 140,
        63, 93, 63, 185, 227, 97, 140, 79, 96, 99,
    ];
    assert_eq!(
        extract(&cx, "purple-software/cpz5", data)[0],
        ("a.txt".into(), b"abc".to_vec())
    );
}

#[test]
fn cpz6_client_plugin_fixture() {
    let mut cx = DecodeContext::new(ClientId(77));
    let keys = MemoryKeyProvider::default();
    keys.insert(cx.client, "cpz-fixture", (0..96).collect())
        .unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert("purple-software/cpz6".into(),serde_json::json!({"key_id":"cpz-fixture","hash_iv":[0x67452301u32,0xefcdab89u32,0x98badcfeu32,0x10325476u32],"hash_permutation":[0,1,2,3],"hash_xor":[0,0,0,0],"hash_add":[0,0,0,0],"crypt_23_mul":1,"crypt_1a":{"add1":0,"add2":0,"tail_sub":0},"crypt_1c":{"addends":[0,0,0,0],"init_key":0},"crypt_3":{"start_pos":0,"tail_xor":0}}));
    let data = vec![
        67, 80, 90, 54, 219, 83, 58, 254, 253, 152, 242, 55, 51, 58, 111, 122, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 26, 124, 222, 67, 22, 244, 101, 204, 61, 169, 22, 208, 155, 186,
        163, 151, 207, 111, 73, 188, 86, 169, 115, 251, 12, 191, 14, 26, 0, 0, 0, 0, 90, 11, 102,
        156, 5, 238, 169, 248, 105, 62, 57, 44, 163, 144, 88, 207, 148, 239, 152, 193, 211, 56,
        186, 45, 255, 247, 194, 188, 192, 126, 203, 209, 132, 65, 218, 26, 25, 53, 12, 135, 182,
        224, 22, 140, 63, 93, 63, 185, 227, 97, 140, 79, 96, 99,
    ];
    assert_eq!(
        extract(&cx, "purple-software/cpz6", data)[0],
        ("a.txt".into(), b"abc".to_vec())
    );
}
#[test]
fn kaguya_and_npa_encrypted_entries_use_client_key_aliases() {
    let mut cx = DecodeContext::new(ClientId(77));
    let keys = MemoryKeyProvider::default();
    keys.insert(cx.client, "kaguya-fixture", vec![0x5a])
        .unwrap();
    keys.insert(cx.client, "npa-word", vec![0; 4]).unwrap();
    keys.insert(cx.client, "npa-permutation", (0..=255).collect())
        .unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert(
        "kaguya/link3".into(),
        serde_json::json!({"key_id":"kaguya-fixture","decrypt_anm":true}),
    );
    let mut body = b"BM".to_vec();
    body.resize(54, 0);
    body.extend(b"abc".iter().map(|b| b ^ 0x5a));
    let mut v = b"LINK3".to_vec();
    v.resize(8, 0);
    v.extend(le((16 + 5 + body.len()) as u32));
    v.extend(sh(4));
    v.extend([0; 7]);
    v.push(5);
    v.extend([0; 2]);
    v.extend(b"a.bmp");
    v.extend(body);
    v.extend(le(0));
    let mut expected = b"BM".to_vec();
    expected.resize(54, 0);
    expected.extend(b"abc");
    assert_eq!(extract(&cx, "kaguya/link3", v)[0].1, expected);
    cx.parameters.insert("nitroplus/npa".into(),serde_json::json!({"file_name_key_id":"npa-word","data_key_id":"npa-word","key_id":"npa-permutation"}));
    let mut t = le(5).to_vec();
    t.extend(
        b"a.txt"
            .iter()
            .enumerate()
            .map(|(i, b)| b.wrapping_sub((i as u8).wrapping_mul(0xfc))),
    );
    t.push(2);
    t.extend(le(0));
    t.extend(le(0));
    t.extend(le(3));
    t.extend(le(3));
    let mut v = b"NPA\x01\0\0\0".to_vec();
    v.extend(le(0));
    v.extend(le(0));
    v.extend([0, 1]);
    v.extend(le(1));
    v.extend(le(0));
    v.extend(le(1));
    v.extend([0; 8]);
    v.extend(le(t.len() as u32));
    v.extend(t);
    v.extend([96, 98, 100]);
    assert_eq!(extract(&cx, "nitroplus/npa", v)[0].1, b"abc");
}
fn qlie_cipher_fixture(body: &[u8]) -> Vec<u8> {
    let mut t = sh(5).to_vec();
    let x = 0x43u8;
    t.extend(b"a.txt".iter().enumerate().map(|(i, b)| {
        let i = (i + 1) as u8;
        b ^ (i ^ x).wrapping_add(i)
    }));
    t.extend((0u64).to_le_bytes());
    t.extend(le(body.len() as u32));
    t.extend(le(body.len() as u32));
    t.extend(le(0));
    t.extend(le(1));
    t.extend(le(0));
    t.extend([0; 28]);
    t.extend(le(0));
    t.extend([0; 36]);
    for i in 0u16..32 {
        let k = (i + 1).wrapping_mul(0x307);
        for _ in 0..4 {
            t.extend(k.to_le_bytes());
        }
    }
    let mut v = body.to_vec();
    v.extend(t);
    v.extend(b"FilePackVer3.0\0\0");
    v.extend(le(1));
    v.extend((body.len() as u64).to_le_bytes());
    v
}

#[test]
fn qlie_basic_and_external_key_ciphers() {
    let mut cx = DecodeContext::new(ClientId(77));
    cx.parameters
        .insert("qlie/pack".into(), serde_json::json!({"basic":true}));
    assert_eq!(
        extract(
            &cx,
            "qlie/pack",
            qlie_cipher_fixture(&[151, 67, 203, 239, 147, 71, 207, 227])
        )[0]
        .1,
        b"abcdefgh"
    );
    let keys = MemoryKeyProvider::default();
    keys.insert(cx.client, "qlie-self-fixture", (0..=255).collect())
        .unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert(
        "qlie/pack".into(),
        serde_json::json!({"key1_id":"qlie-self-fixture"}),
    );
    assert_eq!(
        extract(
            &cx,
            "qlie/pack",
            qlie_cipher_fixture(&[99, 158, 155, 172, 64, 243, 94, 33])
        )[0]
        .1,
        b"abcdefgh"
    );
}
