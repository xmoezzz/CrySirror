// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored closed/open/closed door, worker geometry, direct PCM and echo queries.
use anyhow::{Result, ensure};
use crysirror_acoustics::{
    AcousticMeshUpdate, AcousticQuery, AcousticScene, AcousticSceneSpec, AcousticSource,
    reflections::ReflectionQuery,
};
use crysirror_audio::{
    decoder,
    mixer::{Mixer, Spatial, VoiceSpec},
};
use crysirror_core::service::PcmAudio;
use crysirror_resource::WaveSpec;
use crysirror_tasks::TaskPool;
use std::sync::Arc;
fn rms(pcm: &[[f32; 2]]) -> f64 {
    (pcm.iter().map(|s| (s[0] as f64).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt()
}
fn main() -> Result<()> {
    let path = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "/tmp/crysirror-moving-door.wav".into()),
    );
    if let Some(p) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(p)?;
    }
    let spec: AcousticSceneSpec = serde_json::from_value(
        serde_json::json!({"materials":[{"absorption":[0,0,0],"transmission":[0,0,0],"scattering":0}],"meshes":[{"id":7,"vertices":[[-10,-10,-2],[10,-10,-2],[10,10,-2],[-10,10,-2]],"triangles":[[0,1,2],[0,2,3]]},{"id":8,"vertices":[[-10,-10,4],[10,-10,4],[10,10,4],[-10,10,4]],"triangles":[[0,1,2],[0,2,3]]}]}),
    )?;
    let pool = TaskPool::new("door-geometry", 2, 4)?;
    let mut scene = Arc::new(pool.spawn(move |_| AcousticScene::build(spec))?.wait()?);
    let mut mixer = Mixer::default();
    let rate = mixer.sample_rate();
    let samples = (0..rate)
        .map(|i| [(std::f32::consts::TAU * 440. * i as f32 / rate as f32).sin() * 0.2; 2])
        .collect();
    let voice = mixer.play(
        1,
        decoder::open_pcm(PcmAudio::new(rate, samples)?),
        VoiceSpec {
            looping: true,
            spatial: Some(Spatial {
                position: [0., 0., -4.],
                min_distance: 10.,
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    let mut all = Vec::new();
    let mut levels = Vec::new();
    let mut paths = Vec::new();
    for x in [0., 30., 0.] {
        let old = scene.clone();
        let owner = std::thread::current().id();
        let (next, direct, echoes) = pool
            .spawn(move |cancel| {
                ensure!(
                    std::thread::current().id() != owner,
                    "geometry ran on owner"
                );
                let update: AcousticMeshUpdate = serde_json::from_value(
                    serde_json::json!({"id":7,"transform":[1,0,0,0,0,1,0,0,0,0,1,0,x,0,0,1]}),
                )?;
                let scene = old.updated_cancellable(&[update], || cancel.check())?;
                let settings = AcousticSource {
                    air_absorption: [0.; 3],
                    radius: 0.,
                    ..Default::default()
                };
                let query = AcousticQuery {
                    source: [0., 0., -4.],
                    listener: [0.; 3],
                    settings: settings.clone(),
                };
                let direct = crysirror_acoustics::simulate(&scene, &query)?;
                let echoes = crysirror_acoustics::reflections::simulate(
                    &scene,
                    &ReflectionQuery {
                        source: query.source,
                        listener: query.listener,
                        listener_forward: [0., 0., -1.],
                        listener_up: [0., 1., 0.],
                        options: crysirror_acoustics::reflections::ReflectionOptions {
                            propagation: settings,
                            ..Default::default()
                        },
                    },
                )?;
                Ok((scene, direct, echoes))
            })?
            .wait()?;
        ensure!(
            next.geometry_stats().reused_mesh_bvhs == 2,
            "local BVHs were not retained"
        );
        scene = Arc::new(next);
        mixer.set_acoustic_gains(1, voice, Some(direct.gains))?;
        let mut pcm = vec![[0.; 2]; rate as usize / 2];
        mixer.render(&mut pcm)?;
        levels.push(rms(&pcm[pcm.len() / 2..]));
        paths.push(echoes.paths.len());
        all.extend(pcm);
    }
    ensure!(
        levels[0] < 1e-7 && levels[1] > 0.09 && levels[2] < 1e-7,
        "door PCM levels {levels:?}"
    );
    ensure!(paths == [0, 1, 0], "door reflection paths {paths:?}");
    ensure!(
        mixer.playback_info(1, voice)?.age_frames == all.len() as u64,
        "door update restarted voice"
    );
    let bytes = all
        .iter()
        .flat_map(|s| {
            s.iter()
                .flat_map(|v| ((v.clamp(-1., 1.) * 32767.) as i16).to_le_bytes())
        })
        .collect::<Vec<_>>();
    std::fs::write(
        &path,
        WaveSpec {
            sample_rate: rate,
            channels: 2,
            ..Default::default()
        }
        .encode(&bytes)?,
    )?;
    println!(
        "{}",
        serde_json::json!({"file":path,"rate":rate,"frames":all.len(),"phase_rms":levels,"reflection_paths":paths,"worker_geometry":true,"native_vm":false,"device":false})
    );
    Ok(())
}
