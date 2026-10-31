// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Public Media metadata tickets over the same device-free native contract.
use crate::{EngineState, Value};
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{HostServices, MediaProbeJob, MediaProbeState},
    state::ClientId,
};
use std::collections::BTreeMap;
#[derive(Default)]
pub(crate) struct MediaProbes {
    next: u64,
    jobs: BTreeMap<u64, (ClientId, u64, MediaProbeJob)>,
}
pub(crate) fn call(state: &mut EngineState, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(method, "beginProbe" | "probeStatus" | "releaseProbe") {
        return Ok(None);
    }
    let owner = state.client;
    let epoch = state.runtime.restored;
    if method == "beginProbe" {
        state
            .media_probes
            .jobs
            .retain(|_, (_, generation, _)| *generation == epoch);
        ensure!(
            state.media_probes.jobs.len() < 64,
            "media probe ticket budget"
        );
        let path = VfsPath::new(Mount::Asset, args.first().context("media asset")?.as_str()?)?;
        let id = state
            .media_probes
            .next
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .context("media probe ticket overflow")?;
        let job = state.runtime.host.begin_media_probe(owner, &path)?;
        state.media_probes.jobs.insert(id, (owner, epoch, job));
        state.media_probes.next = id;
        return Ok(Some(Value::Int(id as i64)));
    }
    let id = u64::try_from(args.first().context("media probe ticket")?.as_i64()?)?;
    let (client, generation, _) = state
        .media_probes
        .jobs
        .get(&id)
        .context("unknown media probe ticket")?;
    ensure!(*client == owner, "foreign media probe ticket");
    if *generation != epoch {
        state.media_probes.jobs.remove(&id);
        anyhow::bail!("obsolete media probe ticket after restore");
    }
    if method == "releaseProbe" {
        state.media_probes.jobs.remove(&id);
        return Ok(Some(Value::Null));
    }
    let (_, _, job) = state.media_probes.jobs.get_mut(&id).unwrap();
    let value = match job.poll()? {
        MediaProbeState::Pending => serde_json::json!({"state":"pending"}),
        MediaProbeState::Failed(error) => serde_json::json!({"state":"failed","error":error}),
        MediaProbeState::Ready(info) => {
            serde_json::json!({"state":"ready","info":{"client":info.client.0,"size":info.size,"container":info.container,"video_decoder":info.video_decoder,"header_validated":info.header_validated,"audio_decoder":info.audio_decoder,"has_audio":info.has_audio,"duration_ms":info.duration_ms.map(|n|n.to_string()),"frame_ms":info.frame_ms}})
        }
    };
    Ok(Some(Value::from_json(value)))
}
