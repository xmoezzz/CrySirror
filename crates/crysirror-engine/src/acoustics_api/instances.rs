// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! World-pose capture and coalesced instance commits; all geometry lives in audio.
use super::*;
use crysirror_scene::world::EntityHandle;
use glam::Mat4;
use serde::{Deserialize, Serialize};
fn identity() -> [f32; 16] {
    Mat4::IDENTITY.to_cols_array()
}
/// Derived binding between an acoustic mesh and a client-owned World entity.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcousticMeshBindingSpec {
    /// Existing acoustic mesh ID in the calling client's prepared scene.
    pub mesh: u32,
    /// Generation-checked World entity handle, owned by the same client.
    pub entity: EntityHandle,
    /// Mesh-local offset; final pose is entity global matrix times this matrix.
    #[serde(default = "identity")]
    pub local_transform: [f32; 16],
    /// Explicitly couple acoustic participation to ancestor-aware World visibility.
    #[serde(default)]
    pub follow_visibility: bool,
}
#[derive(Clone)]
struct Binding {
    spec: AcousticMeshBindingSpec,
    desired: Option<AcousticMeshUpdate>,
    error: Option<String>,
}
#[derive(Default)]
struct Owner {
    ticket: Option<u64>,
    submitted: Vec<AcousticMeshUpdate>,
    failed: Option<Vec<AcousticMeshUpdate>>,
    submitted_at: Option<u64>,
    published_at: Option<u64>,
    published_revision: Option<u64>,
    completed: u64,
    next_due: u64,
    waiting: Option<String>,
    error: Option<String>,
}
#[derive(Default)]
pub(super) struct InstanceRuntime {
    bindings: BTreeMap<(u32, u32), Binding>,
    owners: BTreeMap<u32, Owner>,
    clock: u64,
}
impl InstanceRuntime {
    pub(super) fn is_bound(&self, owner: u32, mesh: u32) -> bool {
        self.bindings.contains_key(&(owner, mesh))
    }
    pub(super) fn is_automatic(&self, owner: u32, ticket: u64) -> bool {
        self.owners
            .get(&owner)
            .is_some_and(|s| s.ticket == Some(ticket))
    }
    pub(super) fn canceled(&mut self, owner: u32, ticket: u64) {
        if let Some(s) = self
            .owners
            .get_mut(&owner)
            .filter(|s| s.ticket == Some(ticket))
        {
            s.ticket = None;
            s.submitted.clear();
            s.failed = None;
            s.waiting = Some("superseded".into());
        }
    }
    pub(super) fn finished(
        &mut self,
        owner: u32,
        ticket: u64,
        revision: Option<u64>,
        error: Option<String>,
    ) {
        let Some(s) = self
            .owners
            .get_mut(&owner)
            .filter(|s| s.ticket == Some(ticket))
        else {
            return;
        };
        s.ticket = None;
        s.waiting = None;
        s.error = error;
        if let Some(revision) = revision {
            s.published_at = Some(self.clock);
            s.published_revision = Some(revision);
            s.completed = s.completed.saturating_add(1);
            s.failed = None;
        } else {
            s.failed = Some(s.submitted.clone());
        }
    }
    pub(super) fn clear(&mut self, owner: u32) {
        self.bindings.retain(|k, _| k.0 != owner);
        self.owners.remove(&owner);
    }
    pub(super) fn status(&self, owner: u32) -> Json {
        let bindings = self
            .bindings
            .iter()
            .filter(|(k, _)| k.0 == owner)
            .map(|(_, b)| json!({"settings":b.spec,"desired":b.desired,"error":b.error}))
            .collect::<Vec<_>>();
        let Some(s) = self.owners.get(&owner) else {
            return json!({"bindings":bindings,"pending":false,"clock_ms":self.clock,"completed_updates":0});
        };
        json!({"bindings":bindings,"pending":s.ticket.is_some()||s.waiting.is_some(),"waiting_for":s.waiting,"error":s.error,"submitted_at_ms":s.submitted_at,"published_at_ms":s.published_at,"published_revision":s.published_revision,"published_age_ms":s.published_at.map(|v|self.clock.saturating_sub(v)),"submitted":s.submitted,"clock_ms":self.clock,"completed_updates":s.completed,"next_due_ms":s.next_due,"refresh_interval_ms":50})
    }
}
pub(super) fn call(state: &mut EngineState, method: &str, args: &[Value]) -> Result<Value> {
    let owner = state.client.0;
    if method == "bindAcousticMesh" {
        let input = arg(args, 0)?;
        let mut input_json = input.to_json();
        if let Some(entity) = input
            .field("entity")
            .filter(|v| matches!(v, Value::Object(_)))
        {
            let entry = state.objects.get(entity.as_object()?)?;
            ensure!(
                entry.class == "Entity",
                "acoustic binding requires an Entity"
            );
            input_json["entity"] = entry.payload.to_json();
        }
        let spec: AcousticMeshBindingSpec = serde_json::from_value(input_json)?;
        ensure!(
            spec.entity.client == owner,
            "foreign acoustic entity handle"
        );
        let world = &state.runtime.host.engine.world;
        let matrix =
            world.global_matrix(spec.entity)? * Mat4::from_cols_array(&spec.local_transform);
        AcousticMeshUpdate {
            id: spec.mesh,
            transform: Some(matrix.to_cols_array()),
            enabled: None,
        }
        .validate()?;
        let runtime = &mut state.mixer_audio.acoustics;
        ensure!(
            runtime.scene(owner)?.1.scene.mesh_pose(spec.mesh).is_some(),
            "unknown acoustic mesh binding ID"
        );
        let r = &mut runtime.instances;
        ensure!(
            r.is_bound(owner, spec.mesh)
                || (r.bindings.len() < 1024
                    && r.bindings.keys().filter(|k| k.0 == owner).count() < 256),
            "acoustic binding capacity exceeded"
        );
        r.bindings.insert(
            (owner, spec.mesh),
            Binding {
                spec,
                desired: None,
                error: None,
            },
        );
        let s = r.owners.entry(owner).or_default();
        s.failed = None;
        s.error = None;
        s.next_due = 0;
        s.waiting = Some("capture".into());
    } else {
        let mesh = u32::try_from(arg(args, 0)?.as_i64()?)?;
        let runtime = &mut state.mixer_audio.acoustics;
        ensure!(
            runtime.instances.bindings.remove(&(owner, mesh)).is_some(),
            "acoustic mesh is not entity-bound"
        );
        if let Some(ticket) = runtime.instances.owners.get(&owner).and_then(|s| s.ticket) {
            runtime.jobs.cancel(owner, ticket)?;
            runtime.scene_jobs.remove(&owner);
            runtime.instances.canceled(owner, ticket);
        }
        if !runtime.instances.bindings.keys().any(|k| k.0 == owner) {
            runtime.instances.owners.remove(&owner);
        }
    }
    Ok(Value::Null)
}
pub(super) fn update(state: &mut EngineState, dt: u64) -> Result<()> {
    let captures = state
        .mixer_audio
        .acoustics
        .instances
        .bindings
        .iter()
        .map(|(key, b)| (*key, b.spec.clone()))
        .collect::<Vec<_>>();
    let mut desired = Vec::with_capacity(captures.len());
    for (key, spec) in captures {
        let world = &state.runtime.host.engine.world;
        let result = world.global_matrix(spec.entity).and_then(|global| {
            let u = AcousticMeshUpdate {
                id: spec.mesh,
                transform: Some(
                    (global * Mat4::from_cols_array(&spec.local_transform)).to_cols_array(),
                ),
                enabled: Some(
                    !spec.follow_visibility
                        || world.effective_visible(spec.entity.client, spec.entity.id),
                ),
            };
            u.validate()?;
            Ok(u)
        });
        match result {
            Ok(update) => desired.push((key, Some(update), None)),
            Err(error) if world.entity(spec.entity).is_err() => desired.push((
                key,
                Some(AcousticMeshUpdate {
                    id: spec.mesh,
                    transform: None,
                    enabled: Some(false),
                }),
                Some(format!(
                    "entity unavailable; disabling acoustic mesh: {error:#}"
                )),
            )),
            Err(error) => desired.push((
                key,
                None,
                Some(format!("invalid entity acoustic pose: {error:#}")),
            )),
        }
    }
    let pool = state.runtime.assets.cpu_tasks();
    let epoch = state.runtime.restored;
    let runtime = &mut state.mixer_audio.acoustics;
    runtime.instances.clock = runtime
        .instances
        .clock
        .checked_add(dt)
        .context("acoustic instance clock overflow")?;
    let clock = runtime.instances.clock;
    for (key, update, error) in desired {
        if let Some(b) = runtime.instances.bindings.get_mut(&key) {
            b.desired = update;
            b.error = error;
        }
    }
    let owners = runtime.instances.owners.keys().copied().collect::<Vec<_>>();
    for owner in owners {
        let Some((_, scene)) = runtime.scenes.get(&owner) else {
            continue;
        };
        let updates = runtime
            .instances
            .bindings
            .iter()
            .filter(|(key, _)| key.0 == owner)
            .filter_map(|(_, b)| b.desired.as_ref())
            .filter(|u| {
                scene.scene.mesh_pose(u.id).is_some_and(|p| {
                    u.transform.is_some_and(|m| p.transform != Some(m))
                        || u.enabled.is_some_and(|v| p.enabled != Some(v))
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let s = runtime.instances.owners.get_mut(&owner).unwrap();
        if runtime.scene_jobs.contains_key(&owner) {
            s.waiting = Some("acoustic_scene".into());
            continue;
        }
        if updates.is_empty() {
            s.waiting = None;
            s.error = None;
            s.failed = None;
            continue;
        }
        if s.failed.as_ref() == Some(&updates) {
            s.waiting = None;
            continue;
        }
        if clock < s.next_due {
            s.waiting = Some("refresh_interval".into());
            continue;
        }
        if runtime.jobs.admit(owner).is_err() {
            s.waiting = Some("operation_capacity".into());
            continue;
        }
        if runtime.budget.load(Ordering::Relaxed) > HOST_BUDGET - RESERVATION {
            s.waiting = Some("geometry_budget".into());
            continue;
        }
        let ticket = runtime.begin_update(owner, epoch, &pool, updates.clone());
        let s = runtime.instances.owners.get_mut(&owner).unwrap();
        match ticket {
            Ok(ticket) => {
                s.ticket = Some(ticket);
                s.submitted = updates;
                s.submitted_at = Some(clock);
                s.next_due = clock.saturating_add(50);
                s.waiting = None;
                s.error = None;
            }
            Err(error) => {
                s.error = Some(format!("{error:#}"));
                s.failed = Some(updates);
                s.waiting = None;
            }
        }
    }
    Ok(())
}
