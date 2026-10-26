// SPDX-License-Identifier: GPL-3.0-or-later
// Native layouts and codecs ported from arc_unpacker (C) 2016 rr-.
use super::*;
mod crypt;
mod pictures;
mod tables;

pub(crate) static FORMATS: &[Legacy] = &[
    Legacy {
        id: "kaguya/link",
        probe: Some(tables::probe_link),
        open: tables::link,
    },
    Legacy {
        id: "kaguya/link2",
        probe: Some(|s| magic(s, 0, b"LIN2")),
        open: tables::link2,
    },
    Legacy {
        id: "kaguya/link3",
        probe: Some(|s| magic(s, 0, b"LINK3")),
        open: |s, c| tables::linkn(s, c, 3),
    },
    Legacy {
        id: "kaguya/link4",
        probe: Some(|s| magic(s, 0, b"LINK4")),
        open: |s, c| tables::linkn(s, c, 4),
    },
    Legacy {
        id: "kaguya/link5",
        probe: Some(|s| magic(s, 0, b"LINK5")),
        open: |s, c| tables::linkn(s, c, 5),
    },
    Legacy {
        id: "kaguya/link6",
        probe: Some(|s| magic(s, 0, b"LINK6")),
        open: |s, c| tables::linkn(s, c, 6),
    },
    Legacy {
        id: "kaguya/wfl",
        probe: Some(|s| magic(s, 0, b"WFL1")),
        open: tables::wfl,
    },
    Legacy {
        id: "kaguya/an00",
        probe: Some(|s| magic(s, 0, b"AN00")),
        open: |s, c| pictures::animation(s, c, 0),
    },
    Legacy {
        id: "kaguya/an10",
        probe: Some(|s| magic(s, 0, b"AN10")),
        open: |s, c| pictures::animation(s, c, 10),
    },
    Legacy {
        id: "kaguya/an20",
        probe: Some(|s| magic(s, 0, b"AN20")),
        open: |s, c| pictures::animation(s, c, 20),
    },
    Legacy {
        id: "kaguya/an21",
        probe: Some(|s| magic(s, 0, b"AN21")),
        open: |s, c| pictures::animation(s, c, 21),
    },
    Legacy {
        id: "kaguya/pl00",
        probe: Some(|s| magic(s, 0, b"PL00")),
        open: |s, c| pictures::animation(s, c, 100),
    },
    Legacy {
        id: "kaguya/pl10",
        probe: Some(|s| magic(s, 0, b"PL10")),
        open: |s, c| pictures::animation(s, c, 110),
    },
    Legacy {
        id: "leaf/a",
        probe: Some(|s| magic(s, 0, b"\x1e\xaf")),
        open: tables::leaf_a,
    },
    Legacy {
        id: "leaf/ar10",
        probe: Some(|s| magic(s, 0, b"ar10")),
        open: tables::ar10,
    },
    Legacy {
        id: "leaf/cz10",
        probe: Some(|s| magic(s, 0, b"cz10")),
        open: pictures::cz10,
    },
    Legacy {
        id: "leaf/kcap",
        probe: Some(|s| magic(s, 0, b"KCAP")),
        open: tables::kcap,
    },
    Legacy {
        id: "leaf/lac",
        probe: Some(|s| magic(s, 0, b"LAC\0")),
        open: tables::lac,
    },
    Legacy {
        id: "leaf/leafpack",
        probe: Some(|s| magic(s, 0, b"LEAFPACK")),
        open: crypt::leafpack,
    },
    Legacy {
        id: "leaf/pak1",
        probe: None,
        open: tables::pak1,
    },
    Legacy {
        id: "leaf/pak2",
        probe: None,
        open: tables::pak2,
    },
    Legacy {
        id: "leaf/pak2-image",
        probe: Some(|s| magic(s, 4, b"\x5f\xf8\x6d\x75")),
        open: pictures::pak2_image,
    },
    Legacy {
        id: "leaf/pak2-texture",
        probe: Some(|s| magic(s, 4, b"\x88\x33\x67\x82")),
        open: pictures::pak2_texture,
    },
    Legacy {
        id: "leaf/px",
        probe: None,
        open: pictures::px,
    },
    Legacy {
        id: "nekopack/nekopack4",
        probe: Some(|s| magic(s, 0, b"NEKOPACK4A")),
        open: tables::nekopack4,
    },
    Legacy {
        id: "nitroplus/npa",
        probe: Some(|s| magic(s, 0, b"NPA\x01\0\0\0")),
        open: crypt::npa,
    },
    Legacy {
        id: "nitroplus/npa-sg",
        probe: None,
        open: crypt::npa_sg,
    },
    Legacy {
        id: "nitroplus/npk2",
        probe: Some(|s| magic(s, 0, b"NPK2")),
        open: crypt::npk2,
    },
    Legacy {
        id: "pajamas/gamedat",
        probe: Some(|s| magic(s, 0, b"GAMEDAT PAC")),
        open: tables::gamedat,
    },
    Legacy {
        id: "playstation/gpda",
        probe: Some(tables::probe_gpda),
        open: tables::gpda,
    },
    Legacy {
        id: "playstation/gxt",
        probe: Some(|s| magic(s, 0, b"GXT\0")),
        open: pictures::gxt,
    },
    Legacy {
        id: "propeller/mgr",
        probe: None,
        open: tables::mgr,
    },
    Legacy {
        id: "purple-software/cpz5",
        probe: Some(|s| magic(s, 0, b"CPZ5")),
        open: |s, c| crypt::cpz(s, c, 5),
    },
    Legacy {
        id: "purple-software/cpz6",
        probe: Some(|s| magic(s, 0, b"CPZ6")),
        open: |s, c| crypt::cpz(s, c, 6),
    },
    Legacy {
        id: "qlie/abmp7",
        probe: Some(|s| magic(s, 0, b"ABMP7")),
        open: tables::abmp7,
    },
    Legacy {
        id: "qlie/abmp10",
        probe: Some(|s| {
            Ok(magic(s, 0, b"abmp10\0\0\0\0\0\0\0\0\0\0")?
                || magic(s, 0, b"abmp11\0\0\0\0\0\0\0\0\0\0")?
                || magic(s, 0, b"abmp12\0\0\0\0\0\0\0\0\0\0")?)
        }),
        open: tables::abmp10,
    },
    Legacy {
        id: "qlie/pack",
        probe: Some(crypt::probe_pack),
        open: crypt::pack,
    },
];
fn u32_at(s: &dyn AssetReader, at: u64) -> Result<u32> {
    le32(&bytes(s, at, 4, 4)?, 0)
}
fn u16_at(s: &dyn AssetReader, at: u64) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes(s, at, 2, 2)?.try_into().unwrap()))
}
fn profile(cx: &DecodeContext, id: &str) -> Result<serde_json::Value> {
    cx.parameters
        .get(id)
        .cloned()
        .with_context(|| format!("missing native archive parameters: {id}"))
}
fn key_bytes(cx: &DecodeContext, id: &str) -> Result<crysirror_resource::KeyMaterial> {
    let p = profile(cx, id)?;
    let alias = p["key_id"]
        .as_str()
        .context("native archive parameters require key_id")?;
    cx.keys.key(cx.client, alias)
}
fn inflate(data: &[u8], size: usize) -> Result<Vec<u8>> {
    let out = inflate_index(data, size as u64)?;
    ensure!(out.len() == size, "inflated size mismatch");
    Ok(out)
}
fn lzss(data: &[u8], size: usize) -> Result<Vec<u8>> {
    crysirror_resource::lzss_decompress(data, size, &Default::default())
}
fn sized(cx: &DecodeContext, size: u64) -> Result<usize> {
    ensure!(
        size <= cx.limits.max_entry_size && size <= usize::MAX as u64,
        "archive entry byte limit"
    );
    Ok(size as usize)
}
