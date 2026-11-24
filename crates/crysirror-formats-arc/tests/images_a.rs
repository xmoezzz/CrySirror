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
fn cmvs_msk0_registry_uses_native_data_offset_and_plane_order() {
    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();
    let mut bytes = b"MSK0".to_vec();
    bytes.extend(19u32.to_le_bytes());
    bytes.extend(2u32.to_le_bytes());
    bytes.extend(2u32.to_le_bytes());
    bytes.extend([91, 92, 93, 0, 64, 127, 255]);

    let surface = registry.decode("fixture.bin", &bytes).unwrap();
    assert_eq!((surface.width, surface.height), (2, 2));
    assert_eq!(
        surface.rgba,
        [0, 0, 0, 255, 64, 64, 64, 255, 127, 127, 127, 255, 255, 255, 255, 255]
    );
    assert!(registry.decode("fixture.bin", &bytes[..22]).is_err());
    bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(registry.decode("fixture.bin", &bytes).is_err());
    bytes[4..8].copy_from_slice(&19u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(registry.decode("fixture.bin", &bytes).is_err());
}

fn pb2_fixture(mode: u16, width: u16, height: u16, depth: u16, frames: u32) -> Vec<u8> {
    let mut bytes = vec![0; 32];
    bytes[..4].copy_from_slice(b"PB2A");
    bytes[8..12].copy_from_slice(&frames.to_le_bytes());
    for (field, value) in bytes[16..24]
        .chunks_exact_mut(2)
        .zip([mode, width, height, depth])
    {
        field.copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[test]
fn pb2_numbered_frames_patch_the_base_independently() {
    let mut image = pb2_fixture(5, 16, 8, 32, 3);
    // All four base planes share 128 literal zeroes.
    for _ in 0..4 {
        image.extend(0u32.to_le_bytes());
        image.extend(16u32.to_le_bytes());
    }
    image.extend([0; 16 + 128]);
    let table = image.len() as u32;
    image[12..16].copy_from_slice(&table.to_le_bytes());
    for (mask, bgr) in [(0x40, [1, 2, 3]), (0x80, [4, 5, 6])] {
        let mut delta = vec![1, 0, 0, 0, 0, 0, 0, 0, mask];
        delta.extend(bgr.repeat(64));
        let flags = delta.len().div_ceil(8);
        for field in [16 + flags + delta.len(), flags, delta.len(), delta.len()] {
            image.extend((field as u32).to_le_bytes());
        }
        image.resize(image.len() + flags, 0);
        image.extend(delta);
    }
    let bytes = seal_pb2(image);
    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();
    let decode = |frame: u32| {
        let mut context = crysirror_resource::DecodeContext::new(crysirror_core::state::ClientId(1));
        context.parameters.insert("purple-software/pb2-frame".into(), frame.into());
        registry.decode_with("indexed.pb2", &bytes, None, Some(&context), None)
    };
    let first = decode(1).unwrap();
    let second = decode(2).unwrap();
    for row in 0..8 {
        assert_eq!(first.rgba[row * 64..row * 64 + 32], [3, 2, 1, 255].repeat(8));
        assert_eq!(first.rgba[row * 64 + 32..(row + 1) * 64], [0; 32]);
        assert_eq!(second.rgba[row * 64..row * 64 + 32], [0; 32]);
        assert_eq!(second.rgba[row * 64 + 32..(row + 1) * 64], [6, 5, 4, 255].repeat(8));
    }
    assert!(decode(3).is_err());
}

fn seal_pb2(mut bytes: Vec<u8>) -> Vec<u8> {
    let mask = [0xa2, 0xfb];
    for (index, byte) in bytes[8..32].iter_mut().enumerate() {
        let subtractor = (index as u8).wrapping_mul(39);
        *byte = byte.wrapping_add(subtractor) ^ mask[index % 2];
    }
    bytes.extend((0u8..24).map(|index| index.wrapping_mul(39)));
    bytes.extend([mask[0], mask[1], 0]);
    bytes
}

#[test]
fn cmvs_pb2_tiled_planes_and_lzss() {
    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();

    let mut mode1 = pb2_fixture(1, 9, 2, 24, 0);
    mode1[24..28].copy_from_slice(&32u32.to_le_bytes());
    mode1[28..32].copy_from_slice(&39u32.to_le_bytes());
    mode1.extend([0; 7]);
    for base in [0u8, 20, 40] {
        mode1.extend((0..18).map(|i| base + i));
    }
    let surface = registry.decode("tile.bin", &seal_pb2(mode1)).unwrap();
    let pixel = |x: usize, y: usize| &surface.rgba[(y * 9 + x) * 4..][..4];
    assert_eq!(pixel(0, 0), [40, 20, 0, 255]);
    assert_eq!(pixel(0, 1), [48, 28, 8, 255]);
    assert_eq!(pixel(8, 0), [56, 36, 16, 255]);
    assert_eq!(pixel(8, 1), [57, 37, 17, 255]);

    let mut backref = pb2_fixture(1, 1, 1, 32, 0);
    backref[24..28].copy_from_slice(&32u32.to_le_bytes());
    backref[28..32].copy_from_slice(&33u32.to_le_bytes());
    backref.extend([0x40, 91]);
    backref.extend((2014u16 << 5).to_le_bytes());
    assert_eq!(
        registry
            .decode("tile.bin", &seal_pb2(backref.clone()))
            .unwrap()
            .rgba,
        [91; 4]
    );
    backref[34] |= 1; // Match spans four bytes with only three left.
    assert!(registry.decode("tile.bin", &seal_pb2(backref)).is_err());

    let mut mode2 = pb2_fixture(2, 9, 1, 24, 0);
    mode2[24..28].copy_from_slice(&32u32.to_le_bytes());
    mode2[28..32].copy_from_slice(&89u32.to_le_bytes());
    for size in [15u32; 3] {
        mode2.extend(size.to_le_bytes());
    }
    for color in [2, 4, 6] {
        for size in [1u32, 1, 1] {
            mode2.extend(size.to_le_bytes());
        }
        mode2.extend([0x80, color, 0]);
    }
    for size in [1u32; 3] {
        mode2.extend(size.to_le_bytes());
    }
    mode2.extend([11, 22, 33]);
    let tiled = registry
        .decode("tile.bin", &seal_pb2(mode2.clone()))
        .unwrap();
    assert_eq!(tiled.rgba[..32], [6, 4, 2, 255].repeat(8));
    assert_eq!(tiled.rgba[32..], [33, 22, 11, 255]);
    mode2.pop();
    // The final red literal now comes from the first key byte at EOF.
    let tail = registry.decode("tile.bin", &seal_pb2(mode2.clone())).unwrap();
    assert_eq!(tail.rgba[32..], [0, 22, 11, 255]);
    mode2[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(registry.decode("tile.bin", &seal_pb2(mode2)).is_err());
}

fn neutral_jbp() -> Vec<u8> {
    let mut jbp = b"JBP1".to_vec();
    jbp.extend(36u32.to_le_bytes());
    jbp.extend(0x1800_0000u32.to_le_bytes()); // 16-pixel alignment and explicit quantization tables.
    jbp.extend([0; 4]);
    for word in [16u16, 16, 24, 0, 0, 0] {
        jbp.extend(word.to_le_bytes());
    }
    jbp.extend(1u32.to_le_bytes());
    jbp.extend(1u32.to_le_bytes());
    // Both trees take the zero branch. The first chooses DC bit count zero.
    // The second chooses AC end of block.
    for (first, second) in [(0, 1), (15, 1)] {
        for index in 0..16 {
            let frequency = if index == first {
                1
            } else if index == second {
                2
            } else {
                2_100_000_000u32
            };
            jbp.extend(frequency.to_le_bytes());
        }
    }
    jbp.extend([0; 16]);
    jbp.extend([1; 128]);
    jbp.extend([0, 0]);
    jbp
}

#[test]
fn cmvs_pb2_jbp_and_alpha() {
    let jbp = neutral_jbp();
    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();
    let mut mode3 = pb2_fixture(3, 3, 1, 24, 0);
    mode3.extend(&jbp);
    assert_eq!(
        registry.decode("photo.bin", &seal_pb2(mode3)).unwrap().rgba,
        [128, 128, 128, 255].repeat(3)
    );
    let mut mode4 = pb2_fixture(4, 3, 1, 32, 0);
    mode4[24..28].copy_from_slice(&(32 + jbp.len() as u32).to_le_bytes());
    mode4.extend(&jbp);
    mode4.extend([0, 1, 91, 255, 1]);
    assert_eq!(
        registry
            .decode("photo.bin", &seal_pb2(mode4.clone()))
            .unwrap()
            .rgba,
        [128, 128, 128, 0, 128, 128, 128, 91, 128, 128, 128, 255]
    );
    *mode4.last_mut().unwrap() = 2;
    assert!(registry.decode("photo.bin", &seal_pb2(mode4)).is_err());
}

#[test]
fn cmvs_pb2_first_frame_and_missing_named_variant() {
    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();
    let mut mode5 = pb2_fixture(5, 1, 1, 32, 2);
    for index in 0..4u32 {
        mode5.extend(0u32.to_le_bytes());
        mode5.extend((index + 1).to_le_bytes());
    }
    mode5.extend([0, 1, 2, 4, 8]);
    assert_eq!(
        registry
            .decode("frame.bin", &seal_pb2(mode5.clone()))
            .unwrap()
            .rgba,
        [12, 14, 15, 8]
    );
    mode5[8..12].fill(0);
    assert!(registry.decode("frame.bin", &seal_pb2(mode5)).is_err());

    let error = registry
        .decode("frame.bin", &seal_pb2(pb2_fixture(6, 1, 1, 32, 1)))
        .unwrap_err();
    assert!(format!("{error:#}").contains("requires a native variant name"));
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
fn tlg5_rgba_predictor_crosses_rows_and_blocks() {
    for channels in [3, 4] {
        let mut bytes = b"TLG5.0\0raw\x1a".to_vec();
        bytes.push(channels);
        for value in [2u32, 3, 2, 0, 0] {
            bytes.extend(value.to_le_bytes());
        }
        let planes = [
            [250u8, 2, 3, 4, 5, 6],
            [8, 9, 10, 11, 12, 13],
            [25, 26, 27, 28, 29, 30],
            [128, 129, 1, 2, 3, 4],
        ];
        for block in [0..4, 4..6] {
            for plane in &planes[..channels as usize] {
                bytes.push(1);
                bytes.extend((block.len() as u32).to_le_bytes());
                bytes.extend(&plane[block.clone()]);
            }
        }
        let mut expected = [
            [33, 8, 2, 128], [68, 17, 13, 1],
            [70, 18, 15, 129], [144, 38, 41, 4],
            [111, 30, 32, 132], [228, 63, 77, 11],
        ];
        if channels == 3 {
            for pixel in &mut expected { pixel[3] = 255; }
        }
        assert_eq!(decode("kirikiri/tlg", &bytes).unwrap().rgba, expected.concat());
    }
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
    b.extend(68u32.to_le_bytes());
    b.extend([0; 8]);
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
    b[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        decode("purple-software/pb3", &b).unwrap().rgba,
        [1, 2, 3, 77]
    );
    b[0x18..0x1c].fill(0);
    assert!(decode("purple-software/pb3", &b).is_err());
    b[0x1c..0x1e].copy_from_slice(&4u16.to_le_bytes());
    assert!(decode("purple-software/pb3", &b).is_err());
}

fn pb3_test_header(mode: u16, width: u16, depth: u16, size: usize) -> Vec<u8> {
    let mut bytes = vec![0; size];
    bytes[..4].copy_from_slice(b"PB3B");
    bytes[28..30].copy_from_slice(&mode.to_le_bytes());
    bytes[30..32].copy_from_slice(&width.to_le_bytes());
    bytes[32..34].copy_from_slice(&1u16.to_le_bytes());
    bytes[34..36].copy_from_slice(&depth.to_le_bytes());
    bytes
}

#[test]
fn pb3_type7_decodes_interleaved_bgra() {
    let mut image = pb3_test_header(7, 1, 32, 60);
    image[56..60].copy_from_slice(&1u32.to_le_bytes());
    image.extend([0, 4, 3, 2, 91]);
    let footer = image.len();
    image.resize(footer + 47, 0);
    assert_eq!(
        decode("purple-software/pb3", &image).unwrap().rgba,
        [2, 3, 4, 91]
    );

    image[34..36].copy_from_slice(&24u16.to_le_bytes());
    assert_eq!(
        decode("purple-software/pb3", &image).unwrap().rgba,
        [2, 3, 4, 255]
    );
    image[60] = 0x80;
    image[61..63].copy_from_slice(&2u16.to_le_bytes()); // Five bytes into a four-byte output.
    assert!(decode("purple-software/pb3", &image).is_err());
}

#[test]
fn pb3_type8_requires_its_named_base_and_replaces_only_selected_tiles() {
    let mut image = pb3_test_header(8, 9, 32, 84);
    image[12..16].copy_from_slice(&52u32.to_le_bytes());
    image[24..28].copy_from_slice(&13u32.to_le_bytes());
    image[44..48].copy_from_slice(&2u32.to_le_bytes());
    for (i, key) in [0xa6, 0x75, 0xf3, 0x9c, 0xc5].into_iter().enumerate() {
        image[52 + i] = b"base\0"[i] ^ key;
    }
    image.extend([0, 0]); // LZSS flags for thirteen literals.
    image.extend([1, 0, 0, 0, 0, 0, 0, 0, 0x80, 4, 3, 2, 91]);
    image.resize(image.len() + 47, 0);

    let mut base = pb3_test_header(7, 9, 32, 60);
    base[56..60].copy_from_slice(&5u32.to_le_bytes());
    base.extend([0; 5]);
    base.extend([12, 11, 10, 111].repeat(9));
    base.resize(base.len() + 47, 0);

    let mut registry = DecoderRegistry::default();
    crysirror_formats_arc::register_images(&mut registry).unwrap();
    let source = |path: &str| {
        assert_eq!(path, "sub/base.pb3");
        Ok(base.clone())
    };
    let result = registry
        .decode_with("sub/image.pb3", &image, Some(&source), None, None)
        .unwrap();
    assert_eq!(result.rgba[..32], [10, 11, 12, 111].repeat(8));
    assert_eq!(result.rgba[32..], [2, 3, 4, 91]);
    assert!(registry.decode("sub/image.pb3", &image).is_err());

    image[28..30].copy_from_slice(&6u16.to_le_bytes());
    let mut resources = crysirror_resource::DecodeContext::new(crysirror_core::state::ClientId(1));
    resources
        .parameters
        .insert("purple-software/pb3-native-base".into(), true.into());
    let native = registry
        .decode_with(
            "sub/image.pb3",
            &image,
            Some(&source),
            Some(&resources),
            None,
        )
        .unwrap();
    assert_eq!(native.rgba, result.rgba);
    let missing = |path: &str| {
        assert_eq!(path, "sub/base.pb3");
        Err(crysirror_core::Error::NotFound)
    };
    assert!(registry
        .decode_with(
            "sub/image.pb3",
            &image,
            Some(&missing),
            Some(&resources),
            None
        )
        .is_err());
}

#[test]
fn cmvs_memory_preview_uses_the_native_bmp_and_png_upload_bytes() {
    let context = crysirror_resource::DecodeContext::new(crysirror_core::state::ClientId(1));
    let decode = |bytes: &[u8]| {
        crysirror_formats_arc::cmvs_image::decode(
            bytes,
            &|_| panic!("unrelated image dependency"),
            &context,
        )
    };
    let mut bmp = vec![0; 54];
    bmp[..2].copy_from_slice(b"BM");
    bmp[10..14].copy_from_slice(&99u32.to_le_bytes()); // Memory loader uses the fixed pixel offset.
    bmp[18..22].copy_from_slice(&1u32.to_le_bytes());
    bmp[22..26].copy_from_slice(&2u32.to_le_bytes());
    bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
    bmp.extend([30, 20, 10, 0, 60, 50, 40, 0]);
    let (pixels, alpha) = decode(&bmp).unwrap();
    assert!(!alpha);
    assert_eq!(pixels.rgba, [40, 50, 60, 255, 10, 20, 30, 255]);
    bmp[28..30].copy_from_slice(&32u16.to_le_bytes());
    bmp[57] = 13;
    bmp[61] = 37;
    let (pixels, alpha) = decode(&bmp).unwrap();
    assert!(alpha);
    assert_eq!(pixels.rgba, [40, 50, 60, 37, 10, 20, 30, 13]);
    assert!(decode(&bmp[..61]).is_err());

    let rgba = crysirror_image::ImageSurface::filled(1, 2, [3, 82, 111, 117]).unwrap();
    let png = crysirror_image::encode_png_uncompressed(&rgba).unwrap();
    let (pixels, alpha) = decode(&png).unwrap();
    assert!(alpha);
    assert_eq!(pixels.rgba, [111, 82, 3, 117].repeat(2));

    let mut mask = b"MSK0".to_vec();
    for word in [16u32, 1, 1] {
        mask.extend(word.to_le_bytes());
    }
    mask.push(2);
    assert!(decode(&mask)
        .unwrap_err()
        .to_string()
        .contains("scene upload stride"));
}

#[test]
fn cmvs_memory_preview_uses_pb3_depth_and_decodes_all_four_differential_planes() {
    let context = crysirror_resource::DecodeContext::new(crysirror_core::state::ClientId(1));
    let decode = |bytes: &[u8]| {
        crysirror_formats_arc::cmvs_image::decode(
            bytes,
            &|_| panic!("unrelated image dependency"),
            &context,
        )
    };
    for mode in [2, 3] {
        let mut image = pb3_test_header(mode, 1, 32, 52);
        image.extend(neutral_jbp());
        let alpha_offset = image.len() as u32;
        image[44..48].copy_from_slice(&alpha_offset.to_le_bytes());
        image.extend([65, 0xff]); // Only one alpha byte belongs to this image.
        image.resize(image.len() + 47, 0);
        let (pixels, alpha) = decode(&image).unwrap();
        assert!(alpha);
        assert_eq!(pixels.rgba, [128, 128, 128, 65]);
    }
    let mut image = pb3_test_header(5, 2, 24, 52);
    for index in 0..4u32 {
        image.extend(0u32.to_le_bytes());
        image.extend((1 + index * 2).to_le_bytes());
    }
    image.extend([0, 4, 5, 3, 6, 2, 7, 55, 3]);
    image.resize(image.len() + 47, 0);
    let (pixels, alpha) = decode(&image).unwrap();
    assert!(!alpha);
    assert_eq!(pixels.rgba, [2, 3, 4, 55, 9, 9, 9, 58]);
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
