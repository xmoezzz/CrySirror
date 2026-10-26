// SPDX-License-Identifier: GPL-3.0-or-later
// Native archive semantics ported from arc_unpacker (C) 2016 rr-.
use super::*;
mod tsa;
mod twilight;
pub(crate) static FORMATS: &[Legacy] = &[
    Legacy {
        id: "team-shanghai-alice/anm",
        probe: None,
        open: tsa::anm,
    },
    Legacy {
        id: "team-shanghai-alice/pbg3",
        probe: Some(|s| magic(s, 0, b"PBG3")),
        open: tsa::pbg3,
    },
    Legacy {
        id: "team-shanghai-alice/pbg4",
        probe: Some(|s| magic(s, 0, b"PBG4")),
        open: tsa::pbg4,
    },
    Legacy {
        id: "team-shanghai-alice/pbgz",
        probe: Some(|s| magic(s, 0, b"PBGZ")),
        open: tsa::pbgz,
    },
    Legacy {
        id: "team-shanghai-alice/tha1",
        probe: Some(tsa::probe_tha1),
        open: tsa::tha1,
    },
    Legacy {
        id: "team-shanghai-alice/thbgm",
        probe: Some(|s| magic(s, 0, b"ZWAV")),
        open: tsa::thbgm,
    },
    Legacy {
        id: "twilight-frontier/pak1",
        probe: None,
        open: twilight::pak1,
    },
    Legacy {
        id: "twilight-frontier/pak1-gfx",
        probe: None,
        open: twilight::pak1_gfx,
    },
    Legacy {
        id: "twilight-frontier/pak1-sfx",
        probe: None,
        open: twilight::pak1_sfx,
    },
    Legacy {
        id: "twilight-frontier/pak2",
        probe: None,
        open: twilight::pak2,
    },
    Legacy {
        id: "twilight-frontier/tfpk",
        probe: Some(|s| Ok(magic(s, 0, b"TFPK")? && bytes(s, 4, 1, 1)?[0] <= 1)),
        open: twilight::tfpk,
    },
];
fn u32_at(s: &dyn AssetReader, at: u64) -> Result<u32> {
    le32(&bytes(s, at, 4, 4)?, 0)
}
fn u16_at(s: &dyn AssetReader, at: u64) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes(s, at, 2, 2)?.try_into().unwrap()))
}
fn profile<'a>(cx: &'a DecodeContext, id: &str) -> Result<&'a serde_json::Value> {
    cx.parameters
        .get(id)
        .with_context(|| format!("missing native archive parameters: {id}"))
}
fn number(p: &serde_json::Value, id: &str) -> Result<u64> {
    p[id]
        .as_u64()
        .with_context(|| format!("missing native integer parameter {id}"))
}
fn entry_size(cx: &DecodeContext, n: u64) -> Result<()> {
    ensure!(
        n <= cx.limits.max_entry_size,
        "archive decoded entry byte limit"
    );
    Ok(())
}
fn png_size(cx: &DecodeContext, w: u32, h: u32) -> Result<u64> {
    let size = crysirror_image::png_uncompressed_len(w, h)? as u64;
    entry_size(cx, size)?;
    Ok(size)
}
fn png(w: u32, h: u32, rgba: Vec<u8>) -> Result<Vec<u8>> {
    crysirror_image::encode_png_uncompressed(&crysirror_image::ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
fn ext(path: &str, extension: &str) -> String {
    match path.rsplit_once('.') {
        Some((base, _)) => format!("{base}.{extension}"),
        None => format!("{path}.{extension}"),
    }
}
fn pixel(c: &mut Cursor, kind: u8) -> Result<[u8; 4]> {
    Ok(match kind {
        1 => {
            let b = c.take(4)?;
            [b[2], b[1], b[0], b[3]]
        }
        3 => {
            let v = c.le16()?;
            [
                ((v & 0xf800) >> 8) as u8,
                ((v & 0x7e0) >> 3) as u8,
                ((v & 31) << 3) as u8,
                255,
            ]
        }
        5 => {
            let v = c.le16()?;
            [
                ((v & 0xf00) >> 4) as u8,
                (v & 0xf0) as u8,
                ((v & 15) << 4) as u8,
                ((v & 0xf000) >> 8) as u8,
            ]
        }
        7 => {
            let v = c.u8()?;
            [v, v, v, 255]
        }
        16 => {
            let v = c.le16()?;
            [
                ((v & 0x7c00) >> 7) as u8,
                ((v & 0x3e0) >> 2) as u8,
                ((v & 31) << 3) as u8,
                if v & 0x8000 != 0 { 255 } else { 0 },
            ]
        }
        _ => anyhow::bail!("unknown native pixel format"),
    })
}
