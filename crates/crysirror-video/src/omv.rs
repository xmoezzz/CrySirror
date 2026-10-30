// SPDX-License-Identifier: MPL-2.0
//! Bounded OMV metadata and plane conversion, independent of any VM or host.
use anyhow::{Result, anyhow, bail, ensure};
pub const OMV_THEORA_TYPE_RGB: u32 = 0;
pub const OMV_THEORA_TYPE_RGBA: u32 = 1;
pub const OMV_THEORA_TYPE_YUV: u32 = 2;
#[derive(Debug, Clone)]
pub struct OmvHeader {
    pub header_size: u32,
    pub version: u32,
    pub theora_type: u32,
    pub display_width: u32,
    pub display_height: u32,
    pub frame_time_us: u32,
    pub max_data_size: u32,
    pub page_count_hint: u32,
    pub packet_count_hint: u32,
}
fn read_omv_header_from_bytes(buf: &[u8]) -> Result<OmvHeader> {
    if buf.len() < 0x58 {
        bail!("OMV header too small");
    }
    let header_size = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let version = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let theora_type = u32::from_le_bytes([buf[0x28], buf[0x29], buf[0x2a], buf[0x2b]]);
    let display_width = u32::from_le_bytes([buf[0x2c], buf[0x2d], buf[0x2e], buf[0x2f]]);
    let display_height = u32::from_le_bytes([buf[0x30], buf[0x31], buf[0x32], buf[0x33]]);
    let frame_time_us = u32::from_le_bytes([buf[0x3c], buf[0x3d], buf[0x3e], buf[0x3f]]);
    let max_data_size = u32::from_le_bytes([buf[0x40], buf[0x41], buf[0x42], buf[0x43]]);
    let page_count_hint = u32::from_le_bytes([buf[0x4c], buf[0x4d], buf[0x4e], buf[0x4f]]);
    let packet_count_hint = u32::from_le_bytes([buf[0x50], buf[0x51], buf[0x52], buf[0x53]]);
    if header_size < 0x58 {
        bail!("invalid OMV header size: {header_size:#x}");
    }
    if theora_type > OMV_THEORA_TYPE_YUV {
        bail!("invalid OMV theora type: {theora_type}");
    }
    if display_width == 0 || display_height == 0 {
        bail!(
            "invalid OMV display size: {}x{}",
            display_width,
            display_height
        );
    }
    Ok(OmvHeader {
        header_size,
        version,
        theora_type,
        display_width,
        display_height,
        frame_time_us,
        max_data_size,
        page_count_hint,
        packet_count_hint,
    })
}

fn extract_ogg_from_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    let needle = b"OggS";
    let pos = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .ok_or_else(|| anyhow!("OggS not found in OMV payload"))?;
    Ok(bytes[pos..].to_vec())
}

fn omv_plane_layout(
    frame_width: i32,
    frame_height: i32,
    fmt: i32,
) -> (usize, usize, usize, usize, usize) {
    // The decoder now transports the complete th_decode_ycbcr_out() planes,
    // not the Theora picture rectangle. Plane sizes therefore follow the
    // coded-frame geometry and pixel format regardless of the Siglus OMV
    // interpretation (RGB/RGBA/YUV). RGB/RGBA assets used by the original
    // engine are 4:4:4, so all three planes are full-size in those files.
    let w = frame_width.max(1) as usize;
    let h = frame_height.max(1) as usize;
    let y_len = w.saturating_mul(h);
    let (uv_w, uv_h) = yuv_plane_size(frame_width, frame_height, fmt);
    let uv_len = uv_w.saturating_mul(uv_h);
    (uv_w, uv_h, y_len, uv_len, uv_len)
}

fn convert_omv_frame(
    data: &[u8],
    frame_width: i32,
    frame_height: i32,
    fmt: i32,
    display_width: i32,
    display_height: i32,
    theora_type: u32,
) -> Vec<u8> {
    convert_omv_frame_to_size(
        data,
        frame_width,
        frame_height,
        fmt,
        display_width,
        display_height,
        theora_type,
        display_width.max(1) as usize,
        display_height.max(1) as usize,
    )
}

fn convert_omv_frame_to_size(
    data: &[u8],
    frame_width: i32,
    frame_height: i32,
    fmt: i32,
    display_width: i32,
    display_height: i32,
    theora_type: u32,
    output_width: usize,
    output_height: usize,
) -> Vec<u8> {
    // tona3 does not apply Theora pic_x/pic_y to OMV video. It asks
    // th_decode_ycbcr_out() for the decoded planes, starts at plane.data, and
    // advances each row by the plane stride while using the OMV header's
    // theora_size as the visible rectangle. `data` is the same set of full
    // decoded planes repacked tightly, so frame_width/frame_height are the
    // source strides here and display_width/display_height are the output size.
    let (uv_w, _uv_h, y_plane_len, u_plane_len, _v_plane_len) =
        omv_plane_layout(frame_width, frame_height, fmt);
    let planes = PackedOmvPlanes {
        data,
        offsets: [0, y_plane_len, y_plane_len.saturating_add(u_plane_len)],
        widths: [frame_width.max(1) as usize, uv_w, uv_w],
    };
    convert_omv_planes_to_size(
        &planes,
        frame_width,
        frame_height,
        fmt,
        display_width,
        display_height,
        theora_type,
        output_width,
        output_height,
    )
}

trait OmvPlanes {
    /// Plane `pli`'s sample at (`x`, `y`), or `default` outside it.
    fn sample(&self, pli: usize, x: usize, y: usize, default: u8) -> u8;

    /// Row `y` of plane `pli` as `len` bytes, when byte `x` of it is
    /// `sample(pli, x, y, _)` for every `x < len` (None otherwise).
    fn row(&self, pli: usize, y: usize, len: usize) -> Option<&[u8]>;
}

/// The planes repacked tightly one after another (worker messages).
struct PackedOmvPlanes<'a> {
    data: &'a [u8],
    offsets: [usize; 3],
    widths: [usize; 3],
}

impl OmvPlanes for PackedOmvPlanes<'_> {
    fn sample(&self, pli: usize, x: usize, y: usize, default: u8) -> u8 {
        get_plane_sample(
            self.data,
            self.offsets[pli],
            self.widths[pli],
            x,
            y,
            default,
        )
    }

    fn row(&self, pli: usize, y: usize, len: usize) -> Option<&[u8]> {
        let width = self.widths[pli];
        if width == 0 {
            return None;
        }
        let start = self.offsets[pli].checked_add(y.checked_mul(width)?)?;
        self.data.get(start..start.checked_add(len)?)
    }
}

fn bilinear_omv_sample(
    planes: &impl OmvPlanes,
    pli: usize,
    x: (usize, usize, u32),
    y: (usize, usize, u32),
    default: u8,
) -> u8 {
    let (x0, x1, fx) = x;
    let (y0, y1, fy) = y;
    let p00 = u64::from(planes.sample(pli, x0, y0, default));
    let p10 = u64::from(planes.sample(pli, x1, y0, default));
    let p01 = u64::from(planes.sample(pli, x0, y1, default));
    let p11 = u64::from(planes.sample(pli, x1, y1, default));
    const ONE: u64 = 1 << 16;
    let fx = u64::from(fx);
    let fy = u64::from(fy);
    let top = p00 * (ONE - fx) + p10 * fx;
    let bottom = p01 * (ONE - fx) + p11 * fx;
    ((top * (ONE - fy) + bottom * fy + (1 << 31)) >> 32) as u8
}

fn convert_omv_planes_to_size(
    planes: &impl OmvPlanes,
    frame_width: i32,
    frame_height: i32,
    fmt: i32,
    display_width: i32,
    display_height: i32,
    theora_type: u32,
    output_width: usize,
    output_height: usize,
) -> Vec<u8> {
    let mut rgba = Vec::new();
    convert_omv_planes_into(
        planes,
        frame_width,
        frame_height,
        fmt,
        display_width,
        display_height,
        theora_type,
        output_width,
        output_height,
        &mut rgba,
    );
    rgba
}

fn convert_omv_planes_into(
    planes: &impl OmvPlanes,
    frame_width: i32,
    frame_height: i32,
    fmt: i32,
    display_width: i32,
    display_height: i32,
    theora_type: u32,
    output_width: usize,
    output_height: usize,
    rgba: &mut Vec<u8>,
) {
    let sw = frame_width.max(1) as usize;
    let sh = frame_height.max(1) as usize;
    let dw = display_width.max(1) as usize;
    let dh = display_height.max(1) as usize;
    let (uv_w, uv_h) = yuv_plane_size(frame_width, frame_height, fmt);

    rgba.resize(
        output_width.saturating_mul(output_height).saturating_mul(4),
        0,
    );
    // Source column of each output column, and how much of a row they
    // span: rows are then read as slices (per-pixel sampling with a division
    // and bounds checks took longer than decoding a 1080p frame).
    let source_x: Vec<usize> = (0..output_width)
        .map(|out_x| out_x * dw / output_width)
        .collect();
    let row_len = source_x.last().map_or(0, |&x| x + 1);

    match (theora_type, fmt) {
        (OMV_THEORA_TYPE_RGB, _) => {
            for out_y in 0..output_height {
                let y = out_y * dh / output_height;
                if let (Some(b_row), Some(g_row), Some(r_row)) = (
                    planes.row(0, y, row_len),
                    planes.row(1, y, row_len),
                    planes.row(2, y, row_len),
                ) {
                    let out_row =
                        &mut rgba[out_y * output_width * 4..(out_y + 1) * output_width * 4];
                    for (px, &x) in out_row.chunks_exact_mut(4).zip(&source_x) {
                        px[0] = r_row[x];
                        px[1] = g_row[x];
                        px[2] = b_row[x];
                        px[3] = 0xff;
                    }
                    continue;
                }
                for out_x in 0..output_width {
                    let x = out_x * dw / output_width;
                    let b = planes.sample(0, x, y, 0);
                    let g = planes.sample(1, x, y, 0);
                    let r = planes.sample(2, x, y, 0);
                    let out = (out_y * output_width + out_x) * 4;
                    rgba[out] = r;
                    rgba[out + 1] = g;
                    rgba[out + 2] = b;
                    rgba[out + 3] = 0xff;
                }
            }
        }
        (OMV_THEORA_TYPE_RGBA, _) => {
            // Original layout: visible B/G/R occupy the first `dh` rows of
            // the three 4:4:4 planes. Alpha follows below that visible region:
            // top third in Y, middle third in U, bottom third in V.
            let alpha_h = dh.div_ceil(3);
            let alpha_h_2 = alpha_h * 2;
            for out_y in 0..output_height {
                let y = out_y * dh / output_height;
                let (a_pli, local_y) = if y < alpha_h {
                    (0, y)
                } else if y < alpha_h_2 {
                    (1, y - alpha_h)
                } else {
                    (2, y - alpha_h_2)
                };
                let alpha_y = dh.saturating_add(local_y);
                if let (Some(b_row), Some(g_row), Some(r_row), Some(a_row)) = (
                    planes.row(0, y, row_len),
                    planes.row(1, y, row_len),
                    planes.row(2, y, row_len),
                    planes.row(a_pli, alpha_y, row_len),
                ) {
                    let out_row =
                        &mut rgba[out_y * output_width * 4..(out_y + 1) * output_width * 4];
                    for (px, &x) in out_row.chunks_exact_mut(4).zip(&source_x) {
                        px[0] = r_row[x];
                        px[1] = g_row[x];
                        px[2] = b_row[x];
                        px[3] = a_row[x];
                    }
                    continue;
                }
                for out_x in 0..output_width {
                    let x = out_x * dw / output_width;
                    let b = planes.sample(0, x, y, 0);
                    let g = planes.sample(1, x, y, 0);
                    let r = planes.sample(2, x, y, 0);
                    let a = planes.sample(a_pli, x, alpha_y, 0xff);
                    let out = (out_y * output_width + out_x) * 4;
                    rgba[out] = r;
                    rgba[out + 1] = g;
                    rgba[out + 2] = b;
                    rgba[out + 3] = a;
                }
            }
        }
        (_, siglus_omv_decoder::TH_PF_444) => {
            // This is the exact tona3 YUV path: all three source planes are
            // sampled at the same x/y and the float result is truncated by the
            // C++ `(int)` cast before clamping.
            for out_y in 0..output_height {
                let y = out_y * dh / output_height;
                for out_x in 0..output_width {
                    let x = out_x * dw / output_width;
                    let yv = planes.sample(0, x, y, 0) as f32;
                    let u = planes.sample(1, x, y, 128) as f32 - 128.0;
                    let v = planes.sample(2, x, y, 128) as f32 - 128.0;
                    let out = (out_y * output_width + out_x) * 4;
                    rgba[out] = clamp_f(yv + 1.40200 * v);
                    rgba[out + 1] = clamp_f(yv - 0.34414 * u - 0.71414 * v);
                    rgba[out + 2] = clamp_f(yv + 1.77200 * u);
                    rgba[out + 3] = 0xff;
                }
            }
        }
        _ => {
            // Siglus-authored RGB/RGBA/YUV OMVs are 4:4:4 in the original
            // decoder path. Retain the existing 4:2:0/4:2:2 compatibility
            // fallback for nonstandard files, but base it on the full coded
            // frame rather than the Theora picture rectangle.
            let chroma_x: Vec<_> = (0..dw)
                .map(|x| centred_resample_coordinate(x, uv_w, sw))
                .collect();
            let chroma_y: Vec<_> = (0..dh)
                .map(|y| centred_resample_coordinate(y, uv_h, sh))
                .collect();

            for out_y in 0..output_height {
                let y = out_y * dh / output_height;
                let y_coord = chroma_y[y];
                for out_x in 0..output_width {
                    let x = out_x * dw / output_width;
                    let yv = planes.sample(0, x, y, 0) as f32;
                    let x_coord = chroma_x[x];
                    let u = bilinear_omv_sample(planes, 1, x_coord, y_coord, 128) as f32 - 128.0;
                    let v = bilinear_omv_sample(planes, 2, x_coord, y_coord, 128) as f32 - 128.0;

                    let out = (out_y * output_width + out_x) * 4;
                    rgba[out] = clamp_f(yv + 1.40200 * v);
                    rgba[out + 1] = clamp_f(yv - 0.34414 * u - 0.71414 * v);
                    rgba[out + 2] = clamp_f(yv + 1.77200 * u);
                    rgba[out + 3] = 0xff;
                }
            }
        }
    }
}

fn get_plane_sample(
    data: &[u8],
    plane_off: usize,
    plane_width: usize,
    x: usize,
    y: usize,
    default: u8,
) -> u8 {
    if plane_width == 0 {
        return default;
    }
    data.get(
        plane_off
            .saturating_add(y.saturating_mul(plane_width))
            .saturating_add(x),
    )
    .copied()
    .unwrap_or(default)
}

fn centred_resample_coordinate(
    output_index: usize,
    input_len: usize,
    output_len: usize,
) -> (usize, usize, u32) {
    if input_len <= 1 || output_len <= 1 {
        return (0, 0, 0);
    }

    // Map pixel centres instead of aligning sample corners. For a 2:1
    // subsampled plane this maps the first luma pixel to -0.25 chroma pixels
    // (clamped at the edge), and then advances by 0.5 per output pixel.
    let numerator = (2_i128 * output_index as i128 + 1) * input_len as i128 * (1_i128 << 15);
    let mut position = numerator / output_len as i128 - (1_i128 << 15);
    let max_position = ((input_len - 1) as i128) << 16;
    position = position.clamp(0, max_position);

    let i0 = (position >> 16) as usize;
    let i1 = (i0 + 1).min(input_len - 1);
    let fraction = (position & 0xFFFF) as u32;
    (i0, i1, fraction)
}

fn clamp_f(v: f32) -> u8 {
    if v <= 0.0 {
        0
    } else if v >= 255.0 {
        255
    } else {
        v as u8
    }
}

fn yuv_plane_size(width: i32, height: i32, fmt: i32) -> (usize, usize) {
    let w = width.max(1) as usize;
    let h = height.max(1) as usize;
    match fmt {
        siglus_omv_decoder::TH_PF_420 => (w / 2, h / 2),
        siglus_omv_decoder::TH_PF_422 => (w / 2, h),
        siglus_omv_decoder::TH_PF_444 => (w, h),
        _ => (w / 2, h / 2),
    }
}

pub fn header(bytes: &[u8]) -> Result<OmvHeader> {
    let header = read_omv_header_from_bytes(bytes)?;
    crysirror_image::rgba_len(header.display_width, header.display_height)?;
    Ok(header)
}
pub fn embedded_ogg(bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(bytes.len() <= 1024 * 1024 * 1024, "OMV encoded byte budget");
    extract_ogg_from_bytes(bytes)
}
pub fn packed_len(width: i32, height: i32, fmt: i32) -> Result<usize> {
    ensure!(
        (1..=8192).contains(&width) && (1..=8192).contains(&height),
        "Theora coded size"
    );
    ensure!(
        matches!(
            fmt,
            siglus_omv_decoder::TH_PF_420
                | siglus_omv_decoder::TH_PF_422
                | siglus_omv_decoder::TH_PF_444
        ),
        "unsupported Theora chroma layout"
    );
    if fmt != siglus_omv_decoder::TH_PF_444 {
        ensure!(width % 2 == 0, "subsampled Theora coded width");
    }
    if fmt == siglus_omv_decoder::TH_PF_420 {
        ensure!(height % 2 == 0, "subsampled Theora coded height");
    }
    let (_, _, y, u, v) = omv_plane_layout(width, height, fmt);
    let n = y + u + v;
    ensure!(n <= 64 * 1024 * 1024, "Theora plane budget");
    Ok(n)
}
pub fn to_rgba(
    data: &[u8],
    width: i32,
    height: i32,
    fmt: i32,
    display_width: i32,
    display_height: i32,
    kind: u32,
) -> Result<Vec<u8>> {
    ensure!(
        data.len() == packed_len(width, height, fmt)?,
        "Theora plane input length"
    );
    crysirror_image::rgba_len(
        u32::try_from(display_width)?,
        u32::try_from(display_height)?,
    )?;
    ensure!(kind <= OMV_THEORA_TYPE_YUV, "OMV plane interpretation");
    ensure!(
        display_width <= width && display_height <= height,
        "OMV display exceeds coded frame"
    );
    if kind != OMV_THEORA_TYPE_YUV {
        ensure!(
            fmt == siglus_omv_decoder::TH_PF_444,
            "OMV RGB planes require full chroma resolution"
        );
    }
    if kind == OMV_THEORA_TYPE_RGBA {
        ensure!(
            height >= display_height + (display_height + 2) / 3,
            "OMV alpha rows exceed coded frame"
        );
    }
    Ok(convert_omv_frame(
        data,
        width,
        height,
        fmt,
        display_width,
        display_height,
        kind,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use siglus_omv_decoder::{TH_PF_420, TH_PF_444};

    #[test]
    fn native_bgr_planes_and_vertical_alpha_remain_distinct() {
        // Three visible rows followed by one alpha row in each B/G/R plane.
        let planes = [10, 11, 12, 40, 20, 21, 22, 50, 30, 31, 32, 60];
        assert_eq!(
            to_rgba(&planes, 1, 4, TH_PF_444, 1, 3, OMV_THEORA_TYPE_RGB).unwrap(),
            [30, 20, 10, 255, 31, 21, 11, 255, 32, 22, 12, 255]
        );
        assert_eq!(
            to_rgba(&planes, 1, 4, TH_PF_444, 1, 3, OMV_THEORA_TYPE_RGBA).unwrap(),
            [30, 20, 10, 40, 31, 21, 11, 50, 32, 22, 12, 60]
        );
    }

    #[test]
    fn invalid_planes_cannot_fall_back_to_another_layout() {
        assert!(packed_len(2, 2, 99).is_err());
        assert!(packed_len(1, 2, TH_PF_420).is_err());
        assert!(packed_len(8192, 8192, TH_PF_444).is_err());
        assert!(to_rgba(&[0; 11], 1, 4, TH_PF_444, 1, 3, OMV_THEORA_TYPE_RGBA).is_err());
        assert!(to_rgba(&[0; 12], 1, 4, TH_PF_444, 2, 3, OMV_THEORA_TYPE_RGB).is_err());
        assert!(to_rgba(&[0; 9], 1, 3, TH_PF_444, 1, 3, OMV_THEORA_TYPE_RGBA).is_err());
        assert!(embedded_ogg(b"no embedded stream").is_err());
        assert_eq!(
            embedded_ogg(b"container-prefixOggSsynthetic-payload").unwrap(),
            b"OggSsynthetic-payload"
        );
    }
}
