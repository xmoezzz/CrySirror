// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{ensure, Result};
use crysirror_image::{DecoderRegistry, ImageDecodeContext, ImageFormat, ImageSurface};
use std::sync::Arc;
type Probe = fn(&str, &str, &[u8]) -> bool;
type Decode = fn(&str, &[u8], &ImageDecodeContext<'_>) -> Result<ImageSurface>;
struct Plugin {
    id: &'static str,
    probe: Probe,
    decode: Decode,
}
impl ImageFormat for Plugin {
    fn id(&self) -> &'static str {
        self.id
    }
    fn priority(&self) -> u8 {
        match self.id {
            "png/png" | "jpeg/jpeg" | "google/webp" | "microsoft/bmp" | "microsoft/dds"
            | "truevision/tga" => 10,
            _ => 50,
        }
    }
    fn recognize(&self, path: &str, bytes: &[u8]) -> bool {
        (self.probe)(self.id, path, bytes)
    }
    fn priority_for(&self,path:&str,_:&[u8])->u8{
        let extension=path.rsplit_once('.').map(|(_,extension)|extension.to_ascii_lowercase()).unwrap_or_default();
        match self.id {
            "minato-soft/fil"=>if extension=="fil"{50}else{1},
            "kaguya/raw-mask"=>if ["msk","mask","raw"].contains(&extension.as_str()){50}else{1},
            "alice-soft/vsp"|"nscripter/spb"|"eagls/gr"|"twilight-frontier/pak2-gfx"|"scene-player/pmp"|"nekopack/masked-bmp"|"truevision/tga"|"real-live/g00"|"real-live/pdt9"=>5,
            _=>self.priority(),
        }
    }
    fn decode(&self, bytes: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
        (self.decode)(self.id, bytes, ctx)
    }
}
pub fn image_format_ids() -> Vec<&'static str> {
    let mut ids = vec!["bgi/cbg"];
    ids.extend_from_slice(crate::images_a::FORMATS);
    ids.extend_from_slice(crate::images_b::FORMATS);
    ids.extend_from_slice(crate::images_c::FORMATS);
    ids.sort_unstable();
    ids.into_iter()
        .chain(crate::images_d::FORMATS.iter().copied())
        .collect()
}
pub fn register_images(registry: &mut DecoderRegistry) -> Result<()> {
    for &id in crate::images_d::FORMATS {
        registry.register_format(Arc::new(Plugin {
            id,
            probe: crate::images_d::probe,
            decode: crate::images_d::decode,
        }))?;
    }
    for &id in crate::images_a::FORMATS {
        registry.register_format(Arc::new(Plugin {
            id,
            probe: crate::images_a::probe,
            decode: crate::images_a::decode,
        }))?;
    }
    for &id in crate::images_b::FORMATS {
        registry.register_format(Arc::new(Plugin {
            id,
            probe: crate::images_b::probe,
            decode: crate::images_b::decode,
        }))?;
    }
    for &id in crate::images_c::FORMATS {
        registry.register_format(Arc::new(Plugin {
            id,
            probe: crate::images_c::probe,
            decode: crate::images_c::decode,
        }))?;
    }
    registry.register_format(Arc::new(Plugin {
        id: "bgi/cbg",
        probe: |_, _, bytes| bytes.starts_with(b"CompressedBG___\0"),
        decode: |_, bytes, _| {
            let info = ethornell_image::parse_cbg_metadata(bytes)
                .map_err(|e| anyhow::anyhow!("CBG header: {e}"))?;
            crysirror_image::rgba_len(info.width, info.height)?;
            ensure!(
                info.intermediate_length.unwrap_or(0) as usize
                    <= crysirror_image::MAX_DECODED_BYTES,
                "CBG intermediate byte limit"
            );
            ensure!(
                info.encoded_length.unwrap_or(0) as usize <= crysirror_image::MAX_DECODED_BYTES,
                "CBG encoded byte limit"
            );
            let image = ethornell_image::decode_cbg(bytes)
                .map_err(|e| anyhow::anyhow!("CBG decode: {e}"))?;
            image.validate()?;
            Ok(image)
        },
    }))?;
    Ok(())
}
