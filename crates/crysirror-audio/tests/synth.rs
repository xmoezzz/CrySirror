// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored scores only. No device, game, ROM or legacy VM.
use crysirror_audio::{
    decoder::{self, AudioSource},
    synth::{RegisterScore, RegisterWrite, SynthSource},
};
use std::sync::Arc;
fn score() -> RegisterScore {
    RegisterScore {
        schema: 1,
        output_rate: 48000,
        frames: 4800,
        writes: vec![
            RegisterWrite {
                frame: 100,
                port: 0,
                register: 0,
                value: 71,
            },
            RegisterWrite {
                frame: 100,
                port: 0,
                register: 1,
                value: 0,
            },
            RegisterWrite {
                frame: 100,
                port: 0,
                register: 7,
                value: 0x3e,
            },
            RegisterWrite {
                frame: 100,
                port: 0,
                register: 8,
                value: 15,
            },
            RegisterWrite {
                frame: 4000,
                port: 0,
                register: 8,
                value: 0,
            },
        ],
    }
}
fn render(s: &mut dyn AudioSource, n: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.; 2]; n];
    let mut at = 0;
    while at < n {
        let k = s.read(&mut out[at..]).unwrap();
        if k == 0 {
            out.truncate(at);
            break;
        }
        at += k;
    }
    out
}
#[test]
fn register_events_use_sample_clock_partitioning_and_exact_replay_seek() {
    let spec = score();
    let bytes: Arc<[u8]> = serde_json::to_vec(&spec).unwrap().into();
    let mut a = decoder::open_memory(bytes.clone(), Some("opna")).unwrap();
    let all = render(&mut *a, 10000);
    assert_eq!(all.len(), 4800);
    assert!(all[..100].iter().all(|v| *v == [0.; 2]));
    assert!(all[100..4000].iter().any(|v| v[0].abs() > 0.01));
    assert!(all.iter().all(|v| v.iter().all(|x| x.is_finite())));
    let mut b = SynthSource::registers(spec).unwrap();
    let mut split = render(&mut b, 137);
    split.extend(render(&mut b, 4663));
    assert_eq!(all, split);
    b.seek(1973).unwrap();
    assert_eq!(render(&mut b, 503), all[1973..2476]);
    b.seek(4800).unwrap();
    assert!(render(&mut b, 1).is_empty());
    assert!(b.seek(4801).is_err());
}
#[test]
fn invalid_scores_and_unimplemented_chip_modes_fail_before_playback() {
    let mut s = score();
    s.writes[0].port = 2;
    assert!(SynthSource::registers(s).is_err());
    let mut s = score();
    s.writes[0].register = 0xa8;
    assert!(SynthSource::registers(s).is_err());
    let mut s = score();
    s.writes.swap(0, 4);
    assert!(SynthSource::registers(s).is_err());
    let mut s = score();
    s.schema = 2;
    assert!(SynthSource::registers(s).is_err());
    assert!(decoder::open_memory(Arc::from(&b"not pmd"[..]), Some("mmm")).is_err());
}
fn pmd(part: &[u8]) -> Vec<u8> {
    let mut md = vec![0; 26];
    let address = md.len();
    md.extend(part);
    let empty = md.len();
    md.push(0x80);
    let voice = md.len();
    let mut patch = vec![0, 1, 1, 1, 1, 127, 127, 127, 0, 31, 31, 31, 31];
    patch.extend([0; 8]);
    patch.extend([15, 15, 15, 15, 7]);
    md.extend(patch);
    for i in 0..12 {
        let at = if i == 0 { address } else { empty };
        md[i * 2..i * 2 + 2].copy_from_slice(&(at as u16).to_le_bytes());
    }
    md[24..26].copy_from_slice(&(voice as u16).to_le_bytes());
    let mut file = vec![0];
    file.extend(md);
    file
}
#[test]
fn pmd_shared_decoder_produces_music_releases_at_eof_and_replays_its_own_state() {
    let file: Arc<[u8]> = pmd(&[255, 0, 253, 127, 0x49, 24, 0x80]).into();
    let mut a = decoder::open_memory(file.clone(), Some("mmm")).unwrap();
    let all = render(&mut *a, 96000);
    assert!(all.len() < 96000 && all.len() > 48000);
    assert!(all.iter().map(|v| v[0] * v[0]).sum::<f32>() > 1.);
    a.seek(0).unwrap();
    assert_eq!(render(&mut *a, 256), all[..256]);
}
#[test]
fn malformed_pmd_and_zero_time_cycles_return_errors_instead_of_hanging() {
    for part in [&[0xf6, 0x80][..], &[0x81][..], &[0xef, 0xa8, 0][..]] {
        let mut s = SynthSource::pmd(pmd(part).into()).unwrap();
        assert!(s.read(&mut [[0.; 2]; 4096]).is_err());
    }
    let mut file = pmd(&[0x49, 24, 0x80]);
    file[1 + 18..1 + 20].copy_from_slice(&26u16.to_le_bytes());
    assert!(SynthSource::pmd(file.into()).is_err());
}
