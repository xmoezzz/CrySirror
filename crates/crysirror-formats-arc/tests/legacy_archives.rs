// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-authored archives for the table-driven legacy plugins. Layouts follow
//! the pinned arc_unpacker readers; no game or upstream fixture bytes are used.
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use crysirror_resource::{read_all, DecodeContext, FormatRegistry};
use std::{io::Write, sync::Arc};

pub fn memory(data: Vec<u8>) -> Arc<dyn AssetReader> {
    Arc::new(MemoryAssetReader(Arc::from(data)))
}
pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    out.write_all(data).unwrap();
    out.finish().unwrap()
}
pub fn sjis(name: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(name).0.into_owned()
}
pub fn le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}
pub fn fixed(name: &[u8], n: usize) -> Vec<u8> {
    let mut v = name.to_vec();
    v.resize(n, 0);
    v
}
/// Bytewise LZSS made only of literals: valid input for every LZSS decoder.
pub fn lzss_literals(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in data.chunks(8) {
        out.push(0xFF);
        out.extend_from_slice(chunk);
    }
    out
}
fn registry() -> FormatRegistry {
    let mut r = FormatRegistry::default();
    crysirror_formats_arc::register(&mut r).unwrap();
    r
}
/// Opens with auto-detection when the format has a signature, otherwise by
/// explicit ID, and returns every entry's decoded bytes.
pub fn extract(id: &str, data: Vec<u8>, detect: bool) -> Vec<(String, Vec<u8>)> {
    let r = registry();
    let cx = DecodeContext::new(ClientId(7));
    let source = memory(data);
    if detect {
        let found: Vec<_> = r
            .probe(source.clone(), &cx)
            .unwrap()
            .into_iter()
            .map(|m| m.0)
            .collect();
        assert_eq!(found, [id], "auto-detection for {id}");
    }
    let (got, archive) = r
        .open(source, detect.then_some(()).map_or(Some(id), |_| None), &cx)
        .unwrap();
    assert_eq!(got, id);
    let out = archive
        .entries()
        .iter()
        .map(|e| {
            let reader = archive.open(e.id, &cx).unwrap();
            (e.path.clone(), read_all(reader.as_ref(), 1 << 20).unwrap())
        })
        .collect();
    assert_eq!(cx.resident_bytes(), 0);
    out
}
fn pairs(v: &[(&str, &[u8])]) -> Vec<(String, Vec<u8>)> {
    v.iter().map(|(a, b)| (a.to_string(), b.to_vec())).collect()
}

#[test]
fn alice_soft_ald_afa_alk_dat() {
    // ALD: offsets are 24-bit sector numbers; first value is the table size.
    let mut ald = vec![0u8; 0x400];
    let r24 = |v: u32| [(v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8];
    ald[0..3].copy_from_slice(&r24(0x100));
    ald[3..6].copy_from_slice(&r24(0x200));
    ald[6..9].copy_from_slice(&r24(0x300));
    for (at, name, body) in [
        (0x200usize, "背景.qnt", b"ONE".as_slice()),
        (0x300, "b.vsp", b"TWO!"),
    ] {
        let mut h = Vec::new();
        h.extend(le(32));
        h.extend(le(body.len() as u32));
        h.extend([0; 8]);
        h.extend(fixed(&sjis(name), 16));
        h.extend(body);
        ald[at..at + h.len()].copy_from_slice(&h);
    }
    assert_eq!(
        extract("alice-soft/ald", ald, false),
        pairs(&[("背景.qnt", b"ONE"), ("b.vsp", b"TWO!")])
    );

    let mut table = Vec::new();
    for (i, (name, len)) in [("cg/a.qnt", 3u32), ("b.ajp", 2)].iter().enumerate() {
        table.extend(le(0));
        table.extend(le(12));
        table.extend(fixed(name.as_bytes(), 12));
        table.extend([0; 8]);
        table.extend(le(i as u32 * 3));
        table.extend(le(*len));
    }
    let packed = zlib(&table);
    let mut afa = b"AFAH\0\0\0\0AlicArch".to_vec();
    afa.extend(le(2));
    afa.extend(le(0));
    let data_start = 44 + packed.len() as u32;
    afa.extend(le(data_start));
    afa.extend(b"INFO");
    afa.extend(le(packed.len() as u32));
    afa.extend(le(table.len() as u32));
    afa.extend(le(2));
    afa.extend(&packed);
    afa.extend(b"AAABB");
    assert_eq!(
        extract("alice-soft/afa", afa, true),
        pairs(&[("cg/a.qnt", b"AAA"), ("b.ajp", b"BB")])
    );

    let mut alk = b"ALK0".to_vec();
    alk.extend(le(3));
    for (o, s) in [(32u32, 2u32), (0, 0), (34, 1)] {
        alk.extend(le(o));
        alk.extend(le(s));
    }
    alk.extend(b"xyZ");
    assert_eq!(
        extract("alice-soft/alk", alk, true),
        pairs(&[("000.dat", b"xy"), ("001.dat", b"Z")])
    );

    let mut dat = vec![0u8; 0x300 + 5];
    dat[0..2].copy_from_slice(&2u16.to_le_bytes());
    dat[2..4].copy_from_slice(&2u16.to_le_bytes());
    dat[4..6].copy_from_slice(&3u16.to_le_bytes());
    dat[0x100..0x103].copy_from_slice(b"abc");
    dat[0x200] = b'q';
    dat[0x300..].copy_from_slice(b"tail!");
    let out = extract("alice-soft/dat", dat, false);
    assert_eq!(out[0].0, "000.dat");
    assert_eq!(out[1].0, "001.dat");
    assert_eq!(out[1].1.len(), 0x105);
    assert_eq!(&out[0].1[..3], b"abc");
}

fn kid_key(name: &str, data: &mut [u8], pos: usize) {
    let mut key = name.bytes().fold(0u8, |a, b| a.wrapping_add(b));
    for b in data.iter_mut().skip(pos).take(0x100) {
        *b = b.wrapping_add(key);
        key = key.wrapping_mul(0x6D).wrapping_sub(0x25);
    }
}
#[test]
fn kid_lnk_decrypts_and_unpacks_lnd() {
    // LND ops: literal run, byte fill, back reference, additive repeat.
    let plain = b"ABCDxxxxxABCDxxxx12121212".to_vec();
    let mut lnd_body = vec![3, b'A', b'B', b'C', b'D'];
    lnd_body.extend([0xC0 | 3, b'x']); // fill 5
    lnd_body.extend([0x80 | (6 << 2), 8]); // copy 8 from 9 back
    lnd_body.extend([0x40, 3, b'1', b'2']); // 4 repetitions of "12"
    let mut lnd = b"lnd\0".to_vec();
    lnd.extend(le(0));
    lnd.extend(le(plain.len() as u32));
    lnd.extend(le(0));
    lnd.extend(&lnd_body);
    let mut wav = lnd.clone();
    kid_key("v.wav", &mut wav, 0);
    let mut table = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    for (name, body, compressed) in [
        ("v.wav", &wav, true),
        ("plain.txt", &b"hello".to_vec(), false),
    ] {
        table.extend(le(data.len() as u32));
        table.extend(le(((body.len() as u32) << 1) | compressed as u32));
        table.extend(fixed(name.as_bytes(), 24));
        data.extend(body);
    }
    let mut lnk = b"LNK\0".to_vec();
    lnk.extend(le(2));
    lnk.extend([0; 8]);
    lnk.extend(table);
    lnk.extend(data);
    assert_eq!(
        extract("kid/lnk", lnk, true),
        vec![
            ("v.wav".into(), plain),
            ("plain.txt".into(), b"hello".to_vec())
        ]
    );
}

#[test]
fn will_arc_and_pulltop() {
    // Two directories, 9-byte names; scripts are rotated left by two bits.
    let script = b"SCRIPT";
    let mut arc = le(2).to_vec();
    let entries_at = 4 + 2 * 12;
    arc.extend(fixed(b"WSC", 4));
    arc.extend(le(1));
    arc.extend(le(entries_at));
    arc.extend(fixed(b"OGG", 4));
    arc.extend(le(1));
    arc.extend(le(entries_at + 17));
    let data_at = entries_at + 34;
    arc.extend(fixed(b"start", 9));
    arc.extend(le(script.len() as u32));
    arc.extend(le(data_at));
    arc.extend(fixed(b"bgm01", 9));
    arc.extend(le(3));
    arc.extend(le(data_at + script.len() as u32));
    arc.extend(script.iter().map(|c| c.rotate_left(2)));
    arc.extend(b"ogg");
    assert_eq!(
        extract("will/arc-will", arc, true),
        pairs(&[("start.WSC", script), ("bgm01.OGG", b"ogg")])
    );

    let names: Vec<Vec<u8>> = ["画像.pna", "b"]
        .iter()
        .map(|n| {
            n.encode_utf16()
                .flat_map(u16::to_le_bytes)
                .chain([0, 0])
                .collect()
        })
        .collect();
    let table_size = (names[0].len() + names[1].len() + 16) as u32;
    let mut p = le(2).to_vec();
    p.extend(le(table_size));
    p.extend(le(2));
    p.extend(le(0));
    p.extend(&names[0]);
    p.extend(le(3));
    p.extend(le(2));
    p.extend(&names[1]);
    p.extend(b"PNabc");
    assert_eq!(
        extract("will/arc-pulltop", p, true),
        pairs(&[("画像.pna", b"PN"), ("b", b"abc")])
    );
}

#[test]
fn nitroplus_pak_inflates_entries() {
    let body = b"nitroplus nitroplus nitroplus".to_vec();
    let packed = zlib(&body);
    let mut table = Vec::new();
    for (name, offset, size, compressed, stored) in [
        (
            "sys/a.nss",
            0u32,
            body.len() as u32,
            1u32,
            packed.len() as u32,
        ),
        ("b.txt", packed.len() as u32, 2, 0, 2),
    ] {
        table.extend(le(name.len() as u32));
        table.extend(name.as_bytes());
        table.extend(le(offset));
        table.extend(le(size));
        table.extend(le(0));
        table.extend(le(compressed));
        table.extend(le(stored));
    }
    let ztable = zlib(&table);
    let mut pak = le(2).to_vec();
    pak.extend(le(2));
    pak.extend(le(table.len() as u32));
    pak.extend(le(ztable.len() as u32));
    pak.extend(vec![0; 0x104]);
    pak.extend(ztable);
    pak.extend(packed);
    pak.extend(b"ok");
    assert_eq!(
        extract("nitroplus/pak", pak, true),
        vec![("sys/a.nss".into(), body), ("b.txt".into(), b"ok".to_vec())]
    );
}

#[test]
fn fvp_bin_reads_string_pool() {
    let pool = [sjis("立ち絵"), b"\0".to_vec(), b"bgm\0".to_vec()].concat();
    let names_start = 8 + 2 * 12;
    let data_at = names_start + pool.len() as u32;
    let mut bin = le(2).to_vec();
    bin.extend(le(0));
    bin.extend(le(0));
    bin.extend(le(data_at));
    bin.extend(le(2));
    bin.extend(le(7));
    bin.extend(le(data_at + 2));
    bin.extend(le(1));
    bin.extend(pool);
    bin.extend(b"NVm");
    assert_eq!(
        extract("fvp/bin", bin, false),
        pairs(&[("立ち絵", b"NV"), ("bgm", b"m")])
    );
}

#[test]
fn yuris_ypf_guesses_name_table_and_key() {
    let key = 0x5Au8;
    let mut table = Vec::new();
    let body = b"ybn script".to_vec();
    let packed = zlib(&body);
    let data_at = 0x20u32;
    let mut entries = Vec::new();
    for (name, compressed, size, stored) in [
        (
            "ysbin/yst00000.ybn",
            1u8,
            body.len() as u32,
            packed.len() as u32,
        ),
        ("cg.png", 0, 3, 3),
    ] {
        entries.push((name, compressed, size, stored));
    }
    let mut offset = 0u32;
    let mut offsets = Vec::new();
    for (_, _, _, stored) in &entries {
        offsets.push(offset);
        offset += stored;
    }
    for ((name, compressed, size, stored), rel) in entries.iter().zip(&offsets) {
        // Name lengths below 0x03 are untouched by every swap table.
        let mut n = name.len() as u8;
        if n == 0x12 {
            n = 0x12;
        }
        table.extend(le(0));
        table.push(n ^ 0xFF);
        table.extend(name.bytes().map(|b| b ^ key));
        table.push(0);
        table.push(*compressed);
        table.extend(le(*size));
        table.extend(le(*stored));
        table.extend(le(*rel)); // patched below
        table.extend(le(0));
    }
    let base = data_at + table.len() as u32;
    let mut c = 0usize;
    for (name, rel) in entries.iter().map(|e| e.0).zip(&offsets) {
        c += 4 + 1 + name.len() + 2 + 8;
        table[c..c + 4].copy_from_slice(&le(base + rel));
        c += 8;
    }
    let mut ypf = b"YPF\0".to_vec();
    ypf.extend(le(0x100));
    ypf.extend(le(2));
    ypf.extend(le(table.len() as u32));
    ypf.resize(0x20, 0);
    ypf.extend(table);
    ypf.extend(packed);
    ypf.extend(b"PNG");
    assert_eq!(
        extract("yuris/ypf", ypf, true),
        vec![
            ("ysbin/yst00000.ybn".into(), body),
            ("cg.png".into(), b"PNG".to_vec())
        ]
    );
}

#[test]
fn small_signature_formats() {
    // YKC
    let mut ykc = b"YKC001".to_vec();
    ykc.resize(16, 0);
    ykc.extend(le(24));
    ykc.extend(le(20));
    ykc.extend(le(44));
    ykc.extend(le(6));
    ykc.extend(le(50));
    ykc.extend(le(2));
    ykc.extend(le(0));
    ykc.extend(b"a.yks\0");
    ykc.extend(b"hi");
    assert_eq!(
        extract("yuka-script/ykc", ykc, true),
        pairs(&[("a.yks", b"hi")])
    );

    // Triangle MED
    let mut med = b"MD\0\0".to_vec();
    med.extend(16u16.to_le_bytes());
    med.extend(1u16.to_le_bytes());
    med.extend([0; 8]);
    med.extend(fixed(b"SCN01", 8));
    med.extend(le(3));
    med.extend(le(32));
    med.extend(b"med");
    assert_eq!(
        extract("triangle/med", med, true),
        pairs(&[("SCN01", b"med")])
    );

    // KISS PLG
    let mut plg = b"LIB\0\0\0\x01\0".to_vec();
    plg.extend(le(1));
    plg.extend(le(0));
    plg.extend(fixed(b"x.bmp", 0x20));
    plg.extend(le(0));
    plg.extend(le(2));
    plg.extend(le(64));
    plg.extend(le(0));
    plg.extend(b"BM");
    assert_eq!(extract("kiss/plg", plg, true), pairs(&[("x.bmp", b"BM")]));

    // Liar-soft XFL and LWG
    let mut xfl = b"LB\x01\0".to_vec();
    xfl.extend(le(0x28));
    xfl.extend(le(1));
    xfl.extend(fixed(&sjis("文字.gsc"), 0x20));
    xfl.extend(le(0));
    xfl.extend(le(3));
    xfl.extend(b"gsc");
    assert_eq!(
        extract("liar-soft/xfl", xfl, true),
        pairs(&[("文字.gsc", b"gsc")])
    );
    let mut lwg = b"LG\x01\0".to_vec();
    lwg.extend(le(640));
    lwg.extend(le(480));
    lwg.extend(le(1));
    lwg.extend(le(0));
    let rec = [
        vec![0; 9],
        le(0).to_vec(),
        le(2).to_vec(),
        vec![3],
        b"a01".to_vec(),
    ]
    .concat();
    lwg.extend(le(rec.len() as u32));
    lwg.extend(rec);
    lwg.extend(le(0));
    lwg.extend(b"WC");
    assert_eq!(
        extract("liar-soft/lwg", lwg, true),
        pairs(&[("a01", b"WC")])
    );

    // N-System FJSYS
    let mut fj = b"FJSYS\0\0\0".to_vec();
    let header_size = 84 + 16 + 6;
    fj.extend(le(header_size));
    fj.extend(le(6));
    fj.extend(le(1));
    fj.extend([0; 64]);
    fj.extend(le(0));
    fj.extend(le(2));
    fj.extend((header_size as u64).to_le_bytes());
    fj.extend(b"s.mgd\0");
    fj.extend(b"MG");
    assert_eq!(
        extract("nsystem/fjsys", fj, true),
        pairs(&[("s.mgd", b"MG")])
    );
}

#[test]
fn signatureless_formats_need_explicit_ids() {
    // KISS ARC: sizes from successive offsets.
    let mut arc = le(2).to_vec();
    arc.extend(b"a\0");
    arc.extend(le(24));
    arc.extend(le(0));
    arc.extend(b"b\0");
    arc.extend(le(26));
    arc.extend(le(0));
    arc.extend(b"AAB");
    assert_eq!(
        extract("kiss/arc", arc, false),
        pairs(&[("a", b"AA"), ("b", b"B")])
    );

    // Propeller MPK: name, offset and size XORed by the last name byte.
    let key = 0x33u8;
    let k32 = u32::from_le_bytes([key; 4]);
    let mut mpk = le(8).to_vec();
    mpk.extend(le(1));
    let mut name = fixed(b"\\bg\\a.bmp", 32);
    name[31] = 0;
    let mut enc: Vec<u8> = name.iter().map(|b| b ^ key).collect();
    enc[31] = key;
    mpk.extend(enc);
    mpk.extend(le(48 ^ k32));
    mpk.extend(le(2 ^ k32));
    mpk.extend(b"BM");
    assert_eq!(
        extract("propeller/mpk", mpk, false),
        pairs(&[("bg/a.bmp", b"BM")])
    );

    // Silky ARC: shifted names, big-endian sizes, bytewise LZSS.
    let body = b"Silky's Plus".to_vec();
    let packed = lzss_literals(&body);
    let name = b"S01.MES";
    let n = name.len();
    let mut table = vec![n as u8];
    table.extend(
        name.iter()
            .enumerate()
            .map(|(j, b)| b.wrapping_sub((n - j) as u8)),
    );
    table.extend((packed.len() as u32).to_be_bytes());
    table.extend((body.len() as u32).to_be_bytes());
    let data_at = 4 + table.len() as u32 + 4;
    table.extend(data_at.to_be_bytes());
    let mut silky = le(table.len() as u32).to_vec();
    silky.extend(table);
    silky.extend(packed);
    assert_eq!(
        extract("silky/arc", silky, false),
        vec![("S01.MES".into(), body)]
    );

    let r = registry();
    let cx = DecodeContext::new(ClientId(1));
    assert!(r
        .open(
            memory(b"not an archive at all".to_vec()),
            Some("alice-soft/ald"),
            &cx
        )
        .is_err());
}

#[test]
fn malformed_tables_fail_visibly() {
    let r = registry();
    let cx = DecodeContext::new(ClientId(1));
    let mut alk = b"ALK0".to_vec();
    alk.extend(le(1));
    alk.extend(le(0));
    alk.extend(le(999));
    assert!(r.open(memory(alk), Some("alice-soft/alk"), &cx).is_err());
    let mut lnk = b"LNK\0".to_vec();
    lnk.extend(le(u32::MAX));
    lnk.extend([0; 8]);
    assert!(r.open(memory(lnk), Some("kid/lnk"), &cx).is_err());
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

#[test]
fn group_c_signature_formats() {
    // Active Soft ADPACK32: count is stored plus one; sizes from offsets.
    let adpack = cat(&[
        b"ADPACK32",
        &le(0),
        &le(3),
        &fixed(b"a.bmp", 0x18),
        &le(0),
        &le(0x50),
        &fixed(b"b", 0x18),
        &le(0),
        &le(0x52),
        b"AAB",
    ]);
    assert_eq!(
        extract("active-soft/adpack", adpack, true),
        pairs(&[("a.bmp", b"AA"), ("b", b"B")])
    );

    // AST ARC2: inverted names; LZSS + inversion; RIFF payloads not inverted.
    let script = b"script!!".to_vec();
    let inv: Vec<u8> = script.iter().map(|b| !b).collect();
    let packed = lzss_literals(&inv);
    let name1: Vec<u8> = b"s.txt".iter().map(|b| !b).collect();
    let name2: Vec<u8> = b"v.wav".iter().map(|b| !b).collect();
    let table_len = 2 * 9 + 10;
    let at1 = 8 + table_len as u32;
    let at2 = at1 + packed.len() as u32;
    let ast = cat(&[
        b"ARC2",
        &le(2),
        &le(at1),
        &le(script.len() as u32),
        &[5],
        &name1,
        &le(at2),
        &le(6),
        &[5],
        &name2,
        &packed,
        b"RIFFxx",
    ]);
    assert_eq!(
        extract("ast/arc", ast, true),
        vec![
            ("s.txt".into(), script),
            ("v.wav".into(), b"RIFFxx".to_vec())
        ]
    );

    // AVG GXP: per-entry key ^ position.
    let name: Vec<u8> = "ｇ.png".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let rec_len = 32 + name.len() as u32;
    let rec = cat(&[
        &le(rec_len),
        &le(4),
        &le(0),
        &le(5),
        &[0; 8],
        &0u64.to_le_bytes(),
        &name,
    ]);
    let crypt = |d: &[u8]| -> Vec<u8> {
        let key = b"\x40\x21\x28\x38\xA6\x6E\x43\xA5\x40\x21\x28\x38\xA6\x43\xA5\x64\x3E\x65\x24\x20\x46\x6E\x74";
        d.iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % 23] ^ i as u8)
            .collect()
    };
    let data_offset = 48 + rec.len() as u64;
    let mut gxp = b"GXP\0".to_vec();
    gxp.resize(24, 0);
    gxp.extend(cat(&[
        &le(1),
        &le(rec.len() as u32),
        &[0; 8],
        &data_offset.to_le_bytes(),
        &crypt(&rec),
        &crypt(b"PNG!"),
    ]));
    assert_eq!(extract("avg/gxp", gxp, true), pairs(&[("ｇ.png", b"PNG!")]));

    // Bishop BSA v2 with a nested directory; directory records are 32 bytes.
    let data_at = 16 + 32 + 40 + 32 + 40;
    let bsa = cat(&[
        b"BSArc\0\0\0",
        &2u16.to_le_bytes(),
        &4u16.to_le_bytes(),
        &le(16),
        &fixed(b">cg", 32),
        &fixed(b"a.bsg", 32),
        &le(data_at),
        &le(1),
        &fixed(b"<", 32),
        &fixed(b"top", 32),
        &le(data_at + 1),
        &le(2),
        b"Xyz",
    ]);
    assert_eq!(
        extract("bishop/bsa", bsa, true),
        pairs(&[("cg/a.bsg", b"X"), ("top", b"yz")])
    );

    // GS DataPack5: XOR(i & key) over a bytewise-LZSS table.
    let key = 0x0F;
    let mut table = cat(&[&fixed(b"bg01.bmp", 0x40), &le(0), &le(3)]);
    table.resize(0x48, 0);
    let mut packed = lzss_literals(&table);
    for (i, b) in packed.iter_mut().enumerate() {
        *b ^= (i as u32 & key) as u8;
    }
    let mut pak = fixed(b"DataPack5", 0x30);
    let table_at = 0x48u32;
    let data_at = table_at + packed.len() as u32;
    pak.extend(cat(&[
        &le(0x0004_0000),
        &le(packed.len() as u32),
        &le(key),
        &le(1),
        &le(data_at),
        &le(table_at),
    ]));
    pak.extend(&packed);
    pak.extend(b"BMP");
    assert_eq!(extract("gs/pak", pak, true), pairs(&[("bg01.bmp", b"BMP")]));

    // GS GsSYMBOL5BINDATA: numbered LZSS entries.
    let body = b"symbol".to_vec();
    let entry = lzss_literals(&body);
    let table = cat(&[
        &le(0),
        &le(entry.len() as u32),
        &le(body.len() as u32),
        &[0; 12],
    ]);
    let tpacked = lzss_literals(&table);
    let mut dat = fixed(b"GsSYMBOL5BINDATA", 0xA8);
    let table_at = 0xA8 + 36;
    let data_at = table_at + tpacked.len() as u32;
    dat.extend(cat(&[
        &le(1),
        &[0; 12],
        &le(table_at),
        &le(tpacked.len() as u32),
        &le(0),
        &le(table.len() as u32),
        &le(data_at),
    ]));
    dat.extend(&tpacked);
    dat.extend(&entry);
    assert_eq!(
        extract("gs/dat", dat, true),
        vec![("00000.dat".into(), body)]
    );

    // ISM, MAGES, Riddle, sysadv, yumemiru, MYK, ACP, GPK2
    let isa = cat(&[
        b"ISM ARCHIVED",
        &1u16.to_le_bytes(),
        &[0, 0],
        &fixed(b"a.isf", 12),
        &le(0),
        &le(48),
        &le(2),
        &[0; 8],
        b"ok",
    ]);
    assert_eq!(extract("ism/isa", isa, true), pairs(&[("a.isf", b"ok")]));
    let mut mpk = cat(&[b"MPK\0", &0u16.to_le_bytes(), &2u16.to_le_bytes(), &le(2)]);
    mpk.resize(64, 0);
    mpk.extend(vec![0; 256]); // alignment entry with zero offset and size
    mpk.extend(cat(&[
        &[0; 8],
        &576u64.to_le_bytes(),
        &2u64.to_le_bytes(),
        &2u64.to_le_bytes(),
        &fixed(b"script/a.scx", 224),
        b"SC",
    ]));
    assert_eq!(
        extract("mages/mpk", mpk, true),
        pairs(&[("script/a.scx", b"SC")])
    );
    let pac = cat(&[
        b"PAC1",
        &le(2),
        &fixed(b"a", 16),
        &le(1),
        b"pref",
        &le(7),
        &le(7),
        &fixed(b"b", 16),
        &le(2),
        b"pref",
        &le(0),
        &le(0),
        b"ABC",
    ]);
    assert_eq!(
        extract("riddle-soft/pac", pac, true),
        pairs(&[("a", b"A"), ("b", b"BC")])
    );
    let sys = cat(&[
        b"\x05PACK2",
        &le(1),
        &[3],
        &[!b'a', !b'.', !b'b'],
        &le(22),
        &le(1),
        b"Q",
    ]);
    assert_eq!(extract("sysadv/pak", sys, true), pairs(&[("a.b", b"Q")]));
    let yume = cat(&[
        b"PackDat3",
        &le(2),
        &fixed(b"a", 0x100),
        &le(12 + 2 * 0x10C),
        &le(1),
        &le(1),
        &fixed(b"z", 0x100),
        &le(12 + 2 * 0x10C + 1),
        &le(9),
        &le(1),
        b"AZ",
    ]);
    let r = registry();
    let cx = DecodeContext::new(ClientId(2));
    let (_, archive) = r.open(memory(yume), None, &cx).unwrap();
    assert_eq!(
        read_all(archive.open(0, &cx).unwrap().as_ref(), 8).unwrap(),
        b"A"
    );
    assert!(
        archive.open(1, &cx).is_err(),
        "compressed Yumemiru entries fail visibly"
    );
    let myk = cat(&[
        b"MYK00\x1A\0\0",
        &1u16.to_le_bytes(),
        &le(18),
        &[0, 0],
        b"ab",
        &fixed(b"x.dat", 12),
        &le(2),
    ]);
    assert_eq!(
        extract("cherry-soft/myk", myk, true),
        pairs(&[("x.dat", b"ab")])
    );
    let acp = cat(&[
        b"ACPXPK01",
        &le(1),
        &fixed(b"bg/a.bmp", 32),
        &le(52),
        &le(1),
        b"M",
    ]);
    assert_eq!(
        extract("escude/acp-pk1", acp, true),
        pairs(&[("bg/a.bmp", b"M")])
    );
    let gpk = cat(&[
        b"GPK2",
        &le(10),
        b"hi",
        &le(1),
        &le(8),
        &le(2),
        &fixed(b"n.txt", 0x80),
    ]);
    assert_eq!(extract("gpk2/gpk2", gpk, true), pairs(&[("n.txt", b"hi")]));
}

#[test]
fn group_c_structural_and_keyed_formats() {
    // ADV DAT: key recovered from "ARCHIVE\0" plaintext.
    let key = *b"k3y!K3Y?";
    let header_offset = 81u32;
    let table_offset = 0x24u32;
    let mut header = vec![0u8; 0x24];
    header[..8].copy_from_slice(b"ARCHIVE\0");
    header[16..20].copy_from_slice(&le(table_offset));
    header[32..36].copy_from_slice(&le(1));
    let data_rel = table_offset + 0x114;
    let mut rec = vec![0u8; 0x114];
    rec[1..6].copy_from_slice(b"a.txt");
    rec[0x108..0x10C].copy_from_slice(&le(data_rel));
    rec[0x10C..0x110].copy_from_slice(&le(3));
    let mut body = Vec::new();
    body.extend(&header);
    body.extend(&rec);
    body.extend(b"adv");
    let enc: Vec<u8> = body
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % 8])
        .collect();
    let mut adv = vec![0u8; 53];
    adv.extend(vec![0; 8 + 20]);
    adv.extend(enc);
    // The data's own size is excluded from the probe's table equation.
    let probe_ok = adv.len() - 3;
    let out = extract("adv/dat", adv.clone(), false);
    assert_eq!(out, pairs(&[("a.txt", b"adv")]));
    adv.truncate(probe_ok);
    let r = registry();
    let found: Vec<_> = r
        .probe(memory(adv), &DecodeContext::new(ClientId(1)))
        .unwrap()
        .into_iter()
        .map(|m| m.0)
        .collect();
    assert!(found.contains(&"adv/dat"));
    let _ = header_offset;

    // GSD GSP, Libido ARC, Lilim AOS2/DPK: recognised by size equations.
    let gsp = cat(&[&le(1), &le(68), &le(2), &fixed(b"a.bmp", 0x38), b"GS"]);
    assert_eq!(extract("gsd/gsp", gsp, true), pairs(&[("a.bmp", b"GS")]));
    let body = b"libido".to_vec();
    let packed = lzss_literals(&body);
    let mut name = fixed(b"x.bmp", 20);
    name[19] = 0x21;
    let name: Vec<u8> = name
        .iter()
        .enumerate()
        .map(|(i, b)| if i == 19 { *b } else { b ^ 0x21 })
        .collect();
    let lib = cat(&[
        &le(1),
        &name,
        &le(body.len() as u32),
        &le(packed.len() as u32),
        &le(36),
        &packed,
    ]);
    assert_eq!(
        extract("libido/arc", lib, true),
        vec![("x.bmp".into(), body)]
    );
    let aos2 = cat(&[
        &le(0),
        &le(12 + 0x28),
        &le(0x28),
        &fixed(b"s.scr", 0x20),
        &le(0),
        &le(2),
        b"OK",
    ]);
    assert_eq!(
        extract("lilim/aos2", aos2, true),
        pairs(&[("s.scr", b"OK")])
    );
    let dpk = cat(&[
        b"PA",
        &1u16.to_le_bytes(),
        &le(8 + 0x14 + 1),
        &fixed(b"d", 0x10),
        &le(1),
        b"D",
    ]);
    assert_eq!(extract("lilim/dpk", dpk, true), pairs(&[("d", b"D")]));
}

#[test]
fn group_c_explicit_formats() {
    // Lilim AOS1 with a skipped 0xFF block.
    let mut skip = vec![0xFF];
    skip.resize(16, 0);
    let aos = cat(&[
        &skip,
        &le(3),
        &[0; 12],
        b"zzz",
        &fixed(b"a.abm", 16),
        &le(16),
        &le(2),
        &[0; 8],
        &[0; 16],
        b"AB",
    ]);
    let out = extract("lilim/aos1", aos, false);
    assert_eq!(out, pairs(&[("a.abm", b"AB")]));

    // Tabito, Crowd PCK, Dogenzaka (both layouts), GLib GML.
    let tab = cat(&[&le(2), &le(0), &3u16.to_le_bytes(), b"a.b", b"TB", &le(0)]);
    assert_eq!(extract("tabito/dat", tab, false), pairs(&[("a.b", b"TB")]));
    let pck = cat(&[
        &le(2),
        &le(0),
        &le(36),
        &le(1),
        &le(0),
        &le(37),
        &le(2),
        &sjis("一.ogg"),
        b"\0b\0",
        b"OGG",
    ]);
    let pck = {
        let mut v = pck;
        let names_end = 4 + 24 + sjis("一.ogg").len() + 3;
        v[8..12].copy_from_slice(&le(names_end as u32));
        v[20..24].copy_from_slice(&le(names_end as u32 + 1));
        v
    };
    assert_eq!(
        extract("crowd/pck", pck, false),
        pairs(&[("一.ogg", b"O"), ("b", b"GG")])
    );
    let dz = cat(&[&le(2), &le(0), &le(1), b"ABC"]);
    assert_eq!(
        extract("dogenzaka/bin", dz, false),
        pairs(&[("unk_0.dat", b"A"), ("unk_1.dat", b"BC")])
    );
    let dz2 = cat(&[&le(1), &le(0), &le(14), b"q"]);
    assert_eq!(extract("dogenzaka/bin", dz2, false).len(), 1);

    let mut perm: Vec<u8> = (0..=255u8).collect();
    perm.reverse();
    let table = cat(&[&perm, &le(1), &le(5), b"a.pgx", &le(0), &le(6), b"PGX\0"]);
    let packed: Vec<u8> = lzss_literals(&table).iter().map(|b| !b).collect();
    let data_start = 20 + packed.len() as u32;
    let gml = cat(&[
        b"GML_ARC\0",
        &le(data_start),
        &le(table.len() as u32),
        &le(packed.len() as u32),
        &packed,
        b"PGX\0",
        &[!b'h', !b'i'],
    ]);
    assert_eq!(
        extract("glib/gml", gml, true),
        pairs(&[("a.pgx", b"PGX\0hi")])
    );
}
