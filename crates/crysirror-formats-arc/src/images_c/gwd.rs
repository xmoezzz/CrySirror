// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Result};
use crysirror_image::ImageSurface;
fn gamma(bits: &mut Bits<'_>) -> Result<usize> {
    let mut n = 1;
    while bits.get(1)? == 0 {
        n += 1;
        ensure!(n < 31, "GWD gamma overflow");
    }
    Ok(bits.get(n)? + (1 << n) - 2)
}
fn transform(code: u8, previous: u8) -> u8 {
    let tmp = if previous >= 128 {
        255 - previous
    } else {
        previous
    } as i32;
    let result = if tmp * 2 >= code as i32 {
        if code & 1 != 0 {
            tmp + (code as i32 + 1) / 2
        } else {
            tmp - code as i32 / 2
        }
    } else {
        code as i32
    };
    if previous >= 128 {
        (255 - result) as u8
    } else {
        result as u8
    }
}
fn stream(c: &mut Cursor<'_>) -> Result<ImageSurface> {
    c.skip(4)?;
    ensure!(c.take(3)? == b"GWD", "missing nested GWD signature");
    let w = u16::from_be_bytes(c.take(2)?.try_into()?) as u32;
    let h = u16::from_be_bytes(c.take(2)?.try_into()?) as u32;
    let depth = c.u8()?;
    ensure!(depth == 8 || depth == 24, "unsupported GWD bit depth");
    let n = pixels(w, h)?;
    let channels = (depth / 8) as usize;
    let mut data = vec![0; n * channels];
    let mut bits = Bits::new(c.rest());
    for y in 0..h as usize {
        for channel in 0..channels {
            let mut row = Vec::with_capacity(w as usize);
            while row.len() < w as usize {
                let width = bits.get(3)?;
                let count = gamma(&mut bits)? + 1;
                ensure!(count <= w as usize - row.len(), "GWD run exceeds scanline");
                for _ in 0..count {
                    row.push(if width == 0 {
                        0
                    } else {
                        bits.get(width + 1)? as u8
                    });
                }
            }
            for x in 1..row.len() {
                row[x] = transform(row[x], row[x - 1]);
            }
            for x in 0..w as usize {
                data[(y * w as usize + x) * channels + channel] = row[x];
            }
        }
    }
    let used = bits.bytes_used();
    c.skip(used)?;
    raw(
        w,
        h,
        &data,
        if depth == 8 {
            Format::Gray
        } else {
            Format::Bgr
        },
    )
}
pub(super) fn decode(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::new(b);
    let mut out = stream(&mut c)?;
    if c.u8()? != 0 {
        let mask = stream(&mut c)?;
        ensure!(
            mask.width == out.width && mask.height == out.height,
            "GWD mask dimensions differ"
        );
        for (p, m) in out.rgba.chunks_exact_mut(4).zip(mask.rgba.chunks_exact(4)) {
            p[3] = 255 - m[0];
        }
    }
    Ok(out)
}
