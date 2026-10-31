//! PCM/owner tests for the shared release actor, independent of any native VM.
use super::*;
use crate::{
    channels::{prepare, wrap_source},
    decoder::{AudioSource, StereoFrame},
    mixer::{Bus, BusSettings},
};
use crysirror_core::service::{AudioMixGroupSettings, AudioParameters};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
struct Samples {
    at: u64,
    len: u64,
    seeks: Arc<AtomicUsize>,
}
impl AudioSource for Samples {
    fn sample_rate(&self) -> u32 {
        48000
    }
    fn frame_count(&self) -> Option<u64> {
        Some(self.len)
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        let n = out.len().min((self.len - self.at) as usize);
        for f in &mut out[..n] {
            *f = [(self.at + 1) as f32 / 1000.; 2];
            self.at += 1;
        }
        Ok(n)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        self.seeks.fetch_add(1, Ordering::SeqCst);
        self.at = frame;
        Ok(())
    }
}
fn spec() -> Spec {
    Spec {
        source_channel: 0,
        tail_channel: 1,
        threshold_db100: -3000,
        target_above_db100: -3000,
        target_below_db100: -10000,
        steps: 30,
        pan: Some(0.),
        playback_rate: Some(1.),
        freeze_group_gain: true,
        release_tail_when_source_missing: true,
    }
}
fn play(
    a: &mut ChannelAudio,
    ch: u32,
    gain: f32,
    group: Option<&str>,
    limit: Option<u64>,
    looping: bool,
) -> Arc<AtomicUsize> {
    let seeks = Arc::new(AtomicUsize::new(0));
    let src = wrap_source(
        Box::new(Samples {
            at: 0,
            len: 1000,
            seeks: seeks.clone(),
        }),
        limit,
        1.,
    )
    .unwrap();
    let p = prepare(
        src,
        None,
        AudioParameters {
            bus: crysirror_core::service::PcmBus::Music,
            gain,
            mix_group: group.map(str::to_owned),
            looping,
            loop_start: 2,
            loop_end: limit,
            ..Default::default()
        },
    )
    .unwrap();
    a.publish(7, ch, p).unwrap();
    seeks
}
fn render(a: &mut ChannelAudio, n: usize) -> Vec<StereoFrame> {
    let mut pcm = vec![[0.; 2]; n];
    a.mixer.render(&mut pcm).unwrap();
    pcm
}
#[test]
fn transfer_preserves_consumed_cursor_despite_read_ahead_and_never_reopens() {
    let mut a = ChannelAudio::default();
    let seeks = play(&mut a, 0, 1., None, None, false);
    let first = render(&mut a, 7);
    assert!((first[6][0] - 0.007).abs() < 1e-6);
    let h = a.begin_audio_release(7, spec()).unwrap();
    let i = a.audio_release_info(7, h).unwrap();
    assert_eq!(i.phase, Phase::Active);
    assert_eq!(i.source_position_frames, Some(7));
    assert_eq!(i.completed_updates, 0);
    assert!(!a.is_playing(7, 0));
    assert!(a.is_playing(7, 1));
    let tail = render(&mut a, 3);
    assert!((tail[0][0] - 0.008).abs() < 1e-6);
    assert_eq!(seeks.load(Ordering::SeqCst), 0);
}
#[test]
fn native_f32_oracles_and_actual_pcm_gain_every_update() {
    let cases = [
        (0, 29, -100),
        (-500, 30, -583),
        (-800, 30, -873),
        (-1000, 29, -1066),
        (-1500, 29, -1550),
        (-2000, 30, -2033),
        (-2500, 29, -2516),
        (-2999, 29, -2999),
        (-3000, 1, 0),
        (-3001, 30, -3234),
        (-4000, 29, -4200),
        (-9999, 30, -9999),
        (-10000, 0, 0),
    ];
    for (start, stop, first) in cases {
        let mut a = ChannelAudio::default();
        play(
            &mut a,
            0,
            10f64.powf(f64::from(start) / 2000.) as f32,
            None,
            None,
            false,
        );
        let h = a.begin_audio_release(7, spec()).unwrap();
        if stop == 0 {
            assert_eq!(a.audio_release_info(7, h).unwrap().phase, Phase::Complete);
            continue;
        }
        for tick in 1..=stop {
            a.advance_audio_release(7, h, tick).unwrap();
            let i = a.audio_release_info(7, h).unwrap();
            assert_eq!(i.completed_updates, tick);
            if tick == stop {
                assert_eq!(i.phase, Phase::Complete, "start={start}");
                assert_eq!(i.stopped_at_update, Some(stop));
                assert_eq!(render(&mut a, 1)[0], [0.; 2]);
            } else {
                assert_eq!(i.phase, Phase::Active);
                if tick == 1 {
                    assert_eq!(i.volume_db100, Some(first));
                }
                let pcm = render(&mut a, 1);
                let source = tick as f32 / 1000.;
                let expected =
                    source * 10f64.powf(f64::from(i.volume_db100.unwrap()) / 2000.) as f32;
                assert!(
                    (pcm[0][0] - expected).abs() < 1e-7,
                    "start={start},tick={tick},sample={:?},expected={expected}",
                    pcm[0]
                );
            }
        }
    }
}
#[test]
fn gain_product_is_current_frozen_and_engine_bus_remains_live() {
    let mut a = ChannelAudio::default();
    for (name, gain, parent) in [("root", 0.5, None), ("native", 0.5, Some("root"))] {
        a.submit(
            7,
            AudioRequest::SetMixGroup {
                name: name.into(),
                settings: AudioMixGroupSettings {
                    gain,
                    parent: parent.map(str::to_owned),
                    ..Default::default()
                },
                fade_ms: 0.,
            },
        )
        .unwrap();
    }
    play(&mut a, 0, 1., Some("native"), None, false);
    let h = a.begin_audio_release(7, spec()).unwrap();
    assert_eq!(
        a.audio_release_info(7, h).unwrap().volume_db100,
        Some(-1204)
    );
    a.submit(
        7,
        AudioRequest::SetMixGroupGain {
            name: "root".into(),
            gain: 1.,
            fade_ms: 0.,
        },
    )
    .unwrap();
    a.submit(
        7,
        AudioRequest::SetMixGroupGain {
            name: "native".into(),
            gain: 1.,
            fade_ms: 0.,
        },
    )
    .unwrap();
    let f = render(&mut a, 1)[0][0];
    assert!((f - 0.00025).abs() < 1e-7);
    a.mixer
        .set_bus(
            7,
            Bus::Music,
            BusSettings {
                gain: 0.5,
                ..Default::default()
            },
        )
        .unwrap();
    assert!((render(&mut a, 1)[0][0] - 0.00025).abs() < 1e-7);
    assert_eq!(a.audio_release_info(7, h).unwrap().phase, Phase::Active);
}
#[test]
fn tail_replacement_source_reuse_generation_and_owner_fences() {
    let mut a = ChannelAudio::default();
    play(&mut a, 0, 1., None, None, false);
    play(&mut a, 1, 0.3, None, None, false);
    let h = a.begin_audio_release(7, spec()).unwrap();
    play(&mut a, 0, 0.5, None, None, false);
    a.advance_audio_release(7, h, 1).unwrap();
    assert!(a.is_playing(7, 0));
    assert!(a.advance_audio_release(8, h, 2).is_err());
    assert!(a.audio_release_info(8, h).is_err());
    assert!(a.advance_audio_release(7, h, 0).is_err());
    a.advance_audio_release(7, h, 1).unwrap();
    assert_eq!(a.audio_release_info(7, h).unwrap().completed_updates, 1);
    let next = a.begin_audio_release(7, spec()).unwrap();
    assert_ne!(next.generation, h.generation);
    assert_eq!(a.audio_release_info(7, h).unwrap().phase, Phase::Cancelled);
    a.release_audio_release(7, h).unwrap();
    assert!(a.audio_release_info(7, h).is_err());
    a.reset();
    assert!(!a.has_release_state());
    assert!(a.audio_release_info(7, next).is_err());
}
#[test]
fn nonlooping_retired_tail_ends_at_original_declared_buffer() {
    let mut a = ChannelAudio::default();
    play(&mut a, 0, 1., None, Some(10), true);
    render(&mut a, 6);
    let h = a.begin_audio_release(7, spec()).unwrap();
    let pcm = render(&mut a, 8);
    assert!((pcm[0][0] - 0.007).abs() < 1e-6);
    assert!((pcm[3][0] - 0.010).abs() < 1e-6);
    assert!(pcm[4..].iter().all(|v| *v == [0.; 2]));
    assert_eq!(a.audio_release_info(7, h).unwrap().phase, Phase::Complete);
    assert!(!a.has_release_state());
}
#[test]
fn pending_worker_transfer_waits_for_actual_owner_completion() {
    let mut a = ChannelAudio::default();
    play(&mut a, 0, 1., None, None, false);
    let mut worker = a.mixer.lease_render(991).unwrap();
    let h = a.begin_audio_release(7, spec()).unwrap();
    a.advance_audio_release(7, h, 2).unwrap();
    let i = a.audio_release_info(7, h).unwrap();
    assert_eq!(i.phase, Phase::Pending);
    assert_eq!(i.completed_updates, 0);
    let mut pcm = [[0.; 2]; 5];
    worker.render(&mut pcm).unwrap();
    assert!(a.mixer.finish_render(991, worker));
    let i = a.audio_release_info(7, h).unwrap();
    assert_eq!(i.phase, Phase::Active);
    assert_eq!(i.source_position_frames, Some(5));
    assert_eq!(i.completed_updates, 2);
    assert_eq!(i.volume_db100, Some(-200));
}
#[test]
fn preflight_rejected_targets_and_controls_do_not_cancel_or_destroy() {
    let mut a = ChannelAudio::default();
    play(&mut a, 0, 0.01, None, None, false);
    play(&mut a, 1, 0.5, None, None, false);
    let mut bad = spec();
    bad.target_below_db100 = -3000;
    assert!(a.begin_audio_release(7, bad).is_err());
    assert!(a.is_playing(7, 0));
    assert!(a.is_playing(7, 1));
    assert!(!a.has_release_state());
    let h = a.begin_audio_release(7, spec()).unwrap();
    assert!(
        a.submit(
            7,
            AudioRequest::SetGain {
                channel: 1,
                gain: f32::NAN,
                fade_ms: 0.
            }
        )
        .is_err()
    );
    assert_eq!(a.audio_release_info(7, h).unwrap().phase, Phase::Active);
    a.submit(
        7,
        AudioRequest::Pause {
            channel: 1,
            paused: true,
            fade_ms: 0.,
        },
    )
    .unwrap();
    assert_eq!(a.audio_release_info(7, h).unwrap().phase, Phase::Cancelled);
}
#[test]
fn repeated_release_uses_existing_tail_and_absent_sources_complete() {
    let mut a = ChannelAudio::default();
    let none = a.begin_audio_release(7, spec()).unwrap();
    assert_eq!(
        a.audio_release_info(7, none).unwrap().phase,
        Phase::Complete
    );
    play(&mut a, 0, 1., None, None, false);
    let h = a.begin_audio_release(7, spec()).unwrap();
    a.advance_audio_release(7, h, 5).unwrap();
    let h2 = a.begin_audio_release(7, spec()).unwrap();
    assert_eq!(
        a.audio_release_info(7, h2).unwrap().volume_db100,
        Some(-500)
    );
    assert_eq!(a.audio_release_info(7, h).unwrap().phase, Phase::Cancelled);
    assert!(a.release_audio_release(7, h2).is_err());
}
#[test]
fn buffered_decoder_read_ahead_is_not_the_transferred_play_cursor() {
    use crate::buffered::{BufferedSource, StreamBufferSpec};
    let pool = crysirror_tasks::TaskPool::new("release-buffer", 1, 8).unwrap();
    let seeks = Arc::new(AtomicUsize::new(0));
    let source = BufferedSource::prepare(
        Box::new(Samples {
            at: 0,
            len: 1000,
            seeks: seeks.clone(),
        }),
        pool,
        StreamBufferSpec {
            chunk_frames: 256,
            prefetch_chunks: 2,
        },
        None,
    )
    .unwrap();
    assert!(source.buffer_status().unwrap().decoded_frames >= 512);
    let before = seeks.load(Ordering::SeqCst);
    let mut a = ChannelAudio::default();
    a.publish(
        7,
        0,
        prepare(Box::new(source), None, AudioParameters::default()).unwrap(),
    )
    .unwrap();
    render(&mut a, 7);
    let h = a.begin_audio_release(7, spec()).unwrap();
    assert_eq!(
        a.audio_release_info(7, h).unwrap().source_position_frames,
        Some(7)
    );
    assert!((render(&mut a, 1)[0][0] - 0.008).abs() < 1e-7);
    assert_eq!(seeks.load(Ordering::SeqCst), before);
}
struct EmptyStorage;
impl crysirror_core::service::ClientStorage for EmptyStorage {
    fn read(&self, _: &crysirror_core::path::VfsPath) -> crysirror_core::Result<Vec<u8>> {
        Err(crysirror_core::Error::NotFound)
    }
    fn write_atomic(
        &self,
        _: &crysirror_core::path::VfsPath,
        _: &[u8],
    ) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
    fn remove(&self, _: &crysirror_core::path::VfsPath) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
    fn list(&self, _: crysirror_core::path::Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
        Ok(vec![])
    }
}
fn encoded_play(channel: u32) -> AudioRequest {
    let size = 1000u32 * 2;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((size + 36).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(48000u32.to_le_bytes());
    bytes.extend(96000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    for _ in 0..1000 {
        bytes.extend(100i16.to_le_bytes());
    }
    AudioRequest::Play {
        channel,
        source: crysirror_core::service::EncodedAudio {
            bytes: bytes.into(),
            frame_limit: None,
        },
        intro: None,
        parameters: AudioParameters::default(),
    }
}
#[test]
fn release_cancels_older_pending_preparation_before_it_can_resurrect_source() {
    use std::time::{Duration, Instant};
    let mut a = ChannelAudio::default();
    play(&mut a, 0, 1., None, None, false);
    let pool = crysirror_tasks::TaskPool::new("release-intent", 1, 8).unwrap();
    a.configure_preparation(pool.clone(), true);
    let (tx, rx) = std::sync::mpsc::channel();
    let gate = pool
        .spawn(move |_| {
            rx.recv_timeout(Duration::from_secs(5))?;
            Ok(())
        })
        .unwrap();
    let ticket = a
        .begin_preparation(7, encoded_play(0), Arc::new(EmptyStorage))
        .unwrap();
    let h = a.begin_audio_release(7, spec()).unwrap();
    assert!(a.is_playing(7, 1));
    assert!(!a.is_playing(7, 0));
    tx.send(()).unwrap();
    gate.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = a.preparation(7, ticket).unwrap();
        if status.phase != crysirror_core::service::AudioPreparePhase::Pending {
            assert_eq!(
                status.phase,
                crysirror_core::service::AudioPreparePhase::Failed
            );
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(!a.is_playing(7, 0));
    assert_eq!(a.audio_release_info(7, h).unwrap().phase, Phase::Active);
}
#[test]
fn delayed_rejected_target_leaves_both_voices_and_pending_intent_untouched() {
    let mut a = ChannelAudio::default();
    play(&mut a, 0, 0.01, None, None, false);
    play(&mut a, 1, 0.5, None, None, false);
    let worker = a.mixer.lease_render(992).unwrap();
    let mut bad = spec();
    bad.target_below_db100 = -3000;
    let h = a.begin_audio_release(7, bad).unwrap();
    assert_eq!(a.audio_release_info(7, h).unwrap().phase, Phase::Pending);
    assert!(a.mixer.finish_render(992, worker));
    assert!(a.audio_release_info(7, h).is_err());
    assert!(a.is_playing(7, 0));
    assert!(a.is_playing(7, 1));
}
