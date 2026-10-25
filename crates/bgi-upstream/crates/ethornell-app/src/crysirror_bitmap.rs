// SPDX-License-Identifier: MPL-2.0
//! CPU-embedded native bitmap calls execute the shared image kernels directly.
//! VM dispatch, native formats/parameters, clipping and resource ownership stay native.
use ethornell_image::DecodedImage;

fn result<T>(result: anyhow::Result<T>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            crate::snapshot::record_raster_error(format!(
                "shared native bitmap capability: {error:#}"
            ));
            None
        }
    }
}
pub(crate) fn blit_mode(
    destination: &mut DecodedImage,
    df: i32,
    source: &DecodedImage,
    sf: i32,
    x: i32,
    y: i32,
    mode: i32,
    parameter: i32,
) -> bool {
    result(crysirror_image::packed::blit(
        destination,
        df,
        source,
        sf,
        x,
        y,
        mode,
        parameter,
    ))
    .unwrap_or(false)
}
pub(crate) fn blit_copy(
    destination: &mut DecodedImage,
    df: i32,
    source: &DecodedImage,
    sf: i32,
    x: i32,
    y: i32,
) -> bool {
    blit_mode(destination, df, source, sf, x, y, 0x80, 0)
}
pub(crate) fn blit_alpha_over_parameter(
    destination: &mut DecodedImage,
    df: i32,
    source: &DecodedImage,
    sf: i32,
    x: i32,
    y: i32,
    parameter: i32,
) -> bool {
    result(crysirror_image::packed::alpha_over_parameter(
        destination,
        df,
        source,
        sf,
        x,
        y,
        parameter,
    ))
    .unwrap_or(false)
}
pub(crate) fn crossfade_cache(
    primary: &DecodedImage,
    secondary: &DecodedImage,
    format: i32,
    transition: i32,
) -> Option<DecodedImage> {
    result(crysirror_image::packed::crossfade(
        primary, secondary, format, transition,
    ))
}
pub(crate) fn crossfade_pixel(
    primary: [u8; 4],
    secondary: [u8; 4],
    format: i32,
    transition: i32,
) -> [u8; 4] {
    // The temporary result is discarded when the enclosing frame observes the
    // recorded error. Returning the old pixel avoids partial visible corruption.
    result(crysirror_image::packed::mix_pixel(
        primary, secondary, format, transition,
    ))
    .unwrap_or(primary)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_kernels_keep_direct_parameter_boundary_and_report_unknown_capabilities() {
        let source = DecodedImage::filled(1, 1, [200, 100, 50, 128]).unwrap();
        let mut destination = DecodedImage::filled(1, 1, [20, 30, 40, 77]).unwrap();
        let _ = crate::snapshot::take_raster_error();
        assert!(!blit_alpha_over_parameter(
            &mut destination,
            1,
            &source,
            2,
            0,
            0,
            256
        ));
        assert!(crate::snapshot::take_raster_error().is_none());
        let before = destination.clone();
        assert!(!blit_mode(&mut destination, 1, &source, 0, 0, 0, 0, 0));
        assert_eq!(destination, before);
        assert!(
            crate::snapshot::take_raster_error()
                .unwrap()
                .contains("unsupported packed bitmap format")
        );
        assert!(!blit_mode(&mut destination, 1, &source, 2, 0, 0, 0, -1));
        assert!(
            crate::snapshot::take_raster_error()
                .unwrap()
                .contains("parameter must be 0..256")
        );
    }
}
