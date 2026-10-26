// SPDX-License-Identifier: GPL-3.0-or-later
//! Synthetic native-audio fixtures produce reusable standard WAVE resources.
use crysirror_core::{service::MemoryAssetReader, state::ClientId};
use crysirror_resource::{read_all, DecodeContext, FileRegistry, MemoryKeyProvider};
use std::sync::Arc;
fn p16(d: &mut [u8], p: usize, n: u16) {
    d[p..p + 2].copy_from_slice(&n.to_le_bytes());
}
fn p32(d: &mut [u8], p: usize, n: u32) {
    d[p..p + 4].copy_from_slice(&n.to_le_bytes());
}
fn fmt() -> Vec<u8> {
    let mut d = vec![0; 16];
    p16(&mut d, 0, 1);
    p16(&mut d, 2, 1);
    p32(&mut d, 4, 22050);
    p32(&mut d, 8, 44100);
    p16(&mut d, 12, 2);
    p16(&mut d, 14, 16);
    d
}
fn wav() -> Vec<u8> {
    let mut d = b"RIFF".to_vec();
    d.extend_from_slice(&40u32.to_le_bytes());
    d.extend_from_slice(b"WAVEfmt ");
    d.extend_from_slice(&16u32.to_le_bytes());
    d.extend(fmt());
    d.extend_from_slice(b"data\x04\0\0\0\x12\x34\x56\x78");
    d
}
fn decode(id: &str, path: &str, d: Vec<u8>) -> (Vec<u8>, serde_json::Value) {
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_audio(&mut r).unwrap();
    let cx = DecodeContext::new(ClientId(1));
    let (_, out) = r
        .decode(path, Arc::new(MemoryAssetReader(d.into())), Some(id), &cx)
        .unwrap_or_else(|e| panic!("{id}: {e:#}"));
    assert_eq!(
        out.extension,
        if out.metadata["loops"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
        {
            "wavloop"
        } else {
            "wav"
        }
    );
    let b = read_all(out.reader.as_ref(), u64::MAX).unwrap();
    assert_eq!(&b[..4], b"RIFF");
    assert_eq!(&b[8..12], b"WAVE");
    assert_eq!(
        u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize,
        b.len() - 8
    );
    (b, out.metadata)
}
fn data(wav: &[u8]) -> &[u8] {
    let mut p = 12;
    while p + 8 <= wav.len() {
        let n = u32::from_le_bytes(wav[p + 4..p + 8].try_into().unwrap()) as usize;
        if &wav[p..p + 4] == b"data" {
            return &wav[p + 8..p + 8 + n];
        }
        p += 8 + n + (n & 1);
    }
    panic!("WAVE data missing")
}
#[test]
fn pcm_and_wave_family() {
    let expected = b"\x12\x34\x56\x78";
    let (w, m) = decode("leaf/p16", "x.p16", expected.to_vec());
    assert_eq!(data(&w), expected);
    assert_eq!(m["sample_rate"], 44100);
    let (w, _) = decode("microsoft/wav", "x.wav", wav());
    assert_eq!(data(&w), expected);
    let mut d = vec![0; 18];
    d[0] = 1;
    d[1] = 2;
    p16(&mut d, 2, 22050);
    p16(&mut d, 4, 16);
    p32(&mut d, 6, 44100);
    p32(&mut d, 10, 4);
    p32(&mut d, 14, 1);
    d.extend(expected);
    let (w, m) = decode("leaf/w", "x.w", d);
    assert_eq!(data(&w), expected);
    assert_eq!(m["loops"][0], serde_json::json!([1, 4, 0]));
    let mut d = b"DW\0\0".to_vec();
    d.extend_from_slice(&16u32.to_le_bytes());
    d.extend_from_slice(&4u32.to_le_bytes());
    d.extend(fmt());
    d.extend(expected);
    let (w, _) = decode("lilim/dwv", "x.dwv", d);
    assert_eq!(data(&w), expected);
    for (id, header) in [
        ("twilight-frontier/pak2-sfx", vec![]),
        ("twilight-frontier/tfwa", b"TFWA\0".to_vec()),
    ] {
        let mut d = header;
        d.extend(fmt());
        d.extend_from_slice(&[0; 2]);
        d.extend_from_slice(&4u32.to_le_bytes());
        d.extend(expected);
        let (w, _) = decode(id, "x.cv3", d);
        assert_eq!(data(&w), expected);
    }
}
#[test]
fn waf_preserves_ms_adpcm_header() {
    let mut d = b"WAF\0\0\0".to_vec();
    let f = fmt();
    d.extend_from_slice(&f[2..]);
    d.resize(52, 0);
    p16(&mut d, 18, 4);
    d.extend_from_slice(&4u32.to_le_bytes());
    d.extend_from_slice(b"abcd");
    let (w, m) = decode("kid/waf", "x.waf", d);
    assert_eq!(data(&w), b"abcd");
    assert_eq!(m["codec"], 2);
    assert_eq!(u16::from_le_bytes(w[20..22].try_into().unwrap()), 2);
}
#[test]
fn wpn_and_wwa_sections() {
    let f = fmt();
    let mut d = b"WBD\x1aWAV\0".to_vec();
    d.extend_from_slice(&2u32.to_le_bytes());
    d.extend_from_slice(b"fmt ");
    d.extend_from_slice(&36u32.to_le_bytes());
    d.extend_from_slice(&16u32.to_le_bytes());
    d.extend_from_slice(b"data");
    d.extend_from_slice(&52u32.to_le_bytes());
    d.extend_from_slice(&4u32.to_le_bytes());
    d.extend(&f);
    d.extend_from_slice(b"abcd");
    let (w, _) = decode("wild-bug/wpn", "x", d);
    assert_eq!(data(&w), b"abcd");
    let mut d = vec![0; 48];
    d[..8].copy_from_slice(b"WPX\x1aWAV\0");
    d[12] = 1;
    d[14] = 2;
    d[16] = 0x20;
    d[17] = 128;
    p32(&mut d, 20, 48);
    p32(&mut d, 24, 16);
    d[32] = 0x21;
    d[33] = 128;
    p32(&mut d, 36, 64);
    p32(&mut d, 40, 4);
    d.extend(f);
    d.extend_from_slice(b"abcd");
    let (w, _) = decode("wild-bug/wwa", "x", d);
    assert_eq!(data(&w), b"abcd");
}
#[test]
fn abogado_adpcm_and_triangle_wady() {
    let mut d = vec![0; 196];
    p32(&mut d, 0, 22050);
    p16(&mut d, 4, 1);
    p32(&mut d, 188, 2);
    d.push(0x77);
    let (w, m) = decode("abogado/v", "x", d);
    assert_eq!(m["bits_per_sample"], 16);
    assert_eq!(data(&w), [13, 0, 43, 0]);
    let mut d = vec![0; 48];
    d[..4].copy_from_slice(b"WADY");
    d[5] = 2;
    p16(&mut d, 6, 1);
    p32(&mut d, 8, 22050);
    d.extend_from_slice(&[1, 1, 0x41, 0x80]);
    let (w, _) = decode("triangle/wady", "x", d);
    assert_eq!(data(&w), [4, 0, 8, 0, 4, 0, 0, 0]);
}
#[test]
fn ivory_wady_v1_and_v2() {
    let header = || {
        let mut d = vec![0; 48];
        d[..4].copy_from_slice(b"WADY");
        p16(&mut d, 6, 1);
        p32(&mut d, 8, 22050);
        p32(&mut d, 12, 3);
        p32(&mut d, 16, 3);
        p32(&mut d, 20, 6);
        p16(&mut d, 34, 1);
        p32(&mut d, 36, 22050);
        p32(&mut d, 40, 44100);
        p16(&mut d, 44, 2);
        p16(&mut d, 46, 16);
        d
    };
    let mut d = header();
    d.extend_from_slice(&[1, 1, 0x41]);
    let (w, _) = decode("ivory/wady", "x", d);
    assert_eq!(data(&w), [4, 0, 8, 0, 4, 0]);
    let mut d = header();
    d.extend_from_slice(&[0, 0, 0, 0]);
    d.extend_from_slice(&2u32.to_le_bytes());
    d.extend_from_slice(&100i16.to_le_bytes());
    d.extend_from_slice(&[3, 3]);
    let (w, _) = decode("ivory/wady", "x", d);
    assert_eq!(data(&w), [100, 0, 104, 0, 108, 0]);
}
#[test]
fn nwa_raw_and_compressed_blocks() {
    let mut d = vec![0; 40];
    p16(&mut d, 0, 1);
    p16(&mut d, 2, 16);
    p32(&mut d, 4, 22050);
    p32(&mut d, 8, u32::MAX);
    p32(&mut d, 20, 4);
    p32(&mut d, 28, 2);
    d.extend_from_slice(&[1, 2, 3, 4]);
    let (w, _) = decode("real-live/nwa", "x.nwa", d);
    assert_eq!(data(&w), [1, 2, 3, 4]);
    let mut d = vec![0; 48];
    p16(&mut d, 0, 1);
    p16(&mut d, 2, 16);
    p32(&mut d, 4, 22050);
    p32(&mut d, 16, 1);
    p32(&mut d, 20, 4);
    p32(&mut d, 28, 2);
    p32(&mut d, 36, 2);
    p32(&mut d, 44, 48);
    d.extend_from_slice(&100i16.to_le_bytes());
    d.push(0);
    let (w, _) = decode("real-live/nwa", "x.nwa", d);
    assert_eq!(data(&w), [100, 0, 100, 0]);
}
#[test]
fn mebius_keys_remain_client_scoped() {
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_audio(&mut r).unwrap();
    let mut cx = DecodeContext::new(ClientId(1));
    let keys = Arc::new(MemoryKeyProvider::default());
    keys.insert(cx.client, "mebius/koe.koe", vec![0xaa])
        .unwrap();
    cx.keys = keys;
    let input = Arc::new(MemoryAssetReader(wav().into()));
    let (_, out) = r
        .decode("x.koe", input.clone(), Some("mebius/koe"), &cx)
        .unwrap();
    let w = read_all(out.reader.as_ref(), 1024).unwrap();
    assert_eq!(data(&w), [0xb8, 0x9e, 0xfc, 0xd2]);
    drop(out);
    assert_eq!(cx.resident_bytes(), 0);
    cx.client = ClientId(2);
    assert!(r.decode("x.koe", input, Some("mebius/koe"), &cx).is_err());
}
#[test]
fn malformed_audio_fails_without_leaking_budget() {
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_audio(&mut r).unwrap();
    let mut cx = DecodeContext::new(ClientId(1));
    cx.limits.max_entry_size = 20;
    assert!(r
        .decode(
            "x.p16",
            Arc::new(MemoryAssetReader(vec![0; 2].into())),
            Some("leaf/p16"),
            &cx
        )
        .is_err());
    assert_eq!(cx.resident_bytes(), 0);
    assert!(r
        .decode(
            "x.p16",
            Arc::new(MemoryAssetReader(vec![0; 1].into())),
            Some("leaf/p16"),
            &cx
        )
        .is_err());
}
fn crc16(d: &[u8]) -> u16 {
    let mut c = 0u16;
    for b in d {
        c ^= (*b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 {
                c.wrapping_shl(1) ^ 0x8005
            } else {
                c << 1
            };
        }
    }
    c
}
#[test]
fn hca_silent_block_and_checksum_failure() {
    let mut h = b"HCA\0\x02\0\0\x40fmt\0".to_vec();
    h.extend_from_slice(&[1, 0, 0x56, 0x22]);
    h.extend_from_slice(&1u32.to_be_bytes());
    h.extend_from_slice(&[0; 4]);
    h.extend_from_slice(b"comp");
    h.extend_from_slice(&[0, 8, 1, 15, 1, 0, 1, 1, 0, 0, 0, 0]);
    h.extend_from_slice(b"ath\0\0\0ciph\0\0pad\0");
    h.resize(62, 0);
    let crc = crc16(&h);
    h.extend_from_slice(&crc.to_be_bytes());
    let mut block = vec![255, 255, 0, 0, 0, 0];
    let crc = crc16(&block);
    block.extend_from_slice(&crc.to_be_bytes());
    h.extend(block);
    let (w, m) = decode("cri/hca", "x.hca", h.clone());
    assert_eq!(m["sample_rate"], 22050);
    assert_eq!(data(&w), vec![0; 2048]);
    h[71] ^= 1;
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_audio(&mut r).unwrap();
    assert!(r
        .decode(
            "x.hca",
            Arc::new(MemoryAssetReader(h.into())),
            Some("cri/hca"),
            &DecodeContext::new(ClientId(0))
        )
        .is_err());
}
fn msb(seq: &[(u32, usize)]) -> Vec<u8> {
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
fn gamma(n: u32) -> Vec<(u32, usize)> {
    if n == 1 {
        return vec![(0, 1)];
    }
    let bits = n.ilog2();
    let mut v = vec![(1, 1)];
    for i in (0..bits).rev() {
        v.push(((n >> i) & 1, 1));
        v.push((u32::from(i != 0), 1));
    }
    v
}
fn section(name: &str, d: &[u8]) -> Vec<u8> {
    let mut o = vec![b' '; 8];
    o[..name.len()].copy_from_slice(name.as_bytes());
    o.extend_from_slice(&(d.len() as u64).to_le_bytes());
    o.extend_from_slice(d);
    o
}
fn mio_fixture(transformation: u32, channels: u32, entropy: Vec<u8>, count: u32) -> Vec<u8> {
    let mut h = vec![0; 40];
    p32(&mut h, 4, transformation);
    p32(&mut h, 8, (-4i32) as u32);
    p32(&mut h, 12, channels);
    p32(&mut h, 16, 22050);
    p32(&mut h, 24, 8);
    p32(&mut h, 28, count);
    p32(&mut h, 32, 1);
    p32(&mut h, 36, 16);
    let mut c = vec![0, 1, 0, 0];
    c.extend_from_slice(&count.to_le_bytes());
    c.extend(entropy);
    let mut d = b"Entis\x1a\0\0\0\x01\0\x03\0\0\0\0Music Interleaved and Orthogonal".to_vec();
    d.resize(64, 0);
    d.extend(section("Header", &section("SoundInf", &h)));
    d.extend(section("Stream", &section("SoundStm", &c)));
    d
}
#[test]
fn mio_lossless_and_lot_mss_silence() {
    let mut raw = vec![(0, 8)];
    raw.extend(gamma(2));
    let (w, _) = decode(
        "entis/mio",
        "x.mio",
        mio_fixture(0x3020000, 1, msb(&raw), 1),
    );
    assert_eq!(data(&w), [0, 0]);
    for (transform, channels) in [(5, 1), (0x105, 2)] {
        let mut seq = vec![(0, 1), (0, 2)];
        if channels == 2 {
            seq.push((0, 2));
        }
        seq.extend([(0, 32), (1, 16)]);
        if channels == 2 {
            seq.push((0, 2));
        }
        seq.extend([(0, 32), (1, 16), (0, 1), (0, 8)]);
        seq.extend(gamma(512 * channels));
        let (w, _) = decode(
            "entis/mio",
            "x.mio",
            mio_fixture(transform, channels, msb(&seq), 256),
        );
        assert_eq!(data(&w), vec![0; 512 * channels as usize]);
    }
}
#[test]
#[ignore = "requires separately downloaded pinned upstream corpus; no source media is distributed"]
fn external_upstream_audio_pcm_parity() {
    let root = std::path::PathBuf::from(
        std::env::var_os("ARC_UNPACKER_SOURCE").expect("set ARC_UNPACKER_SOURCE"),
    );
    for (id, dir, input, expected, zipped) in [
        (
            "ivory/wady",
            "ivory/files/wady",
            "m01-zlib",
            "m01-zlib-out.wav",
            true,
        ),
        (
            "ivory/wady",
            "ivory/files/wady",
            "10510-zlib",
            "10510-zlib-out.wav",
            true,
        ),
        (
            "ivory/wady",
            "ivory/files/wady",
            "071-zlib",
            "071-zlib-out.wav",
            true,
        ),
        (
            "kid/waf",
            "kid/files/waf",
            "CEP037.waf",
            "CEP037-out.wav",
            false,
        ),
        (
            "leaf/p16",
            "leaf/files/p16",
            "PCM01-zlib.P16",
            "PCM01-zlib-out.wav",
            true,
        ),
        (
            "leaf/w",
            "leaf/files/w",
            "se068-zlib.w",
            "se068-zlib-out.wav",
            true,
        ),
        (
            "lilim/dwv",
            "lilim/files/dwv",
            "SE010.dwv",
            "SE010-out.wav",
            false,
        ),
        (
            "twilight-frontier/pak2-sfx",
            "twilight_frontier/files/pak2",
            "049.cv3",
            "049-out.wav",
            false,
        ),
        (
            "twilight-frontier/tfwa",
            "twilight_frontier/files/tfwa",
            "2592.wav",
            "2592-out.wav",
            false,
        ),
        (
            "entis/mio",
            "entis/files/mio",
            "explosion.mio",
            "explosion-out.wav",
            false,
        ),
        (
            "entis/mio",
            "entis/files/mio",
            "SE_017.mio",
            "SE_017-out.wav",
            false,
        ),
        (
            "cri/hca",
            "cri/files/hca",
            "test.hca",
            "test-out.wav",
            false,
        ),
        (
            "real-live/nwa",
            "real_live/files/nwa",
            "BT_KOE_HCC01-zlib.nwa",
            "BT_KOE_HCC01-zlib-out.wav",
            true,
        ),
        (
            "real-live/nwa",
            "real_live/files/nwa",
            "BATSWING-zlib.nwa",
            "BATSWING-zlib-out.wav",
            true,
        ),
        (
            "triangle/wady",
            "triangle/files/wady",
            "M03-zlib",
            "M03-out-zlib.wav",
            true,
        ),
        (
            "wild-bug/wpn",
            "wild_bug/files/wpn",
            "SE_107-zlib.WPN",
            "SE_107-zlib-out.wav",
            true,
        ),
        (
            "wild-bug/wwa",
            "wild_bug/files/wwa",
            "06A5A009.WWA",
            "06A5A009-out.wav",
            false,
        ),
    ] {
        let base = root.join("tests/dec").join(dir);
        let mut a = std::fs::read(base.join(input)).unwrap();
        let mut b = std::fs::read(base.join(expected)).unwrap();
        if zipped {
            use std::io::Read;
            let mut x = Vec::new();
            flate2::read::ZlibDecoder::new(a.as_slice())
                .read_to_end(&mut x)
                .unwrap();
            a = x;
            let mut x = Vec::new();
            flate2::read::ZlibDecoder::new(b.as_slice())
                .read_to_end(&mut x)
                .unwrap();
            b = x;
        }
        let path = if id == "real-live/nwa" {
            "x.nwa"
        } else {
            input
        };
        let (actual, _) = decode(id, path, a);
        let actual = data(&actual);
        let expected = data(&b);
        assert_eq!(actual.len(), expected.len(), "{id} {input}");
        let mismatch = actual
            .chunks_exact(2)
            .zip(expected.chunks_exact(2))
            .filter(|(a, b)| a != b)
            .count();
        let max = actual
            .chunks_exact(2)
            .zip(expected.chunks_exact(2))
            .map(|(a, b)| {
                (i16::from_le_bytes(a.try_into().unwrap()) as i32
                    - i16::from_le_bytes(b.try_into().unwrap()) as i32)
                    .unsigned_abs()
            })
            .max()
            .unwrap_or(0);
        eprintln!(
            "{id} {input}: {} samples, {mismatch} unequal, maximum delta {max}",
            actual.len() / 2
        );
        assert_eq!(max, 0, "{id} {input}");
    }
}
#[test]
fn native_loop_header_units_survive_shared_wave_encoder() {
    let mut d = vec![0; 18];
    d[0] = 1;
    d[1] = 2;
    p16(&mut d, 2, 22050);
    p16(&mut d, 4, 16);
    p32(&mut d, 6, 44100);
    p32(&mut d, 10, 4);
    p32(&mut d, 14, 2);
    d.extend_from_slice(&[0; 4]);
    let (w, m) = decode("leaf/w", "x.w", d);
    assert_eq!(m["loops"][0], serde_json::json!([2, 4, 0]));
    assert_eq!(u32::from_le_bytes(w[16..20].try_into().unwrap()), 18);
    assert_eq!(&w[50..54], b"smpl");
    assert_eq!(&w[58..86], &[0; 28]);
    assert_eq!(u32::from_le_bytes(w[102..106].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(w[106..110].try_into().unwrap()), 4);
}
#[test]
fn truncated_audio_signatures_do_not_panic() {
    let mut r = FileRegistry::default();
    crysirror_formats_arc::register_audio(&mut r).unwrap();
    let mut cx = DecodeContext::new(ClientId(0));
    cx.limits.max_entry_size = 8192;
    for (id, path, magic) in [
        ("cri/hca", "x.hca", b"HCA\0".as_slice()),
        ("ivory/wady", "x", b"WADY".as_slice()),
        ("triangle/wady", "x", b"WADY".as_slice()),
        ("wild-bug/wwa", "x", b"WPX\x1aWAV\0".as_slice()),
        ("wild-bug/wpn", "x", b"WBD\x1aWAV\0".as_slice()),
        ("kid/waf", "x", b"WAF\0\0\0".as_slice()),
        ("lilim/dwv", "x.dwv", b"DW".as_slice()),
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
