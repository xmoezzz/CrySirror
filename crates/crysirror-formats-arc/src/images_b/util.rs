// SPDX-License-Identifier: GPL-3.0-or-later
// Port of arc_unpacker helpers at 456834ecf2e5686813802c37efd829310485c57d.
use anyhow::{ensure, Context, Result};
use crysirror_image::{rgba_len, ImageSurface, MAX_DECODED_BYTES};
pub fn slice(b: &[u8], p: usize, n: usize) -> Result<&[u8]> {
    b.get(p..p.checked_add(n).context("image offset overflow")?)
        .context("truncated image")
}
pub fn u16le(b: &[u8], p: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(slice(b, p, 2)?.try_into()?))
}
pub fn u32le(b: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(slice(b, p, 4)?.try_into()?))
}
pub fn u32be(b: &[u8], p: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(slice(b, p, 4)?.try_into()?))
}
pub fn limit(n: usize) -> Result<usize> {
    ensure!(n <= MAX_DECODED_BYTES, "decoded payload exceeds 64 MiB");
    Ok(n)
}
pub fn inflate(b: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = flate2::Decompress::new(true);
    let mut out = Vec::new();
    loop {
        let mut chunk = [0u8; 8192];
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let input = b
            .get(usize::try_from(before_in)?..)
            .context("zlib input overflow")?;
        let status = decoder.decompress(input, &mut chunk, flate2::FlushDecompress::None)?;
        let written = usize::try_from(decoder.total_out() - before_out)?;
        ensure!(
            out.len() + written <= MAX_DECODED_BYTES,
            "decoded payload exceeds 64 MiB"
        );
        out.extend_from_slice(&chunk[..written]);
        if status == flate2::Status::StreamEnd {
            return Ok(out);
        }
        ensure!(
            decoder.total_in() > before_in || written > 0,
            "truncated zlib stream"
        );
    }
}

pub fn raw(w: u32, h: u32, b: &[u8], channels: usize) -> Result<ImageSurface> {
    let len = rgba_len(w, h)?;
    let n = len / 4;
    ensure!([1, 2, 3, 4].contains(&channels), "unsupported pixel stride");
    let b = slice(b, 0, n * channels)?;
    let mut rgba = Vec::with_capacity(len);
    for p in b.chunks_exact(channels) {
        let c = match channels {
            1 => [p[0], p[0], p[0], 255],
            2 => color16(u16::from_le_bytes([p[0], p[1]]), false),
            3 => [p[2], p[1], p[0], 255],
            _ => [p[2], p[1], p[0], p[3]],
        };
        rgba.extend(c);
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
pub fn color16(c: u16, alpha: bool) -> [u8; 4] {
    [
        ((c >> 7) & 248) as u8,
        ((c >> 2) & 248) as u8,
        ((c << 3) & 248) as u8,
        if !alpha || c & 0x8000 != 0 { 255 } else { 0 },
    ]
}
pub fn lzss(b: &[u8], size: usize, initial: usize, bitwise: bool) -> Result<Vec<u8>> {
    limit(size)?;
    let mask = if bitwise { 2047 } else { 4095 };
    let mut dict = vec![0; mask + 1];
    let mut pos = initial;
    let mut out = Vec::with_capacity(size);
    let mut ptr = 0;
    let mut flags = 0u16;
    let mut bit = 0;
    while out.len() < size {
        let literal = if bitwise {
            bits(b, &mut bit, 1)? != 0
        } else {
            flags >>= 1;
            if flags & 0x100 == 0 {
                flags = u16::from(*b.get(ptr).context("truncated LZSS control")?) | 0xff00;
                ptr += 1;
            }
            flags & 1 != 0
        };
        if literal {
            let v = if bitwise {
                bits(b, &mut bit, 8)? as u8
            } else {
                let v = *b.get(ptr).context("truncated LZSS literal")?;
                ptr += 1;
                v
            };
            out.push(v);
            dict[pos] = v;
            pos = (pos + 1) & mask;
        } else {
            let (mut src, count) = if bitwise {
                (
                    bits(b, &mut bit, 11)? as usize,
                    bits(b, &mut bit, 4)? as usize + 2,
                )
            } else {
                let v = slice(b, ptr, 2)?;
                ptr += 2;
                (
                    usize::from(v[0]) | ((usize::from(v[1]) & 0xf0) << 4),
                    usize::from(v[1] & 15) + 3,
                )
            };
            for _ in 0..count.min(size - out.len()) {
                let v = dict[src];
                src = (src + 1) & mask;
                out.push(v);
                dict[pos] = v;
                pos = (pos + 1) & mask;
            }
        }
    }
    Ok(out)
}
pub fn bits(b: &[u8], bit: &mut usize, n: usize) -> Result<u32> {
    let mut v = 0;
    for _ in 0..n {
        let byte = *b.get(*bit / 8).context("truncated image bitstream")?;
        v = (v << 1) | u32::from((byte >> (7 - *bit % 8)) & 1);
        *bit += 1;
    }
    Ok(v)
}
pub struct Reader<'a> {
    pub b: &'a [u8],
    pub p: usize,
}
impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8], p: usize) -> Self {
        Self { b, p }
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let v = slice(self.b, self.p, n)?;
        self.p += n;
        Ok(v)
    }
    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn le16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    pub fn be16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into()?))
    }
    pub fn le32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
}
pub fn copy_back(out: &mut Vec<u8>, distance: usize, count: usize, total: usize) -> Result<()> {
    ensure!(
        distance > 0 && distance <= out.len(),
        "invalid image back-reference"
    );
    ensure!(count <= total - out.len(), "image run exceeds output");
    for _ in 0..count {
        let v = out[out.len() - distance];
        out.push(v);
    }
    Ok(())
}
pub fn flip(image: &mut ImageSurface) {
    let stride = image.width as usize * 4;
    for y in 0..image.height as usize / 2 {
        for x in 0..stride {
            image
                .rgba
                .swap(y * stride + x, (image.height as usize - 1 - y) * stride + x);
        }
    }
}
pub fn indexed(w: u32, h: u32, data: &[u8], palette: &[[u8; 4]]) -> Result<ImageSurface> {
    let n = rgba_len(w, h)? / 4;
    let data = slice(data, 0, n)?;
    let mut rgba = Vec::with_capacity(n * 4);
    for &i in data {
        rgba.extend(palette.get(usize::from(i)).copied().unwrap_or([i, i, i, 0]));
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
pub fn offset(
    image: ImageSurface,
    x: u32,
    y: u32,
    canvas: Option<(u32, u32)>,
) -> Result<ImageSurface> {
    let (w, h) = canvas.unwrap_or((
        image
            .width
            .checked_add(x)
            .context("image offset overflow")?,
        image
            .height
            .checked_add(y)
            .context("image offset overflow")?,
    ));
    let mut rgba = vec![0; rgba_len(w, h)?];
    for sy in 0..image.height.min(h.saturating_sub(y)) {
        let n = image.width.min(w.saturating_sub(x)) as usize * 4;
        let to = ((sy + y) as usize * w as usize + x as usize) * 4;
        let from = sy as usize * image.width as usize * 4;
        if n > 0 {
            rgba[to..to + n].copy_from_slice(&image.rgba[from..from + n]);
        }
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
