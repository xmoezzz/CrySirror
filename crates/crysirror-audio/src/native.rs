// SPDX-License-Identifier: MPL-2.0
//! Standalone native audio resources over the engine's streaming-source API.
//! NWA uses the pinned siglus_assets codec, not a VM or original sound engine.
//! KOEPAC PCM math is from siglus_rs/reallive/src/sound.rs at
//! 5f358ef1360b0439e92aed2b749e8f25d8721954 (MPL-2.0).
use crate::decoder::{AudioSource, MAX_PACKET_FRAMES, StereoFrame};
use anyhow::{Context, Result, ensure};
use siglus_assets::nwa::NwaReader;
use std::{io::Cursor, sync::Arc};

pub const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_NWA_UNITS: u64 = 262_144;
pub const MAX_NWA_BLOCK_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_PCM_SAMPLES: usize = 16 * 1024 * 1024;

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    let value = bytes
        .get(at..at + 2)
        .context("truncated native audio u16")?;
    Ok(u16::from_le_bytes(
        value.try_into().expect("checked length"),
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    let value = bytes
        .get(at..at + 4)
        .context("truncated native audio u32")?;
    Ok(u32::from_le_bytes(
        value.try_into().expect("checked length"),
    ))
}

/// NWA has no magic. Only a consistent mono/stereo PCM header, exact encoded
/// length, scalar sample count and (for compression) bounded block table match.
/// Other containers return None and continue through the standard decoder.
/// A recognizable but malformed NWA header fails instead of silently succeeding.
#[derive(Clone, Copy)]
struct NwaSpec {
    channels: u16,
    bits: u16,
    rate: u32,
    frames: u64,
}

pub(crate) fn may_be_nwa(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && matches!(u16::from_le_bytes([bytes[0], bytes[1]]), 1 | 2)
        && matches!(u16::from_le_bytes([bytes[2], bytes[3]]), 8 | 16)
}
fn metadata(bytes: &[u8], encoded_len: u64) -> Result<Option<NwaSpec>> {
    if bytes.len() < 4 {
        return Ok(None);
    }
    let channels = u16_at(&bytes, 0)?;
    let bits = u16_at(&bytes, 2)?;
    if !(1..=2).contains(&channels) || !matches!(bits, 8 | 16) {
        return Ok(None);
    }
    ensure!(
        bytes.len() >= 44 && (44..=u64::from(u32::MAX)).contains(&encoded_len),
        "NWA encoded/header byte budget"
    );
    let rate = u32_at(&bytes, 4)?;
    let pack = u32_at(&bytes, 8)? as i32;
    let zero = u32_at(&bytes, 12)?;
    let units = u64::from(u32_at(&bytes, 16)?);
    let original_bytes = u64::from(u32_at(&bytes, 20)?);
    let packed_bytes = u64::from(u32_at(&bytes, 24)?);
    let samples = u64::from(u32_at(&bytes, 28)?);
    let unit_samples = u64::from(u32_at(&bytes, 32)?);
    let last_samples = u64::from(u32_at(&bytes, 36)?);
    let last_packed = u64::from(u32_at(&bytes, 40)?);
    ensure!((8000..=192000).contains(&rate), "NWA sample rate");
    ensure!(
        (-1..=5).contains(&pack) && zero <= 1,
        "NWA compression/header mode"
    );
    ensure!(
        samples > 0 && samples % u64::from(channels) == 0,
        "NWA scalar samples/channel alignment"
    );
    ensure!(
        original_bytes == samples * u64::from(bits / 8),
        "NWA original size/sample count mismatch"
    );
    // Raw-PCM files (pack -1) may leave the packed size unset (0), as
    // RealLive's AIR SE BGM does; the reference reader never consults it.
    let packed_bytes = if pack == -1 && packed_bytes == 0 {
        encoded_len
    } else {
        packed_bytes
    };
    ensure!(
        packed_bytes == encoded_len,
        "NWA packed size differs from encoded input"
    );
    if pack == -1 {
        ensure!(
            44 + original_bytes == packed_bytes,
            "NWA raw PCM byte length"
        );
    } else {
        ensure!(bits == 16, "compressed NWA requires 16-bit PCM");
        ensure!(
            units > 0 && units <= MAX_NWA_UNITS,
            "NWA block table budget"
        );
        ensure!(
            unit_samples > 0 && unit_samples <= (MAX_PACKET_FRAMES as u64 * u64::from(channels)),
            "NWA decoded block budget"
        );
        ensure!(
            last_samples > 0
                && last_samples <= unit_samples
                && unit_samples % u64::from(channels) == 0
                && last_samples % u64::from(channels) == 0,
            "NWA decoded block alignment"
        );
        ensure!(
            (units - 1) * unit_samples + last_samples == samples,
            "NWA block sample counts differ from header"
        );
        let table_end = 44 + units * 4;
        ensure!(table_end <= packed_bytes, "NWA block table exceeds input");
        for index in 0..units {
            let start = u64::from(u32_at(&bytes, 44 + index as usize * 4)?);
            let end = if index + 1 == units {
                start
                    .checked_add(last_packed)
                    .context("NWA final block range overflow")?
            } else {
                u64::from(u32_at(&bytes, 44 + (index as usize + 1) * 4)?)
            };
            ensure!(
                start >= table_end
                    && end <= packed_bytes
                    && end >= start + u64::from(channels) * 2
                    && end - start <= MAX_NWA_BLOCK_BYTES,
                "NWA compressed block range/budget"
            );
        }
    }
    Ok(Some(NwaSpec {
        channels,
        bits,
        rate,
        frames: samples / u64::from(channels),
    }))
}

fn source<R: std::io::Read + std::io::Seek + Send + 'static>(
    reader: R,
    spec: NwaSpec,
) -> Result<Box<dyn AudioSource>> {
    Ok(Box::new(NwaSource {
        reader: NwaReader::open_from_reader(reader)?,
        channels: spec.channels,
        bits: spec.bits,
        rate: spec.rate,
        frames: spec.frames,
    }))
}

pub fn open_if_native(bytes: Arc<[u8]>) -> Result<Option<Box<dyn AudioSource>>> {
    if !may_be_nwa(&bytes) {
        return Ok(None);
    }
    ensure!(
        bytes.len() <= MAX_ENCODED_BYTES,
        "resident NWA encoded byte budget"
    );
    let Some(spec) = metadata(&bytes, bytes.len() as u64)? else {
        return Ok(None);
    };
    Ok(Some(source(Cursor::new(bytes), spec)?))
}

/// Stream from a caller-owned capability reader. Only the header/table and one
/// compressed block are resident; total decoded song size is never allocated.
pub fn open_reader(
    mut reader: Box<dyn symphonia::core::io::MediaSource>,
) -> Result<Box<dyn AudioSource>> {
    use std::io::{Read, Seek, SeekFrom};
    ensure!(reader.is_seekable(), "NWA needs a seekable source");
    let start = reader.stream_position()?;
    let end = reader.seek(SeekFrom::End(0))?;
    let size = end
        .checked_sub(start)
        .context("NWA source start exceeds end")?;
    ensure!(
        (44..=u64::from(u32::MAX)).contains(&size),
        "NWA encoded source length"
    );
    reader.seek(SeekFrom::Start(start))?;
    let mut prefix = vec![0u8; 44];
    reader.read_exact(&mut prefix)?;
    ensure!(may_be_nwa(&prefix), "source is not NWA");
    if u32_at(&prefix, 8)? as i32 != -1 {
        let units = u64::from(u32_at(&prefix, 16)?);
        ensure!(
            units > 0 && units <= MAX_NWA_UNITS && 44 + units * 4 <= size,
            "NWA block table budget"
        );
        prefix.resize(44 + units as usize * 4, 0);
        reader.read_exact(&mut prefix[44..])?;
    }
    let spec = metadata(&prefix, size)?.context("source is not NWA")?;
    reader.seek(SeekFrom::Start(start))?;
    source(reader, spec)
}

struct NwaSource {
    reader: NwaReader,
    channels: u16,
    bits: u16,
    rate: u32,
    frames: u64,
}
impl AudioSource for NwaSource {
    fn channel_count(&self) -> Option<u16> {
        Some(self.channels)
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn frame_count(&self) -> Option<u64> {
        Some(self.frames)
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        let remaining = self
            .frames
            .saturating_sub(u64::from(self.reader.get_read_sample_pos()));
        let wanted = out.len().min(MAX_PACKET_FRAMES).min(remaining as usize);
        if wanted == 0 {
            return Ok(0);
        }
        let pcm = self.reader.read_samples(wanted as u32)?;
        let stride = usize::from(self.channels) * usize::from(self.bits / 8);
        ensure!(
            pcm.len() % stride == 0 && pcm.len() <= wanted * stride,
            "NWA codec frame alignment"
        );
        let frames = pcm.len() / stride;
        let sample = |bytes: &[u8]| -> f32 {
            if self.bits == 16 {
                f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32768.
            } else {
                (f32::from(bytes[0]) - 128.) / 128.
            }
        };
        for (frame, bytes) in out[..frames].iter_mut().zip(pcm.chunks_exact(stride)) {
            let left = sample(bytes);
            *frame = if self.channels == 1 {
                [left, left]
            } else {
                [left, sample(&bytes[usize::from(self.bits / 8)..])]
            };
        }
        Ok(frames)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        ensure!(frame <= self.frames, "NWA frame seek outside source");
        self.reader.set_read_sample_pos(frame as u32);
        Ok(())
    }
}

/// Bounded signed PCM returned by a resource codec, independent of VM/channel
/// policy. Conversion to WAVE allows the existing shared mixer API to consume it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}
impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }
    pub fn duration_ms(&self) -> u64 {
        (self.frames() as u64)
            .saturating_mul(1000)
            .checked_div(u64::from(self.rate))
            .unwrap_or(0)
    }
    pub fn to_wav(&self) -> Result<Vec<u8>> {
        ensure!(
            (8000..=192000).contains(&self.rate) && (1..=2).contains(&self.channels),
            "PCM rate/channels"
        );
        ensure!(
            self.samples.len() <= MAX_PCM_SAMPLES
                && self.samples.len() % usize::from(self.channels) == 0,
            "PCM sample budget/alignment"
        );
        let data_len = (self.samples.len() * 2) as u32;
        let block = self.channels * 2;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&self.channels.to_le_bytes());
        wav.extend_from_slice(&self.rate.to_le_bytes());
        wav.extend_from_slice(&(self.rate * u32::from(block)).to_le_bytes());
        wav.extend_from_slice(&block.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for sample in &self.samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(wav)
    }
}

/// Expansion of 8-bit `KOEPAC` samples to 16 bits.
fn koe_sample(byte: u8) -> i16 {
    // A signed square law, saturated at both ends.
    match byte {
        0 => i16::MIN,
        128 => 0,
        255 => i16::MAX,
        _ => {
            let v = i32::from(byte) - 128;
            (v.signum() * (2 * v * v - 1)) as i16
        }
    }
}

/// DPCM step for a code (0, -1, +1, -2, +2, ...).
fn koe_delta(code: u8) -> u8 {
    if code & 1 == 0 {
        code >> 1
    } else {
        0xff - (code >> 1)
    }
}

/// Decodes one `KOEPAC` entry: a table of block sizes, then the blocks.
pub fn decode_koepac(bytes: &[u8], offset: usize, blocks: usize, rate: u32) -> Result<Pcm> {
    ensure!((8000..=192000).contains(&rate), "KOEPAC sample rate");
    ensure!(
        blocks <= MAX_PCM_SAMPLES / 0x400,
        "KOEPAC decoded sample budget"
    );
    let table_end = offset
        .checked_add(blocks.checked_mul(2).context("KOEPAC table overflow")?)
        .context("KOEPAC table range overflow")?;
    ensure!(table_end <= bytes.len(), "KOEPAC block table exceeds input");
    let mut sizes = Vec::with_capacity(blocks);
    for i in 0..blocks {
        sizes.push(usize::from(u16_at(bytes, offset + i * 2)?));
    }
    let mut at = table_end;
    let mut samples = Vec::with_capacity(blocks * 0x400);
    for size in sizes {
        let block = bytes
            .get(
                at..at
                    .checked_add(size)
                    .context("KOEPAC block range overflow")?,
            )
            .context("reallive: truncated KOE block")?;
        match size {
            0 => samples.extend(std::iter::repeat_n(0, 0x400)),
            0x400 => samples.extend(block.iter().map(|&b| koe_sample(b))),
            _ => {
                let mut d = 0u8;
                let mut produced = 0;
                let mut j = 0;
                while j < size && produced < 0x400 {
                    let mut s = block[j];
                    if (s.wrapping_add(1)) & 0x0f != 0 {
                        d = d.wrapping_sub(koe_delta(s & 0x0f));
                    } else {
                        let low = (s >> 4) & 0x0f;
                        j += 1;
                        s = block.get(j).copied().unwrap_or(0);
                        d = d.wrapping_sub(koe_delta(low | ((s << 4) & 0xf0)));
                    }
                    samples.push(koe_sample(d));
                    s >>= 4;
                    if (s.wrapping_add(1)) & 0x0f != 0 {
                        d = d.wrapping_sub(koe_delta(s & 0x0f));
                    } else {
                        j += 1;
                        d = d.wrapping_sub(koe_delta(block.get(j).copied().unwrap_or(0)));
                    }
                    samples.push(koe_sample(d));
                    produced += 2;
                    j += 1;
                }
                samples.resize(samples.len() + (0x400 - produced.min(0x400)), 0);
            }
        }
        at += size;
    }
    Ok(Pcm {
        rate,
        channels: 1,
        samples,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn raw(channels: u16, bits: u16, pcm: &[u8]) -> Arc<[u8]> {
        let mut out = vec![0; 44];
        out[..2].copy_from_slice(&channels.to_le_bytes());
        out[2..4].copy_from_slice(&bits.to_le_bytes());
        put32(&mut out, 4, 48000);
        put32(&mut out, 8, u32::MAX);
        put32(&mut out, 20, pcm.len() as u32);
        put32(&mut out, 24, 44 + pcm.len() as u32);
        put32(&mut out, 28, (pcm.len() / usize::from(bits / 8)) as u32);
        out.extend_from_slice(pcm);
        out.into()
    }
    fn compressed() -> Arc<[u8]> {
        let mut out = vec![0; 52];
        out[..2].copy_from_slice(&2u16.to_le_bytes());
        out[2..4].copy_from_slice(&16u16.to_le_bytes());
        put32(&mut out, 4, 48000);
        put32(&mut out, 8, 1);
        put32(&mut out, 16, 2);
        put32(&mut out, 20, 16);
        put32(&mut out, 24, 64);
        put32(&mut out, 28, 8);
        put32(&mut out, 32, 4);
        put32(&mut out, 36, 4);
        put32(&mut out, 40, 6);
        put32(&mut out, 44, 52);
        put32(&mut out, 48, 58);
        for n in [1000i16, 2000] {
            out.extend_from_slice(&n.to_le_bytes());
            out.extend_from_slice(&(-n).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
        }
        out.into()
    }
    #[test]
    fn nwa_raw_pcm_seek_eof_and_mono_stereo_conversion_are_exact() {
        let bytes = raw(
            1,
            16,
            &[-32768i16, 0, 16384, 32767]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let mut source = open_if_native(bytes.clone()).unwrap().unwrap();
        assert_eq!(Arc::strong_count(&bytes), 2);
        assert_eq!(source.frame_count(), Some(4));
        assert_eq!(source.sample_rate(), 48000);
        let mut out = [[5.; 2]; 8];
        assert_eq!(source.read(&mut out).unwrap(), 4);
        assert_eq!(out[..3], [[-1.; 2], [0.; 2], [0.5; 2]]);
        assert_eq!(out[3], [32767. / 32768.; 2]);
        assert_eq!(source.read(&mut out).unwrap(), 0);
        source.seek(2).unwrap();
        assert_eq!(source.read(&mut out).unwrap(), 2);
        assert_eq!(out[0], [0.5; 2]);
        assert!(source.seek(5).is_err());
        source.seek(4).unwrap();
        assert_eq!(source.read(&mut out).unwrap(), 0);
        drop(source);
        assert_eq!(Arc::strong_count(&bytes), 1);
        let mut source = open_if_native(raw(2, 8, &[0, 255, 128, 64]))
            .unwrap()
            .unwrap();
        assert_eq!(source.read(&mut out).unwrap(), 2);
        assert_eq!(out[0], [-1., 127. / 128.]);
        assert_eq!(out[1], [0., -0.5]);
    }
    #[test]
    fn nwa_compressed_units_and_frame_seeks_preserve_original_predictors() {
        let bytes = compressed();
        let mut source = open_if_native(bytes.clone()).unwrap().unwrap();
        let mut original = NwaReader::open_from_bytes(bytes.to_vec()).unwrap();
        let expected = original.read_samples(4).unwrap();
        let mut out = [[0.; 2]; 4];
        assert_eq!(source.read(&mut out).unwrap(), 4);
        let expected = expected
            .chunks_exact(4)
            .map(|p| {
                [
                    f32::from(i16::from_le_bytes([p[0], p[1]])) / 32768.,
                    f32::from(i16::from_le_bytes([p[2], p[3]])) / 32768.,
                ]
            })
            .collect::<Vec<_>>();
        assert_eq!(&out, expected.as_slice());
        assert_eq!(out[0], [1000. / 32768., -1000. / 32768.]);
        assert_eq!(out[2], [2000. / 32768., -2000. / 32768.]);
        source.seek(1).unwrap();
        assert_eq!(source.read(&mut out[..2]).unwrap(), 2);
        assert_eq!(out[0], [1000. / 32768., -1000. / 32768.]);
        assert_eq!(out[1], [2000. / 32768., -2000. / 32768.]);
    }
    #[test]
    fn standard_containers_are_not_misidentified_and_bad_native_headers_fail() {
        for bytes in [
            b"RIFF----WAVE".as_slice(),
            b"OggS",
            b"fLaC",
            &[0xff, 0xfb, 0x90, 0],
            b"",
        ] {
            assert!(open_if_native(Arc::from(bytes)).unwrap().is_none());
        }
        for at in [4usize, 8, 16, 20, 24, 28, 32, 36, 40, 44, 48] {
            let mut bytes = compressed().to_vec();
            put32(&mut bytes, at, u32::MAX);
            assert!(
                open_if_native(bytes.into()).is_err(),
                "invalid NWA field {at}"
            );
        }
        assert!(open_if_native(Arc::from([1u8, 0, 16, 0])).is_err());
    }
    #[test]
    fn native_nwa_requests_reach_the_common_channel_mixer() {
        use crysirror_core::service::{AudioParameters, AudioRequest, EncodedAudio};
        let pcm = [16384i16; 16]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        let mut audio = crate::channels::ChannelAudio::default();
        audio
            .submit(
                3,
                AudioRequest::Play {
                    channel: 5,
                    source: EncodedAudio {
                        bytes: raw(2, 16, &pcm),
                        frame_limit: None,
                    },
                    intro: None,
                    parameters: AudioParameters {
                        looping: true,
                        ..Default::default()
                    },
                },
            )
            .unwrap();
        assert!(audio.is_playing(3, 5));
        assert!(!audio.is_playing(4, 5));
        assert!(!audio.is_playing(3, 99));
        audio.render(1).unwrap();
        let output = audio.take_output();
        assert_eq!(output.len(), 48);
        assert!(output.iter().all(|p| p[0] > 0.1 && p[1] > 0.1));
        audio
            .submit(
                3,
                AudioRequest::Stop {
                    channel: 5,
                    fade_ms: 0.,
                },
            )
            .unwrap();
        audio.render(1).unwrap();
        assert!(!audio.is_playing(3, 5));
    }
    #[test]
    fn koepac_sample_math_blocks_and_wav_output_preserve_pinned_semantics() {
        let mut data = Vec::new();
        for size in [0u16, 0x400, 2] {
            data.extend_from_slice(&size.to_le_bytes());
        }
        data.extend(std::iter::repeat_n(128, 0x400));
        data.extend_from_slice(&[0x21, 0]);
        let pcm = decode_koepac(&data, 0, 3, 22050).unwrap();
        assert_eq!(pcm.channels, 1);
        assert_eq!(pcm.samples.len(), 0xc00);
        assert!(pcm.samples[..0x800].iter().all(|x| *x == 0));
        assert_eq!(pcm.samples[0x800], koe_sample(1));
        assert_eq!(pcm.samples[0x801], koe_sample(0));
        assert_eq!(koe_sample(0), i16::MIN);
        assert_eq!(koe_sample(128), 0);
        assert_eq!(koe_sample(255), i16::MAX);
        let wav = pcm.to_wav().unwrap();
        let mut stream = crate::decoder::Stream::from_bytes(wav.into(), Some("wav")).unwrap();
        let mut output = vec![[0.; 2]; pcm.frames()];
        assert_eq!(stream.read(&mut output).unwrap(), pcm.frames());
        for (frame, sample) in output.iter().zip(&pcm.samples) {
            assert_eq!(*frame, [f32::from(*sample) / 32768.; 2]);
        }
        assert!(decode_koepac(&data, usize::MAX, 3, 22050).is_err());
        assert!(decode_koepac(&data, 0, MAX_PCM_SAMPLES / 1024 + 1, 22050).is_err());
        assert!(decode_koepac(&data[..8], 0, 3, 22050).is_err());
        assert!(decode_koepac(&data, 0, 3, 0).is_err());
    }
    #[test]
    #[ignore = "requires CRYSIRROR_NWA_DIRECTORY; reads owner-provided NWA files without copying them into the repository"]
    fn installed_native_audio_prefix_and_seek_match_original_codec() {
        let directory =
            std::env::var_os("CRYSIRROR_NWA_DIRECTORY").expect("set an existing NWA directory");
        let mut tested = 0;
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if !path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("nwa"))
            {
                continue;
            }
            let bytes: Arc<[u8]> = std::fs::read(&path).unwrap().into();
            let mut source = open_if_native(bytes.clone()).unwrap().unwrap();
            let mut streamed =
                crate::decoder::open_reader(Box::new(Cursor::new(bytes.clone())), Some("nwa"))
                    .unwrap();
            let mut original = NwaReader::open_from_reader(Cursor::new(bytes)).unwrap();
            let channels = usize::from(original.header().channels);
            assert_eq!(original.header().bits_per_sample, 16);
            let frames = source.frame_count().unwrap();
            for start in [0, frames / 2, frames.saturating_sub(32)] {
                let wanted = 32.min(frames - start) as usize;
                source.seek(start).unwrap();
                original.set_read_sample_pos(start as u32);
                let expected = original.read_samples(wanted as u32).unwrap();
                let mut output = vec![[0.; 2]; wanted];
                assert_eq!(source.read(&mut output).unwrap(), wanted);
                streamed.seek(start).unwrap();
                let mut via_reader = vec![[0.; 2]; wanted];
                assert_eq!(streamed.read(&mut via_reader).unwrap(), wanted);
                assert_eq!(via_reader, output);
                for (frame, bytes) in output.iter().zip(expected.chunks_exact(channels * 2)) {
                    let l = f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32768.;
                    let r = if channels == 1 {
                        l
                    } else {
                        f32::from(i16::from_le_bytes([bytes[2], bytes[3]])) / 32768.
                    };
                    assert_eq!(*frame, [l, r]);
                }
            }
            tested += 1;
        }
        assert!(tested > 0);
        eprintln!("native NWA sources compared: {tested}");
    }
}
#[cfg(test)]
mod provider_tests {
    use super::*;
    use std::{
        io::{Read, Seek, SeekFrom},
        sync::atomic::{AtomicUsize, Ordering},
    };
    struct Counted {
        bytes: Cursor<Vec<u8>>,
        count: Arc<AtomicUsize>,
    }
    impl Read for Counted {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = self.bytes.read(out)?;
            self.count.fetch_add(n, Ordering::SeqCst);
            Ok(n)
        }
    }
    impl Seek for Counted {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.bytes.seek(from)
        }
    }
    impl symphonia::core::io::MediaSource for Counted {
        fn is_seekable(&self) -> bool {
            true
        }
        fn byte_len(&self) -> Option<u64> {
            Some(self.bytes.get_ref().len() as u64)
        }
    }
    #[test]
    fn provider_nwa_decodes_only_a_bounded_prefix_and_seeks_in_frames() {
        let samples = 1_000_000u32;
        let mut bytes = vec![0; 44 + samples as usize * 2];
        bytes[..2].copy_from_slice(&1u16.to_le_bytes());
        bytes[2..4].copy_from_slice(&16u16.to_le_bytes());
        for (at, n) in [
            (4, 48000),
            (8, u32::MAX),
            (20, samples * 2),
            (24, 44 + samples * 2),
            (28, samples),
        ] {
            bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
        }
        for p in bytes[44..].chunks_exact_mut(2) {
            p.copy_from_slice(&16384i16.to_le_bytes());
        }
        let count = Arc::new(AtomicUsize::new(0));
        let reader = Counted {
            bytes: Cursor::new(bytes),
            count: count.clone(),
        };
        let mut source = crate::decoder::open_reader(Box::new(reader), Some("nwa")).unwrap();
        assert!(count.load(Ordering::SeqCst) <= 128);
        let mut out = [[0.; 2]; 32];
        assert_eq!(source.read(&mut out).unwrap(), 32);
        assert_eq!(out, [[0.5; 2]; 32]);
        source.seek(900_000).unwrap();
        assert_eq!(source.read(&mut out).unwrap(), 32);
        assert!(count.load(Ordering::SeqCst) < 512);
    }
}
