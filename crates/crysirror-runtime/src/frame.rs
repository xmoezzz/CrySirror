// SPDX-License-Identifier: MPL-2.0
//! Immutable render extraction DAG; owner simulation can advance while it runs.
use crate::{Runtime, model::Scene3d};
use anyhow::{Result, ensure};
use crysirror_core::service::HostServices;
use crysirror_tasks::{
    graph::{FrameGraph, NodeId},
    profile::Profiler,
};
use std::sync::Arc;
enum Part {
    Pbr(Option<crysirror_scene::view::PbrScene>),
    Simple(Option<Scene3d>),
    Layers(Vec<crysirror_scene::Primitive>),
    Ready,
}
/// Completed immutable scene data, never GPU/device pointers or mutable World.
pub struct RenderSnapshot {
    pub surfaces: crate::assets::SurfaceSnapshot,
    pub transition: Option<crysirror_scene::layer::TransitionFrame>,
    pub store: u64,
    pub restored: u64,
    pub frame: u64,
    pub clock_ms: u64,
    pub scene: crysirror_core::state::SceneState,
    pub pbr: Option<crysirror_scene::view::PbrScene>,
    pub simple: Option<Scene3d>,
    pub layers: Vec<crysirror_scene::Primitive>,
}
pub struct PendingFrame {
    surfaces: crate::assets::SurfaceSnapshot,
    transition: Option<crysirror_scene::layer::TransitionFrame>,
    restored: u64,
    graph: FrameGraph<Part>,
    nodes: [NodeId; 3],
    header: Option<(u64, u64, u64, crysirror_core::state::SceneState)>,
}
impl PendingFrame {
    /// Returns None while workers are busy; no owner-thread join.
    pub fn poll(&mut self, runtime: &Runtime) -> Result<Option<RenderSnapshot>> {
        let assets = &runtime.assets;
        if !self.graph.poll(&assets.cpu_tasks())? {
            return Ok(None);
        }
        self.take(runtime)
    }
    fn take(&mut self, runtime: &Runtime) -> Result<Option<RenderSnapshot>> {
        let Some((store, frame, clock_ms, scene)) = self.header.take() else {
            return Ok(None);
        };
        ensure!(
            store == runtime.assets.store_id() && self.restored == runtime.restored,
            "render snapshot belongs to a replaced resource store"
        );
        let p = self.graph.get(self.nodes[0])?.unwrap();
        let s = self.graph.get(self.nodes[1])?.unwrap();
        let l = self.graph.get(self.nodes[2])?.unwrap();
        let (Part::Pbr(pbr), Part::Simple(simple), Part::Layers(layers)) =
            (p.as_ref(), s.as_ref(), l.as_ref())
        else {
            unreachable!()
        };
        Ok(Some(RenderSnapshot {
            surfaces: self.surfaces.clone(),
            transition: self.transition.take(),
            store,
            restored: self.restored,
            frame,
            clock_ms,
            scene,
            pbr: pbr.clone(),
            simple: simple.clone(),
            layers: layers.clone(),
        }))
    }
    pub fn wait(mut self, runtime: &Runtime) -> Result<RenderSnapshot> {
        self.graph.wait(&runtime.assets.cpu_tasks())?;
        Ok(self.take(runtime)?.unwrap())
    }
}
impl Runtime {
    pub fn schedule_render_snapshot(&mut self) -> Result<PendingFrame> {
        let frame = self.profiler.frame();
        let _span = self.profiler.span(frame, "snapshot.freeze");
        let transition = self.render_transition()?;
        let state = Arc::new(self.host.engine.clone());
        let base = self.project.scene3d.clone();
        let mut graph = FrameGraph::new(self.profiler.clone(), frame);
        let state1 = state.clone();
        let base1 = base.clone();
        let p = graph.add("extract.pbr", &[], move |cancel, _| {
            cancel.check()?;
            Ok(Part::Pbr(state1.world.pbr_scene(base1.as_ref())?))
        })?;
        let state2 = state.clone();
        let s = graph.add("extract.simple", &[], move |cancel, _| {
            cancel.check()?;
            Ok(Part::Simple(state2.world.render_scene(base.as_ref())?))
        })?;
        let canvas = self.project.canvas;
        let l = graph.add("extract.layers", &[], move |cancel, _| {
            cancel.check()?;
            Ok(Part::Layers(state.frame(canvas)?))
        })?;
        graph.add("extract.ready", &[p, s, l], |_, _| Ok(Part::Ready))?;
        // Admit immediately so independent extraction overlaps subsequent owner work.
        graph.poll(&self.assets.cpu_tasks())?;
        Ok(PendingFrame {
            surfaces: self.assets.freeze_surfaces(),
            transition,
            restored: self.restored,
            graph,
            nodes: [p, s, l],
            header: Some((
                self.assets.store_id(),
                frame,
                self.host.now_ms(),
                self.host.scene.clone(),
            )),
        })
    }
    pub fn profiler(&self) -> Profiler {
        self.profiler.clone()
    }
}
