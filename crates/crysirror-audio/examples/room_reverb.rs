// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored clicks through dry/room/hall buses. No devices, VM or game data.
use anyhow::{Result, ensure};
use crysirror_acoustics::reverb::ReverbSpec;
use crysirror_audio::{
    decoder,
    mixer::{Bus, Mixer, VoiceSpec},
    reverb::{PreparedReverb, ReverbSend, ReverbSettings},
};
use crysirror_core::service::PcmAudio;
fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "/tmp/crysirror-reverb".into()),
    );
    std::fs::create_dir_all(&dir)?;
    let rate = 48000u32;
    let mut frames = vec![[0.; 2]; rate as usize];
    for offset in [0, 24000] {
        for i in 0..192 {
            let t = i as f32 / rate as f32;
            let v = (t * std::f32::consts::TAU * 1800.).sin() * (1. - i as f32 / 192.) * 0.6;
            frames[offset + i] = [v; 2];
        }
    }
    let pcm = PcmAudio::new(rate, frames)?;
    for (name, room, decay) in [("dry", 0., 0.), ("room", 10., 0.7), ("hall", 30., 2.5)] {
        let mut mixer = Mixer::default();
        if room > 0. {
            mixer.set_reverb(
                1,
                Some(PreparedReverb::prepare(
                    rate,
                    ReverbSettings {
                        room: ReverbSpec {
                            room_size_metres: room,
                            decay_seconds: decay,
                            damping: 0.5,
                        },
                        sends: vec![ReverbSend {
                            bus: Bus::Sfx,
                            gain: 1.,
                        }],
                        wet_gain: 0.6,
                        ..Default::default()
                    },
                )?),
            )?;
        }
        mixer.play(1, decoder::open_pcm(pcm.clone()), VoiceSpec::default())?;
        let mut out = vec![[0.; 2]; rate as usize * 6];
        let start = std::time::Instant::now();
        for block in out.chunks_mut(512) {
            mixer.render(block)?;
        }
        let elapsed = start.elapsed().as_secs_f64();
        let peak = out.iter().flatten().fold(0f32, |p, s| p.max(s.abs()));
        ensure!(peak <= 1., "demo clips");
        let size = (out.len() * 4) as u32;
        let mut wave = Vec::with_capacity(size as usize + 44);
        wave.extend(b"RIFF");
        wave.extend((size + 36).to_le_bytes());
        wave.extend(b"WAVEfmt ");
        wave.extend(16u32.to_le_bytes());
        wave.extend(1u16.to_le_bytes());
        wave.extend(2u16.to_le_bytes());
        wave.extend(rate.to_le_bytes());
        wave.extend((rate * 4).to_le_bytes());
        wave.extend(4u16.to_le_bytes());
        wave.extend(16u16.to_le_bytes());
        wave.extend(b"data");
        wave.extend(size.to_le_bytes());
        for sample in out.iter().flatten() {
            wave.extend(((*sample * 32767.).round() as i16).to_le_bytes());
        }
        std::fs::write(dir.join(format!("{name}.wav")), wave)?;
        let tail_energy: f64 = out[rate as usize..]
            .iter()
            .flatten()
            .map(|x| (*x as f64).powi(2))
            .sum();
        println!(
            "{name}: six seconds stereo48k; mix_seconds={elapsed:.6}; realtime_fraction={:.5}; peak={peak:.6}; post_source_tail_energy={tail_energy:.8}",
            elapsed / 6.
        );
    }
    Ok(())
}
