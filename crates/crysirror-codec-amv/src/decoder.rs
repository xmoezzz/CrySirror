use anyhow::{bail, ensure, Context, Result};
use std::io::Read;

use crate::amv::{AmvFile, AmvHeader, AmvMode, FramePacket, QuantTables};
use crate::bit::BitReader;
use crate::huffman::{jpeg_std, HuffmanTable};
use crate::idct::{idct8x8, idct_dc_only};

/// Output of decoding a single frame (full canvas RGBA).
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub index: u32,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

#[derive(Clone)]
struct HuffmanSet {
    dc_luma: HuffmanTable,
    ac_luma: HuffmanTable,
    dc_chroma: HuffmanTable,
    ac_chroma: HuffmanTable,
}

impl HuffmanSet {
    fn jpeg_default() -> Result<Self> {
        Ok(Self {
            dc_luma: jpeg_std::dc_luma()?,
            ac_luma: jpeg_std::ac_luma()?,
            dc_chroma: jpeg_std::dc_chroma()?,
            ac_chroma: jpeg_std::ac_chroma()?,
        })
    }
}

/// A stateful decoder that maintains inter-frame buffers.
///
/// This module intentionally focuses on the file/packet structure and produces
/// a useful pixel dump, rather than re-implementing the original engine's
/// exact internal types.
pub struct AmvDecoder {
    header: AmvHeader,
    mode: AmvMode,
    q: QuantTables,
    huf: HuffmanSet,
    prev_rgba: Option<Vec<u8>>,
    mode_a_has_mask: bool,
}

impl AmvDecoder {
    pub fn new(header: AmvHeader, mode: AmvMode, q: QuantTables) -> Result<Self> {
        let huf = HuffmanSet::jpeg_default()?;
        Ok(Self { header, mode, q, huf, prev_rgba: None, mode_a_has_mask: false })
    }

    pub fn header(&self) -> &AmvHeader {
        &self.header
    }

    pub fn mode(&self) -> AmvMode {
        self.mode
    }

    /// Decode frames from an already-parsed `AmvFile`.
    ///
    /// The iterator yields packets in file order; we decode each packet into a
    /// full-canvas RGBA frame.
    pub fn decode_all<R: std::io::Read + std::io::Seek>(
        &mut self,
        amv: &mut AmvFile<R>,
        max_frames: Option<usize>,
    ) -> Result<Vec<DecodedFrame>> {
        let mut out: Vec<DecodedFrame> = Vec::new();
        let mut it = amv.iter_frames();
        let limit = max_frames.unwrap_or(self.header.frame_count as usize);

        for _ in 0..limit {
            let pkt = match it.read_next()? {
                Some(p) => p,
                None => break,
            };
            let f = self.decode_packet(&pkt)?;
            out.push(f);
        }
        Ok(out)
    }

    /// Decode a single frame packet to a full RGBA canvas.
    pub fn decode_packet(&mut self, pkt: &FramePacket) -> Result<DecodedFrame> {
        let w = self.header.width as usize;
        let h = self.header.height as usize;
        let mut cur = if let Some(prev) = self.prev_rgba.as_ref() {
            prev.clone()
        } else {
            vec![0u8; w * h * 4]
        };

        match &pkt.kind {
            crate::amv::FramePacketKind::A { .. } => self.decode_packet_mode_a(pkt, &mut cur)?,
            crate::amv::FramePacketKind::B { .. } => self.decode_packet_mode_b(pkt, &mut cur)?,
        }

        self.prev_rgba = Some(cur.clone());
        Ok(DecodedFrame {
            index: pkt.index,
            width: self.header.width,
            height: self.header.height,
            rgba: cur,
        })
    }

    fn decode_packet_mode_a(&mut self, pkt: &FramePacket, rgba: &mut [u8]) -> Result<()> {
        let ah = pkt.header_a().context("mode A header missing")?;
        let mask = pkt.seg0.as_deref().context("mode A seg0 missing")?;
        let payload = pkt.seg1.as_deref().context("mode A seg1 missing")?;

        // Rectangle in pixels; sizes are 16-aligned.
        let x0 = ah.x as i32;
        let y0 = ah.y as i32;
        let rw = ah.w as i32;
        let rh = ah.h as i32;

        // A split-alpha packet describes the complete visible rectangle for
        // this frame. Pixels that belonged to an earlier rectangle are clear.
        if !mask.is_empty() || (self.mode_a_has_mask && rw == 0 && rh == 0 && payload.is_empty()) {
            rgba.fill(0);
        }
        self.decode_rect_dct(
            payload,
            x0,
            y0,
            rw,
            rh,
            /*has_alpha=*/ false,
            /*q_chroma=*/ &self.q.q1,
            /*q_luma=*/ &self.q.q0,
            /*q_alpha=*/ None,
            rgba,
        )?;
        if !mask.is_empty() {
            apply_mode_a_mask(
                mask,
                rgba,
                [usize::from(self.header.width), usize::from(self.header.height)],
                [x0, y0],
                [usize::from(ah.w), usize::from(ah.h)],
            )?;
            self.mode_a_has_mask = true;
        }
        Ok(())
    }

    fn decode_packet_mode_b(&self, pkt: &FramePacket, rgba: &mut [u8]) -> Result<()> {
        let bh = pkt.header_b().context("mode B header missing")?;
        let payload = pkt.payload_b().context("mode B payload missing")?;

        // Interpretation consistent with observed code: a 20-byte header,
        // with x/y offsets and a 16-aligned rectangle.
        // p0/p1 are signed offsets; interpret as x/y for now.
        let x0 = bh.p0 as i32;
        let y0 = bh.p1 as i32;
        let rw = bh.w as i32;
        let rh = bh.h as i32;

        let has_alpha = true; // Mode B corresponds to the attribute branch.
        let q_alpha = self.q.q2.as_ref();
        self.decode_rect_dct(
            payload,
            x0,
            y0,
            rw,
            rh,
            has_alpha,
            &self.q.q1,
            &self.q.q0,
            q_alpha,
            rgba,
        )
    }

    /// Decode a JPEG-like macroblock stream into RGBA.
    ///
    /// Layout derived from DLL/sqlite decompilation:
    /// - 4:2:0 macroblocks
    /// - 6 blocks per MB: Cb, Cr, Y0, Y1, Y2, Y3
    /// - 10 blocks per MB (alpha mode): Cb, Cr, Y0..Y3, A0..A3
    /// - q1 is used for the first two chroma blocks
    /// - q0 is used for the four luma blocks
    /// - q2 is used for the four alpha blocks in mode B
    fn decode_rect_dct(
        &self,
        payload: &[u8],
        x0: i32,
        y0: i32,
        rw: i32,
        rh: i32,
        has_alpha: bool,
        q_chroma: &Vec<u8>,
        q_luma: &Vec<u8>,
        q_alpha: Option<&Vec<u8>>,
        rgba: &mut [u8],
    ) -> Result<()> {
        if rw <= 0 || rh <= 0 {
            return Ok(());
        }
        if rw % 16 != 0 || rh % 16 != 0 {
            bail!("rect not 16-aligned: w={} h={}", rw, rh);
        }
        if q_chroma.len() != 64 || q_luma.len() != 64 {
            bail!("quant tables must be 64 bytes each");
        }
        if has_alpha {
            let qa = q_alpha.context("alpha mode requires q2")?;
            if qa.len() != 64 {
                bail!("alpha quant table must be 64 bytes");
            }
        }

        let fw = self.header.width as i32;
        let fh = self.header.height as i32;
        let stride = fw as usize * 4;

        let mb_w = rw >> 4;
        let mb_h = rh >> 4;

        let mut br = BitReader::new(payload);

        // Predictor behavior in the original code looks shared per-context.
        let mut pred_chroma: i32 = 0;
        let mut pred_luma: i32 = 0;

        for my in 0..mb_h {
            for mx in 0..mb_w {
                // Decode blocks.
                let cb = decode_block(
                    &mut br,
                    &self.huf.dc_chroma,
                    &self.huf.ac_chroma,
                    &mut pred_chroma,
                    q_chroma,
                )?;
                let cr = decode_block(
                    &mut br,
                    &self.huf.dc_chroma,
                    &self.huf.ac_chroma,
                    &mut pred_chroma,
                    q_chroma,
                )?;

                let y0b = decode_block(
                    &mut br,
                    &self.huf.dc_luma,
                    &self.huf.ac_luma,
                    &mut pred_luma,
                    q_luma,
                )?;
                let y1b = decode_block(
                    &mut br,
                    &self.huf.dc_luma,
                    &self.huf.ac_luma,
                    &mut pred_luma,
                    q_luma,
                )?;
                let y2b = decode_block(
                    &mut br,
                    &self.huf.dc_luma,
                    &self.huf.ac_luma,
                    &mut pred_luma,
                    q_luma,
                )?;
                let y3b = decode_block(
                    &mut br,
                    &self.huf.dc_luma,
                    &self.huf.ac_luma,
                    &mut pred_luma,
                    q_luma,
                )?;

                let (a0b, a1b, a2b, a3b) = if has_alpha {
                    let qa = q_alpha.unwrap();
                    (
                        Some(decode_block(
                            &mut br,
                            &self.huf.dc_luma,
                            &self.huf.ac_luma,
                            &mut pred_luma,
                            qa,
                        )?),
                        Some(decode_block(
                            &mut br,
                            &self.huf.dc_luma,
                            &self.huf.ac_luma,
                            &mut pred_luma,
                            qa,
                        )?),
                        Some(decode_block(
                            &mut br,
                            &self.huf.dc_luma,
                            &self.huf.ac_luma,
                            &mut pred_luma,
                            qa,
                        )?),
                        Some(decode_block(
                            &mut br,
                            &self.huf.dc_luma,
                            &self.huf.ac_luma,
                            &mut pred_luma,
                            qa,
                        )?),
                    )
                } else {
                    (None, None, None, None)
                };

                // Fast path: all blocks are AC-empty (EOB as first AC symbol).
                // The original code branches here; we implement a DC-only fill.
                let dc_only = cb.ac_empty
                    && cr.ac_empty
                    && y0b.ac_empty
                    && y1b.ac_empty
                    && y2b.ac_empty
                    && y3b.ac_empty
                    && (!has_alpha
                        || (a0b.as_ref().unwrap().ac_empty
                            && a1b.as_ref().unwrap().ac_empty
                            && a2b.as_ref().unwrap().ac_empty
                            && a3b.as_ref().unwrap().ac_empty));

                let base_x = x0 + (mx << 4);
                let base_y = y0 + (my << 4);

                // Clip against canvas.
                if base_x >= fw || base_y >= fh || base_x + 16 <= 0 || base_y + 16 <= 0 {
                    continue;
                }

                let y_blocks = [&y0b, &y1b, &y2b, &y3b];

                let cb_px = if dc_only {
                    cb.dc_only_pixels()
                } else {
                    cb.pixels
                };
                let cr_px = if dc_only {
                    cr.dc_only_pixels()
                } else {
                    cr.pixels
                };

                let y0_px = if dc_only { y0b.dc_only_pixels() } else { y0b.pixels };
                let y1_px = if dc_only { y1b.dc_only_pixels() } else { y1b.pixels };
                let y2_px = if dc_only { y2b.dc_only_pixels() } else { y2b.pixels };
                let y3_px = if dc_only { y3b.dc_only_pixels() } else { y3b.pixels };
                let y_px = [y0_px, y1_px, y2_px, y3_px];

                let a_px = if has_alpha {
                    let a0 = a0b.as_ref().unwrap();
                    let a1 = a1b.as_ref().unwrap();
                    let a2 = a2b.as_ref().unwrap();
                    let a3 = a3b.as_ref().unwrap();
                    Some([
                        if dc_only { a0.dc_only_pixels() } else { a0.pixels },
                        if dc_only { a1.dc_only_pixels() } else { a1.pixels },
                        if dc_only { a2.dc_only_pixels() } else { a2.pixels },
                        if dc_only { a3.dc_only_pixels() } else { a3.pixels },
                    ])
                } else {
                    None
                };

                // Write 16x16 RGBA macroblock.
                for dy in 0..16 {
                    let py = base_y + dy;
                    if py < 0 || py >= fh {
                        continue;
                    }
                    let row_off = py as usize * stride;
                    for dx in 0..16 {
                        let px = base_x + dx;
                        if px < 0 || px >= fw {
                            continue;
                        }
                        let yb = match (dx >= 8, dy >= 8) {
                            (false, false) => 0,
                            (true, false) => 1,
                            (false, true) => 2,
                            (true, true) => 3,
                        };
                        let yx = dx & 7;
                        let yy = dy & 7;
                        let yv = y_px[yb][(yy * 8 + yx) as usize] as i32;

                        let cu = (dx >> 1) as usize;
                        let cv = (dy >> 1) as usize;
                        let cbv = cb_px[cv * 8 + cu] as i32;
                        let crv = cr_px[cv * 8 + cu] as i32;

                        let (r, g, b) = ycbcr_to_rgb(yv, cbv, crv);
                        let a = if let Some(a4) = a_px.as_ref() {
                            a4[yb][(yy * 8 + yx) as usize] as u8
                        } else {
                            255u8
                        };

                        let off = row_off + px as usize * 4;
                        rgba[off] = r;
                        rgba[off + 1] = g;
                        rgba[off + 2] = b;
                        rgba[off + 3] = a;
                    }
                }
            }
        }

        Ok(())
    }
}

/// Mode A segment 0 is a zlib stream of one alpha byte per rectangle pixel.
/// Read just one row at a time, including pixels clipped by the canvas.
fn apply_mode_a_mask(
    compressed: &[u8],
    rgba: &mut [u8],
    [canvas_w, canvas_h]: [usize; 2],
    [left, top]: [i32; 2],
    [width, height]: [usize; 2],
) -> Result<()> {
    ensure!(
        width > 0 && height > 0 && width <= canvas_w + 16 && height <= canvas_h + 16
            && width.checked_mul(height).is_some_and(|bytes| bytes <= 64 * 1024 * 1024),
        "mode A alpha rectangle exceeds the frame bounds"
    );
    let mut mask = flate2::bufread::ZlibDecoder::new(compressed);
    let mut row = vec![0; width];
    for y in 0..height {
        mask.read_exact(&mut row).context("mode A alpha row")?;
        let dest_y = top + y as i32;
        if !(0..canvas_h as i32).contains(&dest_y) {
            continue;
        }
        let start = 0.max(-left) as usize;
        let end = width.min((canvas_w as i32 - left).max(0) as usize);
        if start >= end {
            continue;
        }
        let start_pixel = dest_y as usize * canvas_w + (left + start as i32) as usize;
        let destination = &mut rgba[start_pixel * 4..(start_pixel + end - start) * 4];
        for (pixel, alpha) in destination.chunks_exact_mut(4).zip(&row[start..end]) {
            pixel[3] = *alpha;
        }
    }
    ensure!(
        mask.read(&mut [0]).context("mode A alpha end")? == 0
            && mask.total_in() == compressed.len() as u64,
        "mode A alpha byte count does not match the rectangle"
    );
    Ok(())
}

#[cfg(test)]
mod mode_a_tests {
    use super::*;
    use crate::amv::{FrameAHeader, FramePacketKind, MAGIC_AJPM};
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    fn compress(alpha: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(alpha).unwrap();
        encoder.finish().unwrap()
    }

    fn packet(x: i16, alpha: &[u8]) -> FramePacket {
        let size = if alpha.is_empty() { 0 } else { 16 };
        let mask = if alpha.is_empty() { Vec::new() } else { compress(alpha) };
        // Six zero-coefficient blocks produce a single gray color macroblock.
        let color = if alpha.is_empty() { Vec::new() } else { vec![0, 0x28, 0xa2, 0x8a] };
        FramePacket {
            index: 0,
            file_offset: 0,
            tag: u32::from_le_bytes(*b"FRAM"),
            chunk_size: 16 + mask.len() as u32 + color.len() as u32,
            raw_packet: Vec::new(),
            kind: FramePacketKind::A {
                header: FrameAHeader { frame_id: 0, x, y: 0, w: size, h: size },
                seg0_len: mask.len() as u32,
                seg1_len: color.len() as u32,
            },
            seg0: Some(mask),
            seg1: Some(color),
        }
    }

    #[test]
    fn mode_a_uses_straight_alpha_and_clears_old_rectangles() {
        let header = AmvHeader {
            magic: MAGIC_AJPM, unk04: 0, revision: 0, header_size: 168, unk10: 0,
            frame_count: 3, fps_num: 1, fps_den: 30, width: 32, height: 16,
            attr: 2, reserved: [0; 3],
        };
        let quant = QuantTables::from_bytes(AmvMode::A, &[1; 128]).unwrap();
        let mut decoder = AmvDecoder::new(header, AmvMode::A, quant).unwrap();
        let mut mask = [255; 256];
        mask[..2].copy_from_slice(&[0, 64]);
        let first = decoder.decode_packet(&packet(0, &mask)).unwrap();
        assert_eq!(&first.rgba[..12], &[128, 128, 128, 0, 128, 128, 128, 64, 128, 128, 128, 255]);
        assert_eq!(&first.rgba[64..68], &[0, 0, 0, 0]);
        let second = decoder.decode_packet(&packet(16, &mask)).unwrap();
        assert_eq!(&second.rgba[8..12], &[0, 0, 0, 0]);
        assert_eq!(&second.rgba[68..72], &[128, 128, 128, 64]);
        let empty = decoder.decode_packet(&packet(0, &[])).unwrap();
        assert!(empty.rgba.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn mode_a_mask_length_is_exact_and_clipped_to_the_canvas() {
        let mut canvas = [0; 16];
        apply_mode_a_mask(&compress(&[1, 2, 3, 4, 5, 6]), &mut canvas, [2, 2], [-1, 0], [3, 2]).unwrap();
        assert_eq!([canvas[3], canvas[7], canvas[11], canvas[15]], [2, 3, 5, 6]);
        for bytes in [5, 7] {
            assert!(apply_mode_a_mask(&compress(&vec![0; bytes]), &mut canvas, [2, 2], [-1, 0], [3, 2]).is_err());
        }
    }
}

#[derive(Clone, Copy)]
struct DecodedBlock {
    /// Spatial pixels (8x8) in signed domain (centered around 0) then +128 and clamped.
    pixels: [u8; 64],
    /// Dequantized DC coefficient (scaled) for DC-only fast path.
    dc: i32,
    /// True if the AC decoder hit EOB as the first AC symbol.
    ac_empty: bool,
}

impl DecodedBlock {
    fn dc_only_pixels(&self) -> [u8; 64] {
        let v = idct_dc_only(self.dc) as i32 + 128;
        let v = v.clamp(0, 255) as u8;
        [v; 64]
    }
}

/// Standard zig-zag order.
/// (Maps coefficient index k=0..63 -> linear (row*8+col) index.)
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10,
    17, 24, 32, 25, 18, 11, 4, 5,
    12, 19, 26, 33, 40, 48, 41, 34,
    27, 20, 13, 6, 7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36,
    29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46,
    53, 60, 61, 54, 47, 55, 62, 63,
];

fn decode_block(
    br: &mut BitReader<'_>,
    dc: &HuffmanTable,
    ac: &HuffmanTable,
    pred: &mut i32,
    q: &Vec<u8>,
) -> Result<DecodedBlock> {
    // DC.
    let s = dc.decode_symbol(br)?;
    let diff = br.read_jpeg_signed(s)?;
    *pred += diff;
    let mut coeff = [0i32; 64];
    coeff[0] = *pred;

    // AC.
    let mut k: usize = 1;
    let mut saw_any_ac = false;
    while k < 64 {
        let sym = ac.decode_symbol(br)?;
        if sym == 0x00 {
            // EOB.
            break;
        }
        if sym == 0xF0 {
            // ZRL.
            k += 16;
            continue;
        }
        let run = (sym >> 4) as usize;
        let size = (sym & 0x0F) as u8;
        k += run;
        if k >= 64 {
            break;
        }
        let v = br.read_jpeg_signed(size)?;
        coeff[ZIGZAG[k]] = v;
        saw_any_ac = true;
        k += 1;
    }
    let ac_empty = !saw_any_ac;

    // Dequantize. The original code multiplies then shifts left by 2.
    let mut deq = [0i32; 64];
    for i in 0..64 {
        let qi = q[i] as i32;
        deq[i] = coeff[i] * qi * 4;
    }
    let dc_deq = deq[0];

    // IDCT.
    let spatial = idct8x8(&deq);
    let mut px = [0u8; 64];
    for i in 0..64 {
        let v = spatial[i] as i32 + 128;
        px[i] = v.clamp(0, 255) as u8;
    }

    Ok(DecodedBlock { pixels: px, dc: dc_deq, ac_empty })
}

#[inline]
fn ycbcr_to_rgb(y: i32, cb: i32, cr: i32) -> (u8, u8, u8) {
    // Full-range JPEG-like conversion.
    let cbf = (cb - 128) as f64;
    let crf = (cr - 128) as f64;
    let yf = y as f64;
    let r = yf + 1.402 * crf;
    let g = yf - 0.344136 * cbf - 0.714136 * crf;
    let b = yf + 1.772 * cbf;
    (r.round().clamp(0.0, 255.0) as u8,
     g.round().clamp(0.0, 255.0) as u8,
     b.round().clamp(0.0, 255.0) as u8)
}
