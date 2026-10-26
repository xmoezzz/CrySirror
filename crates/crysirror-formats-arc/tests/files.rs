// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-authored fixtures; no game resources or title keys are checked in.
use crysirror_core::{service::MemoryAssetReader, state::ClientId};
use crysirror_resource::{read_all, DecodeContext, FileRegistry, MemoryKeyProvider};
use std::{io::Write, sync::Arc};
fn put32(d: &mut [u8], p: usize, n: u32) {
    d[p..p + 4].copy_from_slice(&n.to_le_bytes());
}
fn bits(seq: &[(u32, usize)]) -> Vec<u8> {
    let mut d = vec![0; seq.iter().map(|v| v.1).sum::<usize>().div_ceil(8)];
    let mut i = 0;
    for &(v, n) in seq {
        for shift in (0..n).rev() {
            d[i / 8] |= (((v >> shift) & 1) as u8) << (7 - i % 8);
            i += 1;
        }
    }
    d
}
fn zlib(d: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
    e.write_all(d).unwrap();
    e.finish().unwrap()
}
fn run(id: &str, path: &str, input: Vec<u8>, expected: &[u8], extension: &str) {
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_files(&mut r).unwrap();
    let cx = DecodeContext::new(ClientId(7));
    let (_, out) = r
        .decode(
            path,
            Arc::new(MemoryAssetReader(input.into())),
            Some(id),
            &cx,
        )
        .unwrap_or_else(|e| panic!("{id}: {e:#}"));
    assert_eq!(out.extension, extension, "{id}");
    assert_eq!(
        read_all(out.reader.as_ref(), u64::MAX).unwrap(),
        expected,
        "{id}"
    );
}
#[test]
fn wrapped_and_xor_files() {
    let p = b"OggSself";
    let mut d = b"AFF\0".to_vec();
    d.resize(16, 0);
    let k = [0xc8, 0xbb, 0x8f, 0xb7, 0xed, 0x43, 0x99, 0x4a];
    d.extend(p.iter().zip(k).map(|(a, b)| a ^ b));
    run("alice-soft/aff", "x.aff", d, p, "ogg");
    let mut d = b"BGM ".to_vec();
    d.resize(12, 0);
    d.extend_from_slice(p);
    run("amuse-craft/bgm", "x.bgm", d, p, "ogg");
    let mut d = b"AoiOgg".to_vec();
    d.resize(44, 0);
    d.extend_from_slice(p);
    run("aoi/aog", "x.aog", d, p, "ogg");
    run("aoi/aog", "x.aog", p.to_vec(), p, "ogg");
    let mut d = vec![0; 16];
    put32(&mut d, 0, 16);
    d[4..8].copy_from_slice(b"bw  ");
    put32(&mut d, 8, p.len() as u32);
    d.extend_from_slice(p);
    run("bgi/audio", "x", d, p, "ogg");
    let p = b"\xff\xfbself";
    run(
        "complets/vmd",
        "x.vmd",
        p.iter().map(|v| v ^ 0xe5).collect(),
        p,
        "mp3",
    );
    let mut d = b"CRM\0".to_vec();
    d.extend_from_slice(&4u32.to_le_bytes());
    d.extend_from_slice(b"OggS");
    run("crowd/eog", "x.eog", d, b"OggS", "ogg");
    let mut d = vec![0; 20];
    d[4..8].copy_from_slice(&[3, 0x95, 0xad, 0x4b]);
    put32(&mut d, 12, 4);
    d.extend_from_slice(b"OggS");
    run("leaf/pak2-audio", "x", d, b"OggS", "ogg");
}
#[test]
fn bse_checksum_permutation() {
    let plain: Vec<u8> = (0..64).collect();
    let mut d = b"BSE 1.0\0".to_vec();
    d.resize(80, 0);
    d[10] = plain.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    d[11] = plain.iter().fold(0u8, |a, b| a ^ b);
    put32(&mut d, 12, 123);
    let mut seed = 123i32;
    let mut flags = [false; 64];
    let mut rand = || {
        let t = ((seed.wrapping_mul(257) >> 8)
            .wrapping_add(seed.wrapping_mul(97))
            .wrapping_add(23) as u32)
            ^ 0xa6cd9b75;
        seed = t.rotate_right(16) as i32;
        (seed as u32 & 0x7fff) as usize
    };
    for _ in 0..64 {
        let mut i = rand() % 64;
        while flags[i] {
            i = (i + 1) % 64;
        }
        let shift = rand() % 8;
        let key = rand();
        let add = rand();
        d[16 + i] = if key & 1 != 0 {
            plain[i].rotate_right(shift as u32)
        } else {
            plain[i].rotate_left(shift as u32)
        }
        .wrapping_add(add as u8);
        flags[i] = true;
    }
    run("bgi/bse", "x", d, &plain, "dat");
}
#[test]
fn huffman_dictionary_and_transform_files() {
    let mut d = vec![0; 16];
    d[..4].copy_from_slice(b"lnd\0");
    put32(&mut d, 8, 6);
    d.extend_from_slice(&[1, b'A', b'B', 0x80, 1, 0xc0, b'C']);
    run("kid/lnd", "x", d, b"ABABCC", "dat");
    let mut d = vec![0; 36];
    d[4..8].copy_from_slice(&[0xaf, 0xf6, 0x4d, 0x4f]);
    put32(&mut d, 24, 6);
    d.extend_from_slice(&[0x82, b'A', b'B', 0, 2, 0xe0]);
    run("leaf/pak2-compressed-file", "x", d, b"ABAB\0\0", "dat");
    let mut d = vec![0; 20];
    d[..3].copy_from_slice(b"BMR");
    put32(&mut d, 4, 1);
    put32(&mut d, 12, 1);
    d.extend(bits(&[(0, 1), (65, 8)]));
    run("kaguya/bmr", "x", d, b"A", "dat");
    let mut d = 1u32.to_le_bytes().to_vec();
    d.extend(bits(&[(0, 1), (65, 8)]));
    run("lilim/scr", "x.scr", d, b"A", "txt");
    let mut d = b"acp\0".to_vec();
    d.extend(bits(&[(4, 32), (65, 9), (66, 9), (259, 9)]));
    run("escude/acp", "x", d, b"ABAB", "dat");
    let mut d = b"CPS\0".to_vec();
    d.resize(16, 0);
    put32(&mut d, 4, 21);
    put32(&mut d, 12, 1);
    d.push(65);
    d.extend_from_slice(&0x7534682u32.to_le_bytes());
    run("kid/cps", "x", d, b"A", "prt");
}
#[test]
fn dsc_canonical_codes() {
    let mut d = b"DSC FORMAT 1.00\0".to_vec();
    d.resize(544, 0);
    put32(&mut d, 20, 3);
    let mut key = 0u32;
    for i in 0..512 {
        let a = 0x4e35u32.wrapping_mul(key & 0xffff);
        let b = 0x4e35u32.wrapping_mul(key >> 16);
        let t = 0x15au32
            .wrapping_mul(key)
            .wrapping_add(b)
            .wrapping_add(a >> 16);
        key = (t << 16).wrapping_add(a & 0xffff).wrapping_add(1);
        d[32 + i] = (t as u8).wrapping_add(if i == 65 { 1 } else { 0 });
    }
    d.push(0);
    run("bgi/dsc", "x", d, b"AAA", "dat");
}
#[test]
fn sysd_ps2_and_zlib() {
    let mut d = b"CC\0\0DD\0\0".to_vec();
    d.extend_from_slice(&12u32.to_le_bytes());
    d.extend_from_slice(&3u32.to_le_bytes());
    d.push(0);
    d.extend_from_slice(&12u32.to_le_bytes());
    d.extend_from_slice(&3u32.to_le_bytes());
    d.extend_from_slice(b"abc");
    run("lilim/doj", "x.doj", d, b"abc", "dat");
    let mut d = vec![0; 48];
    d[..4].copy_from_slice(b"PS2A");
    put32(&mut d, 36, 2);
    put32(&mut d, 40, 1);
    d.extend([1u8, 65].map(|v| v.rotate_left(1).wrapping_add(0x7c)));
    let mut expected = d[..48].to_vec();
    put32(&mut expected, 36, 1);
    expected.push(65);
    run("purple-software/ps2", "x.ps2", d, &expected, "dat");
    run(
        "scene-player/pmw",
        "x.pmw",
        zlib(b"RIFF").iter().map(|v| v ^ 0x21).collect(),
        b"RIFF",
        "wav",
    );
    let mut plain = Vec::new();
    for n in [1u32, 2, 3] {
        plain.extend_from_slice(&n.to_le_bytes());
    }
    plain.extend_from_slice(b"a,b");
    plain.extend_from_slice(&1u32.to_le_bytes());
    plain.push(b'z');
    let mut d = b"TFCS\0".to_vec();
    let zipped = zlib(&plain);
    d.extend_from_slice(&(zipped.len() as u32).to_le_bytes());
    d.extend_from_slice(&(plain.len() as u32).to_le_bytes());
    d.extend(zipped);
    run("twilight-frontier/tfcs", "x", d, b"\"a,b\",z\n", "csv");
}
#[test]
fn ogv_and_ogg_stream_filter() {
    let mut d = b"OGV\0".to_vec();
    d.resize(12, 0);
    d.extend_from_slice(b"fmt ");
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(b"data");
    d.extend_from_slice(&4u32.to_le_bytes());
    d.extend_from_slice(b"OggS");
    run("shiina-rio/ogv", "x", d, b"OggS", "ogg");
    let page = |serial: u32| {
        let mut d = b"OggS".to_vec();
        d.resize(27, 0);
        put32(&mut d, 14, serial);
        d
    };
    let first = page(1);
    let mut data = first.clone();
    data.extend(page(2));
    let mut d = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(b"fact\x04\0\0\0\0\0\0\0data");
    d.extend_from_slice(&(data.len() as u32).to_le_bytes());
    d.extend(data);
    run("vorbis/wav", "x.wav", d, &first, "ogg");
    let mut d = vec![0; 27];
    d[5] = 2;
    d[26] = 1;
    d.push(3);
    d.extend_from_slice(&[1, 0, 65]);
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_files(&mut r).unwrap();
    let (_, out) = r
        .decode(
            "x.g",
            Arc::new(MemoryAssetReader(d.into())),
            Some("leaf/g"),
            &DecodeContext::new(ClientId(0)),
        )
        .unwrap();
    let o = read_all(out.reader.as_ref(), 1024).unwrap();
    assert_eq!(&o[..4], b"OggS");
    assert_eq!(&o[28..], b"\x01vorbisA");
}
#[test]
fn wbi_plain_and_compressed_sections() {
    for compressed in [false, true] {
        let mut d = vec![0; 32];
        d[..8].copy_from_slice(b"WPX\x1aEX2\0");
        d[12] = 1;
        d[14] = 1;
        d[16] = 2;
        d[17] = if compressed { 0 } else { 128 };
        put32(&mut d, 20, 32);
        put32(&mut d, 24, 3);
        if compressed {
            let payload = [b'A', 0, 0, 0, 0xc0, b'B', b'C'];
            put32(&mut d, 28, payload.len() as u32);
            d.extend(payload);
        } else {
            d.extend_from_slice(b"ABC");
        }
        run("wild-bug/wbi", "x", d, b"ABC", "dat");
    }
}
#[test]
fn caller_keys_are_scoped_and_budget_released() {
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_files(&mut r).unwrap();
    let mut d = vec![0; 3604];
    d[3600..].copy_from_slice(&[1, 2, 3, 4]);
    let input = Arc::new(MemoryAssetReader(d.into()));
    let mut cx = DecodeContext::new(ClientId(3));
    let keys = Arc::new(MemoryKeyProvider::default());
    keys.insert(ClientId(3), "eagls/pak-script.xor", vec![7])
        .unwrap();
    cx.keys = keys;
    let (_, out) = r
        .decode("x.dat", input.clone(), Some("eagls/pak-script"), &cx)
        .unwrap();
    assert_eq!(read_all(out.reader.as_ref(), 99).unwrap(), [6, 2, 4]);
    assert_eq!(cx.resident_bytes(), 3);
    drop(out);
    assert_eq!(cx.resident_bytes(), 0);
    cx.client = ClientId(4);
    assert!(r
        .decode("x.dat", input, Some("eagls/pak-script"), &cx)
        .is_err());
    cx.limits.max_entry_size = 1;
    let mut d = b"lnd\0".to_vec();
    d.resize(16, 0);
    put32(&mut d, 8, 2);
    assert!(r
        .decode(
            "x",
            Arc::new(MemoryAssetReader(d.into())),
            Some("kid/lnd"),
            &cx
        )
        .is_err());
    assert_eq!(cx.resident_bytes(), 0);
}
#[test]
fn dogenzaka_uses_caller_rc4_key() {
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_files(&mut r).unwrap();
    let mut cx = DecodeContext::new(ClientId(1));
    let keys = Arc::new(MemoryKeyProvider::default());
    keys.insert(cx.client, "dogenzaka/a.rc4", b"Key".to_vec())
        .unwrap();
    cx.keys = keys;
    let (_, out) = r
        .decode(
            "x.a",
            Arc::new(MemoryAssetReader(
                vec![0xbb, 0xf3, 0x16, 0xe8, 0xd9, 0x40, 0xaf, 0x0a, 0xd3].into(),
            )),
            Some("dogenzaka/a"),
            &cx,
        )
        .unwrap();
    assert_eq!(read_all(out.reader.as_ref(), 99).unwrap(), b"Plaintext");
}
#[test]
#[ignore = "requires separately downloaded pinned upstream corpus; no source media is distributed"]
fn external_upstream_file_parity() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ARC_UNPACKER_SOURCE").expect("set ARC_UNPACKER_SOURCE"),
    );
    let mut registry = FileRegistry::default();
    crysirror_formats_arc::register_files(&mut registry).unwrap();
    for (id, dir, input, expected, image) in [
        (
            "alice-soft/aff",
            "alice_soft/files/aff",
            "test.aff",
            "test-out.aff",
            false,
        ),
        (
            "bgi/audio",
            "bgi/files/audio",
            "wa_038.dat",
            "wa_038-out.ogg",
            false,
        ),
        (
            "bgi/bse",
            "bgi/files/bse",
            "virtualbitmap",
            "virtualbitmap-out.dat",
            false,
        ),
        (
            "bgi/dsc",
            "bgi/files/dsc",
            "setupforgallery",
            "setupforgallery-out.dat",
            false,
        ),
        (
            "bgi/dsc",
            "bgi/files/dsc",
            "SGTitle000000",
            "SGTitle000000-out.png",
            true,
        ),
        (
            "bgi/dsc",
            "bgi/files/dsc",
            "SGTitle010000",
            "SGTitle010000-out.png",
            true,
        ),
        (
            "bgi/dsc",
            "bgi/files/dsc",
            "SGMsgWnd010300",
            "SGMsgWnd010300-out.png",
            true,
        ),
        (
            "escude/acp",
            "escude/files/acp",
            "misa01.BMP",
            "misa01-out.BMP",
            false,
        ),
        (
            "kaguya/bmr",
            "kaguya/files/bmr",
            "bg_black.bmp",
            "bg_black-out.png",
            true,
        ),
        (
            "kid/cps",
            "kid/files/cps",
            "yh04adm.cps",
            "yh04adm.prt",
            false,
        ),
        (
            "kid/lnd",
            "kid/files/lnd",
            "CEP037.waf.lnd",
            "../waf/CEP037.waf",
            false,
        ),
        (
            "leaf/g",
            "leaf/files/g",
            "asu_1400_080.g",
            "asu_1400_080-out.ogg",
            false,
        ),
        (
            "leaf/pak2-compressed-file",
            "leaf/files/pak2-compressed-file",
            "ttt",
            "ttt-out",
            false,
        ),
        (
            "lilim/doj",
            "lilim/files/doj",
            "trueblue.doj",
            "trueblue-out.doj",
            false,
        ),
        (
            "lilim/scr",
            "lilim/files/scr",
            "var.scr",
            "var-out.txt",
            false,
        ),
        (
            "purple-software/ps2",
            "purple_software/files/ps2",
            "intproc.ps3",
            "intproc-out.ps3",
            false,
        ),
        (
            "shiina-rio/ogv",
            "shiina_rio/files/ogv",
            "TPSE070.OGV",
            "TPSE070-out.ogg",
            false,
        ),
        (
            "twilight-frontier/tfcs",
            "twilight_frontier/files/tfcs",
            "ItemCommon.csv",
            "ItemCommon-out.csv",
            false,
        ),
        (
            "vorbis/wav",
            "vorbis/files/packed_ogg",
            "1306.wav",
            "1306-out.ogg",
            false,
        ),
        (
            "vorbis/wav",
            "vorbis/files/packed_ogg",
            "90WIF020_001.WAV",
            "90WIF020_001-out.ogg",
            false,
        ),
        (
            "wild-bug/wbi",
            "wild_bug/files/wbi",
            "START.WBI",
            "START-out.dat",
            false,
        ),
    ] {
        let base = root.join("tests/dec").join(dir);
        let input_data = std::fs::read(base.join(input)).unwrap();
        let expected = std::fs::read(base.join(expected)).unwrap();
        let (_, out) = registry
            .decode(
                input,
                Arc::new(MemoryAssetReader(input_data.into())),
                Some(id),
                &DecodeContext::new(ClientId(0)),
            )
            .unwrap_or_else(|e| panic!("{id} {input}: {e:#}"));
        let actual = read_all(out.reader.as_ref(), u64::MAX).unwrap();
        if image {
            assert_eq!(
                crysirror_image::decode_standard(&actual).unwrap(),
                crysirror_image::decode_standard(&expected).unwrap(),
                "{id} {input}"
            );
        } else {
            assert_eq!(actual.len(), expected.len(), "{id} {input}");
            assert!(actual == expected, "{id} {input}: unequal bytes");
        }
        eprintln!("{id} {input}: {} bytes matched", actual.len());
    }
}
#[test]
fn truncated_recognized_headers_return_errors() {
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_files(&mut r).unwrap();
    let mut cx = DecodeContext::new(ClientId(0));
    cx.limits.max_entry_size = 8192;
    for (id, path, magic) in [
        ("kaguya/bmr", "x", b"BMR".as_slice()),
        ("bgi/bse", "x", b"BSE 1.0\0".as_slice()),
        ("bgi/dsc", "x", b"DSC FORMAT 1.00\0".as_slice()),
        ("escude/acp", "x", b"acp\0".as_slice()),
        ("wild-bug/wbi", "x", b"WPX\x1aEX2\0".as_slice()),
        ("kid/cps", "x", b"CPS\0".as_slice()),
        ("kid/lnd", "x", b"lnd\0".as_slice()),
    ] {
        for extra in 0..64 {
            let mut d = magic.to_vec();
            d.resize(d.len() + extra, 0xff);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                r.decode(path, Arc::new(MemoryAssetReader(d.into())), Some(id), &cx)
            }));
            assert!(
                result.is_ok(),
                "{id} panicked at length {}",
                magic.len() + extra
            );
            drop(result);
            assert_eq!(cx.resident_bytes(), 0);
        }
    }
}
