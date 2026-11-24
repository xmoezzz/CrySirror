// SPDX-License-Identifier: MPL-2.0
//! Midori indexed body/face replacement on the shared immutable image worker.
use crate::midori_color_filter::ColorFilter;
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    image_processing::{ImageCompositeLayer, ImageCompositeMath, ImageInput},
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(super) struct FacePlan {
    pub actor: u8,
    pub body: u8,
    pub face: u8,
    pub cells: u32,
    pub face_origin: [i32; 2],
    pub correction: [i32; 2],
}

impl FacePlan {
    pub fn read(self, storage: &dyn ClientStorage, filter: ColorFilter) -> Result<ImageInput> {
        let FacePlan {
            actor,
            body,
            face,
            cells,
            face_origin,
            ..
        } = self;
        let stem = format!(
            "res/g/ch/{actor:04}{:04}/{actor:04}{body:04}",
            body / 10 * 10
        );
        let read = |suffix: &str| -> Result<(String, Vec<u8>)> {
            let name = format!("{stem}{suffix}.gyu");
            let path = VfsPath::new(Mount::Asset, &name)?;
            let reader = storage.open_read(&path)?;
            let bytes = crysirror_resource::read_all(reader.as_ref(), 16 << 20)?;
            Ok((name, bytes))
        };
        let (body_name, mut body_bytes) = read("P000").context("Midori P000 actor body")?;
        let body = Indexed::inspect(&body_bytes, true)?;
        let size = body.size;
        let layer = if cells > 1 {
            let (face_name, mut face_bytes) = read("F").context("Midori F actor face atlas")?;
            let atlas = Indexed::inspect(&face_bytes, false)?;
            ensure!(
                atlas.palette == body.palette,
                "Midori actor face palette differs from native body palette"
            );
            let atlas_size = atlas.size;
            let width = atlas_size[0] / cells;
            let height = atlas_size[1];
            ensure!(
                width > 0 && atlas_size[0] % cells == 0 && u32::from(face) < cells,
                "unsupported Midori actor face atlas grid"
            );
            let [x, y] = face_origin;
            ensure!(
                x >= 0 && y >= 0 && x as u32 + width <= size[0] && y as u32 + height <= size[1],
                "unsupported clipped Midori actor face"
            );
            filter.apply_gyu_palette(&mut face_bytes[36..36 + 1024]);
            Some(ImageCompositeLayer {
                input: Box::new(encoded(face_name, face_bytes, atlas_size)),
                source: [
                    (u32::from(face) * width) as i32,
                    0,
                    width as i32,
                    height as i32,
                ],
                destination: face_origin,
                clip: None,
                opacity: 255,
                // Native 0x150fe0 copies face indices with -2 (no key);
                // absent face alpha leaves the independently copied body alpha.
                math: ImageCompositeMath::CopyRgb,
            })
        } else {
            ensure!(cells == 1 && face == 0, "invalid Midori single-face body");
            None
        };
        // Native transforms the PALETTE after drawFaceForDraw. Applying
        // the same transform to both already validated matching palettes
        // before CopyRgb is equivalent; a separate alpha plane is unchanged.
        filter.apply_gyu_palette(&mut body_bytes[36..36 + 1024]);
        let base = encoded(body_name, body_bytes, size);
        Ok(if let Some(layer) = layer {
            base.with_composite_layer(layer)
        } else {
            base
        })
    }
}

fn encoded(name: String, bytes: Vec<u8>, size: [u32; 2]) -> ImageInput {
    ImageInput::Encoded {
        name,
        bytes: Arc::from(bytes),
        size,
    }
}

struct Indexed<'a> {
    size: [u32; 2],
    palette: &'a [u8],
}

impl<'a> Indexed<'a> {
    fn inspect(data: &'a [u8], body: bool) -> Result<Self> {
        let header = data.get(..36).context("short Midori actor GYU")?;
        let n = |i| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
        let flags = u16::from_le_bytes([header[4], header[5]]);
        let mode = u16::from_le_bytes([header[6], header[7]]);
        let alpha = n(28);
        let size = [n(16), n(20)];
        ensure!(
            &header[..4] == b"GYU\x1a"
                && matches!(mode, 0x0100 | 0x0400)
                && n(8) != 0
                && n(12) == 8
                && n(32) == 256
                && n(24) > 0
                && size.into_iter().all(|v| (1..=4096).contains(&v))
                && ((body && flags == 3 && alpha > 0) || (!body && flags == 0 && alpha == 0))
                && 36 + 1024 + u64::from(n(24)) + u64::from(alpha) == data.len() as u64,
            "unsupported Midori actor indexed color/alpha layout"
        );
        Ok(Self {
            size,
            palette: &data[36..36 + 1024],
        })
    }
}
