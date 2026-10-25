// SPDX-License-Identifier: MPL-2.0
//! Checked 2D DDS inputs: BC1/BC2/BC3 plus explicit RGB(A)32/R8/A8.
use crate::ImageSurface;
use anyhow::{bail, ensure, Result};
pub(crate) fn decode(bytes: &[u8]) -> Result<Option<ImageSurface>> {
    ensure!(bytes.len() >= 128, "truncated DDS header");
    let u = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    ensure!(u(4) == 124 && u(76) == 32, "DDS header sizes");
    let (w, h) = (u(16), u(12));
    ensure!(
        u64::from(w) * u64::from(h) * 4 <= crate::MAX_DECODED_BYTES as u64,
        "DDS decoded byte limit"
    );
    ensure!(w > 0 && h > 0 && w <= 8192 && h <= 8192, "DDS dimensions");
    ensure!(
        u(112) & (0x200 | 0x200000) == 0 && u(24) <= 1,
        "DDS cubemap/volume requires explicit import"
    );
    if &bytes[84..88] != [0; 4] {
        ensure!(
            [b"DXT1".as_slice(), b"DXT3", b"DXT5"].contains(&&bytes[84..88]),
            "unsupported DDS FourCC"
        );
        return Ok(None);
    }
    let flags = u(80);
    let depth = u(88);
    let masks = [u(92), u(96), u(100), u(104)];
    let pixel_bytes = depth as usize / 8;
    ensure!(matches!(depth, 8 | 32), "DDS raw bit depth");
    let tight = w as usize * pixel_bytes;
    let pitch = if u(8) & 8 != 0 { u(20) as usize } else { tight };
    ensure!(pitch >= tight && pitch <= tight + 256, "DDS row pitch");
    ensure!(128 + pitch * h as usize <= bytes.len(), "DDS pixel range");
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    let shifts = if depth == 32 {
        ensure!(flags & 0x40 != 0, "DDS RGB flags");
        for mask in masks[..3].iter() {
            ensure!(
                mask.count_ones() == 8 && mask.trailing_zeros() % 8 == 0,
                "DDS RGB channel mask"
            );
        }
        ensure!(
            masks[0] & masks[1] == 0 && masks[0] & masks[2] == 0 && masks[1] & masks[2] == 0,
            "overlapping DDS RGB masks"
        );
        ensure!(
            masks[3] == 0
                || masks[3].count_ones() == 8
                    && masks[3].trailing_zeros() % 8 == 0
                    && masks[..3].iter().all(|m| m & masks[3] == 0),
            "DDS alpha mask"
        );
        masks.map(|m| m.trailing_zeros() / 8)
    } else {
        [0; 4]
    };
    for y in 0..h as usize {
        for x in 0..w as usize {
            let at = 128 + y * pitch + x * pixel_bytes;
            if depth == 32 {
                for shift in &shifts[..3] {
                    rgba.push(bytes[at + *shift as usize]);
                }
                rgba.push(if masks[3] == 0 {
                    255
                } else {
                    bytes[at + shifts[3] as usize]
                });
            } else if flags & 0x20000 != 0 && masks[0] == 255 {
                rgba.extend([bytes[at], bytes[at], bytes[at], 255]);
            } else if flags & 2 != 0 {
                rgba.extend([255, 255, 255, bytes[at]]);
            } else {
                bail!("unsupported DDS 8-bit flags/mask");
            }
        }
    }
    let surface = ImageSurface {
        width: w,
        height: h,
        rgba,
    };
    surface.validate()?;
    Ok(Some(surface))
}
#[cfg(test)]
mod tests {
    use crate::decode_standard;
    fn header(width: u32, height: u32, flags: u32, depth: u32, masks: [u32; 4]) -> Vec<u8> {
        let mut b = vec![0; 128];
        b[..4].copy_from_slice(b"DDS ");
        for (at, v) in [
            (4, 124),
            (8, 0x1007),
            (76, 32),
            (12, height),
            (16, width),
            (80, flags),
            (88, depth),
            (92, masks[0]),
            (96, masks[1]),
            (100, masks[2]),
            (104, masks[3]),
        ] {
            b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
    #[test]
    fn dds_raw_channels_bc1_and_invalid_headers() {
        let mut b = header(1, 1, 0x41, 32, [0xff0000, 0xff00, 0xff, 0xff000000]);
        b.extend([3, 2, 1, 4]);
        assert_eq!(decode_standard(&b).unwrap().rgba, [1, 2, 3, 4]);
        let mut bc = header(4, 4, 4, 0, [0; 4]);
        bc[84..88].copy_from_slice(b"DXT1");
        bc.extend([0x00, 0xf8, 0, 0, 0, 0, 0, 0]);
        assert!(decode_standard(&bc)
            .unwrap()
            .rgba
            .chunks_exact(4)
            .all(|p| p == [255, 0, 0, 255]));
        assert!(decode_standard(&b[..128]).is_err());
        let mut invalid = b.clone();
        invalid[112..116].copy_from_slice(&0x200u32.to_le_bytes());
        assert!(decode_standard(&invalid).is_err());
        b[16..20].copy_from_slice(&99999u32.to_le_bytes());
        assert!(decode_standard(&b).is_err());
    }
}
