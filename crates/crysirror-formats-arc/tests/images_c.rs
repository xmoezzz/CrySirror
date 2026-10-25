// SPDX-License-Identifier: GPL-3.0-or-later
//! All ordinary fixtures are constructed in memory; no game or font is bundled.
use crysirror_formats_arc::{images_c, register_images};
use crysirror_image::{DecoderRegistry, ImageDecodeContext, ImageSurface};
use flate2::{write::ZlibEncoder, Compression};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::PathBuf,
};
fn u16at(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn u32at(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn zlib(b: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(b).unwrap();
    z.finish().unwrap()
}
fn registry() -> DecoderRegistry {
    let mut r = DecoderRegistry::default();
    register_images(&mut r).unwrap();
    r
}
fn decode(id: &str, path: &str, b: &[u8]) -> anyhow::Result<ImageSurface> {
    let r = registry();
    let source = |_: &str| Err(crysirror_core::Error::NotFound);
    let ctx = ImageDecodeContext::new(path, &r).with_source(&source);
    images_c::decode(id, b, &ctx)
}
fn expected(id: &str, path: &str, b: &[u8], w: u32, h: u32, rgba: &[u8]) {
    let image = decode(id, path, b).unwrap_or_else(|e| panic!("{id}: {e:#}"));
    assert_eq!((image.width, image.height), (w, h));
    assert_eq!(image.rgba, rgba, "{id}");
}
fn literal_lzss(b: &[u8], leaf: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for block in b.chunks(8) {
        out.push(if leaf {
            !((((1u16 << block.len()) - 1) as u8) << (8 - block.len()))
        } else {
            ((1u16 << block.len()) - 1) as u8
        });
        out.extend(block.iter().map(|v| if leaf { !v } else { *v }));
    }
    out
}
fn bmp(bgr: &[u8], w: u32, h: u32) -> Vec<u8> {
    let stride = (w as usize * 3 + 3) & !3;
    let mut b = vec![0; 54 + stride * h as usize];
    b[..2].copy_from_slice(b"BM");
    let n = b.len() as u32;
    u32at(&mut b, 2, n);
    u32at(&mut b, 10, 54);
    u32at(&mut b, 14, 40);
    u32at(&mut b, 18, w);
    u32at(&mut b, 22, h);
    u16at(&mut b, 26, 1);
    u16at(&mut b, 28, 24);
    for y in 0..h as usize {
        b[54 + y * stride..54 + y * stride + w as usize * 3]
            .copy_from_slice(&bgr[y * w as usize * 3..(y + 1) * w as usize * 3]);
    }
    b
}
fn rgba_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    crysirror_image::encode_png(&ImageSurface {
        width: w,
        height: h,
        rgba: rgba.to_vec(),
    })
    .unwrap()
}
#[test]
fn rc8_and_rct_literals_and_overlap_runs() {
    let mut rc8 = b"\x98\x5a\x92\x9a8_00".to_vec();
    rc8.extend(2u32.to_le_bytes());
    rc8.extend(1u32.to_le_bytes());
    rc8.extend([0; 4]);
    let mut pal = vec![0; 768];
    pal[..6].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
    rc8.extend(pal);
    rc8.extend([0, 0, 1]);
    expected(
        "majiro/rc8",
        "x.rc8",
        &rc8,
        2,
        1,
        &[3, 2, 1, 255, 6, 5, 4, 255],
    );
    let mut rct = b"\x98\x5a\x92\x9aTC00".to_vec();
    rct.extend(3u32.to_le_bytes());
    rct.extend(1u32.to_le_bytes());
    rct.extend(4u32.to_le_bytes());
    rct.extend([1, 2, 3, 0x81]);
    expected(
        "majiro/rct",
        "x.rct",
        &rct,
        3,
        1,
        &[3, 2, 1, 255, 3, 2, 1, 255, 3, 2, 1, 255],
    );
    let mut encrypted = rct.clone();
    encrypted[5] = b'S';
    assert!(format!(
        "{:#}",
        decode("majiro/rct", "x.rct", &encrypted).unwrap_err()
    )
    .contains("key provider"));
}
fn ge(filter: u16, w: u32, h: u32, payload: &[u8]) -> Vec<u8> {
    let mut b = vec![0; 40];
    b[..4].copy_from_slice(b"GE \0");
    u32at(&mut b, 12, w);
    u32at(&mut b, 16, h);
    u16at(&mut b, 28, filter);
    u32at(&mut b, 32, payload.len() as u32);
    u32at(&mut b, 36, payload.len() as u32 + 2);
    b.extend([0, payload.len() as u8]);
    b.extend(payload);
    b
}
#[test]
fn pgd_ge_delta_and_chroma_and_c00_embedded_tga() {
    let data = [0, 0, 24, 0, 2, 0, 1, 0, 1, 10, 20, 30, 226, 226, 226];
    expected(
        "amuse-craft/pgd-ge",
        "x.pgd",
        &ge(3, 2, 1, &data),
        2,
        1,
        &[30, 20, 10, 255, 60, 50, 40, 255],
    );
    expected(
        "amuse-craft/pgd-ge",
        "x.pgd",
        &ge(2, 2, 2, &[0, 0, 10, 20, 30, 40]),
        2,
        2,
        &[
            10, 10, 10, 255, 20, 20, 20, 255, 30, 30, 30, 255, 40, 40, 40, 255,
        ],
    );
    let mut tga = vec![0; 18];
    tga[2] = 2;
    u16at(&mut tga, 12, 1);
    u16at(&mut tga, 14, 1);
    tga[16] = 24;
    tga[17] = 32;
    tga.extend([1, 2, 3]);
    let mut b = vec![0; 24];
    b.extend(b"00_C");
    b.extend((tga.len() as u32).to_le_bytes());
    b.extend((tga.len() as u32 + 14).to_le_bytes());
    b.extend([0, tga.len() as u8]);
    b.extend(tga);
    expected("amuse-craft/pgd-c00", "x.pgd", &b, 1, 1, &[3, 2, 1, 255]);
}
fn nvsg(fmt: u16, w: u16, h: u16, count: u32, data: &[u8]) -> Vec<u8> {
    let mut b = vec![0; 44];
    b[..4].copy_from_slice(b"hzc1");
    u32at(&mut b, 4, data.len() as u32);
    u32at(&mut b, 8, 32);
    b[12..16].copy_from_slice(b"NVSG");
    u16at(&mut b, 18, fmt);
    u16at(&mut b, 20, w);
    u16at(&mut b, 22, h);
    u32at(&mut b, 32, count);
    b.extend(zlib(data));
    b
}
#[test]
fn nvsg_all_pixel_modes_and_stacked_frames() {
    for (fmt, data, rgba) in [
        (0, vec![1, 2, 3], vec![3, 2, 1, 255]),
        (1, vec![1, 2, 3, 4], vec![3, 2, 1, 4]),
        (3, vec![42], vec![42, 42, 42, 255]),
        (4, vec![2], vec![255, 255, 255, 255]),
    ] {
        expected("fvp/nvsg", "x", &nvsg(fmt, 1, 1, 1, &data), 1, 1, &rgba);
    }
    expected(
        "fvp/nvsg",
        "x",
        &nvsg(2, 1, 1, 2, &[1, 2, 3, 4, 5, 6, 7, 8]),
        1,
        2,
        &[3, 2, 1, 4, 7, 6, 5, 8],
    );
}
#[test]
fn ykg_unswaps_png_and_dpng_places_and_overwrites_tiles() {
    let pixel = [11, 22, 33, 44];
    let mut png = rgba_png(&pixel, 1, 1);
    png[1..4].copy_from_slice(b"GNP");
    let mut b = vec![0; 64];
    b[..6].copy_from_slice(b"YKG000");
    u32at(&mut b, 8, 64);
    u32at(&mut b, 40, 64);
    u32at(&mut b, 44, png.len() as u32);
    b.extend(png);
    expected("yuka-script/ykg", "x.ykg", &b, 1, 1, &pixel);
    let png = rgba_png(&pixel, 1, 1);
    let mut d = vec![0; 20];
    d[..4].copy_from_slice(b"DPNG");
    u32at(&mut d, 8, 1);
    u32at(&mut d, 12, 2);
    u32at(&mut d, 16, 1);
    d.extend(1u32.to_le_bytes());
    d.extend(0u32.to_le_bytes());
    d.extend(1u32.to_le_bytes());
    d.extend(1u32.to_le_bytes());
    d.extend((png.len() as u32).to_le_bytes());
    d.extend([0; 8]);
    d.extend(png);
    expected(
        "qlie/dpng",
        "x.png",
        &d,
        2,
        1,
        &[0, 0, 0, 0, 11, 22, 33, 44],
    );
}
#[test]
fn gfb_bottom_up_alpha_fallback_and_gs_lzss() {
    let mut b = vec![0; 64];
    b[..4].copy_from_slice(b"GFB ");
    u32at(&mut b, 16, 8);
    u32at(&mut b, 20, 64);
    u32at(&mut b, 28, 1);
    u32at(&mut b, 32, 2);
    u16at(&mut b, 38, 32);
    b.extend([1, 2, 3, 0, 4, 5, 6, 0]);
    expected("gpk2/gfb", "x.gfb", &b, 1, 2, &[6, 5, 4, 255, 3, 2, 1, 255]);
    let compressed = literal_lzss(&[1, 2, 3, 4], false);
    let mut g = vec![0; 36];
    g[..4].copy_from_slice(&[0, 0, 4, 0]);
    u32at(&mut g, 4, compressed.len() as u32);
    u32at(&mut g, 8, 4);
    u32at(&mut g, 12, 36);
    u32at(&mut g, 20, 1);
    u32at(&mut g, 24, 1);
    u32at(&mut g, 28, 32);
    u32at(&mut g, 32, 1);
    g.extend(compressed);
    expected("gs/gfx", "x", &g, 1, 1, &[3, 2, 1, 4]);
}
#[test]
fn agf_four_commands_and_iph_raw_and_compressed() {
    let mut b = vec![0; 36];
    b[..4].copy_from_slice(b"AGF\0");
    u32at(&mut b, 4, 1);
    u32at(&mut b, 12, 36);
    u32at(&mut b, 28, 6);
    u32at(&mut b, 32, 1);
    for v in [
        0x101u32, 0x04030201, 0x102, 0x08070605, 0x10203, 0x0c0b0a09, 0x200104,
    ] {
        b.extend(v.to_le_bytes());
    }
    let len = b.len() as u32;
    u32at(&mut b, 8, len);
    expected(
        "aoi/agf",
        "x.agf",
        &b,
        6,
        1,
        &[
            3, 2, 1, 4, 7, 6, 5, 8, 11, 10, 9, 12, 11, 10, 9, 12, 11, 10, 9, 12, 11, 10, 9, 12,
        ],
    );
    let mut iph = vec![0; 0x58];
    iph[8..12].copy_from_slice(b"IPH\0");
    iph[0x38..0x3c].copy_from_slice(b"bmp ");
    u16at(&mut iph, 0x40, 1);
    u16at(&mut iph, 0x42, 1);
    u16at(&mut iph, 0x50, 16);
    iph.extend([0, 0x7c]);
    expected("aoi/iph", "x.iph", &iph, 1, 1, &[248, 0, 0, 255]);
    iph.truncate(0x58);
    u16at(&mut iph, 0x52, 1);
    iph.extend([0, 0, 0x7c, 0, 0]);
    expected("aoi/iph", "x.iph", &iph, 1, 1, &[248, 0, 0, 255]);
}
fn leaf_header(magic: &[u8], w: u16, h: u16, size: usize) -> Vec<u8> {
    let mut b = vec![0; size];
    b[..8].copy_from_slice(magic);
    u16at(&mut b, 12, w);
    u16at(&mut b, 14, h);
    b
}
#[test]
fn leaf_lf2_lf3_lc3_and_lfb() {
    let mut lf2 = leaf_header(b"LEAF256\0", 1, 2, 24);
    u16at(&mut lf2, 22, 2);
    lf2.extend([1, 2, 3, 4, 5, 6]);
    lf2.extend(literal_lzss(&[0, 1], true));
    expected(
        "leaf/lf2",
        "x.lf2",
        &lf2,
        1,
        2,
        &[6, 5, 4, 255, 3, 2, 1, 255],
    );
    let mut lf3 = leaf_header(b"LEAF64K\0", 1, 1, 20);
    u32at(&mut lf3, 16, 20);
    lf3.extend(literal_lzss(&[0, 0x7c], true));
    expected("leaf/lf3", "x.lf3", &lf3, 1, 1, &[248, 0, 0, 255]);
    let alpha = literal_lzss(&[7], true);
    let mut lc3 = leaf_header(b"LEAFC64\0", 1, 1, 24);
    u32at(&mut lc3, 16, 24);
    u32at(&mut lc3, 20, 24 + alpha.len() as u32);
    lc3.extend(alpha);
    lc3.extend(literal_lzss(&[31, 0], true));
    expected("leaf/lc3", "x.lc3", &lc3, 1, 1, &[0, 0, 248, 56]);
    let bytes = bmp(&[1, 2, 3], 1, 1);
    let mut lfb = (bytes.len() as u32).to_le_bytes().to_vec();
    lfb.extend(literal_lzss(&bytes, true));
    expected("leaf/lfb", "x.lfb", &lfb, 1, 1, &[3, 2, 1, 255]);
}
#[test]
fn leaf_bbm_and_grp_palette_mask_and_bjr_filename() {
    let mut bbm = Vec::new();
    for v in [1u16, 1, 1, 1, 1, 1] {
        bbm.extend(v.to_le_bytes());
    }
    bbm.extend(23u32.to_le_bytes());
    bbm.extend([0; 5]);
    bbm.extend(1u16.to_le_bytes());
    bbm.extend([0; 11]);
    bbm.extend([1, 2, 3, 4]);
    bbm.push(0);
    expected("leaf/bbm", "x.bbm", &bbm, 1, 1, &[3, 2, 1, 4]);
    let r = registry();
    let mut files = BTreeMap::new();
    files.insert("x.c16", vec![1, 0, 5, 1, 2, 3]);
    files.insert("x.msk", vec![100]);
    let source = |p: &str| files.get(p).cloned().ok_or(crysirror_core::Error::NotFound);
    let ctx = ImageDecodeContext::new("x.grp", &r).with_source(&source);
    let grp = [1, 0, 1, 0, 5];
    assert_eq!(
        images_c::decode("leaf/grp", &grp, &ctx).unwrap().rgba,
        [3, 2, 1, 155]
    );
    // ASCII name a.bjr has sum 461; 1-row row permutation is fixed.
    expected(
        "leaf/bjr",
        "a.bjr",
        &bmp(&[10, 20, 30], 1, 1),
        1,
        1,
        &[174, 71, 194, 255],
    );
}
#[test]
fn leaf_lfg_both_traversals() {
    for horizontal in [0, 1] {
        let mut b = vec![0; 48];
        b[..8].copy_from_slice(b"LEAFCODE");
        for v in &mut b[8..32] {
            *v = 0x12;
        }
        u16at(&mut b, 36, 0);
        u16at(&mut b, 38, 1);
        b[40] = horizontal;
        u32at(&mut b, 44, 8);
        b.extend([255, 0, 0, 0, 0, 0, 0, 0, 0]);
        expected("leaf/lfg", "x.lfg", &b, 8, 2, &[16, 32, 16, 255].repeat(16));
    }
}
fn cg_plane(entry: &[u8], pixels: u32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend((pixels * entry.len() as u32).to_le_bytes());
    let bits = pixels as usize * 4;
    let mut packed = vec![0; (bits + 7) / 8];
    for p in 0..pixels as usize {
        packed[p * 4 / 8] |= 0x2 << (4 - p * 4 % 8);
    }
    b.extend((packed.len() as u32).to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend([0, 0]);
    b.extend(entry);
    b.extend(packed);
    b
}
#[test]
fn liar_lim_and_wcg_dictionary_planes_and_alpha() {
    let mut lim = b"LM".to_vec();
    lim.extend(0x11u16.to_le_bytes());
    lim.extend(16u16.to_le_bytes());
    lim.extend(8u16.to_le_bytes());
    lim.extend(1u32.to_le_bytes());
    lim.extend(1u32.to_le_bytes());
    lim.extend(cg_plane(&[0, 0xf8], 1));
    lim.extend(cg_plane(&[55], 1));
    expected("liar-soft/lim", "x.lim", &lim, 1, 1, &[248, 0, 0, 200]);
    let mut wcg = b"WG".to_vec();
    wcg.extend(0x41u16.to_le_bytes());
    wcg.extend(32u16.to_le_bytes());
    wcg.extend([0, 0]);
    wcg.extend(1u32.to_le_bytes());
    wcg.extend(1u32.to_le_bytes());
    wcg.extend(cg_plane(&[3, 55], 1));
    wcg.extend(cg_plane(&[1, 2], 1));
    expected("liar-soft/wcg", "x.wcg", &wcg, 1, 1, &[3, 2, 1, 200]);
}
#[test]
fn libido_mnc_and_lilim_abm_dbm() {
    let mut m = vec![0; 40];
    m[..10].copy_from_slice(b"HH6\x10\x0e\0\0\0\0\0");
    u32at(&mut m, 10, 40);
    u32at(&mut m, 18, 1);
    u32at(&mut m, 22, 1);
    u16at(&mut m, 28, 24);
    u32at(&mut m, 34, 3);
    m.extend([1, 2, 3]);
    expected("libido/mnc", "x.mnc", &m, 1, 1, &[3, 2, 1, 255]);
    let mut a = vec![0; 54];
    a[..2].copy_from_slice(b"BM");
    u32at(&mut a, 18, 1);
    u32at(&mut a, 22, 1);
    u16at(&mut a, 28, 32);
    a.extend([10, 1, 20, 2, 30, 3]);
    expected("lilim/abm", "x.abm", &a, 1, 1, &[3, 2, 1, 30]);
    let mut d = vec![0; 24];
    d[..2].copy_from_slice(b"DM");
    u16at(&mut d, 10, 1);
    u16at(&mut d, 12, 1);
    u32at(&mut d, 14, 1);
    u16at(&mut d, 18, 1);
    u32at(&mut d, 20, 12);
    d.extend([0]);
    d.extend(12u32.to_le_bytes());
    d.extend(3u32.to_le_bytes());
    d.extend([1, 2, 3]);
    let len = d.len() as u32;
    u32at(&mut d, 4, len);
    expected("lilim/dbm", "x.dbm", &d, 1, 1, &[3, 2, 1, 255]);
}
#[test]
fn malformed_inputs_and_dimensions_fail_without_panics() {
    for id in images_c::FORMATS {
        for data in [&[][..], &[0][..], &[255; 64][..]] {
            assert!(decode(id, "x.abm", data).is_err(), "{id}");
        }
    }
    let mut bad = nvsg(0, 65535, 65535, 1, &[0]);
    assert!(decode("fvp/nvsg", "x", &bad).is_err());
    bad = ge(3, 1, 1, &[0, 0, 24, 0, 1, 0, 1, 0, 2, 1, 2, 3]);
    assert!(decode("amuse-craft/pgd-ge", "x", &bad).is_err());
}

#[test]
#[ignore = "reads external pinned upstream fixtures; no fixture bytes are bundled"]
fn external_upstream_pixels_match_exactly() {
    let root = std::env::var_os("ARC_UNPACKER_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/reference/arc_unpacker")
        });
    let cases = [
        (
            "cronus/grp",
            "cronus/files/grp/DPBG03D",
            "cronus/files/grp/DPBG03D-out.png",
            "DPBG03D",
        ),
        (
            "cronus/grp",
            "cronus/files/grp/TCF12",
            "cronus/files/grp/TCF12-out.png",
            "TCF12",
        ),
        (
            "cronus/grp",
            "cronus/files/grp/MSGPARTS",
            "cronus/files/grp/MSGPARTS-out.png",
            "MSGPARTS",
        ),
        (
            "cronus/grp",
            "cronus/files/grp/SR1",
            "cronus/files/grp/SR1-out.png",
            "SR1",
        ),
        (
            "cronus/grp",
            "cronus/files/grp/SELWIN",
            "cronus/files/grp/SELWIN-out.png",
            "SELWIN",
        ),
        (
            "lucifen/elg",
            "lucifen/files/elg/rf_bg.elg",
            "lucifen/files/elg/rf_bg-out.png",
            "rf_bg.elg",
        ),
        (
            "lucifen/elg",
            "lucifen/files/elg/tl_ref.elg",
            "lucifen/files/elg/tl_ref-out.png",
            "tl_ref.elg",
        ),
        (
            "lucifen/elg",
            "lucifen/files/elg/rule006.elg",
            "lucifen/files/elg/rule006-out.png",
            "rule006.elg",
        ),
        (
            "lucifen/elg",
            "lucifen/files/elg/if_lb.elg",
            "lucifen/files/elg/if_lb-out.png",
            "if_lb.elg",
        ),
        (
            "nsystem/mgd",
            "nsystem/files/mgd/GS_UD.MGD",
            "nsystem/files/mgd/GS_UD-out.png",
            "GS_UD.MGD",
        ),
        (
            "nsystem/mgd",
            "nsystem/files/mgd/saveload_p.MGD",
            "nsystem/files/mgd/saveload_p-out.png",
            "saveload_p.MGD",
        ),
        (
            "tabito/gwd",
            "tabito/files/gwd/logo",
            "tabito/files/gwd/logo-out.png",
            "logo",
        ),
        (
            "wild-bug/wbm",
            "wild_bug/files/wbm/S_M01.WBM",
            "wild_bug/files/wbm/S_M01-out.png",
            "S_M01.WBM",
        ),
        (
            "wild-bug/wbm",
            "wild_bug/files/wbm/S_Y08.WBM",
            "wild_bug/files/wbm/S_Y08-out.png",
            "S_Y08.WBM",
        ),
        (
            "wild-bug/wbm",
            "wild_bug/files/wbm/EF_WIPE_LR.WBM",
            "wild_bug/files/wbm/EF_WIPE_LR-out.png",
            "EF_WIPE_LR.WBM",
        ),
        (
            "wild-bug/wbm",
            "wild_bug/files/wbm/ZD0211.WBM",
            "wild_bug/files/wbm/ZD0211-out.png",
            "ZD0211.WBM",
        ),
        (
            "wild-bug/wbm",
            "wild_bug/files/wbm/ZD0710.WBM",
            "wild_bug/files/wbm/ZD0710-out.png",
            "ZD0710.WBM",
        ),
        (
            "wild-bug/wbm",
            "wild_bug/files/wbm/E_AT06S.WBM",
            "wild_bug/files/wbm/E_AT06S-out.png",
            "E_AT06S.WBM",
        ),
        (
            "majiro/rc8",
            "majiro/files/rc8/style_tc_geo00_a_.rc8",
            "majiro/files/rc8/style_tc_geo00_a_-out.png",
            "style_tc_geo00_a_.rc8",
        ),
        (
            "majiro/rct",
            "majiro/files/rct/face_dummy.rct",
            "majiro/files/rct/face_dummy-out.png",
            "face_dummy.rct",
        ),
        (
            "majiro/rct",
            "majiro/files/rct/ev04_01c.rct",
            "majiro/files/rct/ev04_01c-out.png",
            "ev04_01c.rct",
        ),
        (
            "majiro/rct",
            "majiro/files/rct/st_kaoru_b_02_14_m-zlib.rct",
            "majiro/files/rct/st_kaoru_b_02_14_m-out.png",
            "st_kaoru_b_02_14_m.rct",
        ),
        (
            "amuse-craft/pgd-c00",
            "amuse_craft/files/pgd-c00/FA03C_01E-zlib.PGD",
            "amuse_craft/files/pgd-c00/FA03C_01E-out.png",
            "FA03C_01E.PGD",
        ),
        (
            "amuse-craft/pgd-ge",
            "amuse_craft/files/pgd-ge/SYSTEM.PGD",
            "amuse_craft/files/pgd-ge/SYSTEM-out.png",
            "SYSTEM.PGD",
        ),
        (
            "amuse-craft/pgd-ge",
            "amuse_craft/files/pgd-ge/SECRET.PGD",
            "amuse_craft/files/pgd-ge/SECRET-out.png",
            "SECRET.PGD",
        ),
        (
            "amuse-craft/pgd-ge",
            "amuse_craft/files/pgd-ge/ETLA023R.PGD",
            "amuse_craft/files/pgd-ge/ETLA023R-out.png",
            "ETLA023R.PGD",
        ),
        (
            "gs/gfx",
            "gs/files/gs-gfx/SYSLOGO",
            "gs/files/gs-gfx/SYSLOGO-out.png",
            "SYSLOGO",
        ),
        (
            "gs/gfx",
            "gs/files/gs-gfx/TRMASK16",
            "gs/files/gs-gfx/TRMASK16-out.png",
            "TRMASK16",
        ),
        (
            "gs/gfx",
            "gs/files/gs-gfx/IMG019KBS",
            "gs/files/gs-gfx/IMG019KBS-out.png",
            "IMG019KBS",
        ),
        (
            "fvp/nvsg",
            "fvp/files/nvsg/BG085_001",
            "fvp/files/nvsg/BG085_001-out.png",
            "BG085_001",
        ),
        (
            "fvp/nvsg",
            "fvp/files/nvsg/CHR_Shigure",
            "fvp/files/nvsg/CHR_Shigure-out.png",
            "CHR_Shigure",
        ),
        (
            "fvp/nvsg",
            "fvp/files/nvsg/bu_effect22_F_1",
            "fvp/files/nvsg/bu_effect22_F_1-out.png",
            "bu_effect22_F_1",
        ),
        (
            "fvp/nvsg",
            "fvp/files/nvsg/gaiji_heart",
            "fvp/files/nvsg/gaiji_heart-out.png",
            "gaiji_heart",
        ),
        (
            "fvp/nvsg",
            "fvp/files/nvsg/diss48",
            "fvp/files/nvsg/diss48-out.png",
            "diss48",
        ),
        (
            "yuka-script/ykg",
            "yuka_script/files/ykg/reimu.ykg",
            "yuka_script/files/ykg/reimu-out.png",
            "reimu.ykg",
        ),
        (
            "qlie/dpng",
            "qlie/files/dpng/electricity02.png",
            "qlie/files/dpng/electricity02-out.png",
            "electricity02.png",
        ),
        (
            "aoi/iph",
            "aoi/files/iph/cgsA00-zlib.iph",
            "aoi/files/iph/cgsA00-out.png",
            "cgsA00.iph",
        ),
        (
            "leaf/bbm",
            "leaf/files/bbm/Stage_14_E0_OBJ-zlib.bbm",
            "leaf/files/bbm/Stage_14_E0_OBJ-out.png",
            "Stage_14_E0_OBJ.bbm",
        ),
        (
            "leaf/bjr",
            "leaf/files/bjr/v00232-zlib.BJR",
            "leaf/files/bjr/v00232-out.png",
            "v00232.BJR",
        ),
        (
            "leaf/grp",
            "leaf/files/grp/ase200.grp",
            "leaf/files/grp/ase200-out.png",
            "ase200.grp",
        ),
        (
            "leaf/lf2",
            "leaf/files/lf2/C1F01.LF2",
            "leaf/files/lf2/C1F01-out.png",
            "C1F01.LF2",
        ),
        (
            "leaf/lf3",
            "leaf/files/lf3/KEY2.LF3",
            "leaf/files/lf3/KEY2-out.png",
            "KEY2.LF3",
        ),
        (
            "leaf/lc3",
            "leaf/files/lc3/KAO0903.LC3",
            "leaf/files/lc3/KAO0903-out.png",
            "KAO0903.LC3",
        ),
        (
            "leaf/lfb",
            "leaf/files/lfb/LIFEM.LFB",
            "leaf/files/lfb/LIFEM-out.png",
            "LIFEM.LFB",
        ),
        (
            "leaf/lfg",
            "leaf/files/lfg/OP_L6.LFG",
            "leaf/files/lfg/OP_L6-out.png",
            "OP_L6.LFG",
        ),
        (
            "leaf/lfg",
            "leaf/files/lfg/MAX_C09.LFG",
            "leaf/files/lfg/MAX_C09-out.png",
            "MAX_C09.LFG",
        ),
        (
            "liar-soft/lim",
            "liar_soft/files/lim/7035.lim",
            "liar_soft/files/lim/7035-out.png",
            "7035.lim",
        ),
        (
            "liar-soft/lim",
            "liar_soft/files/lim/MU_ST.lim",
            "liar_soft/files/lim/MU_ST-out.png",
            "MU_ST.lim",
        ),
        (
            "liar-soft/wcg",
            "liar_soft/files/wcg/0003.wcg",
            "liar_soft/files/wcg/0003-out.png",
            "0003.wcg",
        ),
        (
            "libido/mnc",
            "libido/files/mnc/test-zlib.MNC",
            "libido/files/mnc/test-out.png",
            "test.MNC",
        ),
        (
            "lilim/abm",
            "lilim/files/abm/kj_ase.abm",
            "lilim/files/abm/kj_ase-out.png",
            "kj_ase.abm",
        ),
        (
            "lilim/abm",
            "lilim/files/abm/popsave.abm",
            "lilim/files/abm/popsave-out.png",
            "popsave.abm",
        ),
        (
            "lilim/dbm",
            "lilim/files/dbm/WD04.dbm",
            "lilim/files/dbm/WD04-out.png",
            "WD04.dbm",
        ),
        (
            "lilim/dbm",
            "lilim/files/dbm/TB_fin.dbm",
            "lilim/files/dbm/TB_fin-out.png",
            "TB_fin.dbm",
        ),
        (
            "lilim/dbm",
            "lilim/files/dbm/prompt.dbm",
            "lilim/files/dbm/prompt-out.png",
            "prompt.dbm",
        ),
        (
            "lilim/dbm",
            "lilim/files/dbm/C01.dbm",
            "lilim/files/dbm/C01-out.png",
            "C01.dbm",
        ),
    ];
    let load = |name: &str| -> Vec<u8> {
        let data = std::fs::read(root.join("tests/dec").join(name)).unwrap();
        if name.contains("-zlib.") {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(&data[..])
                .take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut out)
                .unwrap();
            assert!(out.len() <= 64 * 1024 * 1024);
            out
        } else {
            data
        }
    };
    let mut dependencies = BTreeMap::new();
    for (name, file) in [
        (
            "st_kaoru_b_02_01_m.rct",
            "majiro/files/rct/st_kaoru_b_02_01_m-zlib.rct",
        ),
        (
            "st_kaoru_b_02_01_m_.rc8",
            "majiro/files/rct/st_kaoru_b_02_01_m_-zlib.rc8",
        ),
        ("ase200.c16", "leaf/files/grp/ase200.c16"),
        ("ase200.msk", "leaf/files/grp/ase200.msk"),
    ] {
        dependencies.insert(name, load(file));
    }
    let source = |path: &str| {
        dependencies
            .get(path)
            .cloned()
            .ok_or(crysirror_core::Error::NotFound)
    };
    let r = registry();
    let mut failures = Vec::new();
    for (id, input, expected, path) in cases {
        let b = load(input);
        let ctx = ImageDecodeContext::new(path, &r).with_source(&source);
        let expected = crysirror_image::decode_standard(&load(expected)).unwrap();
        match images_c::decode(id, &b, &ctx) {
            Ok(actual) => {
                if actual != expected {
                    let changed = actual
                        .rgba
                        .iter()
                        .zip(&expected.rgba)
                        .filter(|(a, b)| a != b)
                        .count();
                    failures.push(format!(
                        "{id} {input}: dimensions {:?}/{:?}, {changed} differing channels",
                        (actual.width, actual.height),
                        (expected.width, expected.height)
                    ));
                } else {
                    println!(
                        "exact pixels: {id} {input} {}x{}",
                        actual.width, actual.height
                    );
                }
            }
            Err(e) => failures.push(format!("{id} {input}: {e:#}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn bits(fields: &[(usize, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut used = 0;
    for &(n, value) in fields {
        for i in (0..n).rev() {
            if used % 8 == 0 {
                out.push(0);
            }
            if value >> i & 1 != 0 {
                *out.last_mut().unwrap() |= 1 << (7 - used % 8);
            }
            used += 1;
        }
    }
    out
}
#[test]
fn kg_truecolor_and_separate_alpha() {
    let mut b = vec![0; 48];
    b[..3].copy_from_slice(b"KG\x02");
    b[3] = 2;
    u16at(&mut b, 4, 2);
    u16at(&mut b, 6, 1);
    u32at(&mut b, 16, 48);
    u32at(&mut b, 44, 54);
    b.extend([1, 2, 3, 4, 5, 6, 100, 200]);
    expected(
        "abogado/kg",
        "x.kg",
        &b,
        2,
        1,
        &[5, 3, 1, 100, 6, 4, 2, 200],
    );
}
#[test]
fn bsg_planar_raw_and_rle() {
    for compression in [0, 1] {
        let mut b = vec![0; 62];
        b[..12].copy_from_slice(b"BSS-Graphics");
        u16at(&mut b, 22, 1);
        u16at(&mut b, 24, 2);
        b[49] = compression;
        u32at(&mut b, 50, 62);
        u32at(&mut b, 54, 8);
        for channel in [[1, 2], [3, 4], [5, 6], [7, 8]] {
            if compression == 1 {
                b.extend(3u32.to_le_bytes());
                b.push(1);
            }
            b.extend(channel);
        }
        expected("bishop/bsg", "x.bsg", &b, 1, 2, &[6, 4, 2, 8, 5, 3, 1, 7]);
    }
}
#[test]
fn gd_raw_and_sparse_custom_fill_use_original_resolution() {
    for custom in [false, true] {
        let mut b = vec![0; 4 + 3 * 64 * 47];
        b[..3].copy_from_slice(b"GD2");
        b.push(if custom { 0x70 } else { 0x62 });
        b.push(0);
        if custom {
            b.extend(bits(&[
                (2, 0),
                (8, 1),
                (8, 2),
                (8, 3),
                (1, 0),
                (2, 3),
                (32, u32::MAX),
            ]));
        } else {
            b.extend([1, 2, 3].repeat(640 * 480));
        }
        expected(
            "complets/gd",
            "x.gd",
            &b,
            640,
            480,
            &[3, 2, 1, 255].repeat(640 * 480),
        );
    }
}
#[test]
fn cronus_plain_header_and_elg_rgb_mask() {
    let mut b = vec![0; 40];
    u32at(&mut b, 4, 4);
    u32at(&mut b, 16, 1);
    u32at(&mut b, 20, 1);
    u32at(&mut b, 24, 32);
    u32at(&mut b, 36, 1);
    b.extend(literal_lzss(&[1, 2, 3, 4], false));
    expected("cronus/grp", "x", &b, 1, 1, &[3, 2, 1, 4]);
    let mut elg = b"ELG\x20\x01\0\x01\0".to_vec();
    elg.extend([0, 1, 2, 3, 255, 0, 44, 255]);
    expected("lucifen/elg", "x.elg", &elg, 1, 1, &[3, 2, 1, 44]);
}
fn mgd(kind: u32, data: &[u8], w: u16, h: u16) -> Vec<u8> {
    let mut b = vec![0; 96];
    b[..4].copy_from_slice(b"MGD ");
    u16at(&mut b, 12, w);
    u16at(&mut b, 14, h);
    u32at(&mut b, 16, w as u32 * h as u32 * 4);
    u32at(&mut b, 20, data.len() as u32 + 4);
    u32at(&mut b, 24, kind);
    u32at(&mut b, 92, data.len() as u32);
    b.extend(data);
    b
}
#[test]
fn mgd_plain_png_and_sgd_color_delta() {
    let p = [3, 2, 1, 4];
    expected(
        "nsystem/mgd",
        "x.mgd",
        &mgd(0, &[1, 2, 3, 4], 1, 1),
        1,
        1,
        &p,
    );
    expected(
        "nsystem/mgd",
        "x.mgd",
        &mgd(2, &rgba_png(&p, 1, 1), 1, 1),
        1,
        1,
        &p,
    );
    let mut sgd = vec![3, 0, 0, 0, 1, 128, 55];
    sgd.extend(7u32.to_le_bytes());
    sgd.extend([0x40, 1, 2, 3, 0x81, 0x21, 0x84]);
    expected(
        "nsystem/mgd",
        "x.mgd",
        &mgd(1, &sgd, 2, 1),
        2,
        1,
        &[3, 2, 1, 200, 4, 3, 2, 200],
    );
}
#[test]
fn gwd_gray_with_inverted_separate_mask() {
    fn stream(v: u8) -> Vec<u8> {
        let mut b = vec![0; 12];
        b[4..7].copy_from_slice(b"GWD");
        b[8] = 1;
        b[10] = 1;
        b[11] = 8;
        b.extend(bits(&[(3, 7), (1, 1), (1, 0), (8, v as u32)]));
        b
    }
    let mut b = stream(42);
    b.push(1);
    b.extend(stream(10));
    expected("tabito/gwd", "x.gwd", &b, 1, 1, &[42, 42, 42, 245]);
}
#[test]
fn wbm_plain_sections_remove_stride_and_mask() {
    let mut b = vec![0; 64];
    b[..8].copy_from_slice(b"WPX\x1aBMP\0");
    b[12] = 1;
    b[14] = 3;
    for (i, (id, pos, size)) in [(16u8, 64u32, 13u32), (17, 77, 4), (19, 81, 4)]
        .into_iter()
        .enumerate()
    {
        let o = 16 + i * 16;
        b[o] = id;
        b[o + 1] = 128;
        u32at(&mut b, o + 4, pos);
        u32at(&mut b, o + 8, size);
    }
    b.extend([0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 24]);
    b.extend([1, 2, 3, 0, 44, 0, 0, 0]);
    expected("wild-bug/wbm", "x.wbm", &b, 1, 1, &[3, 2, 1, 44]);
}

#[test]
fn kg_escape_recency_and_repetition_bitstreams() {
    // Four samples per channel: two seeds, one escaped value, repeat it once.
    // Table recency is shared between native B/G/R channel decodes.
    let mut fields = Vec::new();
    for (a, b, c) in [(1, 2, 3), (4, 5, 6), (7, 8, 9)] {
        fields.extend([
            (8, a),
            (8, b),
            (1, 0),
            (1, 1),
            (8, c),
            (1, 1),
            (1, 0),
            (2, 1),
        ]);
    }
    let mut data = vec![0; 48];
    data[..3].copy_from_slice(b"KG\x02");
    data[3] = 2;
    u16at(&mut data, 4, 4);
    u16at(&mut data, 6, 1);
    u32at(&mut data, 16, 48);
    data.extend(bits(&fields));
    expected(
        "abogado/kg",
        "x.kg",
        &data,
        4,
        1,
        &[7, 4, 1, 255, 8, 5, 2, 255, 9, 6, 3, 255, 9, 6, 3, 255],
    );
    // Literal-table selection (initial dictionary entry 6) without an escape.
    let mut fields = Vec::new();
    for seeds in [[1, 2], [3, 4], [5, 6]] {
        fields.extend([(8, seeds[0]), (8, seeds[1]), (1, 0), (1, 0), (3, 6)]);
    }
    u16at(&mut data, 4, 3);
    data.truncate(48);
    data.extend(bits(&fields));
    expected(
        "abogado/kg",
        "x.kg",
        &data,
        3,
        1,
        &[5, 3, 1, 255, 6, 4, 2, 255, 6, 6, 6, 255],
    );
}
#[test]
fn gd_sixteen_bit_dictionary_backreferences() {
    let mut fields = Vec::new();
    let mut left = 640 * 480 * 3;
    while left >= 3 {
        let count = left.min(18);
        fields.extend([(1, 0), (16, 0), (4, (count - 3) as u32)]);
        left -= count;
    }
    for _ in 0..left {
        fields.extend([(1, 1), (8, 0)]);
    }
    let mut b = vec![0; 4 + 3 * 64 * 47];
    b[..3].copy_from_slice(b"GD2");
    b.extend([0x6c, 0]);
    b.extend(bits(&fields));
    expected(
        "complets/gd",
        "x.gd",
        &b,
        640,
        480,
        &[0, 0, 0, 255].repeat(640 * 480),
    );
}
#[test]
fn optional_dependency_absence_is_distinct_from_denial_and_missing_resolver() {
    let mut rct = b"\x98\x5a\x92\x9aTC00".to_vec();
    rct.extend(1u32.to_le_bytes());
    rct.extend(1u32.to_le_bytes());
    rct.extend(3u32.to_le_bytes());
    rct.extend([1, 2, 3]);
    let r = registry();
    let context = ImageDecodeContext::new("x.rct", &r);
    assert!(images_c::decode("majiro/rct", &rct, &context).is_err());
    let denied = |_: &str| Err(crysirror_core::Error::ReadOnly);
    let context = ImageDecodeContext::new("x.rct", &r).with_source(&denied);
    assert!(images_c::decode("majiro/rct", &rct, &context).is_err());
    let absent = |_: &str| Err(crysirror_core::Error::NotFound);
    let context = ImageDecodeContext::new("x.rct", &r).with_source(&absent);
    assert_eq!(
        images_c::decode("majiro/rct", &rct, &context).unwrap().rgba,
        [3, 2, 1, 255]
    );
}
#[test]
fn encrypted_rct_uses_original_cipher_and_client_scoped_raw_password_bytes() {
    use crysirror_core::state::ClientId;
    use crysirror_resource::{DecodeContext, MemoryKeyProvider};
    use std::sync::Arc;
    // Ciphertext was independently calculated with Python zlib.crc32 and
    // little-endian derived CRC words, not by calling the Rust decoder.
    let mut bytes = b"\x98\x5a\x92\x9aTS00".to_vec();
    bytes.extend(2u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(7u32.to_le_bytes());
    bytes.extend([148, 92, 215, 66, 7, 107, 213]);
    let registry = registry();
    let source = |_: &str| Err(crysirror_core::Error::NotFound);
    let no_keys = ImageDecodeContext::new("x.rct", &registry).with_source(&source);
    assert!(images_c::decode("majiro/rct", &bytes, &no_keys).is_err());
    let keys = Arc::new(MemoryKeyProvider::default());
    keys.insert(
        ClientId(1),
        "majiro/rct",
        b"CrySirror synthetic password\x00\xff".to_vec(),
    )
    .unwrap();
    let mut first = DecodeContext::new(ClientId(1));
    first.keys = keys.clone();
    let mut other = DecodeContext::new(ClientId(2));
    other.keys = keys.clone();
    let first_ctx = ImageDecodeContext::new("x.rct", &registry)
        .with_source(&source)
        .with_resources(&first);
    assert_eq!(
        images_c::decode("majiro/rct", &bytes, &first_ctx)
            .unwrap()
            .rgba,
        [3, 2, 1, 255, 6, 5, 4, 255]
    );
    let other_ctx = ImageDecodeContext::new("x.rct", &registry)
        .with_source(&source)
        .with_resources(&other);
    assert!(images_c::decode("majiro/rct", &bytes, &other_ctx).is_err());
    keys.remove(ClientId(1), "majiro/rct").unwrap();
    assert!(images_c::decode("majiro/rct", &bytes, &first_ctx).is_err());
}
