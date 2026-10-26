// SPDX-License-Identifier: GPL-3.0-or-later
// Formats translated from arc_unpacker, Copyright (C) 2016 rr-.
//! Engine file transforms. Caller keys are client-scoped; no title key tables.
pub(crate) mod compression;
pub(crate) mod wpx;
use crate::common::Cursor;
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::service::AssetReader;
use crysirror_resource::{
    read_all, read_exact_at, DecodeContext, DecodedFile, FileFormat, FileKind, FileRegistry, Probe,
};
use std::{io::Read, sync::Arc};
const IDS: &[&str] = &[
    "alice-soft/aff",
    "amuse-craft/bgm",
    "aoi/aog",
    "bgi/audio",
    "bgi/bse",
    "bgi/dsc",
    "complets/vmd",
    "crowd/eog",
    "dogenzaka/a",
    "eagls/pak-script",
    "escude/acp",
    "kaguya/bmr",
    "kid/cps",
    "kid/lnd",
    "leaf/g",
    "leaf/pak2-audio",
    "leaf/pak2-compressed-file",
    "lilim/doj",
    "lilim/scr",
    "purple-software/ps2",
    "scene-player/pmw",
    "shiina-rio/ogv",
    "twilight-frontier/tfcs",
    "vorbis/wav",
    "wild-bug/wbi",
];
pub fn file_format_ids() -> Vec<&'static str> {
    IDS.to_vec()
}
pub fn register_files(registry: &mut FileRegistry) -> Result<()> {
    for &id in IDS {
        registry.register(Arc::new(Format(id)))?;
    }
    Ok(())
}
struct Format(&'static str);
impl FileFormat for Format {
    fn id(&self) -> &'static str {
        self.0
    }
    fn kind(&self) -> FileKind {
        FileKind::File
    }
    fn probe(&self, path: &str, source: &dyn AssetReader) -> Result<Probe> {
        let len = source.len().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut d = vec![0; len.min(4096) as usize];
        read_exact_at(source, 0, &mut d)?;
        Ok(if recognized(self.0, path, &d, len as usize) {
            Probe::YES
        } else {
            Probe::NO
        })
    }
    fn decode(
        &self,
        path: &str,
        source: Arc<dyn AssetReader>,
        cx: &DecodeContext,
    ) -> Result<DecodedFile> {
        let d = read_all(source.as_ref(), cx.limits.max_entry_size)?;
        ensure!(
            recognized(self.0, path, &d, d.len()) || (self.0 == "dogenzaka/a" && ext(path, "a")),
            "file does not match {}",
            self.0
        );
        let mut extension = String::new();
        let reader = cx.decode_bounded(cx.limits.max_entry_size, || {
            let (out, e) = decode(self.0, path, &d, cx)?;
            extension = e.into();
            Ok(out)
        })?;
        Ok(DecodedFile {
            reader,
            extension,
            metadata: serde_json::json!({"format":self.0}),
        })
    }
}
pub(crate) fn ext(path: &str, e: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case(e))
}
pub(crate) fn at(d: &[u8], p: usize, n: usize) -> Result<&[u8]> {
    d.get(p..p.checked_add(n).context("file range overflow")?)
        .context("truncated file")
}
pub(crate) fn u16at(d: &[u8], p: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(at(d, p, 2)?.try_into().unwrap()))
}
pub(crate) fn u32at(d: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(at(d, p, 4)?.try_into().unwrap()))
}
pub(crate) fn checked_size(n: usize, cx: &DecodeContext) -> Result<usize> {
    ensure!(n as u64 <= cx.limits.max_entry_size, "decoded byte limit");
    Ok(n)
}
pub(crate) fn guess(d: &[u8]) -> &'static str {
    if d.starts_with(b"OggS") {
        "ogg"
    } else if d.starts_with(b"RIFF") {
        "wav"
    } else if d.starts_with(b"\x89PNG") {
        "png"
    } else if d.starts_with(b"BM") {
        "bmp"
    } else if d.starts_with(b"\xff\xd8") {
        "jpg"
    } else if d.starts_with(b"\xff\xfb") || d.starts_with(b"ID3") {
        "mp3"
    } else {
        "dat"
    }
}
fn recognized(id: &str, path: &str, d: &[u8], len: usize) -> bool {
    let magic: Option<&[u8]> = match id {
        "alice-soft/aff" => Some(b"AFF\0"),
        "amuse-craft/bgm" => Some(b"BGM "),
        "aoi/aog" => {
            return d.starts_with(b"AoiOgg") || (ext(path, "aog") && d.starts_with(b"OggS"))
        }
        "bgi/audio" => return d.get(4..8) == Some(b"bw  "),
        "bgi/bse" => Some(b"BSE 1.0\0"),
        "bgi/dsc" => Some(b"DSC FORMAT 1.00\0"),
        "complets/vmd" => Some(&[0x1a, 0x1e]),
        "crowd/eog" => Some(b"CRM\0"),
        "dogenzaka/a" => Some(&[0x1a, 0xf6, 0xf7, 0xc4]),
        "eagls/pak-script" => {
            return ext(path, "dat")
                && len >= 3601
                && d.len() >= 3600
                && (0..100).all(|i| {
                    let n = &d[i * 36..i * 36 + 32];
                    n.iter()
                        .position(|v| *v == 0)
                        .is_some_and(|p| n[p..].iter().all(|v| *v == 0))
                })
        }
        "escude/acp" => Some(b"acp\0"),
        "kaguya/bmr" => Some(b"BMR"),
        "kid/cps" => Some(b"CPS\0"),
        "kid/lnd" => Some(b"lnd\0"),
        "leaf/g" => return ext(path, "g") && d.get(4..14) == Some(&[0, 2, 0, 0, 0, 0, 0, 0, 0, 0]),
        "leaf/pak2-audio" => return d.get(4..8) == Some(&[3, 0x95, 0xad, 0x4b]),
        "leaf/pak2-compressed-file" => return d.get(4..8) == Some(&[0xaf, 0xf6, 0x4d, 0x4f]),
        "lilim/doj" => return ext(path, "doj") && d.starts_with(b"CC"),
        "lilim/scr" => return ext(path, "scr"),
        "purple-software/ps2" => {
            return d.starts_with(b"PS2A") && u32at(d, 36).ok() != u32at(d, 40).ok()
        }
        "scene-player/pmw" => return ext(path, "pmw"),
        "shiina-rio/ogv" => Some(b"OGV\0"),
        "twilight-frontier/tfcs" => Some(b"TFCS\0"),
        "vorbis/wav" => {
            return ext(path, "wav")
                && u32at(d, 16)
                    .ok()
                    .is_some_and(|n| d.get(40 + n as usize..44 + n as usize) == Some(b"OggS"))
        }
        "wild-bug/wbi" => Some(b"WPX\x1aEX2\0"),
        _ => None,
    };
    magic.is_some_and(|m| d.starts_with(m))
}
pub(crate) fn inflate(d: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(d)
        .take(limit as u64 + 1)
        .read_to_end(&mut out)?;
    ensure!(out.len() <= limit, "inflated byte limit");
    Ok(out)
}
pub(crate) fn key(
    cx: &DecodeContext,
    id: &str,
    default: &str,
) -> Result<crysirror_resource::KeyMaterial> {
    let name = cx
        .parameters
        .get(id)
        .and_then(|v| v.get("key_id"))
        .and_then(|v| v.as_str())
        .unwrap_or(default);
    cx.keys.key(cx.client, name)
}
fn decode(id: &str, _path: &str, d: &[u8], cx: &DecodeContext) -> Result<(Vec<u8>, &'static str)> {
    let limit = cx.limits.max_entry_size as usize;
    let (out, e) = match id {
        "alice-soft/aff" => {
            let mut o = at(d, 16, d.len().saturating_sub(16))?.to_vec();
            let k = [
                0xc8, 0xbb, 0x8f, 0xb7, 0xed, 0x43, 0x99, 0x4a, 0xa2, 0x7e, 0x5b, 0xb0, 0x68, 0x18,
                0xf8, 0x88,
            ];
            for (i, b) in o.iter_mut().take(64).enumerate() {
                *b ^= k[i % 16];
            }
            (o, "")
        }
        "amuse-craft/bgm" => (at(d, 12, d.len().saturating_sub(12))?.to_vec(), ""),
        "aoi/aog" => (
            if d.starts_with(b"AoiOgg") {
                at(d, 44, d.len().saturating_sub(44))?
            } else {
                d
            }
            .to_vec(),
            "ogg",
        ),
        "bgi/audio" => (
            at(d, u32at(d, 0)? as usize, u32at(d, 8)? as usize)?.to_vec(),
            "ogg",
        ),
        "bgi/bse" => (compression::bse(d)?, ""),
        "bgi/dsc" => return compression::dsc(d, cx),
        "complets/vmd" => (d.iter().map(|v| v ^ 0xe5).collect(), "mp3"),
        "crowd/eog" => (at(d, 8, d.len().saturating_sub(8))?.to_vec(), "ogg"),
        "dogenzaka/a" => {
            let k = key(cx, id, "dogenzaka/a.rc4")?;
            (k.with_bytes(|k| crysirror_resource::rc4_crypt(d, k))?, "")
        }
        "eagls/pak-script" => {
            let k = key(cx, id, "eagls/pak-script.xor")?;
            let mut o = at(d, 3600, d.len() - 3601)?.to_vec();
            let mut state = d[d.len() - 1] as i8 as i32 as u32;
            k.with_bytes(|k| {
                for b in o.iter_mut().step_by(2) {
                    state = state.wrapping_mul(214013).wrapping_add(2531011);
                    *b ^= k[((state >> 16) & 0x7fff) as usize % k.len()];
                }
            });
            (o, "txt")
        }
        "escude/acp" => (compression::acp(d, limit)?, ""),
        "kaguya/bmr" => (compression::bmr(d, limit)?, ""),
        "kid/cps" => (compression::cps(d, limit)?, "prt"),
        "kid/lnd" => {
            let n = checked_size(u32at(d, 8)? as usize, cx)?;
            (
                compression::lnd(at(d, 16, d.len().saturating_sub(16))?, n)?,
                "",
            )
        }
        "leaf/g" => (leaf_g(d)?, "ogg"),
        "leaf/pak2-audio" => (at(d, 20, u32at(d, 12)? as usize)?.to_vec(), ""),
        "leaf/pak2-compressed-file" => (
            compression::pak2(
                at(d, 36, d.len().saturating_sub(36))?,
                checked_size(u32at(d, 24)? as usize, cx)?,
            )?,
            "",
        ),
        "lilim/doj" => {
            let p = 4 + u16at(d, 2)? as usize * 6;
            ensure!(at(d, p, 2)? == b"DD", "corrupt DOJ metadata");
            let comp = u32at(d, p + 4)? as usize;
            let n = checked_size(u32at(d, p + 8)? as usize, cx)?;
            let s = at(d, p + 12, comp)?;
            let packed = u32at(s, 1)? as usize;
            let orig = checked_size(u32at(s, 5)? as usize, cx)?;
            let bytes = at(s, 9, packed.checked_sub(9).context("SYSD size")?)?;
            let o = if s[0] != 0 {
                crysirror_resource::lzss_decompress(bytes, orig, &Default::default())?
            } else {
                bytes.to_vec()
            };
            ensure!(o.len() == n, "DOJ size mismatch");
            (o, "")
        }
        "lilim/scr" => (
            compression::huffman(
                at(d, 4, d.len().saturating_sub(4))?,
                checked_size(u32at(d, 0)? as usize, cx)?,
            )?,
            "txt",
        ),
        "purple-software/ps2" => (compression::ps2(d, limit)?, ""),
        "scene-player/pmw" => {
            let x: Vec<_> = d.iter().map(|v| v ^ 0x21).collect();
            (inflate(&x, limit)?, "wav")
        }
        "shiina-rio/ogv" => {
            ensure!(at(d, 12, 4)? == b"fmt ", "OGV fmt chunk");
            let p = 20 + u32at(d, 16)? as usize;
            ensure!(at(d, p, 4)? == b"data", "OGV data chunk");
            (at(d, p + 8, d.len().saturating_sub(p + 8))?.to_vec(), "")
        }
        "twilight-frontier/tfcs" => {
            let n = checked_size(u32at(d, 9)? as usize, cx)?;
            let plain = inflate(at(d, 13, d.len().saturating_sub(13))?, n)?;
            ensure!(plain.len() == n, "TFCS inflated size");
            (tfcs(&plain, limit)?, "csv")
        }
        "vorbis/wav" => (packed_ogg(d)?, "ogg"),
        "wild-bug/wbi" => (wpx::section(d, 2, limit)?, "dat"),
        _ => bail!("unregistered file format {id}"),
    };
    ensure!(out.len() <= limit, "decoded byte limit");
    let e = if e.is_empty() { guess(&out) } else { e };
    Ok((out, e))
}
fn tfcs(d: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(d);
    let rows = c.le32()?;
    let mut o = String::new();
    ensure!(rows as usize <= d.len() / 4, "TFCS row count");
    for _ in 0..rows {
        let cols = c.le32()?;
        ensure!(cols as usize <= c.left() / 4, "TFCS column count");
        for i in 0..cols {
            let n = c.le32()? as usize;
            let s = crate::common::sjis(c.take(n)?)?;
            if i > 0 {
                o.push(',');
            }
            if s.contains(',') {
                o.push('"');
                o.push_str(&s.replace('"', "\"\""));
                o.push('"');
            } else {
                o.push_str(&s);
            }
            ensure!(o.len() <= limit, "TFCS CSV byte limit");
        }
        o.push('\n');
    }
    Ok(o.into_bytes())
}
fn packed_ogg(d: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        at(d, 0, 4)? == b"RIFF" && at(d, 8, 8)? == b"WAVEfmt ",
        "packed Ogg RIFF header"
    );
    let p = 20 + u32at(d, 16)? as usize;
    ensure!(
        at(d, p, 4)? == b"fact" && u32at(d, p + 4)? == 4 && at(d, p + 12, 4)? == b"data",
        "packed Ogg chunks"
    );
    let n = u32at(d, p + 16)? as usize;
    let data = at(d, p + 20, n)?;
    let mut c = Cursor::new(data);
    let mut out = Vec::new();
    let mut serial = None;
    while c.left() > 0 {
        if c.left() < 27 {
            break;
        }
        let start = c.at;
        ensure!(c.take(4)? == b"OggS", "Ogg page magic");
        let h = c.take(23)?;
        let id = u32::from_le_bytes(h[10..14].try_into().unwrap());
        let count = h[22] as usize;
        if c.left() < count {
            break;
        }
        let size = c.take(count)?.iter().map(|n| *n as usize).sum();
        if c.left() < size {
            break;
        }
        c.take(size)?;
        if *serial.get_or_insert(id) == id {
            out.extend_from_slice(&data[start..c.at]);
        }
    }
    Ok(out)
}
fn leaf_g(d: &[u8]) -> Result<Vec<u8>> {
    let mut c = Cursor::new(d);
    let mut out = Vec::new();
    let mut state = 0;
    while c.left() > 0 {
        let mut page = c.take(27)?.to_vec();
        let n = page[26] as usize;
        if n > 0 {
            let mut sizes = c.take(n)?.to_vec();
            let mut remaining: usize = sizes.iter().map(|v| *v as usize).sum();
            let mut payload = Vec::new();
            if state == 0 {
                ensure!(remaining >= 2 && c.u8()? == 1, "Leaf G identification");
                c.u8()?;
                remaining -= 2;
                payload.extend_from_slice(b"\x01vorbis");
                sizes[0] = sizes[0].checked_add(5).context("G segment overflow")?;
                state = 1;
            } else if state == 1 {
                ensure!(remaining >= 4 && c.u8()? == 3, "Leaf G comment");
                c.u8()?;
                remaining -= 2;
                let comment = sizes[0].checked_sub(2).context("G comment length")? as usize;
                payload.extend_from_slice(b"\x03vorbis");
                payload.extend_from_slice(c.take(comment)?);
                remaining = remaining.checked_sub(comment).context("G comment length")?;
                sizes[0] = sizes[0].checked_add(5).context("G segment overflow")?;
                ensure!(c.u8()? == 5, "Leaf G setup");
                c.u8()?;
                remaining = remaining.checked_sub(2).context("G setup length")?;
                payload.extend_from_slice(b"\x05vorbis");
                sizes[n - 1] = sizes[n - 1].checked_add(5).context("G segment overflow")?;
                state = 2;
            }
            page.extend_from_slice(&sizes);
            page.extend_from_slice(&payload);
            page.extend_from_slice(c.take(remaining)?);
        }
        page[..4].copy_from_slice(b"OggS");
        page[22..26].fill(0);
        // arc_unpacker stores IEEE CRC32 in big-endian order for this wrapper.
        let mut crc = flate2::Crc::new();
        crc.update(&page);
        page[22..26].copy_from_slice(&crc.sum().to_be_bytes());
        out.extend_from_slice(&page);
    }
    Ok(out)
}
