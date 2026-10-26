// SPDX-License-Identifier: GPL-3.0-or-later
// S25 row/RLE/incremental layout: arc_unpacker, Copyright (C) 2016 rr-.
use super::super::*;
use std::collections::BTreeMap;
struct Frame {
    width: u32,
    height: u32,
    flags: u32,
    rows: Vec<u64>,
}
fn row(input: &[u8], width: u32, repeats: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(input);
    let mut output = vec![0; width as usize * 4];
    let mut pixel = 0usize;
    while pixel < width as usize {
        if c.at & 1 != 0 {
            c.take(1)?;
        }
        let token = c.le16()?;
        let method = token >> 13;
        let skip = (token >> 11) & 3;
        c.take(skip as usize)?;
        let mut count = usize::from(token & 0x7ff);
        if count == 0 {
            count = c.le32()? as usize;
        }
        count = count.min(width as usize - pixel);
        ensure!(count > 0, "S25 row instruction made no progress");
        match method {
            2 | 4 => {
                let channels = if method == 2 { 3 } else { 4 };
                let mut chunk = c.take(count * channels)?.to_vec();
                for _ in 0..repeats {
                    for i in channels..chunk.len() {
                        chunk[i] = chunk[i].wrapping_add(chunk[i - channels]);
                    }
                }
                for p in chunk.chunks_exact(channels) {
                    let rgba = if method == 2 {
                        [p[2], p[1], p[0], 255]
                    } else {
                        [p[3], p[2], p[1], p[0]]
                    };
                    output[pixel * 4..pixel * 4 + 4].copy_from_slice(&rgba);
                    pixel += 1;
                }
            }
            3 | 5 => {
                let channels = if method == 3 { 3 } else { 4 };
                let p = c.take(channels)?;
                let rgba = if method == 3 {
                    [p[2], p[1], p[0], 255]
                } else {
                    [p[3], p[2], p[1], p[0]]
                };
                for _ in 0..count {
                    output[pixel * 4..pixel * 4 + 4].copy_from_slice(&rgba);
                    pixel += 1;
                }
            }
            _ => pixel += count,
        }
    }
    Ok(output)
}
pub(super) fn open(source: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    ensure!(magic(source, 0, b"S25\0")?, "S25 signature mismatch");
    let count = le32(&bytes(source, 4, 4, 4)?, 0)? as usize;
    limits(cx, count)?;
    let offsets = index(source, cx, 8, count as u64 * 4)?;
    let mut frames = vec![];
    let mut counts: BTreeMap<u64, usize> = BTreeMap::new();
    // Charge the aggregate retained row references, not just each repeated
    // on-disk row table. Duplicate frame offsets must not multiply memory
    // outside the native-index budget.
    let mut index_bytes = count as u64 * 4;
    let row_reference_bytes = std::mem::size_of::<(Arc<[u8]>, usize)>() as u64;
    for raw in offsets.chunks_exact(4) {
        let offset = u64::from(le32(raw, 0)?);
        if offset == 0 {
            continue;
        }
        let h = bytes(source, offset, 20, 20)?;
        let (width, height, flags) = (le32(&h, 0)?, le32(&h, 4)?, le32(&h, 16)?);
        if width == 0 || height == 0 {
            continue;
        }
        crysirror_image::rgba_len(width, height)?;
        let decoded = crysirror_image::png_uncompressed_len(width, height)? as u64;
        ensure!(
            decoded <= cx.limits.max_entry_size,
            "S25 frame decoded entry byte limit"
        );
        index_bytes = index_bytes
            .checked_add(std::mem::size_of::<Frame>() as u64)
            .and_then(|n| n.checked_add(u64::from(height) * row_reference_bytes))
            .context("S25 aggregate row index overflow")?;
        ensure!(
            index_bytes <= cx.limits.max_index_size,
            "S25 aggregate row-reference index budget"
        );
        let table = index(source, cx, offset + 20, u64::from(height) * 4)?;
        let rows = table
            .chunks_exact(4)
            .map(|b| le32(b, 0).map(u64::from))
            .collect::<Result<Vec<_>>>()?;
        for o in &rows {
            let references = counts.entry(*o).or_default();
            *references = references
                .checked_add(1)
                .context("S25 row reference count overflow")?;
        }
        frames.push(Frame {
            width,
            height,
            flags,
            rows,
        });
    }
    let mut row_data = BTreeMap::new();
    let mut total = index_bytes;
    for offset in counts.keys() {
        let n = bytes(source, *offset, 2, 2)?;
        let mut size = u64::from(u16::from_le_bytes(n.try_into().unwrap()));
        let mut start = offset + 2;
        if offset & 1 != 0 {
            size = size.checked_sub(1).context("S25 odd row size")?;
            start += 1;
        }
        total = total.checked_add(size).context("S25 row budget overflow")?;
        ensure!(
            total <= cx.limits.max_index_size,
            "S25 encoded row index budget"
        );
        row_data.insert(*offset, Arc::<[u8]>::from(index(source, cx, start, size)?));
    }
    let mut output = vec![];
    for (i, frame) in frames.into_iter().enumerate() {
        // Repeated incremental references can make the original prefix-sum
        // loop quadratic in a tiny encoded archive. Bound byte visits before
        // publishing a decoder, using the caller's byte-scale resource quota.
        if frame.flags & 0x80000000 != 0 {
            let references = frame.rows.iter().try_fold(0u64, |n, o| {
                n.checked_add(counts[o] as u64)
                    .context("S25 predictor reference overflow")
            })?;
            let work = references
                .checked_mul(u64::from(frame.width) * 4)
                .context("S25 predictor work overflow")?;
            ensure!(
                work <= cx.limits.max_resident_bytes,
                "S25 incremental predictor work budget"
            );
        }
        let rows = frame
            .rows
            .iter()
            .map(|o| {
                (
                    row_data[o].clone(),
                    if frame.flags & 0x80000000 != 0 {
                        counts[o]
                    } else {
                        0
                    },
                )
            })
            .collect::<Vec<_>>();
        let size = crysirror_image::png_uncompressed_len(frame.width, frame.height)?;
        let decode = decode(move |_, _| {
            let mut rgba = Vec::with_capacity(frame.width as usize * frame.height as usize * 4);
            for (data, repeats) in &rows {
                rgba.extend_from_slice(&row(data, frame.width, *repeats)?);
            }
            crysirror_image::encode_png_uncompressed(&crysirror_image::ImageSurface {
                width: frame.width,
                height: frame.height,
                rgba,
            })
        });
        output.push(Row::packed(format!("{i:05}.png"),0,0,size as u64,decode).meta(serde_json::json!({"frame":i,"flags":frame.flags,"width":frame.width,"height":frame.height})));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::service::MemoryAssetReader;
    fn repeated_frames(count: u32, width: u32, height: u32) -> MemoryAssetReader {
        let frame = 8 + count as usize * 4;
        let row = frame + 20 + height as usize * 4;
        let mut data = vec![0; row];
        data[..4].copy_from_slice(b"S25\0");
        data[4..8].copy_from_slice(&count.to_le_bytes());
        for offset in data[8..frame].chunks_exact_mut(4) {
            offset.copy_from_slice(&(frame as u32).to_le_bytes());
        }
        data[frame..frame + 4].copy_from_slice(&width.to_le_bytes());
        data[frame + 4..frame + 8].copy_from_slice(&height.to_le_bytes());
        data[frame + 16..frame + 20].copy_from_slice(&0x80000000u32.to_le_bytes());
        for offset in data[frame + 20..row].chunks_exact_mut(4) {
            offset.copy_from_slice(&(row as u32).to_le_bytes());
        }
        let size = 2 + width as u16 * 3;
        data.extend(size.to_le_bytes());
        data.extend((0x4000 | width as u16).to_le_bytes());
        data.resize(data.len() + width as usize * 3, 0);
        MemoryAssetReader(Arc::from(data))
    }
    #[test]
    fn repeated_frame_tables_charge_aggregate_index_before_allocation() {
        let source = repeated_frames(8, 1, 16);
        let mut cx = DecodeContext::new(crysirror_core::state::ClientId(1));
        cx.limits.max_index_size = 1024;
        let error = open(&source, &cx).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("aggregate row-reference index budget"),
            "{error:#}"
        );
    }
    #[test]
    fn oversized_frame_fails_before_reading_its_missing_row_table() {
        let mut data = b"S25\0".to_vec();
        data.extend(1u32.to_le_bytes());
        data.extend(12u32.to_le_bytes());
        data.extend(16u32.to_le_bytes());
        data.extend(1u32.to_le_bytes());
        data.resize(32, 0);
        let mut cx = DecodeContext::new(crysirror_core::state::ClientId(2));
        cx.limits.max_entry_size = 32;
        let error = open(&MemoryAssetReader(Arc::from(data)), &cx)
            .err()
            .unwrap();
        assert!(
            error.to_string().contains("frame decoded entry byte limit"),
            "{error:#}"
        );
    }
    #[test]
    fn shared_rows_cannot_create_unbounded_incremental_predictor_work() {
        let source = repeated_frames(2, 2, 16);
        let mut cx = DecodeContext::new(crysirror_core::state::ClientId(3));
        cx.limits.max_index_size = 2048;
        cx.limits.max_resident_bytes = 256;
        assert!(crysirror_image::png_uncompressed_len(2, 16).unwrap() <= 256);
        let error = open(&source, &cx).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("incremental predictor work budget"),
            "{error:#}"
        );
    }
}
