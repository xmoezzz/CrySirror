// SPDX-License-Identifier: GPL-3.0-or-later
// Resource archive ports from arc_unpacker, Copyright (C) 2016 rr-.
use super::*;
mod images;
mod keyed;
mod pickle;
pub(crate) static FORMATS: &[Legacy] = &[
    Legacy {
        id: "will/pnap",
        probe: Some(|s| magic(s, 0, b"PNAP")),
        open: pnap,
    },
    Legacy {
        id: "will/wipf",
        probe: Some(|s| magic(s, 0, b"WIPF")),
        open: images::wipf,
    },
    Legacy {
        id: "renpy/rpa",
        probe: Some(|s| Ok(magic(s, 0, b"RPA-2.0 ")? || magic(s, 0, b"RPA-3.0 ")?)),
        open: pickle::rpa,
    },
    Legacy {
        id: "real-live/koepac",
        probe: Some(|s| magic(s, 0, b"KOEPAC\0\0")),
        open: koepac,
    },
    Legacy {
        id: "valkyria/odn",
        probe: None,
        open: images::odn,
    },
    Legacy {
        id: "whale/dat",
        probe: None,
        open: keyed::whale,
    },
    Legacy {
        id: "tactics/arc",
        probe: Some(|s| magic(s, 0, b"TACTICS_ARC_FILE")),
        open: keyed::tactics,
    },
    Legacy {
        id: "tanuki/tac",
        probe: Some(|s| Ok(magic(s, 0, b"TArc1.00\0\0\0\0")? || magic(s, 0, b"TArc1.10\0\0\0\0")?)),
        open: keyed::tac,
    },
];
fn pnap(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 20, 20)?;
    ensure!(h.starts_with(b"PNAP"), "PNAP signature");
    let count = le32(&h, 16)? as usize;
    limits(cx, count)?;
    let table = bytes(s, 20, count as u64 * 40, cx.limits.max_index_size)?;
    let mut offset = 20 + table.len() as u64;
    let mut rows = Vec::new();
    for i in 0..count {
        let n = le32(&table, i * 40 + 36)? as u64;
        if n > 0 {
            let sample = bytes(s, offset, n.min(16), 16)?;
            rows.push(Row::raw(
                format!("{:05}.{}", rows.len(), crate::files::guess(&sample)),
                offset,
                n,
            ));
            offset = offset.checked_add(n).context("PNAP offset overflow")?;
        }
    }
    Ok(rows)
}
fn koepac(s: &dyn AssetReader, cx: &DecodeContext) -> Result<Vec<Row>> {
    let h = bytes(s, 0, 32, 32)?;
    ensure!(h.starts_with(b"KOEPAC\0\0"), "KOEPAC signature");
    let n = le32(&h, 16)? as usize;
    limits(cx, n)?;
    let rate = match le32(&h, 24)? {
        0 => 22050,
        v => v,
    };
    let table = bytes(s, 32, n as u64 * 8, cx.limits.max_index_size)?;
    let mut c = Cursor::new(&table);
    let mut rows = Vec::new();
    for _ in 0..n {
        let id = c.le16()?;
        let blocks = c.le16()? as usize;
        let offset = c.le32()? as u64;
        let sizes = bytes(s, offset, blocks as u64 * 2, cx.limits.max_index_size)?;
        let stored = blocks as u64 * 2
            + sizes
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes(v.try_into().unwrap()) as u64)
                .sum::<u64>();
        range(length(s)?, offset, stored)?;
        let spec = crysirror_resource::WaveSpec {
            sample_rate: rate,
            ..Default::default()
        };
        let size = spec.encoded_len(blocks.checked_mul(2048).context("KOEPAC PCM size")?)?;
        ensure!(
            size as u64 <= cx.limits.max_entry_size,
            "KOEPAC output limit"
        );
        rows.push(Row::packed(
            format!("sample{id:05}.wav"),
            offset,
            stored,
            size as u64,
            decode(move |b, _| {
                let pcm = crysirror_audio::native::decode_koepac(b, 0, blocks, rate)?;
                let samples: Vec<_> = pcm.samples.iter().flat_map(|v| v.to_le_bytes()).collect();
                spec.encode(&samples)
            }),
        ));
    }
    Ok(rows)
}
fn options<'a>(cx: &'a DecodeContext, id: &str) -> Option<&'a serde_json::Value> {
    cx.parameters.get(id)
}
fn caller_key(
    cx: &DecodeContext,
    id: &str,
    default: &str,
) -> Result<crysirror_resource::KeyMaterial> {
    let alias = options(cx, id)
        .and_then(|v| v.get("key_id"))
        .and_then(|v| v.as_str())
        .unwrap_or(default);
    cx.keys.key(cx.client, alias)
}
