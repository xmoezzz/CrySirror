// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
fn custom_lzss(b: &[u8], size: usize) -> Result<Vec<u8>> {
    limit(size)?;
    let mut r = Reader::new(b, 0);
    let mut out = Vec::with_capacity(size);
    let mut dict = [0u8; 4096];
    let mut dp = 0xfee;
    let mut control = 0u16;
    while out.len() < size && r.p < b.len() {
        control >>= 1;
        if control & 256 == 0 {
            control = u16::from(r.byte()?) | 0xff00;
        }
        if control & 1 != 0 {
            let v = r.byte()?;
            out.push(v);
            dict[dp] = v;
            dp = (dp + 1) & 4095;
        } else {
            let tmp = r.le16()?;
            let mut src = usize::from(tmp & 4095);
            let n = usize::from(tmp >> 12) + 3;
            for _ in 0..n.min(size - out.len()) {
                let v = dict[src];
                src = (src + 1) & 4095;
                out.push(v);
                dict[dp] = v;
                dp = (dp + 1) & 4095;
            }
        }
    }
    // This FC01-specific stream explicitly permits EOF at a token boundary:
    // its declared buffer may be larger than the coded bit payload (ACD).
    out.resize(size, 0);
    Ok(out)
}
pub fn acd(b: &[u8]) -> Result<ImageSurface> {
    let offset = u32le(b, 8)? as usize;
    let size = u32le(b, 16)? as usize;
    let w = u32le(b, 20)?;
    let h = u32le(b, 24)?;
    let n = rgba_len(w, h)? / 4;
    let data = custom_lzss(slice(b, offset, u32le(b, 12)? as usize)?, size)?;
    let mut p = 0;
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let mut v = 0i32;
        if bits(&data, &mut p, 1)? != 0 {
            v = -1;
            if bits(&data, &mut p, 1)? == 0 {
                v += 3;
                loop {
                    v = (v << 1) | bits(&data, &mut p, 1)? as i32;
                    let stop = v & 256 != 0;
                    v &= 255;
                    if stop {
                        break;
                    }
                }
                if v != 0 {
                    v = (v + 1).wrapping_mul(0x28ccccd) >> 24;
                }
            }
        }
        out.push(v as u8);
    }
    raw(w, h, &out, 1)
}
pub fn mcg(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let version = slice(b, 4, 4)?;
    ensure!(
        version == b"1.01" || version == b"2.00",
        "unsupported FC01 MCG version"
    );
    ensure!(u32le(b, 16)? == 64, "unsupported FC01 MCG header size");
    let w = u32le(b, 28)?;
    let h = u32le(b, 32)?;
    ensure!(u32le(b, 36)? == 24, "unsupported FC01 MCG depth");
    let n = rgba_len(w, h)? / 4;
    let size = u32le(b, 40)? as usize;
    limit(size)?;
    let material = ctx.key("fc01/mcg")?;
    let key = material.with_bytes(|b| -> Result<u8> {
        ensure!(b.len() == 1, "FC01 MCG key must contain exactly one byte");
        Ok(b[0])
    })?;
    let input = slice(b, 64, b.len().saturating_sub(64))?;
    let data = if version == b"1.01" {
        let mut data = input.to_vec();
        let mut k = key;
        let len = data.len();
        for (i, v) in data.iter_mut().enumerate() {
            *v = v.rotate_left(1) ^ k;
            k = k.wrapping_add((len - 1 - i) as u8);
        }
        custom_lzss(&data, size)?
    } else {
        let mut r = Reader::new(input, 0);
        let mut planes = Vec::new();
        for _ in 0..3 {
            planes.push(arithmetic(&mut r, n, key)?);
        }
        for y in 0..h as usize - 1 {
            for x in 0..w as usize - 1 {
                let p = y * w as usize + x;
                for plane in &mut planes {
                    let a = i32::from(plane[p]);
                    let down = i32::from(plane[p + w as usize]) - a;
                    let right = i32::from(plane[p + 1]) - a;
                    let diag = (right + down).abs();
                    let predictor = if diag >= right.abs() && down.abs() >= right.abs() {
                        plane[p + w as usize]
                    } else if diag < down.abs() {
                        plane[p]
                    } else {
                        plane[p + 1]
                    };
                    let i = p + w as usize + 1;
                    plane[i] = plane[i].wrapping_add(predictor).wrapping_add(128);
                }
            }
        }
        let mut out = Vec::with_capacity(n * 3);
        for i in 0..n {
            let blue = planes[2][i].wrapping_sub(128) as i8;
            let red = planes[1][i].wrapping_sub(128) as i8;
            let green = planes[0][i].wrapping_sub(((i16::from(blue) + i16::from(red)) >> 2) as u8);
            out.extend([
                green.wrapping_add(red as u8),
                green,
                green.wrapping_add(blue as u8),
            ]);
        }
        out
    };
    let stride = (w as usize * 3).div_ceil(4) * 4;
    let mut packed = Vec::with_capacity(n * 3);
    for y in 0..h as usize {
        packed.extend(slice(&data, y * stride, w as usize * 3)?);
    }
    raw(w, h, &packed, 3)
}
fn arithmetic(r: &mut Reader<'_>, size: usize, initial: u8) -> Result<Vec<u8>> {
    let mut cumulative = [0u32; 257];
    let mut frequencies = [0u32; 256];
    let mut lookup = Vec::new();
    let mut k = initial;
    for i in 0..256 {
        let mut v = r.byte()?;
        if initial != 0 {
            v = v.rotate_left(1) ^ k;
            k = k.wrapping_sub(i as u8);
        }
        let f = u32::from(v);
        frequencies[i] = f;
        cumulative[i + 1] = cumulative[i] + f;
        lookup.extend(std::iter::repeat_n(i as u8, usize::from(v)));
    }
    let quant = cumulative[256];
    ensure!(quant != 0, "FC01 MCG empty arithmetic distribution");
    let mut d = (quant - 1) >> 8;
    let mut mask = 255u32;
    while d != 0 {
        d >>= 1;
        mask = (mask << 1) | 1;
    }
    let scale = 65536 / quant;
    let mut a = u32::from_be_bytes(r.take(4)?.try_into()?);
    let mut base = 0u32;
    let mut range = u32::MAX;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        range = (range >> 8).wrapping_mul(scale) >> 8;
        ensure!(range > 0, "FC01 MCG arithmetic interval exhausted");
        let index = a.wrapping_sub(base) / range;
        let v = *lookup
            .get(index as usize)
            .context("FC01 MCG arithmetic index out of range")?;
        out.push(v);
        base = base.wrapping_add(cumulative[v as usize].wrapping_mul(range));
        range = range.wrapping_mul(frequencies[v as usize]);
        while (range.wrapping_add(base) ^ base) & 0xff000000 == 0 {
            a = (a << 8) | u32::from(r.byte()?);
            base <<= 8;
            range <<= 8;
        }
        while range <= mask {
            range = (!base & mask) << 8;
            a = (a << 8) | u32::from(r.byte()?);
            base <<= 8;
        }
    }
    Ok(out)
}
