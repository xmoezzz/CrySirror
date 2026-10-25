// SPDX-License-Identifier: GPL-3.0-or-later
//! C-group CPU image formats translated from arc_unpacker at the pin recorded
//! in docs/arc-images-c.json. Native VM/resource naming remains the caller's job.
mod common;
mod elg;
mod extra;
mod gwd;
mod leaf;
mod liar;
mod majiro;
mod mgd;
mod misc;
mod pgd;
mod wbm;
use anyhow::{bail, ensure, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface, MAX_DECODED_BYTES};
pub const FORMATS: &[&str] = &[
    "abogado/kg",
    "bishop/bsg",
    "complets/gd",
    "cronus/grp",
    "lucifen/elg",
    "nsystem/mgd",
    "tabito/gwd",
    "wild-bug/wbm",
    "majiro/rc8",
    "majiro/rct",
    "amuse-craft/pgd-c00",
    "amuse-craft/pgd-ge",
    "gpk2/gfb",
    "gs/gfx",
    "fvp/nvsg",
    "yuka-script/ykg",
    "qlie/dpng",
    "aoi/agf",
    "aoi/iph",
    "leaf/bbm",
    "leaf/bjr",
    "leaf/grp",
    "leaf/lc3",
    "leaf/lf2",
    "leaf/lf3",
    "leaf/lfb",
    "leaf/lfg",
    "liar-soft/lim",
    "liar-soft/wcg",
    "libido/mnc",
    "lilim/abm",
    "lilim/dbm",
];
fn has(b: &[u8], offset: usize, magic: &[u8]) -> bool {
    b.get(offset..offset.saturating_add(magic.len())) == Some(magic)
}
fn ext(path: &str, extension: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| e.eq_ignore_ascii_case(extension))
}
pub fn probe(id: &str, path: &str, b: &[u8]) -> bool {
    match id {
        "abogado/kg" => has(b, 0, b"KG\x02"),
        "bishop/bsg" => has(b, 0, b"BSS-Graphics"),
        "complets/gd" => has(b, 0, b"GD2") || has(b, 0, b"GD3"),
        "cronus/grp" => extra::cronus_probe(b),
        "lucifen/elg" => has(b, 0, b"ELG"),
        "nsystem/mgd" => has(b, 0, b"MGD "),
        "tabito/gwd" => has(b, 4, b"GWD"),
        "wild-bug/wbm" => has(b, 0, b"WPX\x1aBMP\0"),
        "majiro/rc8" => has(b, 0, b"\x98\x5a\x92\x9a8"),
        "majiro/rct" => has(b, 0, b"\x98\x5a\x92\x9aT"),
        "amuse-craft/pgd-c00" => has(b, 24, b"00_C"),
        "amuse-craft/pgd-ge" => has(b, 0, b"GE \0"),
        "gpk2/gfb" => has(b, 0, b"GFB "),
        "gs/gfx" => has(b, 0, b"\0\0\x04\0"),
        "fvp/nvsg" => has(b, 0, b"hzc1") && has(b, 12, b"NVSG"),
        "yuka-script/ykg" => has(b, 0, b"YKG000"),
        "qlie/dpng" => has(b, 0, b"DPNG"),
        "aoi/agf" => has(b, 0, b"AGF\0"),
        "aoi/iph" => (has(b, 8, b"IPH\0") || has(b, 8, b"IPH ")) && has(b, 0x38, b"bmp "),
        "leaf/bbm" => ext(path, "bbm"),
        "leaf/bjr" => ext(path, "bjr") && has(b, 0, b"BM"),
        "leaf/grp" => {
            let mut c = common::Cursor::new(b);
            (move || -> Result<bool> {
                let mut w = c.u16()? as u32;
                let mut h = c.u16()? as u32;
                if w == 0 && h == 0 {
                    w = c.u16()? as u32;
                    h = c.u16()? as u32;
                }
                Ok(common::pixels(w, h)? == c.rest().len())
            })()
            .unwrap_or(false)
        }
        "leaf/lc3" => has(b, 0, b"LEAFC64\0"),
        "leaf/lf2" => has(b, 0, b"LEAF256\0"),
        "leaf/lf3" => has(b, 0, b"LEAF64K\0"),
        "leaf/lfb" => ext(path, "lfb"),
        "leaf/lfg" => has(b, 0, b"LEAFCODE"),
        "liar-soft/lim" => has(b, 0, b"LM") && b.get(2).is_some_and(|v| v & 0x10 != 0),
        "liar-soft/wcg" => {
            has(b, 0, b"WG")
                && b.get(2..4).is_some_and(|v| {
                    let n = u16::from_le_bytes([v[0], v[1]]);
                    n & 15 == 1 && n & 0x1c0 == 64
                })
        }
        "libido/mnc" => has(b, 0, b"HH6\x10\x0e\0\0\0\0\0"),
        "lilim/abm" => ext(path, "abm") && has(b, 0, b"BM"),
        "lilim/dbm" => {
            has(b, 0, b"DM")
                && b.get(4..8)
                    .is_some_and(|v| u32::from_le_bytes(v.try_into().unwrap()) as usize == b.len())
        }
        _ => false,
    }
}
pub fn decode(id: &str, b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    ensure!(
        b.len() <= MAX_DECODED_BYTES,
        "encoded C-group image exceeds 64 MiB"
    );
    ensure!(probe(id, ctx.path, b), "image does not match {id}");
    let image = match id {
        "abogado/kg" => extra::kg(b),
        "bishop/bsg" => extra::bsg(b),
        "complets/gd" => extra::gd(b),
        "cronus/grp" => extra::cronus(b, ctx.path),
        "lucifen/elg" => elg::decode(b),
        "nsystem/mgd" => mgd::decode(b, ctx),
        "tabito/gwd" => gwd::decode(b),
        "wild-bug/wbm" => wbm::decode(b),
        "majiro/rc8" => majiro::rc8(b),
        "majiro/rct" => majiro::rct(b, ctx),
        "amuse-craft/pgd-c00" => pgd::c00(b, ctx),
        "amuse-craft/pgd-ge" => pgd::ge(b),
        "gpk2/gfb" => misc::gfb(b),
        "gs/gfx" => misc::gs(b),
        "fvp/nvsg" => misc::nvsg(b),
        "yuka-script/ykg" => misc::ykg(b, ctx),
        "qlie/dpng" => misc::dpng(b, ctx),
        "aoi/agf" => misc::agf(b),
        "aoi/iph" => misc::iph(b),
        "leaf/bbm" => leaf::bbm(b),
        "leaf/bjr" => leaf::bjr(b, ctx),
        "leaf/grp" => leaf::grp(b, ctx),
        "leaf/lc3" | "leaf/lf2" | "leaf/lf3" | "leaf/lfb" => leaf::packed(id, b, ctx),
        "leaf/lfg" => leaf::lfg(b),
        "liar-soft/lim" => liar::decode(b, false),
        "liar-soft/wcg" => liar::decode(b, true),
        "libido/mnc" => misc::mnc(b),
        "lilim/abm" => misc::abm(b),
        "lilim/dbm" => misc::dbm(b),
        _ => bail!("unknown C-group image format: {id}"),
    }?;
    image.validate()?;
    Ok(image)
}
