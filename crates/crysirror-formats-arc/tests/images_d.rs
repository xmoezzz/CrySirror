// SPDX-License-Identifier: GPL-3.0-or-later
use crysirror_formats_arc::{images_d, register_images};
use crysirror_image::{DecoderRegistry, ImageDecodeContext};
use std::io::Read;
fn le(b: &mut [u8], p: usize, n: u32) {
    b[p..p + 4].copy_from_slice(&n.to_le_bytes())
}
fn be(b: &mut [u8], p: usize, n: u32) {
    b[p..p + 4].copy_from_slice(&n.to_be_bytes())
}
fn literals(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in b.chunks(8) {
        out.push(((1u16 << part.len()) - 1) as u8);
        out.extend(part);
    }
    out
}
fn pgx(alpha: bool) -> Vec<u8> {
    let payload = literals(&[3, 2, 1, 40, 7, 6, 5, 80]);
    let mut b = vec![0; 24];
    b[..4].copy_from_slice(b"PGX\0");
    le(&mut b, 8, 2);
    le(&mut b, 12, 1);
    b[16] = u8::from(alpha);
    le(&mut b, 20, payload.len() as u32);
    b.extend(payload);
    b
}
#[test]
fn pgx_opacity_and_jpeg_app3_stream_are_native_pixel_decodes() {
    let mut r = DecoderRegistry::default();
    register_images(&mut r).unwrap();
    for alpha in [true, false] {
        let b = pgx(alpha);
        let p = r.decode("pixel.pgx", &b).unwrap();
        assert_eq!(
            p.rgba,
            if alpha {
                vec![1, 2, 3, 40, 5, 6, 7, 80]
            } else {
                vec![1, 2, 3, 255, 5, 6, 7, 255]
            }
        );
    }
    let mut payload = pgx(true);
    payload.splice(24..24, vec![0; 8]);
    let mut jpg = b"\xff\xd8\xff\xe0\0\x02\xff\xe3".to_vec();
    jpg.extend(((payload.len() + 2) as u16).to_be_bytes());
    jpg.extend(payload);
    jpg.extend(b"\xff\xda");
    assert!(r
        .probe("pixel.jpg", &jpg)
        .unwrap()
        .contains(&"glib/jpeg-pgx"));
    assert_eq!(
        r.decode("pixel.jpg", &jpg).unwrap().rgba,
        [1, 2, 3, 40, 5, 6, 7, 80]
    );
}
#[test]
fn gtf_dxt1_uses_the_native_shared_block_decoder() {
    let mut b = vec![0; 48];
    be(&mut b, 0, 0x01050000);
    be(&mut b, 4, 8);
    be(&mut b, 8, 1);
    be(&mut b, 16, 40);
    b[24] = 0x86;
    b[32..34].copy_from_slice(&4u16.to_be_bytes());
    b[34..36].copy_from_slice(&4u16.to_be_bytes());
    b[36..38].copy_from_slice(&1u16.to_be_bytes());
    b[40..42].copy_from_slice(&0xf800u16.to_le_bytes());
    let r = DecoderRegistry::default();
    let p = images_d::decode(
        "playstation/gtf",
        &b,
        &ImageDecodeContext::new("pixel.gtf", &r),
    )
    .unwrap();
    assert_eq!((p.width, p.height), (4, 4));
    assert_eq!(p.rgba, [248, 0, 0, 255].repeat(16));
}
#[test]
fn gim_rgba_and_indexed_swizzled_alignment_are_checked() {
    let mut b = vec![0; 0x30 + 0x50 + 16 * 8 * 4];
    b[..12].copy_from_slice(b"MIG.00.1PSP\0");
    let n = (b.len() - 0x30) as u32;
    le(&mut b, 0x30, 4);
    le(&mut b, 0x34, n);
    le(&mut b, 0x38, n);
    b[0x44..0x46].copy_from_slice(&3u16.to_le_bytes());
    b[0x48..0x4a].copy_from_slice(&16u16.to_le_bytes());
    b[0x4a..0x4c].copy_from_slice(&8u16.to_le_bytes());
    le(&mut b, 0x5c, 0x40);
    for p in b[0x80..].chunks_exact_mut(4) {
        p.copy_from_slice(&[11, 22, 33, 44]);
    }
    let r = DecoderRegistry::default();
    let p = images_d::decode("playstation/gim", &b, &ImageDecodeContext::new("p.gim", &r)).unwrap();
    assert_eq!(p.rgba, [11, 22, 33, 44].repeat(128));
    b[0x46] = 1;
    let p = images_d::decode("playstation/gim", &b, &ImageDecodeContext::new("p.gim", &r)).unwrap();
    assert_eq!(p.rgba, [11, 22, 33, 44].repeat(128));
    for (format, packed, expected) in [
        (0u16, 0x001fu16, [248, 0, 0, 255]),
        (1, 0x801f, [248, 0, 0, 255]),
        (2, 0xf12f, [240, 32, 16, 240]),
    ] {
        b[0x44..0x46].copy_from_slice(&format.to_le_bytes());
        for pixel in b[0x80..].chunks_exact_mut(2) {
            pixel.copy_from_slice(&packed.to_le_bytes());
        }
        let image =
            images_d::decode("playstation/gim", &b, &ImageDecodeContext::new("p.gim", &r)).unwrap();
        assert_eq!(image.rgba, expected.repeat(128));
    }
    b[0x44] = 6;
    assert!(
        images_d::decode("playstation/gim", &b, &ImageDecodeContext::new("p.gim", &r)).is_err()
    );
}
#[test]
fn xtx_rgba_tiling_and_bgi_cbg_use_registered_image_services() {
    let mut r = DecoderRegistry::default();
    register_images(&mut r).unwrap();
    let mut b = vec![0; 32 + 32 * 32 * 4];
    b[..4].copy_from_slice(b"xtx\0");
    for p in [8, 12, 16, 20] {
        be(&mut b, p, 32);
    }
    for p in b[32..].chunks_exact_mut(4) {
        p.copy_from_slice(&[44, 11, 22, 33]);
    }
    let p = r.decode("p.xtx", &b).unwrap();
    assert_eq!(p.rgba, [11, 22, 33, 44].repeat(1024));
    b[4] = 1;
    b.resize(32 + 64 * 32 * 2, 0);
    be(&mut b, 8, 64);
    for pixel in b[32..].chunks_exact_mut(2) {
        pixel.copy_from_slice(&0xf800u16.to_be_bytes());
    }
    assert_eq!(
        r.decode("p.xtx", &b).unwrap().rgba,
        [248, 0, 0, 255].repeat(1024)
    );
    // A 32x32 16-bit half tile fills the first16 scanlines in the native
    // mapping. Unfilled zero BGR565 pixels still have opaque alpha.
    be(&mut b, 8, 32);
    b.truncate(32 + 32 * 32 * 2);
    let mut expected = [248, 0, 0, 255].repeat(512);
    expected.extend([0, 0, 0, 255].repeat(512));
    assert_eq!(r.decode("p.xtx", &b).unwrap().rgba, expected);

    let mut header = [0; 16];
    header[..2].copy_from_slice(&2u16.to_le_bytes());
    header[2..4].copy_from_slice(&1u16.to_le_bytes());
    header[4..6].copy_from_slice(&32u16.to_le_bytes());
    header[8..10].copy_from_slice(&2u16.to_le_bytes());
    let b = ethornell_image::encode_cbg_v1(&header, &[3, 2, 1, 40, 7, 6, 5, 80], 0x12345678);
    assert_eq!(
        r.decode("p.cbg", &b).unwrap().rgba,
        [1, 2, 3, 40, 5, 6, 7, 80]
    );
}
#[test]
#[ignore = "requires pinned external arc_unpacker fixtures, never distributed"]
fn external_glib_and_console_texture_pixels() {
    let root =
        std::path::PathBuf::from(std::env::var("ARC_UNPACKER_SOURCE").unwrap()).join("tests/dec");
    let r = DecoderRegistry::default();
    let mut count = 0;
    let cases = [
        (
            "glib/pgx",
            "glib/files/pgx/CFG_PAGEBTN.PGX",
            "glib/files/pgx/CFG_PAGEBTN-out.png",
            false,
        ),
        (
            "glib/pgx",
            "glib/files/pgx/BG010A.PGX",
            "glib/files/pgx/BG010A-out.png",
            false,
        ),
        (
            "glib/jpeg-pgx",
            "glib/files/jpeg_pgx/BG110D.PGX.JPG",
            "glib/files/jpeg_pgx/BG110D.PGX-out.png",
            false,
        ),
        (
            "playstation/gim",
            "playstation/files/gim/fl_info.gim",
            "playstation/files/gim/fl_info-out.png",
            false,
        ),
        (
            "playstation/gim",
            "playstation/files/gim/fl_icons.gim",
            "playstation/files/gim/fl_icons-out.png",
            false,
        ),
        (
            "playstation/gim",
            "playstation/files/gim/sg_hint_good.gim",
            "playstation/files/gim/sg_hint_good-out.png",
            false,
        ),
        (
            "playstation/gim",
            "playstation/files/gim/bd_smenu.gim",
            "playstation/files/gim/bd_smenu-out.png",
            false,
        ),
        (
            "playstation/gim",
            "playstation/files/gim/AY_6C_3.gim",
            "playstation/files/gim/AY_6C_3-out.png",
            false,
        ),
        (
            "playstation/gtf",
            "playstation/files/gtf/DATA03-zlib.GTF",
            "playstation/files/gtf/DATA03-out.png",
            true,
        ),
        (
            "cri/xtx",
            "cri/files/xtx/unk_1186-zlib.xtx",
            "cri/files/xtx/unk_1186-out.png",
            true,
        ),
        (
            "cri/xtx",
            "cri/files/xtx/unk_3398-zlib.xtx",
            "cri/files/xtx/unk_3398-out.png",
            true,
        ),
        (
            "cri/xtx",
            "cri/files/xtx/unk_1237-zlib.xtx",
            "cri/files/xtx/unk_1237-out.png",
            true,
        ),
        (
            "cri/xtx",
            "cri/files/xtx/unk_4140-zlib.xtx",
            "cri/files/xtx/unk_4140-out.png",
            true,
        ),
    ];
    for (id, input, expected, zipped) in cases {
        let mut bytes = std::fs::read(root.join(input)).unwrap();
        if zipped {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(bytes.as_slice())
                .read_to_end(&mut out)
                .unwrap();
            bytes = out;
        }
        let got = images_d::decode(id, &bytes, &ImageDecodeContext::new(input, &r))
            .unwrap_or_else(|e| panic!("{input}: {e:#}"));
        let want =
            crysirror_image::decode_standard(&std::fs::read(root.join(expected)).unwrap()).unwrap();
        assert_eq!(
            (got.width, got.height),
            (want.width, want.height),
            "{input}"
        );
        assert_eq!(got.rgba, want.rgba, "{input}");
        count += 1;
        println!("exact {id}: {input}");
    }
    assert_eq!(count, 13);
}
#[test]
fn malformed_or_unimplemented_texture_layouts_fail_visibly() {
    let r = DecoderRegistry::default();
    let ctx = ImageDecodeContext::new("invalid.texture", &r);
    for id in images_d::FORMATS {
        for bytes in [&[][..], &[0; 48][..], &[255; 64][..]] {
            assert!(images_d::decode(id, bytes, &ctx).is_err(), "{id}");
        }
    }
    let mut b = pgx(true);
    b.truncate(24);
    assert!(images_d::decode("glib/pgx", &b, &ctx).is_err());
    let mut x = vec![0; 32];
    x[..4].copy_from_slice(b"xtx\0");
    x[4] = 99;
    for at in [8, 12, 16, 20] {
        be(&mut x, at, 32);
    }
    assert!(images_d::decode("cri/xtx", &x, &ctx).is_err());
    let mut g = vec![0; 48];
    be(&mut g, 4, 8);
    be(&mut g, 8, 2);
    be(&mut g, 16, 40);
    g[24] = 0x86;
    g[32..34].copy_from_slice(&4u16.to_be_bytes());
    g[34..36].copy_from_slice(&4u16.to_be_bytes());
    g[36..38].copy_from_slice(&1u16.to_be_bytes());
    assert!(images_d::decode("playstation/gtf", &g, &ctx).is_err());
}
