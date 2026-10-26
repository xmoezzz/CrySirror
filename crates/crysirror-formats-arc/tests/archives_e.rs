// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-authored fixtures. Optional external corpus tests read ignored local
//! reference data at runtime; no upstream or game data is copied into this tree.
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use crysirror_resource::{read_all, DecodeContext, DependencyResolver, MemoryKeyProvider};
use std::{collections::BTreeMap, io::Write, sync::Arc};
fn mem(b: Vec<u8>) -> Arc<dyn AssetReader> {
    Arc::new(MemoryAssetReader(Arc::from(b)))
}
fn cx() -> DecodeContext {
    DecodeContext::new(ClientId(91))
}
fn put(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn fixed(v: &[u8], n: usize) -> Vec<u8> {
    let mut b = v.to_vec();
    b.resize(n, 0);
    b
}
fn zlib(v: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(v).unwrap();
    z.finish().unwrap()
}
fn literals(v: &[u8]) -> Vec<u8> {
    v.chunks(8)
        .flat_map(|p| std::iter::once(255).chain(p.iter().copied()))
        .collect()
}
fn extract(id: &str, b: Vec<u8>, cx: &DecodeContext) -> Vec<(String, Vec<u8>)> {
    let format =
        crysirror_formats_arc::legacy_archive(id).unwrap_or_else(|| panic!("missing {id}"));
    let archive = format
        .open(mem(b), cx)
        .unwrap_or_else(|e| panic!("{id} index: {e:#}"));
    let mut out = Vec::new();
    for e in archive.entries() {
        let reader = archive
            .open(e.id, cx)
            .unwrap_or_else(|err| panic!("{id}:{}: {err:#}", e.path));
        out.push((
            e.path.clone(),
            read_all(reader.as_ref(), cx.limits.max_entry_size).unwrap(),
        ));
    }
    assert_eq!(cx.resident_bytes(), 0);
    out
}
fn body(id: &str, b: Vec<u8>) -> Vec<u8> {
    let rows = extract(id, b, &cx());
    assert_eq!(rows.len(), 1);
    rows[0].1.clone()
}
fn profile(c: &mut DecodeContext, id: &str, key: Vec<u8>, options: serde_json::Value) {
    let provider = MemoryKeyProvider::default();
    provider.insert(c.client, "fixture", key).unwrap();
    c.keys = Arc::new(provider);
    c.parameters.insert(id.into(), options);
}
struct Dependencies(BTreeMap<String, Vec<u8>>);
impl DependencyResolver for Dependencies {
    fn open(&self, path: &str) -> anyhow::Result<Arc<dyn AssetReader>> {
        Ok(mem(self
            .0
            .get(path)
            .ok_or_else(|| anyhow::anyhow!("missing {path}"))?
            .clone()))
    }
}
fn deps(c: &mut DecodeContext, source: &str, entries: Vec<(&str, Vec<u8>)>) {
    c.source_name = Some(source.into());
    c.resolver = Some(Arc::new(Dependencies(
        entries.into_iter().map(|(p, b)| (p.into(), b)).collect(),
    )));
}
#[test]
fn wad_pac2_and_bsc() {
    let mut b = b"AGAR".to_vec();
    b.extend(1u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(5u32.to_le_bytes());
    b.extend(b"a.txt");
    b.extend(3u64.to_le_bytes());
    b.extend(0u64.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(3u32.to_le_bytes());
    b.extend(b"dir");
    b.extend(1u32.to_le_bytes());
    b.extend(5u32.to_le_bytes());
    b.extend(b"a.txt");
    b.push(1);
    b.extend(b"ABC");
    assert_eq!(body("abstraction/wad", b), b"ABC");
    let mut b = fixed(b"PAC2", 12);
    b.extend(b"CCOD");
    b.extend(2u32.to_le_bytes());
    b.extend([0x11, 0x23]);
    b.extend(b"FNUM");
    b.extend(0u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(b"FILE");
    b.extend(0u32.to_le_bytes());
    b.extend(b"NAME");
    b.extend(6u32.to_le_bytes());
    b.extend(b"a.txt\0");
    b.extend(b"DATA");
    b.extend(3u32.to_le_bytes());
    let at = b.len();
    b.extend(
        b"ABC"
            .iter()
            .enumerate()
            .map(|(i, v)| v ^ [0x11, 0x23][(at + i) % 2]),
    );
    assert_eq!(body("almond-collective/pac2", b), b"ABC");
    let mut b = fixed(b"BSS-Composition", 32);
    b.extend([0; 64]);
    put(&mut b, 32 + 0x36, 3);
    b.extend(b"ABC");
    let out = body("bishop/bsc", b);
    assert_eq!(&out[64..], b"ABC");
}
#[test]
fn amuse_v1_v2_vfs_variants() {
    for ver in [1, 2] {
        let (start, name) = if ver == 1 { (0x3fe, 16) } else { (0x804, 32) };
        let mut b = vec![0; start + name + 8];
        if ver == 1 {
            b[..2].copy_from_slice(&1u16.to_le_bytes());
        } else {
            b[..4].copy_from_slice(b"PAC ");
            put(&mut b, 8, 1);
        }
        b[start..start + 5].copy_from_slice(b"d_a.x");
        put(&mut b, start + name, 3);
        let at = b.len();
        put(&mut b, start + name + 4, at as u32);
        b.extend(b"ABC");
        let rows = extract("amuse-craft/pac", b, &cx());
        assert_eq!(rows[0], ("d/a.x".into(), b"ABC".to_vec()));
    }
    for ver in [0x100u16, 0x101, 0x200] {
        let stride = if ver == 0x200 { 18 } else { 27 };
        let mut b = vec![0; 16 + stride];
        b[..2].copy_from_slice(b"VF");
        b[2..4].copy_from_slice(&ver.to_le_bytes());
        b[4..6].copy_from_slice(&1u16.to_le_bytes());
        b[6..8].copy_from_slice(&(stride as u16).to_le_bytes());
        put(&mut b, 8, stride as u32);
        if ver == 0x200 {
            let at = 16 + stride + 8 + 12;
            put(&mut b, 26, at as u32);
            put(&mut b, 30, 3);
            b.extend([0; 8]);
            for v in "a.txt\0".encode_utf16() {
                b.extend(v.to_le_bytes());
            }
        } else {
            b[16..21].copy_from_slice(b"a.txt");
            let at = b.len();
            put(&mut b, 35, at as u32);
            put(&mut b, 39, 3);
        }
        b.extend(b"ABC");
        assert_eq!(body("aoi/vfs", b), b"ABC");
    }
}
#[test]
fn segmented_blue_bin() {
    let first = zlib(b"AB");
    let second = zlib(b"CDE");
    let mut b = first.clone();
    b.extend(&second);
    let mut t = 1u32.to_le_bytes().to_vec();
    t.extend(2u32.to_le_bytes());
    t.extend(fixed(b"a.bin", 44));
    for (off, stored, orig) in [(0, first.len(), 2), (first.len(), second.len(), 3)] {
        t.extend(0u32.to_le_bytes());
        t.extend((off as u32).to_le_bytes());
        t.extend((stored as u32).to_le_bytes());
        t.extend((orig as u32).to_le_bytes());
    }
    let packed = zlib(&t);
    b.extend(&packed);
    b.extend((t.len() as u32).to_le_bytes());
    b.extend((packed.len() as u32).to_le_bytes());
    assert_eq!(body("bluearrowgarden/bin", b), b"ABCDE");
}
#[test]
fn chanchan_all_variants_int_and_bid() {
    for ver in 1..=3 {
        let mut b = if ver == 1 {
            1u32.to_le_bytes().to_vec()
        } else if ver == 2 {
            [272u32.to_le_bytes(), 1u32.to_le_bytes()].concat()
        } else {
            let mut b = vec![0; 32];
            put(&mut b, 0, 1);
            b
        };
        b.extend(fixed(b"a.txt", 256));
        if ver == 1 {
            b.extend(3u32.to_le_bytes());
            b.extend(b"ABC");
        } else if ver == 2 {
            b.extend(3u32.to_le_bytes());
            b.extend(272u32.to_le_bytes());
            b.extend(b"ABC");
        } else {
            b.extend(40u32.to_le_bytes());
            b.extend(304u32.to_le_bytes());
            b.extend([0; 8]);
            b.extend([0; 16]);
            b.extend(3u32.to_le_bytes());
            b.extend(4u32.to_le_bytes());
            b.extend([0; 12]);
            b.extend(literals(b"ABC"));
        }
        assert_eq!(body("chanchan/dat", b), b"ABC");
    }
    let mut b = b"KIF\0".to_vec();
    b.extend(1u32.to_le_bytes());
    b.extend(fixed(b"a.txt", 64));
    b.extend(80u32.to_le_bytes());
    b.extend(3u32.to_le_bytes());
    b.extend(b"ABC");
    assert_eq!(body("cat-system/int", b), b"ABC");
    let mut b = 32u32.to_le_bytes().to_vec();
    b.extend(0u32.to_le_bytes());
    b.extend(fixed(b"a.txt", 16));
    b.extend(0u32.to_le_bytes());
    b.extend(3u32.to_le_bytes());
    b.extend(b"ABC");
    assert_eq!(body("libido/bid", b), b"ABC");
}
#[test]
fn cronus_plain_and_compressed_index() {
    for compressed in [false, true] {
        let mut t = fixed(b"a.txt", 16);
        t.extend(0u32.to_le_bytes());
        t.extend(3u32.to_le_bytes());
        let stored = if compressed {
            let mut d = literals(&t);
            let key = b"CHERRYSOFT".iter().map(|x| *x as u32).sum::<u32>();
            for (i, v) in d.iter_mut().enumerate() {
                *v ^= key.wrapping_add(i as u32 * (key % 32)) as u8;
            }
            d
        } else {
            t
        };
        let mut b = b"CHERRY PACK 2.0\0".to_vec();
        b.extend((compressed as u32).to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend((28u32 + stored.len() as u32).to_le_bytes());
        b.extend(stored);
        b.extend(b"ABC");
        assert_eq!(body("cronus/pak", b), b"ABC");
    }
}
#[test]
fn pkwv_preserves_upstream_wave_output() {
    let mut b = b"PKWV".to_vec();
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(22050u32.to_le_bytes());
    b.extend(22050u32.to_le_bytes());
    b.extend(8u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend([0; 4]);
    b.extend(0u16.to_le_bytes());
    b.extend(fixed(b"a", 10));
    b.extend(3u32.to_le_bytes());
    b.extend(0u64.to_le_bytes());
    b.extend([0, 128, 255]);
    let out = body("crowd/pkwv", b);
    assert_eq!(out.len(), 49);
    assert_eq!(&out[0..4], b"RIFF");
    assert_eq!(u32::from_le_bytes(out[16..20].try_into().unwrap()), 18);
    assert_eq!(&out[46..], [0, 128, 255]);
}
#[test]
fn french_p_mbl_and_mrg() {
    let mut b = 0u32.to_le_bytes().to_vec();
    b.extend((1 ^ 0xe3df59acu32).to_le_bytes());
    b.extend(fixed(b"a.txt", 60).iter().map(|v| v ^ 0x3d));
    b.extend(76u32.to_le_bytes());
    b.extend((3 ^ 0xe3df59acu32).to_le_bytes());
    b.extend(
        b"ABC"
            .iter()
            .enumerate()
            .map(|(i, v)| v ^ b"a.txt"[i].wrapping_add(i as u8 + 3)),
    );
    assert_eq!(body("french-bread/p", b), b"ABC");
    for name in [16usize, 32] {
        let start = if name == 16 { 4 } else { 8 };
        let mut b = vec![0; start];
        put(&mut b, 0, 1);
        if start == 8 {
            put(&mut b, 4, name as u32);
        }
        b.extend(fixed(b"a.txt", name));
        b.extend((start as u32 + name as u32 + 8).to_le_bytes());
        b.extend(3u32.to_le_bytes());
        b.extend(b"ABC");
        assert_eq!(body("ivory/mbl", b), b"ABC");
    }
    for compressed in [false, true] {
        let data = if compressed {
            literals(b"ABC")
        } else {
            b"ABC".to_vec()
        };
        let mut t = vec![0; 64];
        t[..5].copy_from_slice(b"a.txt");
        put(&mut t, 14, 3);
        t[18] = compressed as u8;
        put(&mut t, 28, 80);
        put(&mut t, 60, 80 + data.len() as u32);
        let mut k = 37u8;
        let size = t.len();
        for (i, v) in t.iter_mut().enumerate() {
            *v = (*v ^ k).rotate_right(1);
            k = k.wrapping_add((size - i) as u8);
        }
        let mut b = b"MRG\0".to_vec();
        b.extend(0u32.to_le_bytes());
        b.extend(80u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(t);
        b.extend(data);
        assert_eq!(body("fc01/mrg", b), b"ABC");
    }
}
#[test]
fn concatenated_gzip_with_names_and_crc_failure() {
    let mut all = Vec::new();
    for (name, data) in [("a.txt", b"ABC".as_slice()), ("b.bin", b"DE")] {
        let mut z = flate2::GzBuilder::new()
            .filename(name)
            .comment("synthetic")
            .write(Vec::new(), flate2::Compression::fast());
        z.write_all(data).unwrap();
        all.extend(z.finish().unwrap());
    }
    let rows = extract("gnu/gzip", all.clone(), &cx());
    assert_eq!(
        rows,
        vec![
            ("a.txt".into(), b"ABC".to_vec()),
            ("b.bin".into(), b"DE".to_vec())
        ]
    );
    let at = all.len() - 8;
    all[at] ^= 1;
    let c = cx();
    let a = crysirror_formats_arc::legacy_archive("gnu/gzip")
        .unwrap()
        .open(mem(all), &c)
        .unwrap();
    assert!(a.open(1, &c).is_err());
}
#[test]
fn malie_plain_libs_nested_paths() {
    let mut b = b"LIBU".to_vec();
    b.extend(0x10000u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend([0; 4]);
    let name = "背景.txt"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    b.extend(fixed(&name, 68));
    b.extend(3u32.to_le_bytes());
    b.extend(96u32.to_le_bytes());
    b.extend([0; 4]);
    b.extend(b"ABC");
    let rows = extract("malie/libu", b, &cx());
    assert_eq!(rows[0], ("背景.txt".into(), b"ABC".to_vec()));
    let mut b = b"LIBP".to_vec();
    b.extend(3u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend([0; 4]);
    for (name, flags, off, size) in [("", 0, 1, 1), ("dir", 0, 2, 1), ("a.txt", 0x10000, 0, 3)] {
        b.extend(fixed(name.as_bytes(), 20));
        b.extend((flags as u32).to_le_bytes());
        b.extend((off as u32).to_le_bytes());
        b.extend((size as u32).to_le_bytes());
    }
    b.extend(0u32.to_le_bytes());
    b.resize(4096, 0);
    b.extend(b"ABC");
    let rows = extract("malie/libp", b, &cx());
    assert_eq!(rows[0], ("dir/a.txt".into(), b"ABC".to_vec()));
}
#[test]
fn companion_dsk_eagls_and_dx() {
    let mut c = cx();
    let mut table = 0x08000010u32.to_le_bytes().to_vec();
    table.extend(1u32.to_le_bytes());
    table.extend([0; 8]);
    table.extend(fixed(b"a.txt", 8));
    table.extend(0u32.to_le_bytes());
    table.extend(3u32.to_le_bytes());
    deps(&mut c, "dir/a.DSK", vec![("dir/a.PFT", table)]);
    assert_eq!(extract("abogado/dsk", b"ABC".to_vec(), &c)[0].1, b"ABC");
    let mut table = fixed(b"a.txt", 20);
    table.extend(1234u32.to_le_bytes());
    table.extend(3u32.to_le_bytes());
    table.extend([0; 20]);
    let mut seed = 91u32;
    for v in &mut table {
        seed = seed.wrapping_mul(214013).wrapping_add(2531011);
        *v ^= b"synthetic"[((seed >> 16) & 0x7fff) as usize % 9];
    }
    table.extend(91u32.to_le_bytes());
    let mut c = cx();
    profile(
        &mut c,
        "eagls/pak",
        b"synthetic".to_vec(),
        serde_json::json!({"key_id":"fixture"}),
    );
    deps(&mut c, "dir/a.pak", vec![("dir/a.idx", table)]);
    assert_eq!(extract("eagls/pak", b"ABC".to_vec(), &c)[0].1, b"ABC");
    let mut b = vec![0; 104];
    b[..2].copy_from_slice(b"DX");
    b[2..4].copy_from_slice(&2u16.to_le_bytes());
    put(&mut b, 4, 80);
    put(&mut b, 8, 104);
    put(&mut b, 12, 24);
    put(&mut b, 16, 16);
    put(&mut b, 20, 64);
    b[28..33].copy_from_slice(b"a.txt");
    put(&mut b, 24 + 16 + 32, 0);
    put(&mut b, 24 + 16 + 36, 3);
    put(&mut b, 24 + 16 + 40, u32::MAX);
    put(&mut b, 24 + 64, u32::MAX);
    put(&mut b, 24 + 68, u32::MAX);
    put(&mut b, 24 + 72, 1);
    put(&mut b, 24 + 76, 0);
    b.extend(b"ABC");
    for v in &mut b {
        *v ^= 0x55;
    }
    let mut c = cx();
    profile(
        &mut c,
        "dxlib/dx",
        vec![0x55],
        serde_json::json!({"key_id":"fixture"}),
    );
    assert_eq!(extract("dxlib/dx", b, &c)[0].1, b"ABC");
}
#[test]
fn glib_caller_mapping_and_lpk_filename_keys() {
    let mut t = vec![0; 16 + 24 + 6 + 80];
    t[..4].copy_from_slice(b"CDBD");
    put(&mut t, 4, 1);
    put(&mut t, 8, 30);
    put(&mut t, 16 + 8, u32::MAX);
    put(&mut t, 16 + 12, 0x100);
    put(&mut t, 16 + 20, 80);
    t[40..46].copy_from_slice(b"a.txt\0");
    put(&mut t, 46, 76);
    put(&mut t, 54, 3);
    let body_at = 92 + t.len() as u32;
    put(&mut t, 58, body_at);
    let mut b = fixed(b"GLibArchiveData2.1\0", 92);
    put(&mut b, 84, 92);
    put(&mut b, 88, t.len() as u32);
    b.extend(t);
    b.extend(b"ABC");
    let identity = serde_json::json!({"src":[0,1,2,3],"dst":[0,1,2,3],"funcs":[5,5]});
    let mut c = cx();
    c.parameters.insert(
        "glib/glib2".into(),
        serde_json::json!({"family":"musume","header":identity,"decoders":{"0":identity}}),
    );
    assert_eq!(extract("glib/glib2", b, &c)[0].1, b"ABC");
    let mut t = Vec::new();
    t.extend(1u32.to_le_bytes());
    t.extend([1, b'!', 0]);
    t.extend(8u32.to_le_bytes());
    t.extend([1, b'a', 0, 0, 1, 0, 0, 0]);
    t.push(0);
    t.extend(36u32.to_le_bytes());
    t.extend(3u32.to_le_bytes());
    assert_eq!(t.len(), 28);
    let key2 = (b'x' as u32).rotate_left(7);
    for p in t.chunks_exact_mut(4) {
        let n = u32::from_le_bytes(p.try_into().unwrap()) ^ key2;
        p.copy_from_slice(&n.to_le_bytes());
    }
    let mut b = b"LPK1".to_vec();
    b.extend((28 ^ key2).to_le_bytes());
    b.extend(t);
    b.extend(b"ABC");
    let mut c = cx();
    profile(
        &mut c,
        "lucifen/lpk",
        vec![0; 21],
        serde_json::json!({"key_id":"fixture"}),
    );
    c.source_name = Some("x.lpk".into());
    assert_eq!(extract("lucifen/lpk", b, &c)[0].1, b"!ABC");
}
fn png_pixel(bytes: &[u8]) -> Vec<u8> {
    image::load_from_memory(bytes)
        .unwrap()
        .to_rgba8()
        .into_raw()
}
#[test]
fn image_archives_blue_egr_mca_and_dzi() {
    let mut b = vec![0; 56];
    put(&mut b, 0, 4);
    put(&mut b, 4, 1);
    b[8..10].copy_from_slice(&56u16.to_le_bytes());
    b[10..12].copy_from_slice(&1u16.to_le_bytes());
    b[12..14].copy_from_slice(&1u16.to_le_bytes());
    b[16..20].copy_from_slice(&1f32.to_le_bytes());
    b[20..24].copy_from_slice(&1f32.to_le_bytes());
    b.extend([30, 20, 10, 40]);
    b.extend(b"a.png\0");
    assert_eq!(
        png_pixel(&body("bluearrowgarden/images", b)),
        [10, 20, 30, 40]
    );
    let mut b = vec![0; 12 + 0x574];
    put(&mut b, 0, 1);
    put(&mut b, 4, 1);
    put(&mut b, 8, 1);
    b[13] = 30;
    b[14] = 10;
    b[15] = 20;
    b.push(0);
    assert_eq!(png_pixel(&body("libido/egr", b)), [10, 20, 30, 255]);
    for compressed in [false, true] {
        let plain = [30, 20, 10, 0];
        let mut data = if compressed {
            literals(&plain)
        } else {
            plain.to_vec()
        };
        let mut k = 37u8;
        let n = data.len();
        for (i, v) in data.iter_mut().enumerate() {
            *v = (*v ^ k).rotate_right(1);
            k = k.wrapping_add((n - i) as u8);
        }
        let mut b = vec![0; 72];
        b[..4].copy_from_slice(b"MCA ");
        put(&mut b, 16, 36);
        put(&mut b, 32, 1);
        put(&mut b, 36, 40);
        put(&mut b, 40, compressed as u32);
        put(&mut b, 52, 1);
        put(&mut b, 56, 1);
        put(&mut b, 60, data.len() as u32);
        put(&mut b, 64, 4);
        b.extend(data);
        let mut c = cx();
        profile(
            &mut c,
            "fc01/mca",
            vec![37],
            serde_json::json!({"key_id":"fixture"}),
        );
        assert_eq!(
            png_pixel(&extract("fc01/mca", b, &c)[0].1),
            [10, 20, 30, 255]
        );
    }
    let image = crysirror_image::ImageSurface {
        width: 1,
        height: 1,
        rgba: vec![10, 20, 30, 40],
    };
    let png = crysirror_image::encode_png_uncompressed(&image).unwrap();
    let mut c = cx();
    deps(&mut c, "dir/a.dzi", vec![("dir/tex/block.png", png)]);
    assert_eq!(
        png_pixel(
            &extract(
                "malie/dzi",
                b"DZI\r\n1,1\r\n1\r\n1,1\r\nblock\r\n".to_vec(),
                &c
            )[0]
            .1
        ),
        [10, 20, 30, 40]
    );
}
fn decimal(n: usize) -> Vec<u8> {
    format!("{n:08}")
        .bytes()
        .map(|b| (b - b'0') ^ 127)
        .collect()
}
#[test]
fn cyberworks_appendix_dat_and_profiles() {
    for dep in [false, true] {
        let mut table = Vec::new();
        table.extend((if dep { 23u32 } else { 24 }).to_le_bytes());
        table.extend(7u32.to_le_bytes());
        table.extend(8u32.to_le_bytes());
        table.extend(8u32.to_le_bytes());
        table.extend(0u32.to_le_bytes());
        table.extend(b"w0");
        table.extend(u32::MAX.to_le_bytes());
        table.push(0);
        if !dep {
            table.push(0);
        }
        let packed = literals(&table);
        let mut b = if dep { Vec::new() } else { vec![0; 4] };
        b.extend(decimal(table.len()));
        b.extend(decimal(packed.len()));
        b.extend(packed);
        let mut c = cx();
        let id = if dep {
            "cyberworks/dat"
        } else {
            "cyberworks/appendix"
        };
        if dep {
            c.parameters
                .insert(id.into(), serde_json::json!({"data_files":["data.dat"]}));
            deps(
                &mut c,
                "dir/toc.dat",
                vec![("dir/data.dat", b"12345ABC".to_vec())],
            );
        } else {
            b.extend(b"12345ABC");
        }
        assert_eq!(extract(id, b, &c)[0].1, b"ABC");
    }
}
#[test]
fn noa_raw_and_pe_resource_tree() {
    let mut b = fixed(b"Entis\x1a\0\0\0\x04\0\x02\0\0\0\0ERISA-Archive file", 64);
    let mut t = 1u32.to_le_bytes().to_vec();
    t.extend(3u64.to_le_bytes());
    t.extend(0u32.to_le_bytes());
    t.extend(0u32.to_le_bytes());
    t.extend(66u64.to_le_bytes());
    t.extend([0; 8]);
    t.extend(0u32.to_le_bytes());
    t.extend(6u32.to_le_bytes());
    t.extend(b"a.txt\0");
    assert_eq!(t.len(), 50);
    b.extend(b"DirEntry");
    b.extend((t.len() as u64).to_le_bytes());
    b.extend(t);
    b.extend(b"filedata");
    b.extend(3u64.to_le_bytes());
    b.extend(b"ABC");
    assert_eq!(body("entis/noa", b), b"ABC");
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    put(&mut b, 60, 128);
    b[128..132].copy_from_slice(b"PE\0\0");
    b[134..136].copy_from_slice(&1u16.to_le_bytes());
    b[148..150].copy_from_slice(&224u16.to_le_bytes());
    b[152..154].copy_from_slice(&0x10bu16.to_le_bytes());
    put(&mut b, 152 + 32, 4096);
    put(&mut b, 152 + 36, 512);
    put(&mut b, 152 + 92, 16);
    put(&mut b, 152 + 112, 4096);
    put(&mut b, 152 + 116, 512);
    let at = 152 + 224;
    b[at..at + 5].copy_from_slice(b".rsrc");
    put(&mut b, at + 8, 512);
    put(&mut b, at + 12, 4096);
    put(&mut b, at + 16, 512);
    put(&mut b, at + 20, 512);
    b[512 + 14..512 + 16].copy_from_slice(&1u16.to_le_bytes());
    put(&mut b, 512 + 16, 10);
    put(&mut b, 512 + 20, 24);
    put(&mut b, 512 + 24, 4096 + 40);
    put(&mut b, 512 + 28, 3);
    b[552..555].copy_from_slice(b"ABC");
    b.extend(b"OVERLAY");
    let rows = extract("microsoft/exe", b, &cx());
    assert!(rows.contains(&("RC_DATA".into(), b"ABC".to_vec())));
    assert!(rows.contains(&("extra_data".into(), b"OVERLAY".to_vec())));
}
#[derive(Clone)]
enum UtfValue {
    Num(u64, u8),
    Text(&'static str),
}
fn utf(cols: Vec<(&'static str, UtfValue)>) -> Vec<u8> {
    let mut strings = vec![0];
    let mut descriptors = Vec::new();
    let mut row = Vec::new();
    for (name, value) in cols {
        let name_at = strings.len();
        strings.extend(name.bytes());
        strings.push(0);
        let kind = match &value {
            UtfValue::Num(_, kind) => *kind,
            UtfValue::Text(_) => 10,
        };
        descriptors.push(0x50 | kind);
        descriptors.extend((name_at as u32).to_be_bytes());
        match value {
            UtfValue::Num(v, k) => {
                let n = match k {
                    0 | 1 => 1,
                    2 | 3 => 2,
                    4 | 5 => 4,
                    6 | 7 => 8,
                    _ => panic!(),
                };
                row.extend(&v.to_be_bytes()[8 - n..]);
            }
            UtfValue::Text(v) => {
                let at = strings.len();
                strings.extend(v.bytes());
                strings.push(0);
                row.extend((at as u32).to_be_bytes());
            }
        }
    }
    let rows_at = 32 + descriptors.len();
    let strings_at = rows_at + row.len();
    let len = strings_at + strings.len();
    let mut b = b"@UTF".to_vec();
    b.extend((len as u32 - 8).to_be_bytes());
    b.extend((rows_at as u32 - 8).to_be_bytes());
    b.extend((strings_at as u32 - 8).to_be_bytes());
    b.extend((len as u32 - 8).to_be_bytes());
    b.extend(0u32.to_be_bytes());
    b.extend((descriptors.len() as u16 / 5).to_be_bytes());
    b.extend((row.len() as u16).to_be_bytes());
    b.extend(1u32.to_be_bytes());
    b.extend(descriptors);
    b.extend(row);
    b.extend(strings);
    b
}
fn packet(magic: &[u8], mut b: Vec<u8>, encrypted: bool) -> Vec<u8> {
    if encrypted {
        let mut k = 0x655fu32;
        for v in &mut b {
            *v ^= k as u8;
            k = k.wrapping_mul(0x4115);
        }
    }
    let mut p = magic.to_vec();
    p.extend([0; 4]);
    p.extend((b.len() as u64).to_le_bytes());
    p.extend(b);
    p
}
#[test]
fn cpk_utf_packets_and_crilayla_literals() {
    for compressed in [false, true] {
        let payload = if compressed {
            let mut bits = Vec::new();
            for v in b"CBA" {
                bits.push(false);
                for i in (0..8).rev() {
                    bits.push(v & (1 << i) != 0);
                }
            }
            while bits.len() % 8 != 0 {
                bits.push(false);
            }
            let mut bytes = bits
                .chunks(8)
                .map(|p| p.iter().fold(0u8, |v, b| (v << 1) | u8::from(*b)))
                .collect::<Vec<_>>();
            bytes.reverse();
            let mut p = b"CRILAYLA".to_vec();
            p.extend(3u32.to_le_bytes());
            p.extend((bytes.len() as u32).to_le_bytes());
            p.extend(bytes);
            p.extend(b"PRE");
            p
        } else {
            b"ABC".to_vec()
        };
        let header = utf(vec![
            ("ContentOffset", UtfValue::Num(512, 6)),
            ("TocOffset", UtfValue::Num(256, 6)),
            ("Align", UtfValue::Num(16, 2)),
        ]);
        let toc = utf(vec![
            ("ID", UtfValue::Num(0, 4)),
            ("DirName", UtfValue::Text("dir")),
            ("FileName", UtfValue::Text("a.txt")),
            ("FileOffset", UtfValue::Num(256, 6)),
            ("FileSize", UtfValue::Num(payload.len() as u64, 4)),
        ]);
        let mut b = packet(b"CPK ", header, true);
        b.resize(256, 0);
        b.extend(packet(b"TOC ", toc, false));
        b.resize(512, 0);
        b.extend(payload);
        let out = body("cri/cpk", b);
        assert_eq!(
            out,
            if compressed {
                b"PREABC".as_slice()
            } else {
                b"ABC"
            }
        );
    }
}
#[test]
fn rejects_missing_profiles_bad_tables_and_budgets() {
    for (id, b) in [
        ("almond-collective/pac3", fixed(b"PAC3", 32)),
        ("dxlib/dx", vec![0; 24]),
        ("glib/glib2", vec![0; 92]),
        ("lucifen/lpk", fixed(b"LPK1", 8)),
    ] {
        assert!(crysirror_formats_arc::legacy_archive(id)
            .unwrap()
            .open(mem(b), &cx())
            .is_err());
    }
    let mut b = b"KIF\0".to_vec();
    b.extend(1u32.to_le_bytes());
    b.extend(fixed(b"__key__.dat", 64));
    b.extend([0; 8]);
    assert!(crysirror_formats_arc::legacy_archive("cat-system/int")
        .unwrap()
        .open(mem(b), &cx())
        .is_err());
    let mut c = cx();
    c.limits.max_entries = 0;
    let b = b"PKWV\x01\0\x01\0".to_vec();
    assert!(crysirror_formats_arc::legacy_archive("crowd/pkwv")
        .unwrap()
        .open(mem(b), &c)
        .is_err());
}
#[test]
#[ignore = "reads upstream corpus only from an explicit local runtime path"]
fn upstream_archive_corpus_e() {
    let root = std::env::var("ARC_UNPACKER_REFERENCE")
        .expect("set ARC_UNPACKER_REFERENCE to the ignored pinned source checkout");
    for (id, path) in [
        ("gnu/gzip", "gnu/files/gzip/test.gz"),
        ("french-bread/p", "french_bread/files/p/test-v0.p"),
        ("french-bread/p", "french_bread/files/p/test-v1.p"),
        ("fc01/mrg", "fc01/files/mrg/plain.mrg"),
        ("fc01/mrg", "fc01/files/mrg/compressed.mrg"),
        (
            "cronus/pak",
            "cronus/files/pak/unencrypted-uncompressed.pak",
        ),
        ("cronus/pak", "cronus/files/pak/encrypted-compressed.pak"),
        ("entis/noa", "entis/files/noa/unencrypted.noa"),
    ] {
        let p = std::path::Path::new(&root).join("tests/dec").join(path);
        let bytes = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let mut context = cx();
        if id == "cronus/pak" && path.contains("encrypted-compressed") {
            // The external fixture has two known test files (10 + 26 bytes).
            // Recover this fixture's header mask without embedding title keys.
            let mut keys = Vec::new();
            keys.extend((u32::from_le_bytes(bytes[20..24].try_into().unwrap()) ^ 2).to_le_bytes());
            keys.extend(
                (u32::from_le_bytes(bytes[24..28].try_into().unwrap()) ^ (bytes.len() as u32 - 36))
                    .to_le_bytes(),
            );
            profile(
                &mut context,
                id,
                keys,
                serde_json::json!({"header_key_id":"fixture"}),
            );
        }
        let rows = extract(id, bytes, &context);
        let first = if id == "fc01/mrg" {
            b"123123123 123123123 321".as_slice()
        } else {
            b"1234567890".as_slice()
        };
        let second_name = if matches!(id, "gnu/gzip" | "french-bread/p") {
            "abc.xyz"
        } else {
            "abc.txt"
        };
        assert!(
            rows.iter()
                .any(|(name, data)| name == "123.txt" && data.as_slice() == first),
            "{id}:{path}: first payload"
        );
        assert!(
            rows.iter()
                .any(|(name, data)| name == second_name && data == b"abcdefghijklmnopqrstuvwxyz"),
            "{id}:{path}: second payload"
        );
        if id == "french-bread/p" {
            assert!(rows.iter().any(|(name, data)| name == "big.000"
                && data.len() == 0x3000
                && data.iter().all(|b| *b == 255)));
        }
        eprintln!("{id}:{path}: {} exact files", rows.len());
    }
    let p = std::path::Path::new(&root).join("tests/dec");
    let rows = extract(
        "entis/noa",
        std::fs::read(p.join("entis/files/noa/encrypted-erisan.noa")).unwrap(),
        &cx(),
    );
    assert_eq!(
        rows[0].1,
        std::fs::read(p.join("entis/files/noa/system.inf")).unwrap()
    );
    for (input, prefix, count) in [("BLIN3.MCA", "blin3", 5usize), ("OK.MCA", "ok", 4)] {
        let mut context = cx();
        profile(
            &mut context,
            "fc01/mca",
            vec![209],
            serde_json::json!({"key_id":"fixture"}),
        );
        let rows = extract(
            "fc01/mca",
            std::fs::read(p.join("fc01/files/mca").join(input)).unwrap(),
            &context,
        );
        assert_eq!(rows.len(), count);
        for (i, (_, bytes)) in rows.iter().enumerate() {
            let expected = image::open(
                p.join("fc01/files/mca")
                    .join(format!("{prefix}-out-{i:03}.png")),
            )
            .unwrap()
            .to_rgba8();
            assert_eq!(image::load_from_memory(bytes).unwrap().to_rgba8(), expected);
        }
    }
    let bytes = std::fs::read(p.join("libido/files/egr/test-zlib.EGR")).unwrap();
    let mut raw = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::ZlibDecoder::new(bytes.as_slice()),
        &mut raw,
    )
    .unwrap();
    let rows = extract("libido/egr", raw, &cx());
    for (i, (_, bytes)) in rows.iter().enumerate() {
        let expected = image::open(
            p.join("libido/files/egr")
                .join(format!("Image{i:03}-out.png")),
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(image::load_from_memory(bytes).unwrap().to_rgba8(), expected);
    }
    for (id, path, out) in [(
        "cat-system/hg3",
        "cat_system/files/hg3/Tmic27s_00a.hg3",
        "cat_system/files/hg3/Tmic27s_00a-out.png",
    )] {
        let p = std::path::Path::new(&root).join("tests/dec");
        let actual = extract(id, std::fs::read(p.join(path)).unwrap(), &cx());
        let expected = image::open(p.join(out)).unwrap().to_rgba8();
        let got = image::load_from_memory(&actual[0].1).unwrap().to_rgba8();
        assert_eq!(got.dimensions(), expected.dimensions());
        // The pinned upstream test comparator ignores RGB when both alphas are
        // zero; its HG3 PNG fixture clears those otherwise preserved RGB bytes.
        let mismatches: Vec<_> = got
            .pixels()
            .zip(expected.pixels())
            .enumerate()
            .filter(|(_, (a, b))| a != b && !(a[3] == 0 && b[3] == 0))
            .map(|(i, (a, b))| (i, a.0, b.0))
            .collect();
        assert!(
            mismatches.is_empty(),
            "{id} {path}: {} mismatched pixels; first {:?}",
            mismatches.len(),
            &mismatches[..mismatches.len().min(10)]
        );
    }
}
#[test]
fn hg3_control_planes_canvas_offset() {
    fn chunk(name: &[u8], data: &[u8]) -> Vec<u8> {
        let mut b = name.to_vec();
        b.extend(0u32.to_le_bytes());
        b.extend((data.len() as u32).to_le_bytes());
        b.extend(data);
        b
    }
    let mut info = Vec::new();
    for n in [1u32, 1, 32, 1, 0, 2, 1] {
        info.extend(n.to_le_bytes());
    }
    let mut bits = vec![1u8, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0];
    while bits.len() % 8 != 0 {
        bits.push(0);
    }
    let ctl = bits
        .chunks(8)
        .map(|b| b.iter().enumerate().fold(0u8, |v, (i, b)| v | (b << i)))
        .collect::<Vec<_>>();
    let data = zlib(&[255; 4]);
    let compressed_ctl = zlib(&ctl);
    let mut image = vec![0; 8];
    image.extend((data.len() as u32).to_le_bytes());
    image.extend(4u32.to_le_bytes());
    image.extend((compressed_ctl.len() as u32).to_le_bytes());
    image.extend((ctl.len() as u32).to_le_bytes());
    image.extend(data);
    image.extend(compressed_ctl);
    let mut b = fixed(b"HG-3", 20);
    b.extend(chunk(b"stdinfo\0", &info));
    b.extend(chunk(b"img0000\0", &image));
    assert_eq!(
        png_pixel(&body("cat-system/hg3", b)),
        [0, 0, 0, 0, 128, 128, 128, 128]
    );
}
#[test]
fn noa_erisan_and_bshf_synthetic_zero_streams() {
    for flags in [0x80000010u32, 0x40000000, 0xc0000010] {
        let mut b = fixed(b"Entis\x1a\0\0\0\x04\0\x02\0\0\0\0ERISA-Archive file", 64);
        let mut t = 1u32.to_le_bytes().to_vec();
        t.extend(32u64.to_le_bytes());
        t.extend(0u32.to_le_bytes());
        t.extend(flags.to_le_bytes());
        t.extend(66u64.to_le_bytes());
        t.extend([0; 8]);
        t.extend(0u32.to_le_bytes());
        t.extend(6u32.to_le_bytes());
        t.extend(b"a.bin\0");
        b.extend(b"DirEntry");
        b.extend((t.len() as u64).to_le_bytes());
        b.extend(t);
        b.extend(b"filedata");
        b.extend(68u64.to_le_bytes());
        b.extend([0; 68]);
        let mut c = cx();
        profile(
            &mut c,
            "entis/noa",
            b"synthetic".to_vec(),
            serde_json::json!({"key_id":"fixture"}),
        );
        assert_eq!(extract("entis/noa", b, &c)[0].1, vec![0; 32]);
    }
}
#[test]
fn unity_nonembedded_assets_v14_v15() {
    for version in [14u32, 15] {
        let mut meta = b"5.0-fixture\0".to_vec();
        meta.extend(0u32.to_le_bytes());
        meta.push(0);
        meta.extend(1u32.to_le_bytes());
        meta.extend(1u32.to_le_bytes());
        meta.extend([0; 16]);
        meta.extend(1u32.to_le_bytes());
        while (meta.len() + 20) % 4 != 0 {
            meta.push(0);
        }
        meta.extend(7u64.to_le_bytes());
        meta.extend(0u32.to_le_bytes());
        meta.extend(3u32.to_le_bytes());
        meta.extend(1u32.to_le_bytes());
        meta.extend(1u16.to_le_bytes());
        meta.extend(0u16.to_le_bytes());
        if version == 15 {
            meta.push(0);
        }
        meta.extend(0u32.to_le_bytes());
        while (meta.len() + 20) % 4 != 0 {
            meta.push(0);
        }
        meta.extend(0u32.to_le_bytes());
        let base = 20 + meta.len();
        let mut b = (meta.len() as u32).to_be_bytes().to_vec();
        b.extend((base as u32 + 3).to_be_bytes());
        b.extend(version.to_be_bytes());
        b.extend((base as u32).to_be_bytes());
        b.extend([0; 4]);
        b.extend(meta);
        b.extend(b"ABC");
        assert_eq!(body("unity/assets", b), b"ABC");
    }
}
fn psb_array(values: &[usize]) -> Vec<u8> {
    let width = if values.iter().any(|n| *n > 255) {
        2
    } else {
        1
    };
    assert!(values.iter().all(|n| *n < 65536));
    let mut b = vec![14];
    b.extend((values.len() as u16).to_le_bytes());
    b.push(12 + width);
    for v in values {
        b.extend(&(*v as u32).to_le_bytes()[..width as usize]);
    }
    b
}
fn psb_names(names: &[&str]) -> Vec<u8> {
    let mut nodes = BTreeMap::<Vec<u8>, (usize, usize)>::new();
    nodes.insert(Vec::new(), (0, 0));
    let mut next = 1;
    for name in names {
        for n in 1..=name.len() {
            let part = name.as_bytes()[..n].to_vec();
            if !nodes.contains_key(&part) {
                let parent = nodes[&part[..n - 1]];
                let position = parent.1 * 256 + part[n - 1] as usize + 1;
                nodes.insert(part, (position, next));
                next += 1;
            }
        }
    }
    let max = nodes.values().map(|p| p.0).max().unwrap() + names.len() + 1;
    let mut a = vec![0; max];
    let mut p = vec![0; max];
    for (part, (index, ordinal)) in &nodes {
        a[*index] = ordinal * 256 + 1;
        if !part.is_empty() {
            p[*index] = nodes[&part[..part.len() - 1]].0;
        }
    }
    let base = max - names.len();
    let mut terms = Vec::new();
    for (i, name) in names.iter().enumerate() {
        p[base + i] = nodes[name.as_bytes()].0;
        terms.push(base + i);
    }
    let mut b = psb_array(&a);
    b.extend(psb_array(&p));
    b.extend(psb_array(&terms));
    b
}
#[test]
fn psb_embedded_png_resources() {
    let png = crysirror_image::encode_png_uncompressed(&crysirror_image::ImageSurface {
        width: 1,
        height: 1,
        rgba: vec![10, 20, 30, 40],
    })
    .unwrap();
    let names = psb_names(&["width", "height", "a.png"]);
    let mut b = fixed(b"PSB\0", 40);
    b[4..6].copy_from_slice(&2u16.to_le_bytes());
    put(&mut b, 12, 40);
    b.extend(names);
    let at = b.len();
    put(&mut b, 16, at as u32);
    b.extend(psb_array(&[]));
    let at = b.len();
    put(&mut b, 20, at as u32);
    put(&mut b, 24, at as u32);
    b.extend(psb_array(&[0]));
    let at = b.len();
    put(&mut b, 28, at as u32);
    b.extend(psb_array(&[png.len()]));
    let at = b.len();
    put(&mut b, 36, at as u32);
    b.push(33);
    b.extend(psb_array(&[0, 1, 2]));
    b.extend(psb_array(&[0, 2, 4]));
    b.extend([5, 1, 5, 1, 25, 0]);
    let at = b.len();
    put(&mut b, 32, at as u32);
    b.extend(png);
    let mut c = cx();
    c.source_name = Some("fixture.psb".into());
    let out = extract("yuzusoft/psb", b, &c);
    assert_eq!(out[0].0, "fixture_a.png");
    assert_eq!(png_pixel(&out[0].1), [10, 20, 30, 40]);
}
