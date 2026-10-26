// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-authored fixtures for TSA and Twilight Frontier native archives.
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use crysirror_resource::{read_all, DecodeContext, FormatRegistry};
use std::sync::Arc;
fn le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
fn sh(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn fixed(b: &[u8], n: usize) -> Vec<u8> {
    let mut b = b.to_vec();
    b.resize(n, 0);
    b
}
fn memory(b: Vec<u8>) -> Arc<dyn AssetReader> {
    Arc::new(MemoryAssetReader(b.into()))
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
fn cx() -> DecodeContext {
    DecodeContext::new(ClientId(91))
}
fn one(id: &str, b: Vec<u8>) -> Vec<u8> {
    let out = extract(&cx(), id, b);
    assert_eq!(out.len(), 1);
    out[0].1.clone()
}
struct Bits(Vec<u8>, usize);
impl Bits {
    fn put(&mut self, n: usize, v: u32) {
        for i in (0..n).rev() {
            if self.1 % 8 == 0 {
                self.0.push(0)
            }
            let at = self.1 / 8;
            self.0[at] |= (((v >> i) & 1) as u8) << (7 - self.1 % 8);
            self.1 += 1;
        }
    }
    fn integer(&mut self, v: u32) {
        let n = if v <= 255 {
            1
        } else if v <= 65535 {
            2
        } else if v <= 0xffffff {
            3
        } else {
            4
        };
        self.put(2, n - 1);
        self.put(n as usize * 8, v)
    }
}
fn packed(b: &[u8]) -> Vec<u8> {
    let mut bits = Bits(vec![], 0);
    for b in b {
        bits.put(1, 1);
        bits.put(8, u32::from(*b));
    }
    bits.0
}
// Inverse of the format's strided block transform, written as a fixture encoder.
fn encrypt(b: &[u8], mut key: u8, step: u8, mut block: usize, limit: usize) -> Vec<u8> {
    let mut out = b.to_vec();
    let mut left = b.len() as isize;
    let mut shift = b.len() % block;
    if shift >= block / 4 {
        shift = 0
    }
    shift += b.len() & 1;
    left -= shift as isize;
    let mut at = 0;
    while left > 0 && at < limit {
        block = block.min(left as usize);
        let mut dest = 0;
        for j in 0..2 {
            for i in 0..((block - j + 1) / 2) {
                out[at + dest] = b[at + block - j - 1 - i * 2] ^ key;
                dest += 1;
                key = key.wrapping_add(step);
            }
        }
        at += block;
        left -= block as isize;
    }
    out
}
fn table(pad: bool, offset: u32, orig: u32) -> Vec<u8> {
    let mut t = b"a.txt\0".to_vec();
    if pad {
        t.extend([0; 2])
    }
    t.extend(le(offset));
    t.extend(le(orig));
    t.extend(le(0));
    t
}
#[test]
fn pbg3_and_pbg4_native_bit_indexes() {
    let body = packed(b"abc");
    let mut index = Bits(vec![], 0);
    for v in [0, 0, 0, 16, 3] {
        index.integer(v)
    }
    for b in b"a.txt\0" {
        index.put(8, u32::from(*b));
    }
    let mut h = Bits(vec![], 0);
    h.integer(1);
    h.integer(20);
    let mut v = b"PBG3".to_vec();
    v.extend(h.0);
    v.resize(16, 0);
    v.extend(&body);
    v.extend(index.0);
    assert_eq!(one("team-shanghai-alice/pbg3", v), b"abc");
    let t = table(false, 16, 3);
    let mut v = b"PBG4".to_vec();
    v.extend(le(1));
    v.extend(le(20));
    v.extend(le(t.len() as u32));
    v.extend(body);
    v.extend(packed(&t));
    assert_eq!(one("team-shanghai-alice/pbg4", v), b"abc");
}
#[test]
fn pbgz_and_tha1_explicit_native_decryptors() {
    let parameters = serde_json::json!({"key_id":"tsa-fixture","block_size":4,"limit":8});
    let body = encrypt(b"abc", 12, 3, 4, 8);
    let mut e = b"edz\x07".to_vec();
    e.extend(&body);
    let body = packed(&e);
    let at = 16 + body.len() as u32;
    let t = table(false, 16, 7);
    let mut h = le(123457).to_vec();
    h.extend(le(at + 345678));
    h.extend(le(t.len() as u32 + 567891));
    let mut v = b"PBGZ".to_vec();
    v.extend(encrypt(&h, 0x1b, 0x37, 12, 1024));
    v.extend(body);
    v.extend(encrypt(&packed(&t), 0x3e, 0x9b, 128, 1024));
    let mut cx = cx();
    let keys = crysirror_resource::MemoryKeyProvider::default();
    keys.insert(cx.client, "tsa-fixture", vec![12, 3]).unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert(
        "team-shanghai-alice/pbgz".into(),
        serde_json::json!({"decryptors":{"7":parameters}}),
    );
    assert_eq!(extract(&cx, "team-shanghai-alice/pbgz", v)[0].1, b"abc");
    let t = table(true, 16, 3);
    let comp = packed(&t);
    let mut h = b"THA1".to_vec();
    h.extend(le(t.len() as u32 + 123456789));
    h.extend(le(comp.len() as u32 + 987654321));
    h.extend(le(1 + 135792468));
    let mut v = encrypt(&h, 0x1b, 0x37, 16, 1024);
    v.extend(encrypt(b"abc", 12, 3, 4, 8));
    v.extend(encrypt(&comp, 0x3e, 0x9b, 128, comp.len()));
    cx.parameters.insert(
        "team-shanghai-alice/tha1".into(),
        serde_json::json!({"decryptors":vec![parameters;8]}),
    );
    assert_eq!(extract(&cx, "team-shanghai-alice/tha1", v)[0].1, b"abc");
}
#[test]
fn anm_texture_layout_and_native_bgra() {
    let mut v = vec![0; 64];
    v[12..16].copy_from_slice(&le(1));
    v[16..20].copy_from_slice(&le(1));
    v[28..32].copy_from_slice(&le(64));
    v[48..52].copy_from_slice(&le(80));
    v[52..56].copy_from_slice(&le(1));
    v.extend(b"image\0");
    v.resize(80, 0);
    v.extend(b"THTX");
    v.extend(sh(0));
    v.extend(sh(1));
    v.extend(sh(1));
    v.extend(sh(1));
    v.extend(le(4));
    v.extend([3, 2, 1, 255]);
    let out = one("team-shanghai-alice/anm", v);
    assert_eq!(
        crysirror_image::decode_standard(&out).unwrap().rgba,
        [1, 2, 3, 255]
    );
}
fn second_order(b: &mut [u8], mut a: u8, mut k: u8, d: u8) {
    for b in b {
        *b ^= a;
        a = a.wrapping_add(k);
        k = k.wrapping_add(d)
    }
}
#[test]
fn twilight_pak1_index_and_script_cipher() {
    let mut t = fixed(b"script.sce", 100);
    t.extend(le(3));
    t.extend(le(110));
    second_order(&mut t, 0x64, 0x64, 0x4d);
    let mut body = b"abc".to_vec();
    second_order(&mut body, 0x63, 0x62, 0x42);
    let mut v = sh(1).to_vec();
    v.extend(t);
    v.extend(body);
    assert_eq!(
        extract(&cx(), "twilight-frontier/pak1", v)[0],
        ("script.txt".into(), b"abc".to_vec())
    );
}
#[test]
fn twilight_pak1_gfx_run_pixels_and_sfx_wave() {
    let mut v = vec![0];
    v.extend(le(1));
    v.extend(le(1));
    v.extend(le(0));
    v.push(32);
    v.extend(le(8));
    v.extend(le(1));
    v.extend([3, 2, 1, 255]);
    let out = one("twilight-frontier/pak1-gfx", v);
    assert_eq!(
        crysirror_image::decode_standard(&out).unwrap().rgba,
        [1, 2, 3, 255]
    );
    let mut v = le(1).to_vec();
    v.push(1);
    v.extend(le(2));
    v.extend(sh(1));
    v.extend(sh(1));
    v.extend(le(8000));
    v.extend(le(16000));
    v.extend(sh(2));
    v.extend(sh(16));
    v.extend(sh(0));
    v.extend([0x12, 0x34]);
    let out = one("twilight-frontier/pak1-sfx", v);
    assert_eq!(&out[..4], b"RIFF");
    assert_eq!(&out[46..], [0x12, 0x34]);
}
#[test]
fn thbgm_scoped_companion_and_original_loop_values() {
    let mut def = fixed(b"music", 16);
    def.extend(le(4));
    def.extend(le(0));
    def.extend(le(2));
    def.extend(le(4));
    def.extend(sh(1));
    def.extend(sh(1));
    def.extend(le(8000));
    def.extend(le(16000));
    def.extend(sh(2));
    def.extend(sh(16));
    def.extend(le(0));
    let mut cx = cx();
    cx.source_name = Some("music/thbgm.dat".into());
    cx.resolver = Some(Arc::new(move |path: &str| {
        anyhow::ensure!(path == "music/thbgm.fmt", "unexpected companion path");
        Ok(memory(def.clone()))
    }));
    let out = extract(&cx, "team-shanghai-alice/thbgm", b"ZWAVabcd".to_vec());
    assert_eq!(out[0].0, "music.wavloop");
    let b = &out[0].1;
    assert_eq!(&b[46..50], b"abcd");
    assert_eq!(&b[50..54], b"smpl");
    assert_eq!(&b[102..106], &le(2));
    assert_eq!(&b[106..110], &le(2));
}
#[test]
fn tfpk_plaintext_rsa_blocks_and_both_data_ciphers() {
    for version in [0, 1] {
        let mut v = b"TFPK".to_vec();
        v.push(version);
        v.extend([0; 64]);
        v.extend(fixed(&le(1), 64));
        let mut one = le(if version == 0 { 3 } else { 3 ^ 1 }).to_vec();
        one.extend(le(if version == 0 { 0 } else { 2 }));
        v.extend(fixed(&one, 64));
        v.extend(fixed(&le(if version == 0 { 0 } else { 3 }), 64));
        let key = if version == 0 {
            (0..16).collect::<Vec<u8>>()
        } else {
            let mut k = vec![];
            for n in 1u32..=4 {
                k.extend(le(n))
            }
            k
        };
        v.extend(fixed(&key, 64));
        let body = if version == 0 {
            b"abc"
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ key[i])
                .collect::<Vec<_>>()
        } else {
            b"abc".to_vec()
        };
        v.extend(body);
        assert_eq!(one_extract(&v), b"abc");
    }
}
fn one_extract(v: &[u8]) -> Vec<u8> {
    one("twilight-frontier/tfpk", v.to_vec())
}

#[test]
fn twilight_pak2_mt19937_index_fixture() {
    let data = vec![
        1, 0, 14, 0, 0, 0, 178, 146, 17, 152, 92, 72, 61, 189, 191, 123, 19, 237, 215, 180, 74, 73,
        72,
    ];
    assert_eq!(one("twilight-frontier/pak2", data), b"abc");
}
#[test]
#[ignore = "reads optional pinned ARC_UNPACKER_SOURCE corpus; no fixture binaries distributed"]
fn pinned_tsa_twilight_corpus_images_audio_and_lzss() {
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
    assert!(std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap(),
            "diff",
            "--quiet",
            "HEAD",
            "--",
            "src",
            "tests"
        ])
        .status()
        .unwrap()
        .success());
    let read = |p: &str| std::fs::read(root.join("tests/dec").join(p)).unwrap();
    let cx = cx();
    for (id, path) in [
        (
            "team-shanghai-alice/pbg3",
            "team_shanghai_alice/files/pbg3/test.pbg3",
        ),
        (
            "team-shanghai-alice/pbg4",
            "team_shanghai_alice/files/pbg4/test.pbg4",
        ),
    ] {
        let out = extract(&cx, id, read(path));
        assert_eq!(out[0], ("abc.txt".into(), b"123".to_vec()));
        assert_eq!(out[1], ("another.txt".into(), b"abcdefghijaaabcd".to_vec()));
    }
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "team-shanghai-alice/anm",
            "team_shanghai_alice/files/anm/face_01_00.anm",
            &["team_shanghai_alice/files/anm/face_01_00-out.png"],
        ),
        (
            "team-shanghai-alice/anm",
            "team_shanghai_alice/files/anm/eff01.anm",
            &["team_shanghai_alice/files/anm/eff01-out.png"],
        ),
        (
            "team-shanghai-alice/anm",
            "team_shanghai_alice/files/anm/player00.anm",
            &["team_shanghai_alice/files/anm/player00-out.png"],
        ),
        (
            "team-shanghai-alice/anm",
            "team_shanghai_alice/files/anm/clouds.anm",
            &["team_shanghai_alice/files/anm/clouds-out.png"],
        ),
        (
            "team-shanghai-alice/anm",
            "team_shanghai_alice/files/anm/eff05.anm",
            &[
                "team_shanghai_alice/files/anm/eff05-out.png",
                "team_shanghai_alice/files/anm/eff05-out2.png",
            ],
        ),
        (
            "twilight-frontier/pak1-gfx",
            "twilight_frontier/files/pak1/stage3.dat",
            &[
                "twilight_frontier/files/pak1/stage3-0000-out.png",
                "twilight_frontier/files/pak1/stage3-0001-out.png",
            ],
        ),
        (
            "twilight-frontier/pak1-gfx",
            "twilight_frontier/files/pak1/stage10.dat",
            &["twilight_frontier/files/pak1/stage10-0000-out.png"],
        ),
        (
            "twilight-frontier/pak1-gfx",
            "twilight_frontier/files/pak1/effect.dat",
            &["twilight_frontier/files/pak1/effect-0000-out.png"],
        ),
        (
            "twilight-frontier/pak1-gfx",
            "twilight_frontier/files/pak1/07.dat",
            &["twilight_frontier/files/pak1/07-0000-out.png"],
        ),
    ];
    for &(id, path, expected) in cases {
        let out = extract(&cx, id, read(path));
        assert_eq!(out.len(), expected.len(), "{path}");
        for ((_, b), expected) in out.iter().zip(expected) {
            let actual = crysirror_image::decode_standard(b).unwrap();
            let expected = crysirror_image::decode_standard(&read(expected)).unwrap();
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height),
                "{path}"
            );
            let first = actual
                .rgba
                .chunks_exact(4)
                .zip(expected.rgba.chunks_exact(4))
                .position(|(a, b)| a != b);
            assert!(first.is_none(), "{path} first pixel mismatch {first:?}");
        }
    }
    let out = extract(
        &cx,
        "twilight-frontier/pak1-sfx",
        read("twilight_frontier/files/pak1/se.dat"),
    );
    for ((_, actual), expected) in out.iter().zip([
        "twilight_frontier/files/pak1/se-0001-out.wav",
        "twilight_frontier/files/pak1/se-0002-out.wav",
    ]) {
        assert_eq!(wave_data(actual), wave_data(&read(expected)));
    }
    let defs = extract(
        &cx,
        "team-shanghai-alice/pbg4",
        read("team_shanghai_alice/files/thbgm/thbgm-def.dat"),
    );
    let definitions = defs.into_iter().find(|(n, _)| n == "thbgm.fmt").unwrap().1;
    let mut cx = cx;
    cx.source_name = Some("thbgm.dat".into());
    cx.resolver = Some(Arc::new(move |path: &str| {
        anyhow::ensure!(path == "thbgm.fmt", "unexpected companion");
        Ok(memory(definitions.clone()))
    }));
    let out = extract(
        &cx,
        "team-shanghai-alice/thbgm",
        read("team_shanghai_alice/files/thbgm/thbgm-data.dat"),
    );
    for ((_, actual), expected) in out.iter().zip([
        "team_shanghai_alice/files/thbgm/1-out.wav",
        "team_shanghai_alice/files/thbgm/2-out.wav",
    ]) {
        assert_eq!(wave_data(actual), wave_data(&read(expected)));
    }
}
fn wave_data(b: &[u8]) -> Vec<u8> {
    let mut at = 12;
    while at + 8 <= b.len() {
        let n = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        if &b[at..at + 4] == b"data" {
            return b[at + 8..at + 8 + n].to_vec();
        }
        at += 8 + n;
    }
    panic!("missing WAVE data")
}

#[test]
fn tfpk_rsa_index_uses_client_public_key() {
    let mut cx = cx();
    let keys = crysirror_resource::MemoryKeyProvider::default();
    keys.insert(
        cx.client,
        "self-authored-rsa-modulus",
        vec![
            231, 184, 47, 103, 22, 168, 49, 74, 152, 38, 147, 32, 216, 26, 158, 224, 1, 36, 227,
            32, 166, 37, 93, 149, 241, 203, 104, 13, 55, 62, 64, 115, 188, 157, 116, 76, 173, 102,
            73, 251, 47, 117, 216, 91, 186, 159, 44, 146, 58, 142, 7, 9, 217, 119, 244, 154, 245,
            177, 188, 125, 43, 22, 183, 77,
        ],
    )
    .unwrap();
    cx.keys = Arc::new(keys);
    cx.parameters.insert(
        "twilight-frontier/tfpk".into(),
        serde_json::json!({"rsa_key_id":"self-authored-rsa-modulus","rsa_exponent":65537}),
    );
    let data = vec![
        84, 70, 80, 75, 0, 154, 29, 127, 244, 105, 212, 49, 45, 8, 33, 169, 134, 237, 159, 29, 25,
        165, 148, 175, 75, 109, 50, 252, 12, 76, 181, 3, 112, 146, 41, 72, 90, 182, 86, 193, 231,
        176, 235, 196, 252, 59, 210, 42, 235, 110, 212, 103, 60, 119, 252, 76, 64, 26, 7, 67, 10,
        73, 174, 179, 29, 248, 79, 200, 191, 148, 228, 99, 30, 142, 240, 118, 230, 115, 183, 56,
        169, 66, 198, 197, 64, 37, 102, 20, 185, 64, 255, 178, 145, 147, 15, 56, 116, 227, 60, 203,
        110, 11, 54, 106, 31, 197, 196, 246, 154, 100, 101, 27, 155, 201, 69, 63, 156, 33, 178, 87,
        86, 232, 200, 52, 109, 84, 199, 43, 151, 254, 92, 1, 183, 14, 132, 140, 57, 215, 54, 215,
        226, 196, 175, 75, 219, 215, 5, 178, 206, 175, 251, 78, 116, 104, 179, 54, 56, 3, 230, 244,
        5, 21, 116, 59, 16, 24, 140, 39, 188, 217, 229, 88, 182, 233, 168, 99, 172, 69, 30, 147,
        47, 95, 130, 154, 68, 122, 56, 182, 22, 94, 41, 7, 144, 232, 60, 51, 254, 154, 29, 127,
        244, 105, 212, 49, 45, 8, 33, 169, 134, 237, 159, 29, 25, 165, 148, 175, 75, 109, 50, 252,
        12, 76, 181, 3, 112, 146, 41, 72, 90, 182, 86, 193, 231, 176, 235, 196, 252, 59, 210, 42,
        235, 110, 212, 103, 60, 119, 252, 76, 64, 26, 7, 67, 10, 73, 174, 179, 29, 248, 79, 200,
        191, 154, 29, 127, 244, 105, 212, 49, 45, 8, 33, 169, 134, 237, 159, 29, 25, 165, 148, 175,
        75, 109, 50, 252, 12, 76, 181, 3, 112, 146, 41, 72, 90, 182, 86, 193, 231, 176, 235, 196,
        252, 59, 210, 42, 235, 110, 212, 103, 60, 119, 252, 76, 64, 26, 7, 67, 10, 73, 174, 179,
        29, 248, 79, 200, 191, 97, 98, 99,
    ];
    assert_eq!(extract(&cx, "twilight-frontier/tfpk", data)[0].1, b"abc");
}
