// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-authored archive fixtures; external title resources remain outside Git.
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use crysirror_resource::{read_all, DecodeContext, FormatRegistry, MemoryKeyProvider};
use std::{io::Write, sync::Arc};
fn p32(d: &mut [u8], p: usize, v: u32) {
    d[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
fn p16(d: &mut [u8], p: usize, v: u16) {
    d[p..p + 2].copy_from_slice(&v.to_le_bytes());
}
fn memory(d: Vec<u8>) -> Arc<dyn AssetReader> {
    Arc::new(MemoryAssetReader(d.into()))
}
fn context() -> DecodeContext {
    DecodeContext::new(ClientId(1))
}
fn extract(id: &str, d: Vec<u8>, cx: &DecodeContext) -> Vec<(String, Vec<u8>)> {
    let mut r = FormatRegistry::default();
    crysirror_formats_arc::register(&mut r).unwrap();
    let (_, a) = r
        .open(memory(d), Some(id), cx)
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
fn zlib(d: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
    e.write_all(d).unwrap();
    e.finish().unwrap()
}
fn literals(d: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    for chunk in d.chunks(8) {
        o.push(255);
        o.extend_from_slice(chunk);
    }
    o
}
#[test]
fn pnap_skips_empty_slots() {
    let mut d = vec![0; 100];
    d[..4].copy_from_slice(b"PNAP");
    p32(&mut d, 16, 2);
    p32(&mut d, 56, 4);
    d.extend_from_slice(b"OggS");
    let o = extract("will/pnap", d, &context());
    assert_eq!(o, vec![("00000.ogg".into(), b"OggS".to_vec())]);
}
fn wip(depth: u16, pixel: &[u8]) -> Vec<u8> {
    let mut d = b"WIPF".to_vec();
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&depth.to_le_bytes());
    d.resize(32, 0);
    p32(&mut d, 8, 1);
    p32(&mut d, 12, 1);
    let data = if depth == 8 {
        let mut p = vec![0; 1024];
        p[4..8].copy_from_slice(pixel);
        p.extend_from_slice(&[1, 1]);
        p
    } else {
        literals(pixel)
    };
    p32(&mut d, 28, if depth == 8 { 2 } else { data.len() as u32 });
    d.extend(data);
    d
}
#[test]
fn wipf_planar_palette_and_scoped_mask() {
    let o = extract("will/wipf", wip(24, &[1, 2, 3]), &context());
    assert_eq!(
        crysirror_image::decode_standard(&o[0].1).unwrap().rgba,
        [3, 2, 1, 255]
    );
    let o = extract("will/wipf", wip(8, &[16, 32, 48, 127]), &context());
    assert_eq!(
        crysirror_image::decode_standard(&o[0].1).unwrap().rgba,
        [48, 32, 16, 128]
    );
    let mut cx = context();
    cx.source_name = Some("art/panel.wip".into());
    let mask = wip(8, &[0, 0, 64, 0]);
    cx.resolver = Some(Arc::new(move |path: &str| {
        assert_eq!(path, "art/panel.msk");
        Ok(memory(mask.clone()))
    }));
    let o = extract("will/wipf", wip(24, &[1, 2, 3]), &cx);
    assert_eq!(
        crysirror_image::decode_standard(&o[0].1).unwrap().rgba,
        [3, 2, 1, 64]
    );
}
fn rpa(version: u8, prefix: &[u8], malicious: bool) -> Vec<u8> {
    let key = if version == 3 { 0x12345678 } else { 0 };
    let base = if version == 3 { 34 } else { 25 };
    let mut table = vec![0x80, 2, b'}', b'(', b'U', 5];
    table.extend_from_slice(b"x.txt");
    table.extend_from_slice(&[b']', b'(', b'J']);
    table.extend_from_slice(&((base as u32) ^ key).to_le_bytes());
    table.push(b'J');
    table.extend_from_slice(&(3u32 ^ key).to_le_bytes());
    table.extend_from_slice(&[b'U', prefix.len() as u8]);
    table.extend_from_slice(prefix);
    table.extend_from_slice(&[0x87, b'a', b'u']);
    if malicious {
        table.push(b'c');
    }
    table.push(b'.');
    let mut d = if version == 3 {
        format!("RPA-3.0 {:016x} {key:08x}\n", base + 3).into_bytes()
    } else {
        format!("RPA-2.0 {:016x}\n", base + 3).into_bytes()
    };
    assert_eq!(d.len(), base);
    d.extend_from_slice(b"abc");
    d.extend(zlib(&table));
    d
}
#[test]
fn rpa_two_versions_prefix_size_and_forbidden_opcodes() {
    for v in [2, 3] {
        let o = extract("renpy/rpa", rpa(v, b"pre", false), &context());
        assert_eq!(o, vec![("x.txt".into(), b"preabc".to_vec())]);
    }
    let mut r = FormatRegistry::default();
    crysirror_formats_arc::register(&mut r).unwrap();
    assert!(r
        .open(memory(rpa(3, b"", true)), Some("renpy/rpa"), &context())
        .is_err());
}
#[test]
fn koepac_uses_shared_nonlinear_pcm_decoder() {
    let mut d = vec![0; 42];
    d[..8].copy_from_slice(b"KOEPAC\0\0");
    p32(&mut d, 16, 1);
    p16(&mut d, 32, 17);
    p16(&mut d, 34, 1);
    p32(&mut d, 36, 40);
    let o = extract("real-live/koepac", d, &context());
    assert_eq!(o[0].0, "sample00017.wav");
    assert_eq!(o[0].1.len(), 46 + 2048);
    assert_eq!(&o[0].1[46..], vec![0; 2048]);
}
#[test]
fn odn_variants_and_native_rle() {
    let mut d = b"back000000000000END_000000000000".to_vec();
    d.extend_from_slice(&[1, 1, 2, 3, 0, 1]);
    let o = extract("valkyria/odn", d, &context());
    assert_eq!(o[0].1, [1, 2, 3, 1, 2, 3]);
    for (version, table) in [
        (2, b"data000000000020data000100000023".to_vec()),
        (
            3,
            b"data000000000030........data000100000033........".to_vec(),
        ),
    ] {
        let mut d = table;
        d.extend_from_slice(b"abcdef");
        let mut cx = context();
        cx.parameters.insert(
            "valkyria/odn".into(),
            serde_json::json!({"version":version}),
        );
        let o = extract("valkyria/odn", d, &cx);
        assert_eq!(o[0].1, b"abc");
        assert_eq!(o[1].1, b"def");
    }
}
fn crc64(d: &[u8]) -> u64 {
    let mut v = !0u64;
    for b in d {
        v ^= (*b as u64) << 56;
        for _ in 0..8 {
            v = if v & 0x8000000000000000 != 0 {
                (v << 1) ^ 0x42f0e1eba9ea3693
            } else {
                v << 1
            };
        }
    }
    !v
}
#[test]
fn whale_plain_obfuscated_and_compressed_profiles() {
    let plain = b"0123456789";
    let name = b"a.txt";
    let hash = crc64(name);
    for kind in [0, 1, 2] {
        let mut payload = if kind == 2 {
            zlib(plain)
        } else {
            plain.to_vec()
        };
        if kind == 1 {
            let block = payload.len() / name.len();
            for (i, b) in payload
                .iter_mut()
                .take(block * (name.len() - 1))
                .enumerate()
            {
                *b ^= name[i / block];
            }
        }
        if kind == 2 {
            let xor = (hash ^ crc64(b"synthetic")) as u32;
            for word in payload.chunks_exact_mut(4) {
                let v = u32::from_le_bytes(word.try_into().unwrap()) ^ xor;
                word.copy_from_slice(&v.to_le_bytes());
            }
        }
        let mut d = (1u32 ^ 0x26aca46e).to_le_bytes().to_vec();
        d.extend_from_slice(&hash.to_le_bytes());
        d.push(kind ^ hash as u8);
        let mut offset = 25u32;
        let mut packed = payload.len() as u32;
        let mut orig = plain.len() as u32;
        if kind != 2 {
            offset ^= name[name.len() >> 1] as u32;
            packed ^= name[name.len() >> 2] as u32;
            orig ^= name[name.len() >> 3] as u32;
        }
        d.extend_from_slice(&(offset ^ hash as u32).to_le_bytes());
        d.extend_from_slice(&(packed ^ hash as u32).to_le_bytes());
        d.extend_from_slice(&(orig ^ hash as u32).to_le_bytes());
        d.extend(payload);
        let mut cx = context();
        cx.parameters.insert(
            "whale/dat".into(),
            serde_json::json!({"file_names":["a.txt"]}),
        );
        let keys = Arc::new(MemoryKeyProvider::default());
        keys.insert(cx.client, "whale/dat.game_key", b"synthetic".to_vec())
            .unwrap();
        cx.keys = keys;
        let o = extract("whale/dat", d, &cx);
        assert_eq!(o[0], ("a.txt".into(), plain.to_vec()));
    }
}
#[test]
fn tactics_v0_v1_plain_lzss_and_variable_lzss() {
    let mut table = vec![0];
    for n in [0u32, 3, 0, 5, 0, 0] {
        table.extend_from_slice(&n.to_le_bytes());
    }
    table.extend_from_slice(b"x.txt");
    let packed: Vec<_> = literals(&table).iter().map(|v| v ^ 255).collect();
    let mut d = b"TACTICS_ARC_FILE".to_vec();
    for n in [packed.len() as u32, table.len() as u32, 1, 0] {
        d.extend_from_slice(&n.to_le_bytes());
    }
    d.extend(packed);
    d.extend_from_slice(b"abc");
    assert_eq!(extract("tactics/arc", d, &context())[0].1, b"abc");
    for variable in [false, true] {
        let payload = if variable {
            vec![6, 8, b'A', b'B', b'C', 10, 3, 0]
        } else {
            literals(b"ABCABC")
        };
        let mut d = b"TACTICS_ARC_FILE".to_vec();
        for n in [payload.len() as u32, 6, 5, 0, 0] {
            d.extend_from_slice(&n.to_le_bytes());
        }
        d.extend_from_slice(b"x.txt");
        d.extend(payload);
        let mut cx = context();
        cx.parameters.insert("tactics/arc".into(),serde_json::json!({"version":1,"compression":if variable{"variable-lzss"}else{"plain-lzss"}}));
        assert_eq!(extract("tactics/arc", d, &cx)[0].1, b"ABCABC");
    }
}
fn blowfish_encrypt(d: &[u8], key: &[u8]) -> Vec<u8> {
    use blowfish::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    let cipher: blowfish::Blowfish = blowfish::Blowfish::new_from_slice(key).unwrap();
    let mut out = d.to_vec();
    for block in out.chunks_exact_mut(8) {
        block[..4].reverse();
        block[4..].reverse();
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
        block[..4].reverse();
        block[4..].reverse();
    }
    out
}
#[test]
fn tanuki_blowfish_table_and_payload() {
    for version in [0, 1] {
        for compressed in [false, true] {
            let original = b"testdata";
            let payload = if compressed {
                zlib(original)
            } else {
                blowfish_encrypt(original, b"7_tlib_secure_")
            };
            let mut table = 7u64.to_le_bytes().to_vec();
            for n in [u32::from(compressed), 8, 0, payload.len() as u32] {
                table.extend_from_slice(&n.to_le_bytes());
            }
            let table = zlib(&table);
            let table = blowfish_encrypt(&table, b"TLibArchiveData");
            let mut d = if version == 0 {
                b"TArc1.00\0\0\0\0".to_vec()
            } else {
                b"TArc1.10\0\0\0\0".to_vec()
            };
            d.resize(if version == 0 { 36 } else { 44 }, 0);
            p32(&mut d, 20, 1);
            p32(&mut d, 28, table.len() as u32);
            d.extend(table);
            d.extend(payload);
            assert_eq!(extract("tanuki/tac", d, &context())[0].1, original);
        }
    }
}
#[test]
#[ignore = "requires pinned upstream source corpus, kept outside Git"]
fn external_upstream_archives_g_parity() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ARC_UNPACKER_SOURCE").expect("set ARC_UNPACKER_SOURCE"),
    )
    .join("tests/dec");
    for filename in ["v2.rpa", "v3.rpa", "prefixes.rpa"] {
        let o = extract(
            "renpy/rpa",
            std::fs::read(root.join("renpy/files/rpa").join(filename)).unwrap(),
            &context(),
        );
        assert!(o.contains(&("another.txt".into(), b"abcdefghij".to_vec())));
        assert!(o.contains(&("abc.txt".into(), b"123".to_vec())));
        eprintln!("renpy/rpa {filename}: 2 exact files");
    }
    for filename in [
        "v0-compressed.arc",
        "v0-uncompressed.arc",
        "v1-compressed.arc",
        "v1-uncompressed.arc",
    ] {
        let mut cx = context();
        if filename.starts_with("v1") {
            let key = Arc::new(MemoryKeyProvider::default());
            key.insert(cx.client, "synthetic-key", b"mlnebzqm".to_vec())
                .unwrap();
            cx.keys = key;
            cx.parameters.insert(
                "tactics/arc".into(),
                serde_json::json!({"key_id":"synthetic-key"}),
            );
        }
        let o = extract(
            "tactics/arc",
            std::fs::read(root.join("tactics/files/arc").join(filename)).unwrap(),
            &cx,
        );
        assert!(o.contains(&("123.txt".into(), b"123123123123123123123".to_vec())));
        assert!(o.contains(&("abc.txt".into(), b"abcdefghijklmnopqrstuvwxyz".to_vec())));
        eprintln!("tactics/arc {filename}: 2 exact files");
    }
    for filename in ["plain.dat", "compressed.dat", "encrypted.dat"] {
        let mut cx = context();
        cx.parameters.insert(
            "whale/dat".into(),
            serde_json::json!({"file_names":["123.txt","abc.txt"]}),
        );
        let key = Arc::new(MemoryKeyProvider::default());
        key.insert(cx.client, "whale/dat.game_key", b"A Dog Story".to_vec())
            .unwrap();
        cx.keys = key;
        let o = extract(
            "whale/dat",
            std::fs::read(root.join("whale/files/dat").join(filename)).unwrap(),
            &cx,
        );
        assert!(o.contains(&("123.txt".into(), b"1234567890".to_vec())));
        assert!(o.contains(&("abc.txt".into(), b"abcdefghijklmnopqrstuvwxyz".to_vec())));
        eprintln!("whale/dat {filename}: 2 exact files");
    }
    for (input, expected) in [
        ("SNOW.MSK", vec!["SNOW-out.png"]),
        ("CURSOR.WIP", vec!["CURSOR-out.png"]),
        (
            "YESNO1.WIP",
            vec![
                "YESNO1-out-0.png",
                "YESNO1-out-1.png",
                "YESNO1-out-2.png",
                "YESNO1-out-3.png",
                "YESNO1-out-4.png",
            ],
        ),
    ] {
        let base = root.join("will/files/wipf");
        let actual = extract(
            "will/wipf",
            std::fs::read(base.join(input)).unwrap(),
            &context(),
        );
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected) {
            assert_eq!(
                crysirror_image::decode_standard(&a.1).unwrap(),
                crysirror_image::decode_standard(&std::fs::read(base.join(e)).unwrap()).unwrap()
            );
        }
        eprintln!("will/wipf {input}: {} exact images", actual.len());
    }
    let base = root.join("real_live/files/koepac");
    let o = extract(
        "real-live/koepac",
        std::fs::read(base.join("Z5000.KOE")).unwrap(),
        &context(),
    );
    for (path, actual) in o {
        let stem = path.trim_end_matches(".wav");
        let comp = std::fs::read(base.join(format!("{stem}-out-zlib.wav"))).unwrap();
        use std::io::Read;
        let mut expected = Vec::new();
        flate2::read::ZlibDecoder::new(comp.as_slice())
            .read_to_end(&mut expected)
            .unwrap();
        assert_eq!(actual, expected, "KOEPAC {path}");
        eprintln!("real-live/koepac {path}: exact WAV");
    }
}
