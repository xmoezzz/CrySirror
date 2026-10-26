// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-authored native indexes and ciphertext, without distributed game data.
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use crysirror_resource::{read_all, DecodeContext, FormatRegistry};
use std::{io::Write, sync::Arc};
fn le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
fn fixed(b: &[u8], n: usize) -> Vec<u8> {
    let mut b = b.to_vec();
    b.resize(n, 0);
    b
}
fn set(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&le(v))
}
fn extract(id: &str, b: Vec<u8>) -> Vec<(String, Vec<u8>)> {
    let mut r = FormatRegistry::default();
    crysirror_formats_arc::register(&mut r).unwrap();
    let cx = DecodeContext::new(ClientId(12));
    let (_, a) = r
        .open(
            Arc::new(MemoryAssetReader(b.into())) as Arc<dyn AssetReader>,
            Some(id),
            &cx,
        )
        .unwrap_or_else(|e| panic!("{id}: {e:#}"));
    let out = a
        .entries()
        .iter()
        .map(|e| {
            (
                e.path.clone(),
                read_all(
                    a.open(e.id, &cx)
                        .unwrap_or_else(|e| panic!("{id}: {e:#}"))
                        .as_ref(),
                    cx.limits.max_entry_size,
                )
                .unwrap(),
            )
        })
        .collect();
    assert_eq!(cx.resident_bytes(), 0);
    out
}
#[test]
fn reallive_ovk_nwk_sample_ids_and_native_record_width() {
    for (id, width, ext) in [("real-live/ovk", 16, ""), ("real-live/nwk", 12, ".nwa")] {
        let mut v = le(2).to_vec();
        let mut at = 4 + width * 2;
        for (sample, body) in [(17, b"abc".as_slice()), (300, b"second".as_slice())] {
            v.extend(le(body.len() as u32));
            v.extend(le(at));
            v.extend(le(sample));
            if width == 16 {
                v.extend(le(999))
            }
            at += body.len() as u32;
        }
        v.extend(b"abcsecond");
        assert_eq!(
            extract(id, v),
            [
                (format!("sample00017{ext}"), b"abc".to_vec()),
                (format!("sample00300{ext}"), b"second".to_vec())
            ]
        );
    }
}
fn advance(k: u32) -> u32 {
    k.wrapping_mul(7).wrapping_add(3)
}
fn rgs_data(b: &[u8], mut key: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for p in b.chunks(4) {
        out.extend(
            p.iter()
                .enumerate()
                .map(|(i, b)| b ^ (key >> (i * 8)) as u8),
        );
        key = advance(key);
    }
    out
}
#[test]
fn rgssad_and_rgss3a_names_payload_key_advance_and_short_tail() {
    let files = [
        ("long-name.txt", b"123456789".as_slice()),
        ("other.dat", b"abcdef".as_slice()),
    ];
    let mut v = b"RGSSAD\0\x01".to_vec();
    let mut key = 0xdeadcafeu32;
    for (name, data) in files {
        v.extend(le(name.len() as u32 ^ key));
        key = advance(key);
        for b in name.bytes() {
            v.push(b ^ key as u8);
            key = advance(key);
        }
        v.extend(le(data.len() as u32 ^ key));
        key = advance(key);
        v.extend(rgs_data(data, key));
    }
    let expected = files
        .iter()
        .map(|(n, b)| (n.to_string(), b.to_vec()))
        .collect::<Vec<_>>();
    assert_eq!(extract("rpgmaker/rgssad", v), expected);
    let seed = 123u32;
    let key = seed * 9 + 3;
    let base = 12 + files.iter().map(|(n, _)| 16 + n.len()).sum::<usize>() + 4;
    let mut v = b"RGSSAD\0\x03".to_vec();
    v.extend(le(seed));
    let mut offset = base;
    for (i, (name, data)) in files.iter().enumerate() {
        v.extend(le(offset as u32 ^ key));
        v.extend(le(data.len() as u32 ^ key));
        v.extend(le((0x12345678 + i as u32) ^ key));
        v.extend(le(name.len() as u32 ^ key));
        v.extend(
            name.bytes()
                .enumerate()
                .map(|(i, b)| b ^ key.rotate_right((i % 4 * 8) as u32) as u8),
        );
        offset += data.len();
    }
    v.extend(le(key));
    for (i, (_, data)) in files.iter().enumerate() {
        v.extend(rgs_data(data, 0x12345678 + i as u32));
    }
    assert_eq!(extract("rpgmaker/rgss3a", v), expected);
}
#[test]
fn afs_names_and_afs2_alignment_measure_from_aligned_start() {
    let mut v = b"AFS\0".to_vec();
    v.extend(le(2));
    v.extend(le(128));
    v.extend(le(3));
    v.extend(le(131));
    v.extend(le(6));
    v.extend(le(32));
    v.resize(32, 0);
    for name in [b"a.txt".as_slice(), b"b.dat".as_slice()] {
        v.extend(fixed(name, 32));
        v.extend([0; 16]);
    }
    v.extend(b"abcsecond");
    assert_eq!(
        extract("cri/afs", v),
        [
            ("a.txt".into(), b"abc".to_vec()),
            ("b.dat".into(), b"second".to_vec())
        ]
    );
    let mut v = b"AFS2".to_vec();
    v.extend(le(0));
    v.extend(le(3));
    v.extend(le(0));
    v.extend([0; 6]);
    v.extend(le(33));
    v.extend(le(67));
    v.resize(64, 0);
    v.extend(b"abc");
    v.resize(96, 0);
    v.extend(b"second");
    assert_eq!(
        extract("cri/afs2", v),
        [
            ("0.dat".into(), b"abc".to_vec()),
            ("1.dat".into(), b"second".to_vec())
        ]
    );
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
}
fn huffman_table(b: &[u8]) -> Vec<u8> {
    fn tree(bits: &mut Bits, depth: usize, value: u32) {
        if depth == 8 {
            bits.put(1, 0);
            bits.put(8, value)
        } else {
            bits.put(1, 1);
            tree(bits, depth + 1, value * 2);
            tree(bits, depth + 1, value * 2 + 1);
        }
    }
    let mut bits = Bits(vec![], 0);
    tree(&mut bits, 0, 0);
    for b in b {
        bits.put(8, u32::from(*b));
    }
    bits.0.iter().map(|b| b ^ 255).collect()
}
#[test]
fn minato_huffman_index_with_plain_and_zlib_entries() {
    for compressed in [false, true] {
        let data = if compressed {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            e.write_all(b"abc").unwrap();
            e.finish().unwrap()
        } else {
            b"abc".to_vec()
        };
        let mut table = fixed(b"a.txt", 64);
        table.extend(le(8));
        table.extend(le(3));
        table.extend(le(data.len() as u32));
        let table = huffman_table(&table);
        let mut v = b"PAC\0".to_vec();
        v.extend(le(1));
        v.extend(data);
        v.extend(&table);
        v.extend(le(table.len() as u32));
        assert_eq!(
            extract("minato-soft/pac", v)[0],
            ("a.txt".into(), b"abc".to_vec())
        );
    }
}
fn packdat(data: &[u8], encrypted: bool) -> Vec<u8> {
    let mut v = b"PACKDAT.".to_vec();
    v.extend(le(1));
    v.extend(le(0));
    v.extend(fixed(b"a.txt", 32));
    v.extend(le(64));
    v.extend(le(if encrypted { 0x10000 } else { 0 }));
    v.extend(le(data.len() as u32));
    v.extend(le(data.len() as u32));
    let mut out = data.to_vec();
    if encrypted {
        let n = (out.len() / 4) as u32;
        let mut key = (n << ((n & 7) + 8)) ^ n;
        for w in out.chunks_exact_mut(4) {
            let p = u32::from_le_bytes(w.try_into().unwrap());
            w.copy_from_slice(&(p ^ key).to_le_bytes());
            key = key.rotate_left(p % 24);
        }
    }
    v.extend(out);
    v
}
#[test]
fn packdat_both_ids_plain_and_dword_cipher_preserve_tail() {
    for id in ["system-epsilon/packdat", "innocent-grey/packdat"] {
        for encrypted in [false, true] {
            assert_eq!(
                extract(id, packdat(b"abcdefghijk", encrypted))[0].1,
                b"abcdefghijk"
            );
        }
    }
}
#[test]
fn iga_native_varints_name_pool_and_index_xor() {
    let mut v = b"IGA0".to_vec();
    v.resize(16, 0);
    v.extend([7, 1, 1, 7, 11]);
    v.extend(b"a.txt".iter().map(|b| b.wrapping_mul(2) | 1));
    v.extend(b"abc".iter().enumerate().map(|(i, b)| b ^ (i + 2) as u8));
    assert_eq!(
        extract("innocent-grey/iga", v)[0],
        ("a.txt".into(), b"abc".to_vec())
    );
}
#[test]
fn wbp_directory_buckets_and_multiple_records() {
    let mut v = fixed(b"ARCFORM4 WBUG ", 0xb09);
    set(&mut v, 16, 3);
    set(&mut v, 0x24, 0x900);
    set(&mut v, 0x28, 0x920);
    set(&mut v, 0x424, 0xa00);
    set(&mut v, 0x428, 0xa48);
    for (at, id, name) in [
        (0x900, 0, b"root/".as_slice()),
        (0x920, 1, b"sub/".as_slice()),
    ] {
        v[at + 1] = name.len() as u8 + 1;
        v[at + 2] = id;
        v[at + 4..at + 4 + name.len()].copy_from_slice(name);
    }
    for (at, id, name, data) in [
        (0xa00, 0, b"a.txt".as_slice(), 0xb00),
        (0xa24, 1, b"b.txt".as_slice(), 0xb03),
        (0xa48, 1, b"c.txt".as_slice(), 0xb06),
    ] {
        v[at + 1] = 12;
        v[at + 2] = id;
        set(&mut v, at + 4, data as u32);
        set(&mut v, at + 8, 3);
        v[at + 20..at + 20 + name.len()].copy_from_slice(name);
    }
    v[0xb00..].copy_from_slice(b"aaabbbccc");
    assert_eq!(
        extract("wild-bug/wbp", v),
        [
            ("root/a.txt".into(), b"aaa".to_vec()),
            ("sub/b.txt".into(), b"bbb".to_vec()),
            ("sub/c.txt".into(), b"ccc".to_vec())
        ]
    );
}
#[test]
#[ignore = "requires pinned ARC_UNPACKER_SOURCE; external files remain outside Git"]
fn upstream_d_archive_corpus() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ARC_UNPACKER_SOURCE").expect("ARC_UNPACKER_SOURCE"),
    );
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
    for (id, path, expected) in [
        (
            "real-live/ovk",
            "real_live/files/ovk/test.ovk",
            vec![b"1234567890".as_slice(), b"abcdefghijklmnopqrstuvwxyz"],
        ),
        (
            "rpgmaker/rgssad",
            "rpgmaker/files/rgssad/test.rgssad",
            vec![b"123".as_slice(), b"abcdefghij"],
        ),
        (
            "rpgmaker/rgss3a",
            "rpgmaker/files/rgss3a/test.rgss3a",
            vec![b"1234567890".as_slice(), b"abcdefghijklmnopqrstuvwxyz"],
        ),
        (
            "cri/afs",
            "cri/files/afs/test.afs",
            vec![b"1234567890".as_slice(), b"abcdefghijklmnopqrstuvwxyz"],
        ),
        (
            "cri/afs2",
            "cri/files/afs2/test.awb",
            vec![
                b"1234567890".as_slice(),
                b"1234567890",
                b"abcdefghijklmnopqrstuvwxyz",
            ],
        ),
        (
            "minato-soft/pac",
            "minato_soft/files/pac/uncompressed.pac",
            vec![b"1234567890".as_slice(), b"abcdefghijklmnopqrstuvwxyz"],
        ),
        (
            "minato-soft/pac",
            "minato_soft/files/pac/compressed.pac",
            vec![b"1234567890".as_slice(), b"abcdefghijklmnopqrstuvwxyz"],
        ),
        (
            "innocent-grey/iga",
            "innocent_grey/files/iga/test.iga",
            vec![b"1234567890".as_slice(), b"abcdefghijklmnopqrstuvwxyz"],
        ),
        (
            "wild-bug/wbp",
            "wild_bug/files/wbp/test.wbp",
            vec![
                b"1234567890".as_slice(),
                b"abcdefghijklmnopqrstuvwxyz",
                b"!@#$%^&*()_+",
            ],
        ),
    ] {
        let out = extract(
            id,
            std::fs::read(root.join("tests/dec").join(path)).unwrap(),
        );
        assert_eq!(
            out.iter().map(|(_, b)| b.as_slice()).collect::<Vec<_>>(),
            expected,
            "{path}"
        );
    }
    for path in ["encrypted.dat", "unencrypted.dat"] {
        let out = extract(
            "innocent-grey/packdat",
            std::fs::read(
                root.join("tests/dec/innocent_grey/files/packdat")
                    .join(path),
            )
            .unwrap(),
        );
        assert_eq!(
            out.iter().map(|(_, b)| b.clone()).collect::<Vec<_>>(),
            [
                b"1234567890".to_vec(),
                b"abcdefghijklmnopqrstuvwxyz".to_vec(),
                vec![255; 100]
            ]
        );
    }
}
