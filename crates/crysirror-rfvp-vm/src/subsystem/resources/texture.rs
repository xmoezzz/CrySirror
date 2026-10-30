#[cfg(all(feature = "zlib-flate2", not(feature = "no_std")))]
use ::std::io::Read;
#[cfg(feature = "no_std")]
use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
#[cfg(feature = "no_std")]
use anyhow::anyhow;
#[cfg(all(not(feature = "zlib-flate2"), feature = "uefi-zlib"))]
use anyhow::anyhow;
use anyhow::{bail, Result};
#[cfg(all(feature = "zlib-flate2", not(feature = "no_std")))]
use flate2::read::ZlibDecoder;
use std::path::Path;

use image::{DynamicImage, GrayAlphaImage, ImageBuffer};

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextureType {
    #[default]
    /// for background, etc...
    Single24Bit = 0,
    /// for character, etc...
    Single32Bit = 1,
    /// for parts
    Multi32Bit = 2,
    /// for mask
    Single8Bit = 3,
    /// PlayStation CLUT pictures (rfvp-psx-convert): 8-bit and 4-bit indices.
    Clut8 = 0x10,
    Clut4 = 0x11,
    /// for gaiji
    Single1Bit = 4,
}

impl TextureType {
    /// A single color picture: 24- or 32-bit, or a PlayStation CLUT picture in their place.
    pub fn is_picture(typ: Self) -> bool {
        match typ {
            Self::Single24Bit | Self::Single32Bit => true,
            #[cfg(feature = "psx")]
            Self::Clut8 | Self::Clut4 => true,
            _ => false,
        }
    }
}

impl TryFrom<u16> for TextureType {
    type Error = anyhow::Error;

    fn try_from(value: u16) -> Result<Self> {
        match value {
            0 => Ok(Self::Single24Bit),
            1 => Ok(Self::Single32Bit),
            2 => Ok(Self::Multi32Bit),
            3 => Ok(Self::Single8Bit),
            4 => Ok(Self::Single1Bit),
            #[cfg(feature = "psx")]
            0x10 => Ok(Self::Clut8),
            #[cfg(feature = "psx")]
            0x11 => Ok(Self::Clut4),
            _ => bail!("Invalid texture type: {}", value),
        }
    }
}

const HZC1_SIGNATURE: [u8; 4] = [b'h', b'z', b'c', b'1'];
const NVSG_SIGNATURE: [u8; 4] = [b'N', b'V', b'S', b'G'];

#[derive(Debug, Clone, Default)]
pub struct NvsgTexture {
    unknown1: u16,
    typ: TextureType,
    width: u16,
    height: u16,
    offset_x: u16,
    offset_y: u16,
    u: u16,
    v: u16,
    entry_count: u32,
    unknown3: u32,
    unknown4: u32,
    slices: Vec<Vec<u8>>,
    #[cfg(feature = "psx")]
    clut_stride: u32,
    #[cfg(feature = "psx")]
    clut_alpha: alloc::sync::Arc<Vec<u8>>,
    name: String,
    outlen: Option<usize>,
}

impl NvsgTexture {
    pub fn new(name: &str) -> Self {
        Self {
            unknown1: 0,
            typ: TextureType::Single24Bit,
            width: 0,
            height: 0,
            offset_x: 0,
            offset_y: 0,
            u: 0,
            v: 0,
            entry_count: 0,
            unknown3: 0,
            unknown4: 0,
            slices: vec![],
            #[cfg(feature = "psx")]
            clut_stride: 0,
            #[cfg(feature = "psx")]
            clut_alpha: Default::default(),
            name: name.to_string(),
            outlen: None,
        }
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }

    /// Drop decoded slice data while keeping header metadata and the stored name.
    ///
    /// This matches the original engine behavior for `PartsLoad(id, nil)`: the pixel buffer
    /// is released, but header fields and the name pointer are left untouched.
    pub fn clear_slices_keep_header(&mut self) {
        self.slices.clear();
        self.outlen = None;
    }

    fn read_u16le(&self, buff: &[u8], offset: usize) -> Result<u16> {
        if buff.len() < offset + 2 {
            bail!("buffer too small for u16");
        }
        Ok(u16::from_le_bytes([buff[offset], buff[offset + 1]]))
    }

    fn read_u32le(&self, buff: &[u8], offset: usize) -> Result<u32> {
        if buff.len() < offset + 4 {
            bail!("buffer too small for u32");
        }
        Ok(u32::from_le_bytes([
            buff[offset],
            buff[offset + 1],
            buff[offset + 2],
            buff[offset + 3],
        ]))
    }

    pub fn get_type(&self) -> TextureType {
        self.typ
    }

    pub fn get_width(&self) -> u16 {
        self.width
    }

    pub fn get_height(&self) -> u16 {
        self.height
    }

    pub fn get_offset_x(&self) -> u16 {
        self.offset_x
    }

    pub fn get_offset_y(&self) -> u16 {
        self.offset_y
    }

    pub fn get_u(&self) -> u16 {
        self.u
    }

    pub fn get_v(&self) -> u16 {
        self.v
    }

    pub fn get_entry_count(&self) -> u32 {
        self.entry_count
    }

    pub fn read_texture<F: FnOnce(TextureType) -> bool>(
        &mut self,
        buff: &[u8],
        type_callback: F,
    ) -> Result<()> {
        let decoded = crysirror_image::nvsg::NvsgImage::decode(buff)?;
        let typ = TextureType::try_from(decoded.format)?;
        if !type_callback(typ) {
            bail!("Unexpected texture type: {:?}", typ);
        }
        self.typ = typ;
        self.width = decoded.size[0];
        self.height = decoded.size[1];
        self.offset_x = decoded.offset[0];
        self.offset_y = decoded.offset[1];
        self.u = decoded.uv[0];
        self.v = decoded.uv[1];
        self.unknown1 = decoded.unknown[0] as u16;
        self.unknown3 = decoded.unknown[1];
        self.unknown4 = decoded.unknown[2];
        self.entry_count = decoded.frames.len() as u32;
        self.outlen = Some(decoded.frames.iter().map(|f| f.len()).sum());
        self.slices = decoded.frames;
        Ok(())
    }

    /// A CLUT picture, stored as it is (see rfvp-psx's image.rs): u16 colors, u16 stride, the
    /// palette, then each frame's indices.
    #[cfg(feature = "psx")]
    fn read_clut(&mut self, payload: &[u8]) -> Result<()> {
        let colors = self.read_u16le(payload, 0)? as usize;
        let stride = self.read_u16le(payload, 2)? as usize;
        let palette_end = 4 + colors * 2;
        // Each entry's real alpha follows the palette.
        let alpha_end = palette_end + colors;
        let alpha = payload
            .get(palette_end..alpha_end)
            .ok_or_else(|| anyhow!("CLUT alpha past the end"))?
            .to_vec();
        let row = if self.typ == TextureType::Clut4 {
            stride / 2
        } else {
            stride
        };
        let frame_len = row * self.height as usize;
        // Each frame as the GPU takes it: colors, stride, palette, its indices.
        self.slices.clear();
        for i in 0..self.entry_count as usize {
            let start = alpha_end + i * frame_len;
            match payload.get(start..start + frame_len) {
                Some(frame) => {
                    let mut bytes = Vec::with_capacity(palette_end + frame_len);
                    bytes.extend_from_slice(&payload[..palette_end]);
                    bytes.extend_from_slice(frame);
                    self.slices.push(bytes);
                }
                None => break,
            }
        }
        self.clut_alpha = alloc::sync::Arc::new(alpha);
        self.clut_stride = stride as u32;
        self.outlen = Some(payload.len());
        Ok(())
    }

    /// A one-frame CLUT file made into its picture in place (the file's bytes become the
    /// picture's: nothing is copied but the alpha table). Returns the file back when it is not
    /// one (another type, several frames), for `read_texture`.
    #[cfg(feature = "psx")]
    pub fn read_single_clut(
        &mut self,
        mut buff: Vec<u8>,
    ) -> core::result::Result<DynamicImage, Vec<u8>> {
        let field16 =
            |b: &[u8], at: usize| b.get(at..at + 2).map(|x| u16::from_le_bytes([x[0], x[1]]));
        let field32 = |b: &[u8], at: usize| {
            b.get(at..at + 4)
                .map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
        };
        let (Some(header_len), Some(typ), Some(count)) =
            (field32(&buff, 8), field16(&buff, 18), field32(&buff, 32))
        else {
            return Err(buff);
        };
        if buff.get(0..4) != Some(&HZC1_SIGNATURE[..]) || !(typ == 0x10 || typ == 0x11) || count > 1
        {
            return Err(buff);
        }
        let h = 12usize;
        self.typ = if typ == 0x11 {
            TextureType::Clut4
        } else {
            TextureType::Clut8
        };
        self.width = field16(&buff, h + 8).unwrap_or(0);
        self.height = field16(&buff, h + 10).unwrap_or(0);
        self.offset_x = field16(&buff, h + 12).unwrap_or(0);
        self.offset_y = field16(&buff, h + 14).unwrap_or(0);
        self.u = field16(&buff, h + 16).unwrap_or(0);
        self.v = field16(&buff, h + 18).unwrap_or(0);
        self.entry_count = 1;
        let start = h + header_len as usize;
        let (Some(colors), Some(stride)) = (field16(&buff, start), field16(&buff, start + 2))
        else {
            return Err(buff);
        };
        let (colors, stride) = (colors as usize, stride as usize);
        let palette_end = start + 4 + colors * 2;
        let alpha_end = palette_end + colors;
        let row = if typ == 0x11 { stride / 2 } else { stride };
        let frame_end = alpha_end + row * self.height as usize;
        if buff.len() < frame_end {
            return Err(buff);
        }
        let alpha = buff[palette_end..alpha_end].to_vec();
        buff.truncate(frame_end);
        buff.drain(palette_end..alpha_end);
        buff.drain(..start);
        buff.shrink_to_fit();
        self.clut_alpha = alloc::sync::Arc::new(alpha);
        self.clut_stride = stride as u32;
        Ok(DynamicImage::ImageClut(crate::ClutImage {
            width: self.width as u32,
            height: self.height as u32,
            stride: stride as u32,
            bits: if typ == 0x11 { 4 } else { 8 },
            bytes: buff,
            alpha: self.clut_alpha.clone(),
        }))
    }

    pub fn texture_color_tone_32(
        &mut self,
        index: usize,
        red_value: i32,
        green_value: i32,
        blue_value: i32,
    ) -> Result<()> {
        if index >= self.slices.len() {
            bail!("Invalid index: {}", index);
        }

        if self.typ != TextureType::Single32Bit && self.typ != TextureType::Multi32Bit {
            bail!("Invalid texture type: {:?}", self.typ);
        }

        crysirror_image::associated::tone_channels_percent(
            &mut self.slices[index],
            [blue_value, green_value, red_value],
        )
    }

    #[cfg(not(feature = "no_std"))]
    fn extract_8bit_texture(&self, index: usize, out_path: impl AsRef<Path>) -> Result<()> {
        let slice = &self.slices[index];
        let mut img = GrayAlphaImage::new(self.width as u32, self.height as u32);

        for (x, y, mut pixel) in img.enumerate_pixels_mut() {
            let index = (y * self.width as u32 + x) as usize;
            let alpha_value = slice[index];
            *pixel = image::LumaA([0xff, alpha_value]);
        }

        img.save(out_path)?;
        Ok(())
    }

    fn as_8bit_texture(&self, index: usize) -> Result<GrayAlphaImage> {
        let slice = &self.slices[index];
        let mut img = GrayAlphaImage::new(self.width as u32, self.height as u32);

        for (x, y, mut pixel) in img.enumerate_pixels_mut() {
            let index = (y * self.width as u32 + x) as usize;
            let alpha_value = slice[index];
            *pixel = image::LumaA([0xff, alpha_value]);
        }

        Ok(img)
    }

    #[cfg(not(feature = "no_std"))]
    fn extract_24bit_texture(&self, index: usize, out_path: impl AsRef<Path>) -> Result<()> {
        let slice = &self.slices[index];
        let mut img = ImageBuffer::new(self.width as u32, self.height as u32);

        for (x, y, mut pixel) in img.enumerate_pixels_mut() {
            let index = (y * self.width as u32 + x) as usize * 3;
            let r = slice[index + 2];
            let g = slice[index + 1];
            let b = slice[index + 0];
            *pixel = image::Rgb([r, g, b]);
        }

        img.save(out_path)?;
        Ok(())
    }

    fn as_24bit_texture(&self, index: usize) -> Result<ImageBuffer<image::Rgb<u8>, Vec<u8>>> {
        let slice = &self.slices[index];
        let mut img = ImageBuffer::new(self.width as u32, self.height as u32);

        for (x, y, mut pixel) in img.enumerate_pixels_mut() {
            let index = (y * self.width as u32 + x) as usize * 3;
            let r = slice[index + 2];
            let g = slice[index + 1];
            let b = slice[index + 0];
            *pixel = image::Rgb([r, g, b]);
        }

        Ok(img)
    }

    fn as_24bit_to_32bit_texture(
        &self,
        index: usize,
    ) -> Result<ImageBuffer<image::Rgba<u8>, Vec<u8>>> {
        let slice = self
            .slices
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Invalid index: {index}"))?;
        let rgba = crysirror_image::pixels::to_rgba8(
            [self.width as u32, self.height as u32],
            slice,
            crysirror_image::pixels::PixelLayout::Bgr,
        )?;
        ImageBuffer::from_raw(self.width as u32, self.height as u32, rgba)
            .ok_or_else(|| anyhow::anyhow!("Invalid image dimensions"))
    }

    #[cfg(not(feature = "no_std"))]
    fn extract_32bit_texture(&self, index: usize, out_path: impl AsRef<Path>) -> Result<()> {
        let slice = &self.slices[index];
        let mut img = ImageBuffer::new(self.width as u32, self.height as u32);

        for (x, y, mut pixel) in img.enumerate_pixels_mut() {
            let index = (y * self.width as u32 + x) as usize * 4;
            let r = slice[index + 2];
            let g = slice[index + 1];
            let b = slice[index + 0];
            let a = slice[index + 3];
            *pixel = image::Rgba([r, g, b, a]);
        }

        img.save(out_path)?;
        Ok(())
    }

    fn as_32bit_texture(&self, index: usize) -> Result<ImageBuffer<image::Rgba<u8>, Vec<u8>>> {
        let slice = self
            .slices
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("Invalid index: {index}"))?;
        let rgba = crysirror_image::pixels::to_rgba8(
            [self.width as u32, self.height as u32],
            slice,
            crysirror_image::pixels::PixelLayout::Bgra,
        )?;
        ImageBuffer::from_raw(self.width as u32, self.height as u32, rgba)
            .ok_or_else(|| anyhow::anyhow!("Invalid image dimensions"))
    }

    #[cfg(not(feature = "no_std"))]
    pub fn extract_textures(&self, output_dir: impl AsRef<Path>) -> Result<()> {
        let output_dir = output_dir.as_ref();
        if !output_dir.exists() {
            std::fs::create_dir(output_dir)?;
        }

        for i in 0..self.entry_count as usize {
            let output = output_dir.join(format!("texture_{}.bmp", i));
            match self.typ {
                TextureType::Single8Bit | TextureType::Single1Bit => {
                    self.extract_8bit_texture(i, output)?
                }
                TextureType::Single24Bit => self.extract_24bit_texture(i, output)?,
                TextureType::Single32Bit | TextureType::Multi32Bit => {
                    self.extract_32bit_texture(i, output)?
                }
                _ => bail!("Invalid texture type: {:?}", self.typ),
            }
        }

        Ok(())
    }

    /// PS1-only derivative; CPU reads continue to return the original Area alpha.
    /// Reuse the input allocation for GPU pixels so loading does not duplicate a full mask.
    #[cfg(feature = "psx")]
    pub fn read_single_ps1_mask(
        &mut self,
        mut bytes: Vec<u8>,
    ) -> core::result::Result<DynamicImage, Vec<u8>> {
        let (width, height, start, length) = {
            let plane = match crate::ps1_mask::read(&bytes) {
                Ok(Some(plane)) if plane.frames == 1 => plane,
                _ => return Err(bytes),
            };
            let frame = plane.frame(0).unwrap();
            (
                plane.width,
                plane.height,
                frame.as_ptr() as usize - bytes.as_ptr() as usize,
                frame.len(),
            )
        };
        let field16 = |at| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let field32 = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let expected = usize::from(width) * usize::from(height);
        if field32(4) as usize != expected {
            return Err(bytes);
        }
        let compressed = 12 + field32(8) as usize;
        // Exclude the uncompressed GPU trailer and decode into the exact-sized alpha plane.
        let Some(compressed_end) = start.checked_sub(crate::ps1_mask::HEADER_SIZE) else {
            return Err(bytes);
        };
        if compressed > compressed_end {
            return Err(bytes);
        }
        let mut alpha = vec![0u8; expected];
        match miniz_oxide::inflate::decompress_slice_iter_to_slice(
            &mut alpha,
            core::iter::once(&bytes[compressed..compressed_end]),
            true,
            false,
        ) {
            Ok(length) if length == expected => {}
            _ => return Err(bytes),
        }
        self.typ = TextureType::Single8Bit;
        self.unknown1 = field16(16);
        self.width = width;
        self.height = height;
        self.offset_x = field16(24);
        self.offset_y = field16(26);
        self.u = field16(28);
        self.v = field16(30);
        self.entry_count = 1;
        self.unknown3 = field32(36);
        self.unknown4 = field32(40);
        self.outlen = Some(expected);
        self.slices.clear();
        bytes.copy_within(start..start + length, 0);
        bytes.truncate(length);
        Ok(DynamicImage::ImagePs1Mask(crate::Ps1MaskImage {
            width: u32::from(width),
            height: u32::from(height),
            alpha,
            gpu: bytes,
        }))
    }

    #[cfg(feature = "psx")]
    pub fn get_ps1_mask_texture(&self, index: usize, bytes: &[u8]) -> Result<DynamicImage> {
        let plane = crate::ps1_mask::read(bytes).map_err(|error| anyhow!(error))?;
        let Some(plane) = plane else {
            log::warn!(
                "PS1 mask {} has no offline Bayer plane; reconvert it for mask dissolves",
                self.name
            );
            return self.get_texture(index);
        };
        let alpha = self
            .slices
            .get(index)
            .ok_or_else(|| anyhow!("invalid mask frame"))?;
        let gpu = plane
            .frame(index)
            .ok_or_else(|| anyhow!("invalid mask GPU frame"))?;
        Ok(DynamicImage::ImagePs1Mask(crate::Ps1MaskImage {
            width: u32::from(plane.width),
            height: u32::from(plane.height),
            alpha: alpha.clone(),
            gpu: gpu.to_vec(),
        }))
    }

    pub fn get_texture(&self, index: usize) -> Result<DynamicImage> {
        if index >= self.slices.len() {
            bail!("Invalid index: {}", index);
        }

        let img = match self.typ {
            #[cfg(feature = "psx")]
            TextureType::Clut8 | TextureType::Clut4 => DynamicImage::ImageClut(crate::ClutImage {
                width: self.width as u32,
                height: self.height as u32,
                stride: self.clut_stride,
                bits: if self.typ == TextureType::Clut4 { 4 } else { 8 },
                bytes: self.slices[index].clone(),
                alpha: self.clut_alpha.clone(),
            }),
            TextureType::Single8Bit | TextureType::Single1Bit => {
                DynamicImage::ImageLumaA8(self.as_8bit_texture(index)?)
            }
            TextureType::Single24Bit => {
                DynamicImage::ImageRgba8(self.as_24bit_to_32bit_texture(index)?)
            }
            TextureType::Single32Bit | TextureType::Multi32Bit => {
                DynamicImage::ImageRgba8(self.as_32bit_texture(index)?)
            }
            _ => bail!("Invalid texture type: {:?}", self.typ),
        };

        Ok(img)
    }
}

#[repr(C, packed)]
#[derive(Default)]
struct HZC1HDR {
    signature: [u8; 4],
    original_length: u32,
    header_length: u32,
}

#[cfg(all(test, not(feature = "no_std")))]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_read_texture() {
        let filepath = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/testcase/BGS016b"));
        let mut file = std::fs::File::open(filepath).unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).unwrap();

        let mut container = NvsgTexture::new("");
        container
            .read_texture(&buffer, |typ: TextureType| true)
            .unwrap();
        let output = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/testcase"));
        container.extract_textures(output).unwrap();
    }

    #[test]
    fn test_read_texture_2() {
        let filepath = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testcase/BGS016a_parts"
        ));
        let mut file = std::fs::File::open(filepath).unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).unwrap();
        let mut container = NvsgTexture::new("");
        container
            .read_texture(&buffer, |typ: TextureType| true)
            .unwrap();
        let output = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testcase/BGS016a_parts.dir"
        ));
        container.extract_textures(output).unwrap();
    }

    #[test]
    fn test_read_texture_3() {
        let filepath = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testcase/gaiji_shiru156"
        ));
        let mut file = std::fs::File::open(filepath).unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).unwrap();
        let mut container = NvsgTexture::new("");
        container
            .read_texture(&buffer, |typ: TextureType| true)
            .unwrap();
        assert!(!container.slices.is_empty());

        let output = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testcase/gaiji_shiru156.dir"
        ));
        container.extract_textures(output).unwrap();
    }

    #[test]
    fn test_read_texture_3_2() {
        let filepath = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testcase/gaiji_shiru156"
        ));
        let mut file = std::fs::File::open(filepath).unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).unwrap();
        let mut container = NvsgTexture::new("");
        container
            .read_texture(&buffer, |typ: TextureType| true)
            .unwrap();
        assert!(!container.slices.is_empty());

        // print all metadata
        crate::trace::vm(format_args!("type: {:?}", container.typ));
        crate::trace::vm(format_args!("width: {}", container.width));
        crate::trace::vm(format_args!("height: {}", container.height));
        crate::trace::vm(format_args!("offset_x: {}", container.offset_x));
        crate::trace::vm(format_args!("offset_y: {}", container.offset_y));
        crate::trace::vm(format_args!("u: {}", container.u));
        crate::trace::vm(format_args!("v: {}", container.v));
        crate::trace::vm(format_args!("entry_count: {}", container.entry_count));
        crate::trace::vm(format_args!("unknown3: {}", container.unknown3));
        crate::trace::vm(format_args!("unknown4: {}", container.unknown4));
    }

    #[test]
    fn test_read_texture_4() {
        let filepath = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/testcase/sd_302_70"));
        let mut file = std::fs::File::open(filepath).unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).unwrap();
        let mut container = NvsgTexture::new("");
        container
            .read_texture(&buffer, |typ: TextureType| true)
            .unwrap();
        let output = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testcase/sd_302_70.dir"
        ));
        container.extract_textures(output).unwrap();
    }

    #[test]
    fn test_read_texture_5() {
        let filepath = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/testcase/sd_302_70"));
        let mut file = std::fs::File::open(filepath).unwrap();
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).unwrap();
        let mut container = NvsgTexture::new("");
        container
            .read_texture(&buffer, |typ: TextureType| true)
            .unwrap();
        let output = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testcase/sd_302_70_tone"
        ));

        container.texture_color_tone_32(0, 50, 50, 50).unwrap();
        container.extract_textures(output).unwrap();
    }
}
