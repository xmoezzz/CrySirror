//! WOFF 1.0 and WOFF 2.0 web-font containers decoded to their sfnt
//! (TrueType/OpenType) font, after the W3C WOFF File Format 1.0 and 2.0
//! recommendations: zlib-compressed tables (WOFF), and the Brotli stream
//! with the `glyf`/`loca` and `hmtx` table transforms (WOFF2). Font
//! collections inside WOFF2 (`ttcf` flavor) are rejected.
use anyhow::{Context, Result, bail, ensure};
use std::io::Read;

const WOFF: u32 = u32::from_be_bytes(*b"wOFF");
const WOFF2: u32 = u32::from_be_bytes(*b"wOF2");
const TTCF: u32 = u32::from_be_bytes(*b"ttcf");
/// Upper bound of a decoded font (matches the font source byte limit).
const MAX_SFNT: usize = crate::dynamic::MAX_FONT_BYTES;

/// The sfnt font in `bytes` when it is a WOFF or WOFF2 container.
pub fn decode(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    match bytes.get(..4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])) {
        Some(WOFF) => woff1(bytes).map(Some),
        Some(WOFF2) => woff2(bytes).map(Some),
        _ => Ok(None),
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.b.len()).context("truncated web font")?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// `UIntBase128`.
    fn base128(&mut self) -> Result<u32> {
        let mut v: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            ensure!(!(i == 0 && b == 0x80), "WOFF2 UIntBase128 with a leading zero");
            ensure!(v & 0xFE00_0000 == 0, "WOFF2 UIntBase128 overflow");
            v = (v << 7) | u32::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        bail!("WOFF2 UIntBase128 longer than 5 bytes")
    }
    /// `255UInt16`.
    fn u255(&mut self) -> Result<u16> {
        Ok(match self.u8()? {
            253 => self.u16()?,
            255 => 253 + u16::from(self.u8()?),
            254 => 506 + u16::from(self.u8()?),
            c => u16::from(c),
        })
    }
}

struct Table {
    tag: u32,
    data: Vec<u8>,
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for c in data.chunks(4) {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        sum = sum.wrapping_add(u32::from_be_bytes(w));
    }
    sum
}

/// Builds the sfnt: offset table, sorted table records, 4-byte aligned
/// table data and the `head` checksum adjustment.
fn assemble(flavor: u32, mut tables: Vec<Table>) -> Result<Vec<u8>> {
    tables.sort_by_key(|t| t.tag);
    let n = tables.len() as u16;
    let mut entry_selector = 0u16;
    while (2u32 << entry_selector) <= u32::from(n) {
        entry_selector += 1;
    }
    let search_range = 16u16 << entry_selector;
    let mut out = Vec::new();
    out.extend_from_slice(&flavor.to_be_bytes());
    for v in [n, search_range, entry_selector, n * 16 - search_range] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    let mut head_offset = None;
    for t in &mut tables {
        if t.tag == u32::from_be_bytes(*b"head") && t.data.len() >= 12 {
            t.data[8..12].fill(0);
            head_offset = Some(offset);
        }
        out.extend_from_slice(&t.tag.to_be_bytes());
        out.extend_from_slice(&checksum(&t.data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(t.data.len() as u32).to_be_bytes());
        offset += t.data.len().next_multiple_of(4);
    }
    for t in &tables {
        out.extend_from_slice(&t.data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    ensure!(out.len() <= MAX_SFNT, "decoded web font exceeds the font byte limit");
    if let Some(h) = head_offset {
        let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[h + 8..h + 12].copy_from_slice(&adj.to_be_bytes());
    }
    Ok(out)
}

// WOFF 1.0 -------------------------------------------------------------------------------

fn woff1(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(bytes);
    r.u32()?;
    let flavor = r.u32()?;
    let length = r.u32()? as usize;
    ensure!(length == bytes.len(), "WOFF length does not match the file");
    let num_tables = r.u16()?;
    r.u16()?;
    let total = r.u32()? as usize;
    ensure!(total <= MAX_SFNT, "WOFF font exceeds the font byte limit");
    r.bytes(24)?;
    let mut tables = Vec::with_capacity(usize::from(num_tables));
    for _ in 0..num_tables {
        let tag = r.u32()?;
        let offset = r.u32()? as usize;
        let comp = r.u32()? as usize;
        let orig = r.u32()? as usize;
        r.u32()?;
        ensure!(orig <= MAX_SFNT && comp <= orig, "invalid WOFF table lengths");
        let src = bytes.get(offset..offset + comp).context("WOFF table outside the file")?;
        let data = if comp == orig {
            src.to_vec()
        } else {
            let mut d = Vec::with_capacity(orig);
            flate2_free_inflate(src, &mut d, orig)?;
            d
        };
        ensure!(data.len() == orig, "WOFF table decompressed to the wrong length");
        tables.push(Table { tag, data });
    }
    assemble(flavor, tables)
}

/// zlib stream -> at most `limit` bytes.
fn flate2_free_inflate(src: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<()> {
    let d = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(src, limit)
        .map_err(|e| anyhow::anyhow!("WOFF zlib stream: {e:?}"))?;
    *out = d;
    Ok(())
}

// WOFF 2.0 -------------------------------------------------------------------------------

const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm", b"glyf", b"loca",
    b"prep", b"CFF ", b"VORG", b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern", b"LTSH", b"PCLT", b"VDMX", b"vhea",
    b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC", b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL",
    b"SVG ", b"sbix", b"acnt", b"avar", b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar",
    b"gvar", b"hsty", b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat",
    b"Gloc", b"Feat", b"Sill",
];

fn tag(t: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*t)
}

struct Entry {
    tag: u32,
    version: u8,
    orig: usize,
    stored: usize,
}

impl Entry {
    fn transformed(&self) -> bool {
        if self.tag == tag(b"glyf") || self.tag == tag(b"loca") {
            self.version == 0
        } else {
            self.version != 0
        }
    }
}

fn woff2(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(bytes);
    r.u32()?;
    let flavor = r.u32()?;
    ensure!(flavor != TTCF, "WOFF2 font collections are not supported");
    let length = r.u32()? as usize;
    ensure!(length == bytes.len(), "WOFF2 length does not match the file");
    let num_tables = r.u16()?;
    r.u16()?;
    let total_sfnt = r.u32()? as usize;
    ensure!(total_sfnt <= MAX_SFNT, "WOFF2 font exceeds the font byte limit");
    let compressed = r.u32()? as usize;
    r.bytes(24)?;
    let mut entries = Vec::with_capacity(usize::from(num_tables));
    for _ in 0..num_tables {
        let flags = r.u8()?;
        let t = match flags & 0x3F {
            63 => r.u32()?,
            i => tag(KNOWN_TAGS[usize::from(i)]),
        };
        let version = flags >> 6;
        let orig = r.base128()? as usize;
        let mut e = Entry { tag: t, version, orig, stored: orig };
        if e.transformed() {
            e.stored = r.base128()? as usize;
        }
        ensure!(e.orig <= MAX_SFNT && e.stored <= MAX_SFNT, "invalid WOFF2 table length");
        entries.push(e);
    }
    let stream = r.bytes(compressed)?;
    let expect: usize = entries.iter().map(|e| e.stored).sum();
    ensure!(expect <= MAX_SFNT, "WOFF2 tables exceed the font byte limit");
    let mut data = Vec::with_capacity(expect);
    brotli_decompressor::Decompressor::new(stream, 4096)
        .take(expect as u64 + 1)
        .read_to_end(&mut data)
        .context("WOFF2 Brotli stream")?;
    ensure!(data.len() == expect, "WOFF2 Brotli stream has the wrong length");

    let mut raw = Vec::with_capacity(entries.len());
    let mut at = 0;
    for e in &entries {
        raw.push(&data[at..at + e.stored]);
        at += e.stored;
    }
    let find = |t: &[u8; 4]| entries.iter().position(|e| e.tag == tag(t));
    let mut tables = Vec::with_capacity(entries.len());
    let mut glyf_out: Option<(Vec<u8>, Vec<u8>, Vec<i16>)> = None;
    if let Some(gi) = find(b"glyf").filter(|i| entries[*i].transformed()) {
        ensure!(find(b"loca").is_some_and(|i| entries[i].transformed()), "WOFF2 glyf without a transformed loca");
        glyf_out = Some(decode_glyf(raw[gi])?);
    }
    for (i, e) in entries.iter().enumerate() {
        let d = if e.tag == tag(b"glyf") && e.transformed() {
            glyf_out.as_ref().unwrap().0.clone()
        } else if e.tag == tag(b"loca") && e.transformed() {
            glyf_out.as_ref().context("WOFF2 loca without glyf")?.1.clone()
        } else if e.tag == tag(b"hmtx") && e.transformed() {
            let x_mins = &glyf_out.as_ref().context("WOFF2 hmtx transform without glyf")?.2;
            let hhea = find(b"hhea").map(|i| raw[i]).context("WOFF2 hmtx without hhea")?;
            decode_hmtx(raw[i], hhea, x_mins)?
        } else {
            ensure!(!e.transformed(), "unknown WOFF2 transform of table {:08x}", e.tag);
            raw[i].to_vec()
        };
        tables.push(Table { tag: e.tag, data: d });
    }
    assemble(flavor, tables)
}

fn with_sign(flag: u8, v: i32) -> i32 {
    if flag & 1 != 0 { v } else { -v }
}

/// One WOFF2 point triplet: (dx, dy, on-curve).
fn triplet(flag: u8, g: &mut Reader) -> Result<(i32, i32, bool)> {
    let on = flag & 0x80 == 0;
    let f = flag & 0x7F;
    let (dx, dy) = if f < 10 {
        let b = i32::from(g.u8()?);
        (0, with_sign(f, ((i32::from(f) & 14) << 7) + b))
    } else if f < 20 {
        let b = i32::from(g.u8()?);
        (with_sign(f, (((i32::from(f) - 10) & 14) << 7) + b), 0)
    } else if f < 84 {
        let b0 = i32::from(f) - 20;
        let b1 = i32::from(g.u8()?);
        (with_sign(f, 1 + (b0 & 0x30) + (b1 >> 4)), with_sign(f >> 1, 1 + ((b0 & 0x0C) << 2) + (b1 & 0x0F)))
    } else if f < 120 {
        let b0 = i32::from(f) - 84;
        let (x, y) = (i32::from(g.u8()?), i32::from(g.u8()?));
        (with_sign(f, 1 + ((b0 / 12) << 8) + x), with_sign(f >> 1, 1 + (((b0 % 12) >> 2) << 8) + y))
    } else if f < 124 {
        let b = g.bytes(3)?;
        let (b0, b1, b2) = (i32::from(b[0]), i32::from(b[1]), i32::from(b[2]));
        (with_sign(f, (b0 << 4) + (b1 >> 4)), with_sign(f >> 1, ((b1 & 0x0F) << 8) + b2))
    } else {
        let b = g.bytes(4)?;
        let x = (i32::from(b[0]) << 8) + i32::from(b[1]);
        let y = (i32::from(b[2]) << 8) + i32::from(b[3]);
        (with_sign(f, x), with_sign(f >> 1, y))
    };
    Ok((dx, dy, on))
}

fn push_i16(out: &mut Vec<u8>, v: i32) -> Result<()> {
    ensure!((-32768..=32767).contains(&v), "WOFF2 glyph coordinate out of range");
    out.extend_from_slice(&(v as i16).to_be_bytes());
    Ok(())
}

/// Reconstructs `glyf` and `loca` from the transformed glyf table; also
/// returns each glyph's xMin (for the hmtx transform).
fn decode_glyf(t: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<i16>)> {
    let mut h = Reader::new(t);
    h.u16()?;
    let options = h.u16()?;
    let num_glyphs = usize::from(h.u16()?);
    let index_format = h.u16()?;
    let mut sizes = [0usize; 7];
    for s in &mut sizes {
        *s = h.u32()? as usize;
    }
    let mut streams = Vec::with_capacity(7);
    for s in sizes {
        streams.push(Reader::new(h.bytes(s)?));
    }
    let overlap = if options & 1 != 0 { Some(h.bytes(num_glyphs.div_ceil(8))?) } else { None };
    let [mut n_contour, mut n_points, mut flags, mut glyphs, mut composite, mut bbox, mut instr]: [Reader; 7] =
        streams.try_into().ok().context("WOFF2 glyf streams")?;
    let bitmap_len = 4 * num_glyphs.div_ceil(32);
    let bbox_bitmap = bbox.bytes(bitmap_len)?;
    let has_bbox = |i: usize| bbox_bitmap[i >> 3] & (0x80 >> (i & 7)) != 0;

    let mut glyf = Vec::new();
    let mut loca = Vec::with_capacity((num_glyphs + 1) * 4);
    let mut x_mins = vec![0i16; num_glyphs];
    let mut offsets = Vec::with_capacity(num_glyphs + 1);
    for i in 0..num_glyphs {
        offsets.push(glyf.len());
        let nc = n_contour.i16()?;
        if nc == 0 {
            ensure!(!has_bbox(i), "WOFF2 empty glyph with a bounding box");
            continue;
        }
        if nc < 0 {
            ensure!(has_bbox(i), "WOFF2 composite glyph without a bounding box");
            let b = bbox.bytes(8)?;
            x_mins[i] = i16::from_be_bytes([b[0], b[1]]);
            glyf.extend_from_slice(&(-1i16).to_be_bytes());
            glyf.extend_from_slice(b);
            let start = composite.pos;
            let mut have_instr = false;
            loop {
                let fl = composite.u16()?;
                composite.u16()?;
                let mut n = if fl & 0x0001 != 0 { 4 } else { 2 };
                if fl & 0x0008 != 0 {
                    n += 2;
                } else if fl & 0x0040 != 0 {
                    n += 4;
                } else if fl & 0x0080 != 0 {
                    n += 8;
                }
                composite.bytes(n)?;
                have_instr |= fl & 0x0100 != 0;
                if fl & 0x0020 == 0 {
                    break;
                }
            }
            glyf.extend_from_slice(&composite.b[start..composite.pos]);
            if have_instr {
                let len = glyphs.u255()?;
                glyf.extend_from_slice(&len.to_be_bytes());
                glyf.extend_from_slice(instr.bytes(usize::from(len))?);
            }
        } else {
            let nc = nc as usize;
            let mut end_pts = Vec::with_capacity(nc);
            let mut total = 0usize;
            for _ in 0..nc {
                total += usize::from(n_points.u255()?);
                ensure!(total <= 0xFFFF + 1, "WOFF2 glyph point count overflow");
                end_pts.push((total - 1) as u16);
            }
            let mut pts = Vec::with_capacity(total);
            let (mut x, mut y) = (0i32, 0i32);
            for _ in 0..total {
                let f = flags.u8()?;
                let (dx, dy, on) = triplet(f, &mut glyphs)?;
                x += dx;
                y += dy;
                pts.push((dx, dy, x, y, on));
            }
            let ilen = glyphs.u255()?;
            let ins = instr.bytes(usize::from(ilen))?;
            let bb = if has_bbox(i) {
                let b = bbox.bytes(8)?;
                [0, 2, 4, 6].map(|k| i32::from(i16::from_be_bytes([b[k], b[k + 1]])))
            } else {
                let mut bb = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
                for p in &pts {
                    bb = [bb[0].min(p.2), bb[1].min(p.3), bb[2].max(p.2), bb[3].max(p.3)];
                }
                if pts.is_empty() { [0; 4] } else { bb }
            };
            x_mins[i] = bb[0] as i16;
            glyf.extend_from_slice(&(nc as i16).to_be_bytes());
            for v in bb {
                push_i16(&mut glyf, v)?;
            }
            for e in &end_pts {
                glyf.extend_from_slice(&e.to_be_bytes());
            }
            glyf.extend_from_slice(&ilen.to_be_bytes());
            glyf.extend_from_slice(ins);
            let overlap_first = overlap.is_some_and(|o| o[i >> 3] & (0x80 >> (i & 7)) != 0);
            let (mut fl, mut xs, mut ys) = (Vec::with_capacity(total), Vec::new(), Vec::new());
            for (k, &(dx, dy, _, _, on)) in pts.iter().enumerate() {
                let mut f = u8::from(on);
                if k == 0 && overlap_first {
                    f |= 0x40;
                }
                if dx == 0 {
                    f |= 0x10;
                } else if (-255..=255).contains(&dx) {
                    f |= 0x02 | if dx > 0 { 0x10 } else { 0 };
                    xs.push(dx.unsigned_abs() as u8);
                } else {
                    push_i16(&mut xs, dx)?;
                }
                if dy == 0 {
                    f |= 0x20;
                } else if (-255..=255).contains(&dy) {
                    f |= 0x04 | if dy > 0 { 0x20 } else { 0 };
                    ys.push(dy.unsigned_abs() as u8);
                } else {
                    push_i16(&mut ys, dy)?;
                }
                fl.push(f);
            }
            glyf.extend_from_slice(&fl);
            glyf.extend_from_slice(&xs);
            glyf.extend_from_slice(&ys);
        }
        glyf.resize(glyf.len().next_multiple_of(4), 0);
        ensure!(glyf.len() <= MAX_SFNT, "WOFF2 glyf exceeds the font byte limit");
    }
    offsets.push(glyf.len());
    for o in offsets {
        if index_format == 0 {
            ensure!(o / 2 <= 0xFFFF, "WOFF2 glyf too large for a short loca");
            loca.extend_from_slice(&((o / 2) as u16).to_be_bytes());
        } else {
            loca.extend_from_slice(&(o as u32).to_be_bytes());
        }
    }
    Ok((glyf, loca, x_mins))
}

/// Reconstructs `hmtx`; absent left side bearings are the glyphs' xMin.
fn decode_hmtx(t: &[u8], hhea: &[u8], x_mins: &[i16]) -> Result<Vec<u8>> {
    let n_hm = usize::from(u16::from_be_bytes(hhea.get(34..36).context("short hhea")?.try_into().unwrap()));
    let n = x_mins.len();
    ensure!(n_hm >= 1 && n_hm <= n, "WOFF2 hmtx metric count");
    let mut r = Reader::new(t);
    let flags = r.u8()?;
    let mut adv = Vec::with_capacity(n_hm);
    for _ in 0..n_hm {
        adv.push(r.u16()?);
    }
    let mut out = Vec::with_capacity(4 * n_hm + 2 * (n - n_hm));
    let mut lsb = Vec::with_capacity(n);
    for i in 0..n_hm {
        lsb.push(if flags & 1 == 0 { r.i16()? } else { x_mins[i] });
    }
    for i in n_hm..n {
        lsb.push(if flags & 2 == 0 { r.i16()? } else { x_mins[i] });
    }
    for i in 0..n_hm {
        out.extend_from_slice(&adv[i].to_be_bytes());
        out.extend_from_slice(&lsb[i].to_be_bytes());
    }
    for l in &lsb[n_hm..] {
        out.extend_from_slice(&l.to_be_bytes());
    }
    Ok(out)
}
