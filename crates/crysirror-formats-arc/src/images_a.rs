// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU image codecs translated from the pinned arc_unpacker source (rr-, GPLv3+).
mod alice;
pub(crate) mod common;
mod eri;
pub(crate) mod eri_entropy;
mod extra;
mod jbp1;
mod kaguya;
mod pb3;
mod pdt;
mod spb;
mod tlg;
use anyhow::Result;
use crysirror_image::{ImageDecodeContext, ImageSurface};
pub const FORMATS: &[&str] = &[
    "entis/eri",
    "kid/prt",
    "lizsoft/sotes",
    "purple-software/pb3",
    "alice-soft/ajp",
    "alice-soft/dcf",
    "alice-soft/pms",
    "alice-soft/qnt",
    "alice-soft/vsp",
    "kaguya/ao",
    "kaguya/ap",
    "kaguya/ap0",
    "kaguya/ap2",
    "kaguya/ap3",
    "kaguya/aps3",
    "kaguya/raw-mask",
    "nscripter/spb",
    "real-live/pdt9",
    "real-live/pdt10",
    "real-live/g00",
    "kirikiri/tlg",
];
pub fn probe(id: &str, path: &str, b: &[u8]) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("");
    match id {
        "entis/eri" => b.starts_with(b"Entis\x1a\0\0\0\x01\0\x03\0\0\0\0Entis Rasterized Image"),
        "kid/prt" => b.starts_with(b"PRT\0"),
        "lizsoft/sotes" => extra::sotes_probe(b),
        "purple-software/pb3" => b.starts_with(b"PB3B"),
        "alice-soft/ajp" => b.starts_with(b"AJP\0"),
        "alice-soft/dcf" => b.starts_with(b"dcf "),
        "alice-soft/pms" => b.starts_with(b"PM"),
        "alice-soft/qnt" => b.starts_with(b"QNT\0"),
        "alice-soft/vsp" => ext.eq_ignore_ascii_case("vsp"),
        "kaguya/ao" => b.starts_with(b"AO"),
        "kaguya/ap" => b.starts_with(b"AP") && !b.starts_with(b"AP-"),
        "kaguya/ap0" => b.starts_with(b"AP-0"),
        "kaguya/ap2" => b.starts_with(b"AP-2"),
        "kaguya/ap3" => b.starts_with(b"AP-3"),
        "kaguya/aps3" => b.starts_with(b"\x04APS3"),
        "kaguya/raw-mask" => {
            if b.len() < 8 {
                return false;
            }
            let w = u32::from_le_bytes(b[0..4].try_into().unwrap()) as u64;
            let h = u32::from_le_bytes(b[4..8].try_into().unwrap()) as u64;
            w > 0 && h > 0 && w <= 8192 && h <= 8192 && 8 + w * h == b.len() as u64
        }
        "nscripter/spb" => {
            ext.eq_ignore_ascii_case("bmp") && b.len() > 4 && !b.starts_with(b"BM") && {
                let w = u16::from_be_bytes([b[0], b[1]]);
                let h = u16::from_be_bytes([b[2], b[3]]);
                w > 0 && h > 0 && w <= 5000 && h <= 5000
            }
        }
        "real-live/pdt9" => b.starts_with(b"9") && ext.eq_ignore_ascii_case("pdt"),
        "real-live/pdt10" => b.starts_with(b"PDT10\0\0\0"),
        "real-live/g00" => ext.eq_ignore_ascii_case("g00") && b.first().is_some_and(|v| *v <= 2),
        "kirikiri/tlg" => [b"TLG0.0\0sds\x1a", b"TLG5.0\0raw\x1a", b"TLG6.0\0raw\x1a"]
            .iter()
            .any(|x| b.starts_with(*x)),
        _ => false,
    }
}
pub fn decode(id: &str, b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    match id {
        "entis/eri" => eri::decode(b, ctx),
        "kid/prt" => extra::prt(b),
        "lizsoft/sotes" => extra::sotes(b),
        "purple-software/pb3" => pb3::decode(b, ctx),
        "alice-soft/ajp" => alice::ajp(b),
        "alice-soft/dcf" => alice::dcf(b, ctx),
        "alice-soft/pms" => alice::pms(b),
        "alice-soft/qnt" => alice::qnt(b),
        "alice-soft/vsp" => alice::vsp(b),
        id if id.starts_with("kaguya/") => kaguya::decode(id, b),
        "nscripter/spb" => spb::decode(b),
        "real-live/pdt9" => pdt::pdt9(b),
        "real-live/pdt10" => pdt::pdt10(b),
        // One RealLive g00 implementation: the engine atlas decoder, whose
        // layout matches the pattern regions RealLive objects select.
        "real-live/g00" => {
            let atlas = crysirror_image::g00::decode_g00(b)?;
            Ok(ImageSurface {
                width: u32::try_from(atlas.width)?,
                height: u32::try_from(atlas.height)?,
                rgba: atlas.rgba,
            })
        }
        "kirikiri/tlg" => tlg::decode(b, ctx),
        _ => anyhow::bail!("unsupported A-group image {id}"),
    }
}
