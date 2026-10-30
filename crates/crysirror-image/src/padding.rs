//! Resource-level canvas padding without resampling. Useful for texture
//! allocation contracts and atlas construction, independent of any native VM.
use crate::{rgba_len, ImageSurface};
use anyhow::{ensure, Result};

/// Copy an image at the top-left of a larger, uniformly filled RGBA canvas.
/// Logical source pixels are unchanged; padding never scales/crops the source.
/// Run on a resource worker before publishing the immutable output resource.
pub fn pad_to_extent(
    source: &ImageSurface,
    extent: [u32; 2],
    fill: [u8; 4],
) -> Result<ImageSurface> {
    source.validate()?;
    ensure!(
        extent[0] >= source.width && extent[1] >= source.height,
        "padding extent cannot crop source"
    );
    let len = rgba_len(extent[0], extent[1])?;
    let mut rgba = vec![0; len];
    if fill != [0; 4] {
        for p in rgba.chunks_exact_mut(4) {
            p.copy_from_slice(&fill);
        }
    }
    let source_row = source.width as usize * 4;
    let destination_row = extent[0] as usize * 4;
    for (y, row) in source.rgba.chunks_exact(source_row).enumerate() {
        rgba[y * destination_row..y * destination_row + source_row].copy_from_slice(row);
    }
    Ok(ImageSurface {
        width: extent[0],
        height: extent[1],
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn padding_keeps_pixels_and_fills_only_new_canvas() {
        let image = ImageSurface {
            width: 1,
            height: 2,
            rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let padded = pad_to_extent(&image, [2, 3], [9, 10, 11, 12]).unwrap();
        assert_eq!(
            padded.rgba,
            vec![
                1, 2, 3, 4, 9, 10, 11, 12, 5, 6, 7, 8, 9, 10, 11, 12, 9, 10, 11, 12, 9, 10, 11, 12
            ]
        );
        assert_eq!(image.rgba, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }
    #[test]
    fn crop_and_oversized_padding_are_rejected_before_allocation() {
        let image = ImageSurface {
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        };
        assert!(pad_to_extent(&image, [1, 1], [0; 4]).is_err());
        assert!(pad_to_extent(&image, [8192, 8192], [0; 4]).is_err());
    }
}
