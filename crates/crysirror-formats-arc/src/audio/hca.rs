// SPDX-License-Identifier: GPL-3.0-or-later
// Container/output policy adapted from arc_unpacker, Copyright (C) 2016 rr-.
//! Pinned pure-Rust CRI codec through a bounded engine-resource adapter.
//! Full blocks and 32767/truncate PCM conversion preserve arc_unpacker's policy.
use super::{at, Audio};
use anyhow::{ensure, Context, Result};
use crysirror_resource::DecodeContext;
pub fn decode(d: &[u8], cx: &DecodeContext) -> Result<Audio> {
    ensure!(d.len() >= 8, "HCA header");
    let h = u16::from_be_bytes(d[6..8].try_into().unwrap()) as usize;
    ensure!((40..=4096).contains(&h), "HCA header size limit");
    let header = at(d, 0, h)?;
    // cridecoder's public decoder validates checksums and bounds; additionally
    // reject negative band differences before its unsigned group calculation.
    let masked: Vec<_> = header.iter().map(|v| v & 127).collect();
    let comp = masked.windows(4).position(|w| w == b"comp");
    if let Some(p) = comp {
        let b = at(header, p, 16)?;
        ensure!(
            b[10] as u16 >= b[11] as u16 + b[12] as u16,
            "HCA inconsistent band counts"
        );
    } else if let Some(p) = masked.windows(4).position(|w| w == b"dec\0") {
        let b = at(header, p, 12)?;
        ensure!(
            b[8] >= b[9] || b[11] == 0,
            "HCA inconsistent dec band counts"
        );
    }
    let mut dec = Box::new(cridecoder::hca::ClHca::new());
    dec.decode_header(header)?;
    let n = (dec.frame_count as usize)
        .checked_mul(1024)
        .and_then(|n| n.checked_mul(dec.channels as usize))
        .and_then(|n| n.checked_mul(2))
        .context("HCA decoded size overflow")?;
    ensure!(n as u64 <= cx.limits.max_entry_size, "HCA output limit");
    let packed = (dec.frame_count as usize)
        .checked_mul(dec.frame_size as usize)
        .context("HCA encoded size overflow")?;
    at(d, h, packed)?;
    if dec.ciph_type == 56 {
        let key = crate::files::key(cx, "cri/hca", "cri/hca.keycode")?;
        let code = key.with_bytes(|k| -> Result<u64> {
            ensure!(k.len() == 8, "HCA keycode requires 8 little-endian bytes");
            Ok(u64::from_le_bytes(k.try_into().unwrap()))
        })?;
        ensure!(code != 0, "HCA cipher56 requires nonzero keycode");
        dec.set_key(code);
    }
    let mut out = Vec::with_capacity(n);
    let mut buf = vec![0.0; dec.channels as usize * 1024];
    for i in 0..dec.frame_count as usize {
        let mut block = at(d, h + i * dec.frame_size as usize, dec.frame_size as usize)?.to_vec();
        dec.decode_block(&mut block)?;
        dec.read_samples(&mut buf);
        for s in &buf {
            ensure!(s.is_finite(), "HCA nonfinite sample");
            out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    let mut a = Audio::pcm(dec.channels as u16, dec.sample_rate, 16, out);
    if dec.loop_flag {
        let repetitions = if dec.loop_start_delay == 128 {
            0
        } else {
            dec.loop_start_delay as u64
        };
        a.loops.push([
            dec.loop_start_frame as u64 * 1024 * dec.sample_rate as u64,
            dec.loop_end_frame as u64 * 1024 * dec.sample_rate as u64,
            repetitions,
        ]);
    }
    Ok(a)
}
