// SPDX-License-Identifier: GPL-3.0-or-later
// Audio layouts translated from arc_unpacker, Copyright (C) 2016 rr-.
//! Native audio becomes a standard WAVE resource on shared scoped storage.
mod codecs;
mod hca;
mod mio;
use crate::{
    common::Cursor,
    files::{at, ext, u16at, u32at},
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::service::AssetReader;
use crysirror_resource::{
    read_all, read_exact_at, DecodeContext, DecodedFile, FileFormat, FileKind, FileRegistry, Probe,
};
use std::sync::Arc;
const IDS: &[&str] = &[
    "cri/hca",
    "entis/mio",
    "abogado/v",
    "ivory/wady",
    "kid/waf",
    "leaf/p16",
    "leaf/w",
    "lilim/dwv",
    "mebius/koe",
    "microsoft/wav",
    "real-live/nwa",
    "triangle/wady",
    "twilight-frontier/pak2-sfx",
    "twilight-frontier/tfwa",
    "wild-bug/wpn",
    "wild-bug/wwa",
];
pub fn audio_format_ids() -> Vec<&'static str> {
    IDS.to_vec()
}
pub fn register_audio(registry: &mut FileRegistry) -> Result<()> {
    for &id in IDS {
        registry.register(Arc::new(Format(id)))?;
    }
    Ok(())
}
struct Format(&'static str);
impl FileFormat for Format {
    fn id(&self) -> &'static str {
        self.0
    }
    fn kind(&self) -> FileKind {
        FileKind::Audio
    }
    fn probe(&self, path: &str, source: &dyn AssetReader) -> Result<Probe> {
        let n = source.len().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut d = vec![0; n.min(256) as usize];
        read_exact_at(source, 0, &mut d)?;
        Ok(if recognized(self.0, path, &d, n as usize) {
            Probe::YES
        } else {
            Probe::NO
        })
    }
    fn decode(
        &self,
        path: &str,
        source: Arc<dyn AssetReader>,
        cx: &DecodeContext,
    ) -> Result<DecodedFile> {
        let d = read_all(source.as_ref(), cx.limits.max_entry_size)?;
        ensure!(
            recognized(self.0, path, &d, d.len()),
            "audio does not match {}",
            self.0
        );
        let mut metadata = serde_json::Value::Null;
        let reader = cx.decode_bounded(cx.limits.max_entry_size, || {
            let a = decode(self.0, path, &d, cx)?;
            ensure!(
                a.wav_size()? as u64 <= cx.limits.max_entry_size,
                "WAVE output limit"
            );
            metadata = serde_json::json!({"format":self.0,"codec":a.codec,"channels":a.channels,
                "sample_rate":a.rate,"bits_per_sample":a.bits,"loops":a.loops});
            a.wav()
        })?;
        Ok(DecodedFile {
            reader,
            extension: if metadata["loops"].as_array().is_some_and(|v| !v.is_empty()) {
                "wavloop"
            } else {
                "wav"
            }
            .into(),
            metadata,
        })
    }
}
fn recognized(id: &str, path: &str, d: &[u8], len: usize) -> bool {
    match id {
        "cri/hca" => d.starts_with(b"HCA\0"),
        "entis/mio" => {
            d.starts_with(b"Entis\x1a\0\0\0\x01\0\x03\0\0\0\0Music Interleaved and Orthogonal")
        }
        "abogado/v" => matches!(u32at(d, 0).ok(), Some(22050 | 44100)),
        "ivory/wady" | "triangle/wady" => d.starts_with(b"WADY"),
        "kid/waf" => d.starts_with(b"WAF\0\0\0"),
        "leaf/p16" => ext(path, "p16"),
        "leaf/w" => ext(path, "w") && u32at(d, 10).ok().is_some_and(|n| n as usize + 18 == len),
        "lilim/dwv" => ext(path, "dwv") && d.starts_with(b"DW"),
        "mebius/koe" => {
            d.starts_with(b"RIFF") && ["bgm", "mse", "koe"].iter().any(|e| ext(path, e))
        }
        "microsoft/wav" => d.starts_with(b"RIFF") && d.get(8..12) == Some(b"WAVE"),
        "real-live/nwa" => ext(path, "nwa"),
        "twilight-frontier/pak2-sfx" => ext(path, "cv3"),
        "twilight-frontier/tfwa" => d.starts_with(b"TFWA\0"),
        "wild-bug/wpn" => d.starts_with(b"WBD\x1aWAV\0"),
        "wild-bug/wwa" => d.starts_with(b"WPX\x1aWAV\0"),
        _ => false,
    }
}
#[derive(Default)]
pub(crate) struct Audio {
    pub codec: u16,
    pub channels: u16,
    pub rate: u32,
    pub bits: u16,
    pub align: u16,
    pub byte_rate: u32,
    pub extra: Vec<u8>,
    pub samples: Vec<u8>,
    pub loops: Vec<[u64; 3]>,
}
impl Audio {
    pub fn pcm(channels: u16, rate: u32, bits: u16, samples: Vec<u8>) -> Self {
        Self {
            codec: 1,
            channels,
            rate,
            bits,
            samples,
            ..Default::default()
        }
    }
    fn wave_spec(&self) -> crysirror_resource::WaveSpec {
        crysirror_resource::WaveSpec {
            format: self.codec,
            channels: self.channels,
            sample_rate: self.rate,
            bits_per_sample: self.bits,
            block_align: self.align,
            byte_rate: self.byte_rate,
            extra: self.extra.clone(),
            loops: self
                .loops
                .iter()
                .map(|l| crysirror_resource::WaveLoop {
                    start: l[0] as u32,
                    end: l[1] as u32,
                    play_count: l[2] as u32,
                })
                .collect(),
        }
    }
    fn wav_size(&self) -> Result<usize> {
        self.wave_spec().encoded_len(self.samples.len())
    }
    fn wav(self) -> Result<Vec<u8>> {
        self.wave_spec().encode(&self.samples)
    }
}
fn fmt(d: &[u8]) -> Result<Audio> {
    let mut c = Cursor::new(d);
    let codec = c.le16()?;
    let channels = c.le16()?;
    let rate = c.le32()?;
    let byte_rate = c.le32()?;
    let align = c.le16()?;
    let bits = c.le16()?;
    let extra = if c.left() > 0 {
        let n = c.le16()? as usize;
        c.take(n)?.to_vec()
    } else {
        Vec::new()
    };
    Ok(Audio {
        codec,
        channels,
        rate,
        byte_rate,
        align,
        bits,
        extra,
        ..Default::default()
    })
}
fn wave(d: &[u8]) -> Result<Audio> {
    ensure!(
        d.starts_with(b"RIFF") && at(d, 8, 4)? == b"WAVE",
        "WAVE header"
    );
    let mut c = Cursor::new(at(d, 12, d.len().saturating_sub(12))?);
    let mut head = None;
    let mut data = None;
    let mut loops = Vec::new();
    while c.left() >= 8 {
        let id = c.take(4)?;
        let n = c.le32()? as usize;
        let chunk = c.take(n)?;
        match id {
            b"fmt " => head = Some(fmt(chunk)?),
            b"data" => data = Some(chunk.to_vec()),
            b"smpl" => {
                let count = u32at(chunk, 28)? as usize;
                ensure!(
                    count <= chunk.len().saturating_sub(36) / 24,
                    "WAVE loop table"
                );
                for i in 0..count {
                    let p = 36 + i * 24;
                    loops.push([
                        u32at(chunk, p + 8)? as u64,
                        u32at(chunk, p + 12)? as u64,
                        u32at(chunk, p + 20)? as u64,
                    ]);
                }
            }
            _ => {}
        }
        if n & 1 != 0 && c.left() > 0 {
            c.take(1)?;
        }
    }
    let mut a = head.context("WAVE missing fmt")?;
    a.samples = data.context("WAVE missing data")?;
    a.loops = loops;
    Ok(a)
}
fn decode(id: &str, path: &str, d: &[u8], cx: &DecodeContext) -> Result<Audio> {
    let limit = cx.limits.max_entry_size as usize;
    let a = match id {
        "cri/hca" => hca::decode(d, cx)?,
        "entis/mio" => mio::decode(d, limit)?,
        "abogado/v" => codecs::v(d, limit)?,
        "ivory/wady" => codecs::ivory(d, limit)?,
        "triangle/wady" => codecs::triangle(d, limit)?,
        "leaf/p16" => {
            ensure!(d.len() % 2 == 0, "P16 sample alignment");
            Audio::pcm(1, 44100, 16, d.to_vec())
        }
        "leaf/w" => {
            let mut a = Audio::pcm(
                d[0] as u16,
                u16at(d, 2)? as u32,
                u16at(d, 4)?,
                at(d, 18, u32at(d, 10)? as usize)?.to_vec(),
            );
            a.align = d[1] as u16;
            a.byte_rate = u32at(d, 6)?;
            let lp = u32at(d, 14)? as u64;
            if lp > 0 {
                a.loops.push([lp, a.samples.len() as u64, 0]);
            }
            a
        }
        "kid/waf" => {
            let mut a = Audio {
                codec: 2,
                channels: u16at(d, 6)?,
                rate: u32at(d, 8)?,
                byte_rate: u32at(d, 12)?,
                align: u16at(d, 16)?,
                bits: u16at(d, 18)?,
                extra: at(d, 20, 32)?.to_vec(),
                ..Default::default()
            };
            a.samples = at(d, 56, u32at(d, 52)? as usize)?.to_vec();
            a
        }
        "lilim/dwv" => {
            let n = u32at(d, 4)? as usize;
            let mut a = fmt(at(d, 12, n)?)?;
            a.samples = at(d, 12 + n, u32at(d, 8)? as usize)?.to_vec();
            a
        }
        "microsoft/wav" => wave(d)?,
        "mebius/koe" => {
            let mut a = wave(d)?;
            let e = path
                .rsplit('.')
                .next()
                .context("KOE extension")?
                .to_ascii_lowercase();
            let name = format!("mebius/koe.{e}");
            let k = crate::files::key(cx, id, &name)?;
            k.with_bytes(|k| {
                for (i, b) in a.samples.iter_mut().enumerate() {
                    *b ^= k[i % k.len()];
                }
            });
            a
        }
        "real-live/nwa" => codecs::nwa(d, limit)?,
        "twilight-frontier/pak2-sfx" | "twilight-frontier/tfwa" => {
            let p = if id.ends_with("tfwa") { 5 } else { 0 };
            let mut a = fmt(at(d, p, 18)?)?;
            a.codec = 1;
            a.samples = at(d, p + 22, u32at(d, p + 18)? as usize)?.to_vec();
            a
        }
        "wild-bug/wpn" => {
            let mut c = Cursor::new(at(d, 8, d.len().saturating_sub(8))?);
            let n = c.le32()? as usize;
            ensure!(n <= c.left() / 12, "WPN chunk count");
            let mut f = None;
            let mut s = None;
            for _ in 0..n {
                let id = c.take(4)?;
                let p = c.le32()? as usize;
                let n = c.le32()? as usize;
                let chunk = at(d, p, n)?;
                match id {
                    b"fmt " => f = Some(fmt(chunk)?),
                    b"data" => s = Some(chunk.to_vec()),
                    _ => {}
                }
            }
            let mut a = f.context("WPN fmt missing")?;
            a.samples = s.context("WPN data missing")?;
            a
        }
        "wild-bug/wwa" => {
            let m = crate::files::wpx::section(d, 0x20, limit)?;
            let mut a = fmt(&m)?;
            a.samples = crate::files::wpx::section(d, 0x21, limit)?;
            a
        }
        _ => bail!("unknown audio decoder {id}"),
    };
    ensure!(a.samples.len() <= limit, "audio output limit");
    Ok(a)
}
