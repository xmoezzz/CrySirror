// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Ogg Opus (RFC 7845) decoding: the Symphonia Ogg demuxer supplies packets,
//! pre-skip and 48 kHz timing; the pure-Rust `opus-decoder` (RFC 8251
//! conformant, no unsafe) decodes them. Mapping family 0 (mono/stereo) only.
use std::sync::OnceLock;
use symphonia::core::{
    audio::{AudioBuffer, AudioBufferRef, AsAudioBufferRef, Signal, SignalSpec},
    codecs::{
        CODEC_TYPE_OPUS, CodecDescriptor, CodecParameters, CodecRegistry, Decoder,
        DecoderOptions, FinalizeResult,
    },
    errors::{Error, Result, decode_error, unsupported_error},
    formats::Packet,
    support_codec,
};

/// Opus frames are at most 120 ms: 5760 samples per channel at 48 kHz.
const MAX_FRAMES: usize = 5760;

pub struct OpusCodec {
    params: CodecParameters,
    decoder: opus_decoder::OpusDecoder,
    channels: usize,
    gain: f32,
    pcm: Vec<f32>,
    buf: AudioBuffer<f32>,
}

impl Decoder for OpusCodec {
    fn try_new(params: &CodecParameters, _: &DecoderOptions) -> Result<Self> {
        let layout = params.channels.ok_or(Error::Unsupported("opus: channels"))?;
        let channels = layout.count();
        if !(1..=2).contains(&channels) {
            return unsupported_error("opus: only mapping family 0 (mono/stereo)");
        }
        // OpusHead: output gain is a Q7.8 dB value at byte 16.
        let head = params.extra_data.as_deref().unwrap_or_default();
        if head.len() < 19 || &head[..8] != b"OpusHead" {
            return decode_error("opus: missing OpusHead");
        }
        if head[18] != 0 {
            return unsupported_error("opus: channel mapping family other than 0");
        }
        let gain_q8 = i16::from_le_bytes([head[16], head[17]]);
        let gain = 10f32.powf(f32::from(gain_q8) / (20. * 256.));
        let decoder = opus_decoder::OpusDecoder::new(48_000, channels)
            .map_err(|_| Error::DecodeError("opus: decoder init"))?;
        Ok(Self {
            params: params.clone(),
            decoder,
            channels,
            gain,
            pcm: vec![0.; MAX_FRAMES * channels],
            buf: AudioBuffer::new(MAX_FRAMES as u64, SignalSpec::new(48_000, layout)),
        })
    }
    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[support_codec!(CODEC_TYPE_OPUS, "opus", "Opus")]
    }
    fn reset(&mut self) {
        self.decoder.reset();
    }
    fn codec_params(&self) -> &CodecParameters {
        &self.params
    }
    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        let frames = self
            .decoder
            .decode_float(&packet.data, &mut self.pcm, false)
            .map_err(|_| Error::DecodeError("opus: invalid packet"))?;
        self.buf.clear();
        self.buf.render_reserved(Some(frames));
        for ch in 0..self.channels {
            let plane = self.buf.chan_mut(ch);
            for (i, s) in plane.iter_mut().enumerate() {
                *s = self.pcm[i * self.channels + ch] * self.gain;
            }
        }
        Ok(self.buf.as_audio_buffer_ref())
    }
    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }
    fn last_decoded(&self) -> AudioBufferRef<'_> {
        self.buf.as_audio_buffer_ref()
    }
}

/// Symphonia's enabled codecs plus Opus.
pub fn codecs() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut r = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut r);
        r.register_all::<OpusCodec>();
        r
    })
}
