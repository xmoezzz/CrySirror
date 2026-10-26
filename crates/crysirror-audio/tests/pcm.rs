// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Prepared resources are usable without a VM, media manager or output device.
use crysirror_audio::{channels::ChannelAudio, decoder, pcm::PcmBuilder};
use crysirror_core::service::{AudioParameters, AudioRequest, PcmAudio};
use std::sync::Arc;

#[test]
fn immutable_pcm_validates_before_publication_and_releases_storage_after_last_source() {
    for rate in [0, 7999, 192001] {
        assert!(PcmAudio::new(rate, vec![[0.; 2]]).is_err());
    }
    assert!(PcmAudio::new(48000, vec![]).is_err());
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 16.01, -16.01] {
        assert!(PcmAudio::new(48000, vec![[value, 0.]]).is_err());
    }
    let receipt = Arc::new(());
    let weak = Arc::downgrade(&receipt);
    let mut pcm = PcmAudio::new(48000, vec![[0.25, -0.5]; 8]).unwrap();
    pcm.retain_storage(receipt);
    assert_eq!(pcm.info().sample_bytes, 64);
    assert_eq!(pcm.info().frames, 8);
    let clone = pcm.clone();
    assert!(std::ptr::eq(pcm.frames().as_ptr(), clone.frames().as_ptr()));
    let source = decoder::open_pcm(pcm);
    drop(clone);
    assert!(weak.upgrade().is_some());
    drop(source);
    assert!(weak.upgrade().is_none());
}

#[test]
fn interleaved_builder_preserves_signed_extrema_and_rejects_partial_or_excess_frames() {
    let mut mono = PcmBuilder::new(8000, 1, 3).unwrap();
    mono.append_i16(&[i16::MIN]).unwrap();
    mono.append_i16(&[0, i16::MAX]).unwrap();
    assert!(mono.append_i16(&[1]).is_err());
    assert_eq!(
        mono.finish().unwrap().frames(),
        &[[-1.; 2], [0.; 2], [32767. / 32768.; 2]]
    );
    let mut stereo = PcmBuilder::new(48000, 2, 2).unwrap();
    stereo.append_i16(&[8192, -16384]).unwrap();
    assert!(stereo.append_i16(&[1, 2, 3]).is_err());
    assert_eq!(stereo.frame_count(), 1);
    assert!(stereo.append_i16(&[1, 2, 3, 4]).is_err());
    assert_eq!(stereo.byte_len(), 8);
    stereo.append_i16(&[0, i16::MAX]).unwrap();
    assert_eq!(
        stereo.finish().unwrap().frames(),
        &[[0.25, -0.5], [0., 32767. / 32768.]]
    );
    let mut surround = PcmBuilder::new(48000, 3, 1).unwrap();
    surround.append_i16(&[8192, -8192, i16::MAX]).unwrap();
    assert_eq!(surround.finish().unwrap().frames(), &[[0.25, -0.25]]);
    assert!(PcmBuilder::new(48000, 0, 1).is_err());
    assert!(PcmBuilder::new(48000, 9, 1).is_err());
    assert!(PcmBuilder::new(48000, 1, 0).is_err());
    assert!(PcmBuilder::new(48000, 1, 1).unwrap().finish().is_err());
}

#[test]
fn resident_source_has_exact_seek_end_and_long_movie_capacity() {
    // Longer than Clip's 4,194,304-frame SFX ceiling; no encoded round trip.
    let count = 4_194_305;
    let mut frames = vec![[0.; 2]; count];
    frames[count - 1] = [0.125, -0.25];
    let mut source = decoder::open_pcm(PcmAudio::new(48000, frames).unwrap());
    assert!(source.is_nonblocking());
    source.seek(count as u64 - 1).unwrap();
    let mut output = [[99.; 2]; 2];
    assert_eq!(source.read(&mut output).unwrap(), 1);
    assert_eq!(output[0], [0.125, -0.25]);
    assert_eq!(source.read_position(), Some(count as u64));
    assert_eq!(source.read(&mut output).unwrap(), 0);
    assert!(source.seek(count as u64 + 1).is_err());
    assert_eq!(source.read_position(), Some(count as u64));
    source.seek(0).unwrap();
    assert_eq!(source.read(&mut output).unwrap(), 2);
    assert_eq!(output, [[0.; 2]; 2]);
}

#[test]
fn prepared_channel_applies_start_rate_loop_gain_pan_and_scoped_stop() {
    let mut audio = ChannelAudio::default();
    let mut frames = vec![[0.; 2]; 48];
    frames.extend([[0.25; 2], [0.5; 2], [0.75; 2], [1.; 2]]);
    let pcm = PcmAudio::new(48000, frames).unwrap();
    let lease = Arc::new(());
    let weak = Arc::downgrade(&lease);
    let mut pcm = pcm;
    pcm.retain_storage(lease);
    audio
        .submit(
            1,
            AudioRequest::PlayPcm {
                channel: 7,
                source: pcm,
                parameters: AudioParameters {
                    start_ms: 1,
                    rate: 2.,
                    gain: 0.5,
                    decode_gain: 2.,
                    pan: 0.,
                    looping: true,
                    loop_start: 48,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    audio
        .submit(
            2,
            AudioRequest::Stop {
                channel: 7,
                fade_ms: 0.,
            },
        )
        .unwrap();
    audio.render(1).unwrap();
    let out = audio.take_output();
    for (i, frame) in out.iter().enumerate() {
        let expected = if i % 2 == 0 { 0.25 } else { 0.75 };
        assert!((frame[0] - expected * std::f32::consts::SQRT_2).abs() < 1e-6);
        assert_eq!(frame[1], 0.);
    }
    assert!(weak.upgrade().is_some());
    audio
        .submit(
            1,
            AudioRequest::Stop {
                channel: 7,
                fade_ms: 0.,
            },
        )
        .unwrap();
    audio.render(1).unwrap();
    assert!(audio.take_output().iter().flatten().all(|v| *v == 0.));
    assert!(weak.upgrade().is_none());
}
