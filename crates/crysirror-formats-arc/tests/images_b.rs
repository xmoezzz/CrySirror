// SPDX-License-Identifier: GPL-3.0-or-later
use crysirror_formats_arc::images_b::{decode, probe, FORMATS};
use crysirror_image::{DecoderRegistry, ImageDecodeContext, ImageSurface};
use std::io::{Read, Write};
const PIX: &[u8] = &[30, 20, 10, 40, 70, 60, 50, 80];
fn run(id: &str, path: &str, b: &[u8]) -> ImageSurface {
    let r = DecoderRegistry::default();
    let ctx = ImageDecodeContext::new(path, &r);
    decode(id, b, &ctx).unwrap()
}
fn check(id: &str, path: &str, b: &[u8], expected: &[u8]) {
    assert!(probe(id, path, b), "probe {id}");
    let got = run(id, path, b);
    assert_eq!((got.width, got.height), (2, 1), "{id}");
    assert_eq!(got.rgba, expected, "{id}");
}
fn le(b: &mut [u8], p: usize, n: u32) {
    b[p..p + 4].copy_from_slice(&n.to_le_bytes());
}
fn zlib(b: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(b).unwrap();
    z.finish().unwrap()
}
fn literals(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for c in b.chunks(8) {
        out.push(((1u16 << c.len()) - 1) as u8);
        out.extend(c);
    }
    out
}
fn bit_literals(b: &[u8]) -> Vec<u8> {
    let mut out = vec![0; (b.len() * 9).div_ceil(8)];
    for (i, &v) in b.iter().enumerate() {
        let v = 0x100 | u16::from(v);
        for j in 0..9 {
            out[(i * 9 + j) / 8] |= (((v >> (8 - j)) & 1) as u8) << (7 - (i * 9 + j) % 8);
        }
    }
    out
}
fn bmp() -> Vec<u8> {
    let mut b = vec![0; 62];
    b[..2].copy_from_slice(b"BM");
    le(&mut b, 10, 54);
    le(&mut b, 14, 40);
    le(&mut b, 18, 2);
    le(&mut b, 22, 1);
    b[26] = 1;
    b[28] = 32;
    b[54..].copy_from_slice(&[10, 20, 30, 40, 50, 60, 70, 80]);
    b
}
fn png() -> Vec<u8> {
    crysirror_image::encode_png(&ImageSurface {
        width: 2,
        height: 1,
        rgba: PIX.to_vec(),
    })
    .unwrap()
}
fn cwd() -> Vec<u8> {
    let mut b = vec![0; 60];
    b[..28].copy_from_slice(b"cwd format  - version 1.00 -");
    le(&mut b, 44, 2u32.wrapping_sub(0x259a));
    le(&mut b, 48, 1u32.wrapping_sub(0x259a));
    b[56..].copy_from_slice(&[0, 124, 31, 0]);
    b
}
#[test]
fn crowd_cwd_555_is_truncated() {
    check(
        "crowd/cwd",
        "x.cwd",
        &cwd(),
        &[248, 0, 0, 255, 0, 0, 248, 255],
    );
}
#[test]
fn crowd_cwl_dictionary_stream() {
    let c = cwd();
    let mut b = vec![0; 14];
    b[..4].copy_from_slice(b"SZDD");
    le(&mut b, 10, c.len() as u32);
    b.extend(literals(&c));
    check("crowd/cwl", "x.CWL", &b, &[248, 0, 0, 255, 0, 0, 248, 255]);
}
#[test]
fn crowd_zbm_inverts_first_hundred_bytes() {
    let mut c = bmp();
    for v in c.iter_mut().take(100) {
        *v ^= 255;
    }
    let mut b = vec![0; 14];
    b[..4].copy_from_slice(b"SZDD");
    le(&mut b, 10, c.len() as u32);
    b.extend(literals(&c));
    check("crowd/zbm", "x.zbm", &b, PIX);
}
#[test]
fn crowd_cwp_restores_png_then_swaps_and_forces_alpha() {
    let p = png();
    let mut b = b"CWDP".to_vec();
    b.extend(&p[16..33]);
    let first = u32::from_be_bytes(p[33..37].try_into().unwrap()) as usize;
    assert_eq!(&p[37..41], b"IDAT");
    b.extend((first as u32).to_be_bytes());
    b.extend(&p[41..p.len() - 12]);
    b.push(0);
    check(
        "crowd/cwp",
        "x.cwp",
        &b,
        &[10, 20, 30, 255, 50, 60, 70, 255],
    );
}
#[test]
fn teyl_bgra() {
    let mut b = b"TEYL".to_vec();
    b.extend(2u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend([10, 20, 30, 40, 50, 60, 70, 80]);
    check("almond-collective/teyl", "x.teyl", &b, PIX);
}
#[test]
fn mgf_replaces_only_signature() {
    let mut b = png();
    b[..8].copy_from_slice(b"MalieGF\0");
    check("malie/mgf", "x.mgf", &b, PIX);
}
#[test]
fn fil_grayscale() {
    let mut b = vec![0; 8];
    le(&mut b, 0, 2);
    le(&mut b, 4, 1);
    b.extend([17, 213]);
    check(
        "minato-soft/fil",
        "x.fil",
        &b,
        &[17, 17, 17, 255, 213, 213, 213, 255],
    );
}
#[test]
fn cmp_msb_lzss() {
    let c = bmp();
    let packed = bit_literals(&c);
    let mut b = b"CMP1".to_vec();
    b.extend((c.len() as u32).to_le_bytes());
    b.extend((packed.len() as u32).to_le_bytes());
    b.extend(packed);
    check("riddle-soft/cmp", "x.gcp", &b, PIX);
}
#[test]
fn xyz_rgb_palette_is_opaque() {
    let mut b = b"XYZ1\x02\0\x01\0".to_vec();
    let mut data = vec![0; 768];
    data[3..6].copy_from_slice(&[1, 2, 3]);
    data[765..].copy_from_slice(&[7, 8, 9]);
    data.extend([1, 255]);
    b.extend(zlib(&data));
    check("rpgmaker/xyz", "x.xyz", &b, &[1, 2, 3, 255, 7, 8, 9, 255]);
}
#[test]
fn pmp_xor_zlib() {
    let b: Vec<u8> = zlib(&bmp()).into_iter().map(|v| v ^ 33).collect();
    check("scene-player/pmp", "x.pmp", &b, PIX);
}
#[test]
fn pga_png_header() {
    let mut b = b"PGAPGAH\x0a\0\0\0".to_vec();
    b.extend(&png()[16..]);
    check("sysadv/pga", "x.pga", &b, PIX);
}
#[test]
fn syg_bgr() {
    let mut b = vec![0; 32];
    b[..4].copy_from_slice(b"$SYG");
    le(&mut b, 16, 2);
    le(&mut b, 20, 1);
    b.extend([10, 20, 30, 50, 60, 70]);
    check(
        "west-vision/syg",
        "x.syg",
        &b,
        &[30, 20, 10, 255, 70, 60, 50, 255],
    );
}
#[test]
fn ycg_two_independent_zlib_streams() {
    let p1 = zlib(&[10, 20, 30, 40]);
    let p2 = zlib(&[50, 60, 70, 80]);
    let mut b = vec![0; 56];
    b[..4].copy_from_slice(b"YCG\0");
    for (p, n) in [
        (4, 2),
        (8, 1),
        (12, 32),
        (16, 1),
        (32, 4),
        (36, p1.len() as u32),
        (48, 4),
        (52, p2.len() as u32),
    ] {
        le(&mut b, p, n);
    }
    b.extend(p1);
    b.extend(p2);
    check("yuris/ycg", "x.png", &b, PIX);
}
#[test]
fn epf_lzss_bgra() {
    let c = [10, 20, 30, 40, 50, 60, 70, 80];
    let packed = literals(&c);
    let mut b = b"yga\0".to_vec();
    for n in [2, 1, 1, 8, packed.len() as u32] {
        b.extend(n.to_le_bytes());
    }
    b.extend(packed);
    check("yumemiru/epf", "x.epf", &b, PIX);
}
#[test]
fn bmz_zlib_bmp() {
    let c = bmp();
    let mut b = b"ZLC3".to_vec();
    b.extend((c.len() as u32).to_le_bytes());
    b.extend(zlib(&c));
    check("gsd/bmz", "x.bmz", &b, PIX);
}
fn crc(b: &[u8]) -> u32 {
    let mut c = !0u32;
    for v in b {
        c ^= u32::from(*v);
        for _ in 0..8 {
            c = (c >> 1) ^ if c & 1 != 0 { 0xedb88320 } else { 0 };
        }
    }
    !c
}
#[test]
fn kiss_palette_uses_red_index_and_preserves_alpha() {
    let p = png();
    let mut pal = vec![0; 1024];
    pal[30 * 4..30 * 4 + 4].copy_from_slice(&[3, 2, 1, 0]);
    pal[70 * 4..70 * 4 + 4].copy_from_slice(&[9, 8, 7, 0]);
    let mut b = p[..33].to_vec();
    b.extend(1024u32.to_be_bytes());
    let mut chunk = b"xPAL".to_vec();
    chunk.extend(pal);
    b.extend(&chunk);
    b.extend(crc(&chunk).to_be_bytes());
    b.extend(&p[33..]);
    check("kiss/custom-png", "x.png", &b, &[1, 2, 3, 40, 7, 8, 9, 80]);
    assert!(!probe("kiss/custom-png", "x.png", &p));
}
#[test]
fn png_standard_alpha() {
    check("png/png", "x.png", &png(), PIX);
}
#[test]
fn bmp_preserves_partial_alpha_but_normalizes_all_zero() {
    check("microsoft/bmp", "x.bmp", &bmp(), PIX);
    let mut b = bmp();
    b[57] = 0;
    b[61] = 0;
    check(
        "microsoft/bmp",
        "x.bmp",
        &b,
        &[30, 20, 10, 255, 70, 60, 50, 255],
    );
}
fn dds(four: &[u8; 4]) -> Vec<u8> {
    let mut b = vec![0; 128];
    b[..4].copy_from_slice(b"DDS ");
    for (p, n) in [(4, 124), (12, 1), (16, 2), (76, 32), (80, 4)] {
        le(&mut b, p, n);
    }
    b[84..88].copy_from_slice(four);
    b
}
#[test]
fn dds_dxt1_pad_dimensions_and_565_truncation() {
    let mut b = dds(b"DXT1");
    b.extend([0, 248, 0, 0, 0, 0, 0, 0]);
    let p = run("microsoft/dds", "x.dds", &b);
    assert_eq!((p.width, p.height), (4, 4));
    assert_eq!(p.rgba, [248, 0, 0, 255].repeat(16));
}
#[test]
fn dds_dxt3_original_alpha_nibble_order() {
    let mut b = dds(b"DXT3");
    b.extend([0x12; 8]);
    b.extend([0, 248, 0, 0, 0, 0, 0, 0]);
    let p = run("microsoft/dds", "x.dds", &b);
    assert_eq!(&p.rgba[..8], &[248, 0, 0, 16, 248, 0, 0, 32]);
}
#[test]
fn tga_rle_flips_and_5551() {
    let mut b = vec![0; 18];
    b[2] = 10;
    b[12] = 2;
    b[14] = 1;
    b[16] = 16;
    b[17] = 0x30;
    b.extend([1, 0, 124, 31, 128]);
    check(
        "truevision/tga",
        "x.tga",
        &b,
        &[0, 0, 248, 255, 248, 0, 0, 0],
    );
}
#[test]
fn truncated_unknown_and_oversized_fail() {
    let r = DecoderRegistry::default();
    for id in FORMATS {
        let ctx = ImageDecodeContext::new("x.tga", &r);
        for n in 0..32 {
            assert!(decode(id, &vec![0; n], &ctx).is_err(), "{id}/{n}");
        }
    }
    let mut b = bmp();
    le(&mut b, 18, u32::MAX);
    assert!(decode("microsoft/bmp", &b, &ImageDecodeContext::new("x.bmp", &r)).is_err());
    let mut t = vec![0; 18];
    t[2] = 10;
    t[12] = 1;
    t[14] = 1;
    t[16] = 32;
    t.extend([0xff, 0, 0, 0, 0]);
    assert!(decode("truevision/tga", &t, &ImageDecodeContext::new("x.tga", &r)).is_err());
}
#[test]
fn masked_bmp_reads_scoped_sibling_and_replaces_alpha() {
    let registry = DecoderRegistry::default();
    let source = |path: &str| -> crysirror_core::Result<Vec<u8>> {
        assert_eq!(path, "sprites/face.bmp");
        Ok(bmp())
    };
    let ctx = ImageDecodeContext::new("sprites/face.alp", &registry).with_source(&source);
    let got = decode("nekopack/masked-bmp", &[9, 199], &ctx).unwrap();
    assert_eq!(got.rgba, [30, 20, 10, 9, 70, 60, 50, 199]);
    let missing = |_: &str| Err(crysirror_core::Error::NotFound);
    let ctx = ImageDecodeContext::new("face.alp", &registry).with_source(&missing);
    assert!(decode("nekopack/masked-bmp", &[9, 199], &ctx).is_err());
}
#[test]
fn jpeg_standard_grayscale() {
    let mut b = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut b, 100)
        .encode(&[128, 128], 2, 1, image::ColorType::L8)
        .unwrap();
    check(
        "jpeg/jpeg",
        "x.jpg",
        &b,
        &[128, 128, 128, 255, 128, 128, 128, 255],
    );
}
#[test]
fn webp_standard_lossless_alpha() {
    use image::ImageEncoder;
    let mut b = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut b)
        .write_image(PIX, 2, 1, image::ColorType::Rgba8)
        .unwrap();
    check("google/webp", "x.webp", &b, PIX);
}

// The pinned upstream corpus stays outside this crate; never copy its assets.
// Run with ARC_UNPACKER_ROOT=/absolute/clean/checkout cargo test -p
// crysirror-formats-arc --test images_b external_upstream_corpus -- --ignored --nocapture.
#[test]
#[ignore = "requires external pinned arc_unpacker image fixtures"]
fn external_upstream_corpus() {
    let root =
        std::path::PathBuf::from(std::env::var_os("ARC_UNPACKER_ROOT").expect("ARC_UNPACKER_ROOT"));
    let revision = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(revision.status.success());
    assert_eq!(
        String::from_utf8(revision.stdout).unwrap().trim(),
        "456834ecf2e5686813802c37efd829310485c57d"
    );
    let cases = [
        (
            "active-soft/ed8",
            "active_soft/files/ed8/EFFECT15.ed8",
            "active_soft/files/ed8/EFFECT15-out.png",
            false,
            false,
        ),
        (
            "active-soft/edt",
            "active_soft/files/edt/nowprit.edt",
            "active_soft/files/edt/nowprit-out.png",
            false,
            false,
        ),
        (
            "eagls/gr",
            "eagls/files/gr/mask17.gr",
            "eagls/files/gr/mask17-zlib-out.bmp",
            false,
            true,
        ),
        (
            "fc01/acd",
            "fc01/files/acd/CSD02_35.ACD",
            "fc01/files/acd/CSD02_35-out.png",
            false,
            false,
        ),
        (
            "french-bread/ex3",
            "french_bread/files/ex3/WIN_HISUI&KOHAKU.EX3",
            "french_bread/files/ex3/WIN_HISUI&KOHAKU-zlib-out.bmp",
            false,
            true,
        ),
        (
            "ism/isg",
            "ism/files/isg/MUSICTOP-zlib.ISG",
            "ism/files/isg/MUSICTOP-out.png",
            true,
            false,
        ),
        (
            "ism/isg",
            "ism/files/isg/3D16_019-zlib.ISG",
            "ism/files/isg/3D16_019-out.png",
            true,
            false,
        ),
        (
            "ivory/prs",
            "ivory/files/prs/BMIK_A16",
            "ivory/files/prs/BMIK_A16-out.png",
            false,
            false,
        ),
        (
            "pajamas/ep",
            "pajamas/files/ep/fin.EPA",
            "pajamas/files/ep/fin-out.png",
            false,
            false,
        ),
        (
            "silky/akb",
            "silky/files/akb/HINT02.AKB",
            "silky/files/akb/HINT02-out.png",
            false,
            false,
        ),
        (
            "silky/akb",
            "silky/files/akb/BREATH.AKB",
            "silky/files/akb/BREATH-out.png",
            false,
            false,
        ),
        (
            "triangle/yb",
            "triangle/files/yb/GA3AXXXX15",
            "triangle/files/yb/GA3AXXXX15-out.png",
            false,
            false,
        ),
        (
            "crowd/cwd",
            "crowd/files/cwd/306-301-zlib.cwd",
            "crowd/files/cwd/306-301-out.png",
            true,
            false,
        ),
        (
            "crowd/cwl",
            "crowd/files/cwl/OP04_P-zlib.cwl",
            "crowd/files/cwl/OP04_P-out.png",
            true,
            false,
        ),
        (
            "crowd/cwp",
            "crowd/files/cwp/plogo.cwp",
            "crowd/files/cwp/plogo-out.png",
            false,
            false,
        ),
        (
            "crowd/zbm",
            "crowd/files/zbm/Takuf_26.zbm",
            "crowd/files/zbm/Takuf_26-out.png",
            false,
            false,
        ),
        (
            "minato-soft/fil",
            "minato_soft/files/fil/Rule07-zlib.fil",
            "minato_soft/files/fil/Rule07-out.png",
            true,
            false,
        ),
        (
            "riddle-soft/cmp",
            "riddle_soft/files/cmp/SLParts.gcp",
            "riddle_soft/files/cmp/SLParts-zlib-out.bmp",
            false,
            true,
        ),
        (
            "rpgmaker/xyz",
            "rpgmaker/files/xyz/shallows-room-a.xyz",
            "rpgmaker/files/xyz/shallows-room-a-out.png",
            false,
            false,
        ),
        (
            "sysadv/pga",
            "sysadv/files/pga/flower.pga",
            "sysadv/files/pga/flower-out.png",
            false,
            false,
        ),
        (
            "west-vision/syg",
            "west_vision/files/syg/loadx-zlib.syg",
            "west_vision/files/syg/loadx-out.png",
            true,
            false,
        ),
        (
            "yuris/ycg",
            "yuris/files/ycg/back_0.png",
            "yuris/files/ycg/back_0-out.png",
            false,
            false,
        ),
        (
            "yumemiru/epf",
            "yumemiru/files/epf/IF_cg_bg_minyusyu_c.epf",
            "yumemiru/files/epf/IF_cg_bg_minyusyu_c-out.png",
            false,
            false,
        ),
        (
            "kiss/custom-png",
            "kiss/files/custom-png/x04n_a_.png",
            "kiss/files/custom-png/x04n_a_-out.png",
            false,
            false,
        ),
        (
            "google/webp",
            "google/files/webp/00.webp",
            "google/files/webp/00-out.png",
            false,
            false,
        ),
        (
            "jpeg/jpeg",
            "jpeg/files/NoName.jpeg",
            "jpeg/files/NoName-out.png",
            false,
            false,
        ),
    ];
    fn load(p: &std::path::Path, compressed: bool) -> Vec<u8> {
        let b = std::fs::read(p).unwrap();
        if !compressed {
            return b;
        }
        let mut v = Vec::new();
        flate2::read::ZlibDecoder::new(&b[..])
            .read_to_end(&mut v)
            .unwrap();
        v
    }
    let base = root.join("tests/dec");
    let mut count = 0;
    let mut compare = |id: &str, input: &str, expected: &str, zi: bool, zo: bool| {
        let b = load(&base.join(input), zi);
        let actual = run(id, input, &b);
        let expected = crysirror_image::decode_standard(&load(&base.join(expected), zo)).unwrap();
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height),
            "{input}"
        );
        let diffs = actual
            .rgba
            .iter()
            .zip(&expected.rgba)
            .filter(|(a, b)| a != b)
            .count();
        let maxdiff = actual
            .rgba
            .iter()
            .zip(&expected.rgba)
            .map(|(&a, &b)| a.abs_diff(b))
            .max()
            .unwrap_or(0);
        assert_eq!(actual.rgba.len(), expected.rgba.len());
        if id == "google/webp" || id == "jpeg/jpeg" {
            // The shared pure Rust lossy WebP codec uses different YUV rounding
            // than the pinned libwebp-generated golden. Lossless is exact above.
            assert!(
                maxdiff <= if id == "google/webp" { 8 } else { 1 },
                "{input}: lossy codec difference {maxdiff}"
            );
            println!("lossy codec RGBA {id} {input}: differing_channels={diffs} max_difference={maxdiff}");
        } else {
            assert_eq!(
                diffs, 0,
                "{input}: differing channels {diffs}, max {maxdiff}"
            );
        }
        count += 1;
        if diffs == 0 {
            println!("exact RGBA {id} {input}");
        }
    };
    for (id, i, o, zi, zo) in cases {
        compare(id, i, o, zi, zo);
    }
    for (input, out) in [
        ("pal1bg", "pal1bg"),
        ("pal2", "pal2"),
        ("pal4", "pal4"),
        ("pal8", "pal8"),
        ("pal8-0", "pal8"),
        ("pal8v4", "pal8"),
        ("pal8v5", "pal8"),
        ("rgb16", "rgb16"),
        ("rgb16-565", "rgb16-565"),
        ("rgb16-231", "rgb16-231"),
        ("rgba16-4444", "rgba16-4444"),
        ("rgb24", "rgb24"),
        ("rgb24pal", "rgb24"),
        ("rgb32", "rgb32"),
        ("rgba32", "rgba32"),
        ("rgb32fakealpha", "rgb32fakealpha"),
        ("pal8w126", "pal8w126"),
        ("pal8w125", "pal8w125"),
        ("pal8w124", "pal8w124"),
        ("pal8topdown", "pal8"),
    ] {
        compare(
            "microsoft/bmp",
            &format!("microsoft/files/bmp/{input}.bmp"),
            &format!("microsoft/files/bmp/{out}-out.png"),
            false,
            false,
        );
    }
    for f in ["back0", "006_disconnect", "reimu1", "koishi_7"] {
        compare(
            "microsoft/dds",
            &format!("microsoft/files/dds/{f}.dds"),
            &format!("microsoft/files/dds/{f}-out.png"),
            false,
            false,
        );
    }
    for f in [
        "compressed-8-bit",
        "compressed-16-bit",
        "compressed-24-bit",
        "compressed-32-bit",
        "uncompressed-8-bit-palette",
        "uncompressed-16-bit",
        "uncompressed-24-bit",
        "uncompressed-32-bit",
        "uncompressed-16-bit-flipped",
    ] {
        compare(
            "truevision/tga",
            &format!("truevision/files/tga/{f}.tga"),
            &format!("truevision/files/tga/{f}-out.png"),
            false,
            false,
        );
    }
    println!("external fixture comparisons count={count}");
}
fn lsb_bits(values: &[(u32, usize)]) -> Vec<u8> {
    let count: usize = values.iter().map(|x| x.1).sum();
    let mut out = vec![0; count.div_ceil(8)];
    let mut p = 0;
    for &(v, n) in values {
        for i in (0..n).rev() {
            out[p / 8] |= (((v >> i) & 1) as u8) << (p % 8);
            p += 1;
        }
    }
    out
}
#[test]
fn ed8_custom_bit_order_and_palette() {
    let data = lsb_bits(&[(1, 8), (1, 1), (0, 8)]);
    let mut b = vec![0; 26];
    b[..10].copy_from_slice(b".8Bit\x8d]\x8c\xcb\0");
    b[14] = 2;
    b[16] = 1;
    le(&mut b, 18, 2);
    le(&mut b, 22, data.len() as u32);
    b.extend([10, 20, 30, 50, 60, 70]);
    b.extend(data);
    check(
        "active-soft/ed8",
        "x.ed8",
        &b,
        &[70, 60, 50, 255, 30, 20, 10, 255],
    );
}
#[test]
fn edt_literal_raw_and_diff_transparency() {
    let mut b = vec![0; 34];
    b[..10].copy_from_slice(b".TRUE\x8d]\x8c\xcb\0");
    b[14] = 2;
    b[16] = 1;
    let meta = b".EDT_DIFF\0\x0a\x14\x1e\0base";
    le(&mut b, 22, meta.len() as u32);
    le(&mut b, 26, 1);
    le(&mut b, 30, 6);
    b.extend(meta);
    b.push(0);
    b.extend([10, 20, 30, 50, 60, 70]);
    check(
        "active-soft/edt",
        "x.edt",
        &b,
        &[30, 20, 10, 0, 70, 60, 50, 255],
    );
}
#[test]
fn eagls_fixed_native_key_schedule() {
    let mut plain = bmp();
    plain.resize(70, 0);
    let mut packed = literals(&plain);
    let seed = 7u8;
    let mut state = i64::from(u32::from(seed) ^ 0x75bd924);
    for c in packed.iter_mut().take(0x174b) {
        let hi = state / 44488;
        let lo = state % 44488;
        state = 48271 * lo - 3399 * hi;
        if state < 0 {
            state += 2147483647;
        }
        let index = (state as f64 * 4.656612875245797e-10 * 256.) as usize;
        *c ^= b"EAGLS_SYSTEM"[index % 12];
    }
    packed.push(seed);
    check("eagls/gr", "x.gr", &packed, PIX);
}
#[test]
fn fc01_acd_black_and_white() {
    let data = literals(&[0x60]);
    let mut b = b"ACD 1.00".to_vec();
    for n in [28, data.len() as u32, 1, 2, 1] {
        b.extend(n.to_le_bytes());
    }
    b.extend(data);
    check("fc01/acd", "x.acd", &b, &[0, 0, 0, 255, 255, 255, 255, 255]);
}
fn keyed_mcg(key: u8) -> Vec<u8> {
    let plain = [10, 20, 30, 50, 60, 70, 0, 0];
    let mut payload = literals(&plain);
    let mut k = key;
    let len = payload.len();
    for (i, v) in payload.iter_mut().enumerate() {
        *v = (*v ^ k).rotate_right(1);
        k = k.wrapping_add((len - 1 - i) as u8);
    }
    let mut b = vec![0; 64];
    b[..8].copy_from_slice(b"MCG 1.01");
    for (p, n) in [(16, 64), (28, 2), (32, 1), (36, 24), (40, 8)] {
        le(&mut b, p, n);
    }
    b.extend(payload);
    b
}
#[test]
fn fc01_mcg_explicit_client_key_and_padded_stride() {
    use std::sync::Arc;
    let b = keyed_mcg(23);
    let registry = DecoderRegistry::default();
    let ctx = ImageDecodeContext::new("x.mcg", &registry);
    assert!(decode("fc01/mcg", &b, &ctx).is_err());
    let keys = Arc::new(crysirror_resource::MemoryKeyProvider::default());
    keys.insert(crysirror_core::state::ClientId(7), "fc01/mcg", vec![23])
        .unwrap();
    let mut resource = crysirror_resource::DecodeContext::new(crysirror_core::state::ClientId(7));
    resource.keys = keys.clone();
    let ctx = ctx.with_resources(&resource);
    assert_eq!(
        decode("fc01/mcg", &b, &ctx).unwrap().rgba,
        [30, 20, 10, 255, 70, 60, 50, 255]
    );
    resource.client = crysirror_core::state::ClientId(8);
    assert!(decode(
        "fc01/mcg",
        &b,
        &ImageDecodeContext::new("x.mcg", &registry).with_resources(&resource)
    )
    .is_err());
}
#[test]
fn ex3_identity_dictionary() {
    let c = bmp();
    let mut b = b"LLIF".to_vec();
    b.extend([0; 64]);
    b.extend([255, 128, 254]);
    b.extend((c.len() as u16).to_be_bytes());
    b.extend(c);
    check("french-bread/ex3", "x.ex3", &b, PIX);
}
#[test]
fn isg_rle_and_lzss_variants() {
    for version in [0x10, 0x21] {
        let mut b = vec![0; 48];
        b[..14].copy_from_slice(b"ISM IMAGEFILE\0");
        b[16] = version;
        b[29] = 2;
        b[31] = 1;
        b[35] = 2;
        b.extend([10, 20, 30, 0, 50, 60, 70, 0]);
        b.extend([0, 0, 1]);
        check("ism/isg", "x.isg", &b, &[30, 20, 10, 255, 70, 60, 50, 255]);
    }
}
#[test]
fn ivory_prs_delta_channel() {
    let mut b = b"YB\x01\x03".to_vec();
    b.extend(7u32.to_le_bytes());
    b.extend([0; 4]);
    b.extend([2, 0, 1, 0]);
    b.extend([0, 10, 20, 30, 40, 40, 40]);
    check(
        "ivory/prs",
        "x.prs",
        &b,
        &[30, 20, 10, 255, 70, 60, 50, 255],
    );
}
#[test]
fn mebius_mcg_all_types_and_canvas_offset() {
    for kind in 0..8 {
        let mut b = b"MCG".to_vec();
        b.push(kind);
        for n in [2u16, 1, 0, 0, 2, 1] {
            b.extend(n.to_be_bytes());
        }
        let data = match kind {
            0 | 2 => vec![10, 50, 20, 60, 30, 70],
            1 => vec![10, 20, 30, 40, 50, 60, 70, 80],
            3 => vec![10, 50, 20, 60, 30, 70, 40, 80],
            4 | 5 => vec![17, 213],
            _ => vec![],
        };
        if [2, 3, 5].contains(&kind) {
            b.push(255);
        }
        b.extend(data);
        let expected = match kind {
            0 | 2 => vec![30, 20, 10, 255, 70, 60, 50, 255],
            1 | 3 => PIX.to_vec(),
            4 | 5 => vec![17, 17, 17, 255, 213, 213, 213, 255],
            _ => vec![0, 0, 0, 255, 0, 0, 0, 255],
        };
        check("mebius/mcg", "x.mcg", &b, &expected);
    }
    let mut b = b"MCG\x04".to_vec();
    for n in [2u16, 2, 1, 1, 1, 1] {
        b.extend(n.to_be_bytes());
    }
    b.push(17);
    let got = run("mebius/mcg", "x.mcg", &b);
    assert_eq!(
        got.rgba,
        [0, 0, 0, 0, 17, 17, 17, 255, 0, 0, 0, 0, 0, 0, 0, 0]
    );
}
#[test]
fn pajamas_ep_planar_literals_and_offset() {
    let mut b = b"EP\0\0".to_vec();
    for n in [1u32, 2, 1] {
        b.extend(n.to_le_bytes());
    }
    b.extend([6, 10, 50, 20, 60, 30, 70]);
    check(
        "pajamas/ep",
        "x.ep",
        &b,
        &[30, 20, 10, 255, 70, 60, 50, 255],
    );
}
#[test]
fn silky_akb_predictive_overlay() {
    let mut b = b"AKB ".to_vec();
    b.extend([2, 0, 1, 0]);
    b.extend([0; 4]);
    b.extend([1, 2, 3, 4]);
    for n in [0u32, 0, 2, 1] {
        b.extend(n.to_le_bytes());
    }
    b.extend(literals(&[10, 20, 30, 40, 40, 40, 40, 40]));
    check("silky/akb", "x.akb", &b, PIX);
}
#[test]
fn triangle_yb_delta_alpha() {
    let mut b = b"YB\x80\x04".to_vec();
    b.extend(9u32.to_le_bytes());
    b.extend(8u32.to_le_bytes());
    b.extend([2, 0, 1, 0]);
    b.extend([0, 10, 20, 30, 40, 40, 40, 40, 40]);
    check("triangle/yb", "x.yb", &b, PIX);
}
#[test]
fn twilight_cv2_24_still_uses_four_bytes_and_stride() {
    let mut b = vec![24];
    for n in [2u32, 1, 3, 0] {
        b.extend(n.to_le_bytes());
    }
    b.extend([10, 20, 30, 40, 50, 60, 70, 80, 1, 2, 3, 4]);
    check("twilight-frontier/pak2-gfx", "x.cv2", &b, PIX);
}
#[test]
fn twilight_tfbm_zlib_stride() {
    let data = zlib(&[10, 20, 30, 40, 50, 60, 70, 80, 1, 2, 3, 4]);
    let mut b = b"TFBM\0\x20".to_vec();
    for n in [2u32, 1, 3, data.len() as u32] {
        b.extend(n.to_le_bytes());
    }
    b.extend(data);
    check("twilight-frontier/tfbm", "x.png", &b, PIX);
}
#[test]
#[ignore = "requires external pinned fixtures and explicit ARC_FC01_KEY_101/200"]
fn external_fc01_keys() {
    use crysirror_core::state::ClientId;
    use std::sync::Arc;
    let root =
        std::path::PathBuf::from(std::env::var_os("ARC_UNPACKER_ROOT").expect("ARC_UNPACKER_ROOT"));
    let keys = Arc::new(crysirror_resource::MemoryKeyProvider::default());
    let mut resources = crysirror_resource::DecodeContext::new(ClientId(7));
    resources.keys = keys.clone();
    let registry = DecoderRegistry::default();
    for (name, key_env) in [
        ("FC01LOGO", "ARC_FC01_KEY_101"),
        ("CLE13_06", "ARC_FC01_KEY_101"),
        ("50", "ARC_FC01_KEY_200"),
    ] {
        let key = std::env::var(key_env)
            .expect("explicit external FC01 test key required")
            .parse::<u8>()
            .unwrap();
        keys.insert(ClientId(7), "fc01/mcg", vec![key]).unwrap();
        let b = std::fs::read(root.join(format!("tests/dec/fc01/files/mcg/{name}.MCG"))).unwrap();
        let ctx = ImageDecodeContext::new("x.mcg", &registry).with_resources(&resources);
        let actual = decode("fc01/mcg", &b, &ctx).unwrap();
        let expected = crysirror_image::decode_standard(
            &std::fs::read(root.join(format!("tests/dec/fc01/files/mcg/{name}-out.png"))).unwrap(),
        )
        .unwrap();
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height)
        );
        let diffs = actual
            .rgba
            .iter()
            .zip(&expected.rgba)
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(diffs, 0, "{name}: {diffs} channels");
        println!("exact RGBA fc01/mcg {name}");
    }
}
#[test]
#[ignore = "requires external pinned image/dependency fixtures"]
fn external_dependency_images() {
    let root =
        std::path::PathBuf::from(std::env::var_os("ARC_UNPACKER_ROOT").expect("ARC_UNPACKER_ROOT"))
            .join("tests/dec");
    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();
    let mut count = 0;
    for (id, input, expected, palette) in [
        (
            "twilight-frontier/pak2-gfx",
            "twilight_frontier/files/pak2/0000_00.cv2",
            "twilight_frontier/files/pak2/0000_00-out.png",
            false,
        ),
        (
            "twilight-frontier/pak2-gfx",
            "twilight_frontier/files/pak2/stand000.cv2",
            "twilight_frontier/files/pak2/stand000-out.png",
            false,
        ),
        (
            "twilight-frontier/pak2-gfx",
            "twilight_frontier/files/pak2/stand000.cv2",
            "twilight_frontier/files/pak2/stand000-out2.png",
            true,
        ),
        (
            "twilight-frontier/tfbm",
            "twilight_frontier/files/tfbm/climaxCutA0000.png",
            "twilight_frontier/files/tfbm/climaxCutA0000-out.png",
            false,
        ),
        (
            "twilight-frontier/tfbm",
            "twilight_frontier/files/tfbm/unk-02479-4461dee8.dat",
            "twilight_frontier/files/tfbm/unk-02479-4461dee8-out.png",
            false,
        ),
        (
            "twilight-frontier/tfbm",
            "twilight_frontier/files/tfbm/spellB0000.bmp",
            "twilight_frontier/files/tfbm/spellB0000-out.png",
            false,
        ),
        (
            "twilight-frontier/tfbm",
            "twilight_frontier/files/tfbm/spellB0000.bmp",
            "twilight_frontier/files/tfbm/spellB0000-out2.png",
            true,
        ),
    ] {
        let source = |path: &str| -> crysirror_core::Result<Vec<u8>> {
            if !palette {
                return Err(crysirror_core::Error::NotFound);
            }
            std::fs::read(root.join(path)).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    crysirror_core::Error::NotFound
                } else {
                    crysirror_core::Error::Invalid("fixture I/O failure")
                }
            })
        };
        let ctx = ImageDecodeContext::new(input, &registry).with_source(&source);
        let b = std::fs::read(root.join(input)).unwrap();
        let actual = decode(id, &b, &ctx).unwrap();
        let expected =
            crysirror_image::decode_standard(&std::fs::read(root.join(expected)).unwrap()).unwrap();
        assert_eq!(
            (actual.width, actual.height),
            (expected.width, expected.height)
        );
        let diffs = actual
            .rgba
            .iter()
            .zip(&expected.rgba)
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(diffs, 0, "{input}/palette={palette}: {diffs} channels");
        count += 1;
        println!("exact RGBA {id} {input} palette={palette}");
    }
    let input = "silky/files/akb/homura-overlay-zlib.akb";
    let mut b = Vec::new();
    flate2::read::ZlibDecoder::new(&std::fs::read(root.join(input)).unwrap()[..])
        .read_to_end(&mut b)
        .unwrap();
    let source = |path: &str| -> crysirror_core::Result<Vec<u8>> {
        assert!(path.ends_with("homura-base.bmp") || path.ends_with("homura-base.akb"));
        let data = std::fs::read(root.join("silky/files/akb/homura-base-zlib.akb")).unwrap();
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(&data[..])
            .read_to_end(&mut out)
            .unwrap();
        Ok(out)
    };
    let ctx = ImageDecodeContext::new(input, &registry).with_source(&source);
    let actual = decode("silky/akb", &b, &ctx).unwrap();
    let expected =
        crysirror_image::decode_standard(&std::fs::read(root.join("homura.png")).unwrap()).unwrap();
    assert_eq!(actual, expected);
    count += 1;
    println!("external dependency exact RGBA count={count}");
}
#[test]
fn fc01_documented_zero_tail_but_truncated_token_fails() {
    let data = literals(&[0x60]);
    let mut b = b"ACD 1.00".to_vec();
    for n in [28, data.len() as u32, 10, 2, 1] {
        b.extend(n.to_le_bytes());
    }
    b.extend(data);
    check("fc01/acd", "x.acd", &b, &[0, 0, 0, 255, 255, 255, 255, 255]);
    b.truncate(29);
    le(&mut b, 12, 1);
    let r = DecoderRegistry::default();
    assert!(decode("fc01/acd", &b, &ImageDecodeContext::new("x.acd", &r)).is_err());
}
#[test]
fn zlib_checksum_and_declared_lengths_fail_closed() {
    let c = bmp();
    let mut b = b"ZLC3".to_vec();
    b.extend((c.len() as u32).to_le_bytes());
    b.extend(zlib(&c));
    let registry = DecoderRegistry::default();
    let ctx = ImageDecodeContext::new("x.bmz", &registry);
    for n in 1..=4 {
        assert!(decode("gsd/bmz", &b[..b.len() - n], &ctx).is_err());
    }
    let last = b.len() - 1;
    b[last] ^= 1;
    assert!(decode("gsd/bmz", &b, &ctx).is_err());
    let mut b = bmp();
    le(&mut b, 30, 1);
    assert!(decode(
        "microsoft/bmp",
        &b,
        &ImageDecodeContext::new("x.bmp", &registry)
    )
    .is_err());
}
#[test]
fn twilight_missing_palette_uses_native_gray_and_errors_propagate() {
    let mut b = vec![8];
    for n in [2u32, 1, 2, 0] {
        b.extend(n.to_le_bytes());
    }
    b.extend([0, 255]);
    let registry = DecoderRegistry::default();
    let missing = |_: &str| Err(crysirror_core::Error::NotFound);
    let ctx = ImageDecodeContext::new("sprites/x.cv2", &registry).with_source(&missing);
    let image = decode("twilight-frontier/pak2-gfx", &b, &ctx).unwrap();
    assert_eq!(image.rgba, [0, 0, 0, 255, 254, 254, 254, 255]);
    let denied = |_: &str| Err(crysirror_core::Error::Invalid("denied dependency"));
    let ctx = ImageDecodeContext::new("sprites/x.cv2", &registry).with_source(&denied);
    assert!(decode("twilight-frontier/pak2-gfx", &b, &ctx).is_err());
}
#[test]
fn silky_akb_plus_reads_authored_sibling_and_green_is_transparent() {
    let mut base = b"AKB ".to_vec();
    base.extend([2, 0, 1, 0]);
    base.extend([0; 4]);
    base.extend([0; 4]);
    for n in [0u32, 0, 2, 1] {
        base.extend(n.to_le_bytes());
    }
    base.extend(literals(&[10, 20, 30, 40, 40, 40, 40, 40]));
    let mut b = base[..32].to_vec();
    b[..4].copy_from_slice(b"AKB+");
    let mut name = [0; 32];
    name[..8].copy_from_slice(b"base.bmp");
    b.extend(name);
    b.extend(literals(&[0, 255, 0, 255, 255, 1, 0, 129]));
    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();
    let source = |path: &str| {
        assert_eq!(path, "sprites/base.bmp");
        Ok(base.clone())
    };
    let ctx = ImageDecodeContext::new("sprites/overlay.akb", &registry).with_source(&source);
    let got = decode("silky/akb", &b, &ctx).unwrap();
    assert_eq!(got.rgba, [30, 20, 10, 40, 0, 0, 255, 128]);
}
#[test]
fn dds_multisurface_and_unknown_fourcc_fail() {
    let registry = DecoderRegistry::default();
    let ctx = ImageDecodeContext::new("x.dds", &registry);
    let mut b = dds(b"DX10");
    b.resize(148, 0);
    assert!(decode("microsoft/dds", &b, &ctx).is_err());
    let mut b = dds(b"DXT1");
    le(&mut b, 112, 0x200);
    b.extend([0; 8]);
    assert!(decode("microsoft/dds", &b, &ctx).is_err());
}
fn png_chunk_bytes(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend(name);
    out.extend(data);
    out.extend(crc(&out[4..]).to_be_bytes());
    out
}
fn png_with_chunks(header: &[u8], middle: &[(&[u8; 4], Vec<u8>)], scanlines: &[u8]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(png_chunk_bytes(b"IHDR", header));
    for (name, data) in middle {
        out.extend(png_chunk_bytes(name, data));
    }
    out.extend(png_chunk_bytes(b"IDAT", &zlib(scanlines)));
    out.extend(png_chunk_bytes(b"IEND", &[]));
    out
}
#[test]
fn png_packed_palette_and_trns() {
    let mut header = vec![];
    header.extend(2u32.to_be_bytes());
    header.extend(1u32.to_be_bytes());
    header.extend([2, 3, 0, 0, 0]);
    let p = png_with_chunks(
        &header,
        &[
            (b"PLTE", vec![30, 20, 10, 70, 60, 50]),
            (b"tRNS", vec![40, 80]),
        ],
        &[0, 0x10],
    );
    check("png/png", "x.png", &p, PIX);
}
#[test]
fn png16_strips_low_bytes_instead_of_rounding() {
    let mut header = vec![];
    header.extend(2u32.to_be_bytes());
    header.extend(1u32.to_be_bytes());
    header.extend([16, 6, 0, 0, 0]);
    let mut scan = vec![0];
    for value in [
        0x00ffu16, 0xff00, 0x0180, 0xab7f, 0xff80, 0x7fff, 0x80ff, 0x0100,
    ] {
        scan.extend(value.to_be_bytes());
    }
    let p = png_with_chunks(&header, &[], &scan);
    check("png/png", "x.png", &p, &[0, 255, 1, 171, 255, 127, 128, 1]);
    let mut mgf = p;
    mgf[..8].copy_from_slice(b"MalieGF\0");
    check(
        "malie/mgf",
        "x.mgf",
        &mgf,
        &[0, 255, 1, 171, 255, 127, 128, 1],
    );
}
