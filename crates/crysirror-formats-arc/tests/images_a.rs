// SPDX-License-Identifier: GPL-3.0-or-later
use crysirror_formats_arc::images_a;
use crysirror_image::{DecoderRegistry, ImageDecodeContext};
fn decode(id: &str, b: &[u8]) -> anyhow::Result<crysirror_image::ImageSurface> {
    let r = DecoderRegistry::default();
    images_a::decode(id, b, &ImageDecodeContext::new("fixture.bin", &r))
}
#[test]
fn raw_pixels_flip_and_offset() {
    let mut b = b"AP".to_vec();
    b.extend(1u32.to_le_bytes());
    b.extend(2u32.to_le_bytes());
    b.extend([0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
    let x = decode("kaguya/ap", &b).unwrap();
    assert_eq!(x.rgba, [7, 6, 5, 8, 3, 2, 1, 4]);
    let mut m = Vec::new();
    m.extend(2u32.to_le_bytes());
    m.extend(1u32.to_le_bytes());
    m.extend([12, 34]);
    assert_eq!(
        decode("kaguya/raw-mask", &m).unwrap().rgba,
        [12, 12, 12, 255, 34, 34, 34, 255]
    );
}
#[test]
fn spb_single_pixel_and_truncation() {
    assert_eq!(
        decode("nscripter/spb", &[0, 1, 0, 1, 3, 2, 1])
            .unwrap()
            .rgba,
        [1, 2, 3, 255]
    );
    assert!(decode("nscripter/spb", &[0, 2, 0, 2, 3]).is_err());
}
#[test]
fn pdt10_literal_pixel_and_mask() {
    let mut b = b"PDT10\0\0\0".to_vec();
    b.extend(38u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend([0; 8]);
    b.extend(36u32.to_le_bytes());
    b.extend([128, 3, 2, 1, 128, 77]);
    assert_eq!(decode("real-live/pdt10", &b).unwrap().rgba, [1, 2, 3, 77]);
}
#[test]
fn g00_zero_literal_pixel() {
    let mut b = vec![0, 1, 0, 1, 0];
    b.extend(12u32.to_le_bytes());
    b.extend(3u32.to_le_bytes());
    b.extend([1, 3, 2, 1]);
    assert_eq!(decode("real-live/g00", &b).unwrap().rgba, [1, 2, 3, 255]);
}
#[test]
fn tlg5_literal_planes() {
    let mut b = b"TLG5.0\0raw\x1a".to_vec();
    b.push(3);
    b.extend(1u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    for v in [3u8, 2, 1] {
        b.push(1);
        b.extend(1u32.to_le_bytes());
        b.push(v);
    }
    assert_eq!(decode("kirikiri/tlg", &b).unwrap().rgba, [3, 2, 5, 255]);
}
#[test]
fn malformed_and_unknown_are_errors() {
    for id in images_a::FORMATS {
        assert!(decode(id, &[]).is_err(), "{id}");
    }
    assert!(decode("unknown", &[0; 32]).is_err());
}
/// Public upstream binary fixtures stay outside the repository. No game data or fonts copied.
#[test]
#[ignore = "requires complete external pinned arc_unpacker checkout"]
fn upstream_images_a_pixels() {
    let root = std::env::var("ARC_UNPACKER_SOURCE").expect("ARC_UNPACKER_SOURCE");
    let root = std::path::Path::new(&root);
    let head = std::process::Command::new("git")
        .args(["-C", root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(head.status.success());
    assert_eq!(
        String::from_utf8(head.stdout).unwrap().trim(),
        "456834ecf2e5686813802c37efd829310485c57d"
    );
    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();
    let cases = [
        ("entis/eri", "entis", "eri"),
        ("kid/prt", "kid", "prt"),
        ("lizsoft/sotes", "lizsoft", ""),
        ("purple-software/pb3", "purple_software", "pb3"),
        ("alice-soft/qnt", "alice_soft", "qnt"),
        ("alice-soft/pms", "alice_soft", "pms"),
        ("alice-soft/vsp", "alice_soft", "vsp"),
        ("alice-soft/ajp", "alice_soft", "ajp"),
        ("alice-soft/dcf", "alice_soft", "dcf"),
        ("real-live/pdt9", "real_live", "pdt9"),
        ("real-live/pdt10", "real_live", "pdt10"),
        ("real-live/g00", "real_live", "g00"),
        ("nscripter/spb", "nscripter", "spb"),
        ("kirikiri/tlg", "kirikiri", "tlg"),
        ("kaguya/ap", "kaguya", "ap"),
        ("kaguya/ao", "kaguya", "ao"),
        ("kaguya/ap0", "kaguya", "ap0"),
        ("kaguya/ap2", "kaguya", "ap2"),
        ("kaguya/ap3", "kaguya", "ap3"),
        ("kaguya/aps3", "kaguya", "aps3"),
        ("kaguya/raw-mask", "kaguya", "raw_mask"),
    ];
    let mut count = 0;
    let mut errors = Vec::new();
    for (id, family, dir) in cases {
        let path = root.join("tests/dec").join(family).join("files").join(dir);
        if !path.exists() {
            continue;
        }
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir()
                || path
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("png"))
            {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|x| x.to_str()) else {
                continue;
            };
            let expected = path.with_file_name(format!("{stem}-out.png"));
            if !expected.exists() {
                continue;
            }
            let data = std::fs::read(&path).unwrap();
            let p = path.file_name().unwrap().to_str().unwrap();
            let context = ImageDecodeContext::new(p, &registry);
            match images_a::decode(id, &data, &context) {
                Ok(got) => {
                    let want = crysirror_image::decode_standard(&std::fs::read(expected).unwrap())
                        .unwrap();
                    if id == "alice-soft/ajp"
                        && got.width == want.width
                        && got.height == want.height
                        && got
                            .rgba
                            .iter()
                            .zip(&want.rgba)
                            .enumerate()
                            .all(|(i, (a, b))| {
                                if i % 4 == 3 {
                                    a == b
                                } else {
                                    a.abs_diff(*b) <= 3
                                }
                            })
                    {
                        count += 1;
                        eprintln!("PASS {id} {stem} (alpha exact, RGB JPEG tolerance <=3)");
                    } else if got != want {
                        let diff = got.rgba.iter().zip(&want.rgba).position(|(a, b)| a != b);
                        errors.push(format!(
                            "{id} {stem}: mismatch {:?} vs {:?}; first byte {diff:?}; max diff {}",
                            (got.width, got.height),
                            (want.width, want.height),
                            got.rgba
                                .iter()
                                .zip(&want.rgba)
                                .map(|(a, b)| a.abs_diff(*b))
                                .max()
                                .unwrap_or(0)
                        ));
                    } else {
                        count += 1;
                        eprintln!("PASS {id} {stem}");
                    }
                }
                Err(e) => errors.push(format!("{id} {stem}: {e:#}")),
            }
        }
    }
    assert!(errors.is_empty(), "{} passed; {}", count, errors.join("\n"));
    eprintln!("A-group checked fixtures (AJP uses documented JPEG tolerance): {count}");
    assert_eq!(count, 36, "complete pinned corpus fixture count");
}
#[test]
fn kaguya_variant_headers() {
    let mut b = b"AO".to_vec();
    for v in [1u32, 1] {
        b.extend(v.to_le_bytes());
    }
    b.extend([0, 0]);
    for v in [1u32, 1] {
        b.extend(v.to_le_bytes());
    }
    b.extend([3, 2, 1, 7]);
    let out = decode("kaguya/ao", &b).unwrap();
    assert_eq!((out.width, out.height), (2, 2));
    assert_eq!(&out.rgba[12..], [1, 2, 3, 7]);
    let mut b = b"AP-0".to_vec();
    for v in [1u32, 2] {
        b.extend(v.to_le_bytes());
    }
    b.extend([7, 9]);
    assert_eq!(
        decode("kaguya/ap0", &b).unwrap().rgba,
        [9, 9, 9, 255, 7, 7, 7, 255]
    );
    for v in [2, 3] {
        let mut b = format!("AP-{v}").into_bytes();
        for n in [0u32, 0, 1, 1, 24] {
            b.extend(n.to_le_bytes());
        }
        b.extend([3, 2, 1]);
        if v == 2 {
            b.push(7);
        }
        assert_eq!(
            decode(&format!("kaguya/ap{v}"), &b).unwrap().rgba,
            [1, 2, 3, if v == 2 { 7 } else { 255 }]
        );
    }
}
#[test]
fn aps3_raw_and_literal_lz() {
    let mut raw = b"AP".to_vec();
    for n in [1u32, 1] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend([0, 0, 3, 2, 1, 7]);
    for compressed in [false, true] {
        let data = if compressed {
            let mut bits = Vec::new();
            for v in &raw {
                bits.push(1u8);
                for i in (0..8).rev() {
                    bits.push((v >> i) & 1);
                }
            }
            let mut b = vec![0; bits.len().div_ceil(8)];
            for (i, v) in bits.into_iter().enumerate() {
                b[i / 8] |= v << (7 - i % 8);
            }
            b
        } else {
            raw.clone()
        };
        let mut b = b"\x04APS3".to_vec();
        b.extend(0u32.to_le_bytes());
        b.extend(((if compressed { 10 } else { 6 }) + data.len() as u32).to_le_bytes());
        b.extend(u16::from(compressed).to_le_bytes());
        b.extend((data.len() as u32).to_le_bytes());
        if compressed {
            b.extend((raw.len() as u32).to_le_bytes());
        }
        b.extend(data);
        assert_eq!(decode("kaguya/aps3", &b).unwrap().rgba, [1, 2, 3, 7]);
    }
}
#[test]
fn qnt_odd_padding_and_pms_palette() {
    use std::io::Write;
    fn z(b: &[u8]) -> Vec<u8> {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(b).unwrap();
        z.finish().unwrap()
    }
    let colors = z(&[3, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0]);
    let alpha = z(&[77, 0]);
    let mut b = b"QNT\0".to_vec();
    b.extend(2u32.to_le_bytes());
    b.extend([0; 12]);
    for v in [1u32, 1, 24, 0, colors.len() as u32, alpha.len() as u32] {
        b.extend(v.to_le_bytes());
    }
    b.extend([0; 24]);
    b.extend(colors);
    b.extend(alpha);
    assert_eq!(decode("alice-soft/qnt", &b).unwrap().rgba, [1, 2, 3, 77]);
    let mut b = b"PM".to_vec();
    for n in [1u16, 40, 8] {
        b.extend(n.to_le_bytes());
    }
    b.extend([0; 16]);
    for n in [1u32, 1, 40, 41] {
        b.extend(n.to_le_bytes());
    }
    b.push(1);
    b.extend([0; 3]);
    b.extend([1, 2, 3]);
    b.resize(41 + 768, 0);
    assert_eq!(decode("alice-soft/pms", &b).unwrap().rgba, [1, 2, 3, 255]);
}
#[test]
fn tlg6_zero_golomb_pixel() {
    let mut b = b"TLG6.0\0raw\x1a".to_vec();
    b.extend([3, 0, 0, 0]);
    for n in [1u32, 1, 2] {
        b.extend(n.to_le_bytes());
    }
    b.extend(2u32.to_le_bytes());
    b.extend([0, 0]);
    for _ in 0..3 {
        b.extend(2u32.to_le_bytes());
        b.push(2);
    }
    assert_eq!(decode("kirikiri/tlg", &b).unwrap().rgba, [0, 0, 0, 255]);
}
#[test]
fn prt_padding_alpha_and_sotes_indirect_dimensions() {
    let mut b = b"PRT\0".to_vec();
    for n in [0x66u16, 24, 0, 36, 1, 2] {
        b.extend(n.to_le_bytes());
    }
    for n in [1u32, 0, 0, 0, 0] {
        b.extend(n.to_le_bytes());
    }
    b.extend([30, 20, 10, 0, 60, 50, 40, 0, 77, 88]);
    let im = decode("kid/prt", &b).unwrap();
    assert_eq!(im.rgba, [40, 50, 60, 77, 10, 20, 30, 88]);
    let mut b = vec![0u8; 0x458];
    let put = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut b, 0x438, 0x2711);
    put(&mut b, 0x448, 0);
    put(&mut b, 0x450, 0);
    put(&mut b, 0x440, 0);
    put(&mut b, 4, 1);
    put(&mut b, 0x18, 0);
    put(&mut b, 0x420, 1);
    b[0x430..0x432].copy_from_slice(&24u16.to_le_bytes());
    b.extend([3, 2, 1]);
    assert_eq!(decode("lizsoft/sotes", &b).unwrap().rgba, [1, 2, 3, 255]);
}
#[test]
#[ignore = "requires external pinned arc_unpacker PB3 base-image fixtures"]
fn upstream_pb3_reference_is_client_scoped() {
    let root = std::path::PathBuf::from(std::env::var("ARC_UNPACKER_SOURCE").unwrap())
        .join("tests/dec/purple_software/files/pb3");
    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();
    let data = std::fs::read(root.join("fk1-al_a102.pb3")).unwrap();
    for with_base in [false, true] {
        let source = |name: &str| -> crysirror_core::Result<Vec<u8>> {
            if with_base && name == "fk1-al_a101.png" {
                Ok(std::fs::read(root.join(name)).unwrap())
            } else {
                Err(crysirror_core::Error::NotFound)
            }
        };
        let context = ImageDecodeContext::new("fk1-al_a102.pb3", &registry).with_source(&source);
        let got = images_a::decode("purple-software/pb3", &data, &context).unwrap();
        let want = crysirror_image::decode_standard(
            &std::fs::read(root.join(if with_base {
                "fk1-al_a102-out1.png"
            } else {
                "fk1-al_a102-out2.png"
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(got == want, "PB3 v6 reference={with_base}");
    }
}
#[test]
fn eri_synthetic_gamma_gray_and_unknown_architecture() {
    fn section(name: &[u8; 8], data: &[u8]) -> Vec<u8> {
        let mut b = name.to_vec();
        b.extend((data.len() as u64).to_le_bytes());
        b.extend(data);
        b
    }
    let mut info = Vec::new();
    for v in [0x20100u32, 0x03020000, u32::MAX, 2, 1, u32::MAX, 8] {
        info.extend(v.to_le_bytes());
    }
    info.extend([0; 24]);
    info.extend(1u32.to_le_bytes());
    info.extend([0; 12]);
    let mut b = b"Entis\x1a\0\0\0\x01\0\x03\0\0\0\0Entis Rasterized Image".to_vec();
    b.resize(0x40, 0);
    b.extend(section(b"Header  ", &section(b"ImageInf", &info)));
    b.extend(section(
        b"Stream  ",
        &section(b"ImageFrm", &[8, 0, 0, 8, 0x28]),
    ));
    assert_eq!(decode("entis/eri", &b).unwrap().rgba, [0, 0, 0, 255]);
    b[0x40 + 16 + 16 + 8..0x40 + 16 + 16 + 12].copy_from_slice(&1u32.to_le_bytes());
    assert!(decode("entis/eri", &b).is_err());
}
#[test]
fn pb3_constant_blocks_and_unknown_variant() {
    let mut b = vec![0u8; 220];
    b[..4].copy_from_slice(b"PB3B");
    b[0x18..0x1c].copy_from_slice(&0x10u32.to_le_bytes());
    b[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    for at in [0x1e, 0x20] {
        b[at..at + 2].copy_from_slice(&1u16.to_le_bytes());
    }
    b[0x22..0x24].copy_from_slice(&32u16.to_le_bytes());
    b[0x2c..0x30].copy_from_slice(&84u32.to_le_bytes());
    b[0x30..0x34].copy_from_slice(&156u32.to_le_bytes());
    for ch in 0..4 {
        b[84 + ch * 4..88 + ch * 4].copy_from_slice(&14u32.to_le_bytes());
        let at = 100 + ch * 14;
        b[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        b[at + 4..at + 8].copy_from_slice(&1u32.to_le_bytes());
        b[at + 12] = 128;
        b[at + 13] = [3, 2, 1, 77][ch];
    }
    assert_eq!(
        decode("purple-software/pb3", &b).unwrap().rgba,
        [1, 2, 3, 77]
    );
    b[0x1c..0x1e].copy_from_slice(&4u16.to_le_bytes());
    assert!(decode("purple-software/pb3", &b).is_err());
}
#[test]
fn malformed_dimensions_do_not_overflow_before_validation() {
    for magic in [b"AP".as_slice(), b"AO", b"AP-0", b"AP-2", b"AP-3"] {
        let id = match magic {
            b"AP" => "kaguya/ap",
            b"AO" => "kaguya/ao",
            b"AP-0" => "kaguya/ap0",
            b"AP-2" => "kaguya/ap2",
            _ => "kaguya/ap3",
        };
        let mut b = magic.to_vec();
        for _ in 0..10 {
            b.extend(u32::MAX.to_le_bytes());
        }
        assert!(decode(id, &b).is_err());
    }
    let b = [255u8; 1024];
    for id in images_a::FORMATS {
        assert!(decode(id, &b).is_err(), "{id}");
    }
}
