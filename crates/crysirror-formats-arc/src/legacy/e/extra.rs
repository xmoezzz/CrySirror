// SPDX-License-Identifier: GPL-3.0-or-later
// Unity Assets and Yuzusoft image PSB layouts ported from arc_unpacker,
// Copyright (C) 2016 rr-. No Unity or E-mote runtime is linked or hosted.
use super::*;
use crysirror_image::bitmap::{BitmapRect, BlitOptions, PixelMath};
use crysirror_image::{DecoderRegistry, ImageDecodeContext, ImageSurface};
use std::collections::{BTreeMap, BTreeSet};
pub(super) fn unity(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 20, 20)?;
    let be = |at| u32::from_be_bytes(h[at..at + 4].try_into().unwrap());
    let metadata = be(0) as u64;
    let file_size = be(4) as u64;
    let version = be(8);
    let base = be(12) as u64;
    ensure!(
        file_size == length(s)? && base <= file_size,
        "Unity Assets size/header mismatch"
    );
    ensure!(matches!(version,14|15),"Unity Assets version {version}: pinned reader implements nonembedded type tree v3 only (v14/v15)");
    ensure!(
        h[16] == 0,
        "Unity Assets big-endian object metadata unsupported by pinned decoder"
    );
    let b = index(s, cx, 20, metadata.min(base.saturating_sub(20)))?;
    let mut c = Cursor::new(&b);
    let revision = String::from_utf8(c.zero()?.to_vec())?;
    let attributes = c.le32()?;
    let embedded = c.u8()? != 0;
    ensure!(
        !embedded,
        "Unity embedded type tree nodes unsupported by pinned decoder"
    );
    let classes = count(&mut c, cx)?;
    for _ in 0..classes {
        let class = c.le32()? as i32;
        if class < 0 {
            c.take(16)?;
        }
        c.take(16)?;
    }
    let n = count(&mut c, cx)?;
    let mut map = BTreeMap::new();
    for _ in 0..n {
        let pos = 20 + c.at;
        let padding = (4 - pos % 4) % 4;
        c.take(padding)?;
        let path = c.le64()? as i64;
        let off = base + c.le32()? as u64;
        let size = c.le32()? as u64;
        let type_id = c.le32()? as i32;
        let class = c.le16()? as i16;
        let script = c.le16()? as i16;
        let stripped = if version > 14 { c.u8()? != 0 } else { false };
        ensure!(map.insert(path,Row::raw(String::new(),off,size).meta(serde_json::json!({"path_id":path,"type_id":type_id,"class_id":class,"script_type_index":script,"stripped":stripped,"revision":revision,"attributes":attributes}))).is_none(),"duplicate Unity object id");
    }
    let n = count(&mut c, cx)?;
    c.take(
        n.checked_mul(12)
            .context("Unity object id table overflow")?,
    )?;
    c.take((4 - (20 + c.at) % 4) % 4)?;
    let n = count(&mut c, cx)?;
    for _ in 0..n {
        c.zero()?;
        c.take(20)?;
        c.zero()?;
    }
    let mut rows: Vec<_> = map.into_values().collect();
    name_unnamed(&mut rows);
    checked_rows(s, cx, rows)
}
fn integer(c: &mut Cursor<'_>, n: usize) -> Result<usize> {
    ensure!(n <= 8, "PSB integer width");
    let mut out = 0u64;
    for (i, b) in c.take(n)?.iter().enumerate() {
        out |= (*b as u64) << (i * 8);
    }
    usize::try_from(out).context("PSB integer address-space overflow")
}
fn array(c: &mut Cursor<'_>, cx: &DecodeContext) -> Result<Vec<usize>> {
    let tag = c.u8()?;
    ensure!((12..=20).contains(&tag), "PSB array count marker");
    let n = integer(c, (tag - 12) as usize)?;
    limits(cx, n)?;
    let tag = c.u8()?;
    ensure!((12..=20).contains(&tag), "PSB array width marker");
    let width = (tag - 12) as usize;
    ensure!(width > 0 || n == 0, "PSB zero array item width");
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(integer(c, width)?);
    }
    Ok(out)
}
fn names(b: &[u8], at: usize, cx: &DecodeContext) -> Result<Vec<String>> {
    let mut c = Cursor::new(b.get(at..).context("PSB names offset")?);
    let a = array(&mut c, cx)?;
    let p = array(&mut c, cx)?;
    let t = array(&mut c, cx)?;
    let mut out = Vec::new();
    for n in t {
        let mut pos = *p.get(n).context("PSB name terminal")?;
        let mut raw = Vec::new();
        let mut visited = BTreeSet::new();
        loop {
            ensure!(
                visited.insert(pos) && visited.len() <= 16384,
                "PSB cyclic/deep name trie"
            );
            let parent = *p.get(pos).context("PSB name parent")?;
            let base = *a.get(parent).context("PSB name base")?;
            let byte = pos.checked_sub(base).context("PSB name byte underflow")?;
            ensure!(byte <= 255, "PSB name byte overflow");
            raw.push(byte as u8);
            pos = parent;
            if pos == 0 {
                break;
            }
        }
        raw.reverse();
        out.push(String::from_utf8(raw)?);
    }
    Ok(out)
}
fn named(
    b: &[u8],
    at: usize,
    names: &[String],
    cx: &DecodeContext,
) -> Result<Vec<(String, usize)>> {
    let mut c = Cursor::new(b.get(at..).context("PSB named directory offset")?);
    ensure!(c.u8()? == 33, "PSB named directory marker");
    let ids = array(&mut c, cx)?;
    let offsets = array(&mut c, cx)?;
    ensure!(
        ids.len() == offsets.len(),
        "PSB dictionary key/value count mismatch"
    );
    let base = at + c.at;
    ids.into_iter()
        .zip(offsets)
        .map(|(i, o)| {
            Ok((
                names.get(i).context("PSB dictionary name id")?.clone(),
                base.checked_add(o)
                    .context("PSB dictionary offset overflow")?,
            ))
        })
        .collect()
}
fn field(v: &[(String, usize)], name: &str) -> Result<usize> {
    v.iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| *v)
        .with_context(|| format!("PSB missing field {name}"))
}
fn number(b: &[u8], at: usize) -> Result<f64> {
    let mut c = Cursor::new(b.get(at..).context("PSB number offset")?);
    let tag = c.u8()?;
    let value = match tag {
        1 | 4 => 0.,
        2 | 3 => 1.,
        5..=8 => integer(&mut c, (tag - 4) as usize)? as f64,
        29 | 30 => f32::from_le_bytes(c.take(4)?.try_into()?) as f64,
        31 => f64::from_le_bytes(c.take(8)?.try_into()?),
        _ => bail!("unsupported PSB number type {tag}"),
    };
    ensure!(
        value.is_finite() && value.abs() <= i32::MAX as f64,
        "PSB invalid numeric layout value"
    );
    Ok(value)
}
fn string(b: &[u8], at: usize, offsets: &[usize], base: usize) -> Result<String> {
    let mut c = Cursor::new(b.get(at..).context("PSB string value offset")?);
    let tag = c.u8()?;
    ensure!((20..=24).contains(&tag), "PSB string marker");
    let i = integer(&mut c, (tag - 20) as usize)?;
    let at = base + *offsets.get(i).context("PSB string index")?;
    let mut c = Cursor::new(b.get(at..).context("PSB string pool offset")?);
    Ok(String::from_utf8(c.zero()?.to_vec())?)
}
struct ImageRow {
    path: String,
    offset: u64,
    size: u64,
    x: i32,
    y: i32,
}
fn image(b: &[u8], path: &str) -> Result<ImageSurface> {
    if has_ext(path, "png") {
        crysirror_image::decode_standard(b)
    } else if has_ext(path, "tlg") {
        let registry = DecoderRegistry::default();
        crate::images_a::decode("kirikiri/tlg", b, &ImageDecodeContext::new(path, &registry))
    } else {
        bail!("unknown PSB image extension")
    }
}
pub(super) fn psb(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(s, 0, b"PSB\0")?, "missing PSB");
    let h = bytes(s, 0, 40, 40)?;
    let version = u16::from_le_bytes([h[4], h[5]]);
    let flags = u16::from_le_bytes([h[6], h[7]]);
    ensure!(matches!(version, 2 | 3), "unknown PSB version {version}");
    ensure!(
        flags == 0,
        "encrypted PSB requires caller-decrypted source profile"
    );
    let mut offsets = [0usize; 7];
    for (i, v) in offsets.iter_mut().enumerate() {
        *v = le32(&h, 12 + i * 4)? as usize;
    }
    let [name_at, strings_at, string_data, chunk_offsets, chunk_sizes, chunk_data, root_at] =
        offsets;
    let end = chunk_data as u64;
    let b = index(s, cx, 0, end)?;
    let names = names(&b, name_at, cx)?;
    let offsets = array(
        &mut Cursor::new(b.get(strings_at..).context("PSB string offsets")?),
        cx,
    )?;
    let chunks = array(
        &mut Cursor::new(b.get(chunk_offsets..).context("PSB chunk offsets")?),
        cx,
    )?;
    let sizes = array(
        &mut Cursor::new(b.get(chunk_sizes..).context("PSB chunk sizes")?),
        cx,
    )?;
    ensure!(chunks.len() == sizes.len(), "PSB chunk offset/size count");
    let root = named(&b, root_at, &names, cx)?;
    let _width = number(&b, field(&root, "width")?)?;
    let _height = number(&b, field(&root, "height")?)?;
    let source = cx.source_name.as_deref().unwrap_or("unk.psb");
    let name = source.rsplit('/').next().unwrap_or(source);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let mut records = Vec::new();
    for (name, at) in &root {
        if name.contains(".png") || name.contains(".tlg") {
            let mut c = Cursor::new(b.get(*at..).context("PSB chunk reference")?);
            let tag = c.u8()?;
            ensure!((24..=28).contains(&tag), "PSB resource marker");
            let i = integer(&mut c, (tag - 24) as usize)?;
            let size = *sizes.get(i).context("PSB resource size index")? as u64;
            let off =
                chunk_data as u64 + *chunks.get(i).context("PSB resource offset index")? as u64;
            range(length(s)?, off, size)?;
            records.push(ImageRow {
                path: format!("{stem}_{name}"),
                offset: off,
                size,
                x: 0,
                y: 0,
            });
        }
    }
    let mut chosen = None;
    if let Ok(at) = field(&root, "layers") {
        let mut c = Cursor::new(b.get(at..).context("PSB layers offset")?);
        ensure!(c.u8()? == 32, "PSB layer list marker");
        let offsets_list = array(&mut c, cx)?;
        let base = at + c.at;
        for offset in offsets_list {
            let layer = named(&b, base + offset, &names, cx)?;
            let id = number(&b, field(&layer, "layer_id")?)? as i32;
            let id = id.to_string();
            let index = records
                .iter()
                .rposition(|r| r.path.contains(&id))
                .context("PSB layer id has no image resource")?;
            let record = &mut records[index];
            record.x = number(&b, field(&layer, "left")?)? as i32;
            record.y = number(&b, field(&layer, "top")?)? as i32;
            let _width = number(&b, field(&layer, "width")?)?;
            let _height = number(&b, field(&layer, "height")?)?;
            let name = string(&b, field(&layer, "name")?, &offsets, string_data)?;
            let (stem, ext) = record
                .path
                .rsplit_once('.')
                .context("PSB image filename extension")?;
            record.path = format!("{stem}_{name}.{ext}");
            chosen = Some(index);
        }
    }
    let base_image = if let Some(i) = chosen {
        let r = &records[i];
        let bytes = bytes(s, r.offset, r.size, cx.limits.max_entry_size)?;
        Some(Arc::new(image(&bytes, &r.path)?))
    } else {
        None
    };
    let mut rows = Vec::new();
    for (i, r) in records.into_iter().enumerate() {
        let source = bytes(s, r.offset, r.size, cx.limits.max_entry_size)?;
        let img = image(&source, &r.path)?;
        let base = if chosen != Some(i) {
            base_image.clone()
        } else {
            None
        };
        let (w, h) = base
            .as_ref()
            .map_or((img.width, img.height), |b| (b.width, b.height));
        let size = crysirror_image::png_uncompressed_len(w, h)? as u64;
        ensure!(size <= cx.limits.max_entry_size, "PSB PNG output limit");
        let native = r.path.clone();
        let path = format!(
            "{}.png",
            r.path.rsplit_once('.').map_or(r.path.as_str(), |(s, _)| s)
        );
        rows.push(Row::packed(
            path,
            r.offset,
            r.size,
            size,
            decode(move |b, _| {
                let src = image(b, &native)?;
                let result = if let Some(base) = &base {
                    let mut dst = (**base).clone(); // Native OverwriteNonTransparent copies whole RGBA when alpha != 0.
                    dst.blit(
                        &src,
                        BlitOptions {
                            source_rect: BitmapRect::new(0, 0, src.width, src.height),
                            destination: [r.x, r.y],
                            opacity: 255,
                            mode: PixelMath::CopyNonTransparent,
                            color_key: None,
                        },
                    )?;
                    dst
                } else {
                    src
                };
                crysirror_image::encode_png_uncompressed(&result)
            }),
        ));
    }
    checked_rows(s, cx, rows)
}
