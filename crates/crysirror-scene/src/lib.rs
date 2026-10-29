// SPDX-License-Identifier: MPL-2.0
//! CPU engine systems. Ported algorithms retain explicit upstream provenance.
#![forbid(unsafe_code)]
pub mod ai;
pub mod animation;
pub mod animgraph;
pub mod canvas;
pub mod components;
pub mod gameplay;
pub mod input;
pub mod input_buffer;
pub mod layer;
pub mod material;
pub mod model;
pub mod physics;
pub mod projection;
pub mod sequence;
pub mod terrain;
pub mod ui;
pub mod view;
pub mod world;

use anyhow::Result;
use crysirror_core::{service::InputFrame, state::ClientId};
pub use layer::{LayerSpec, LayerSystem, Primitive};
pub use model::{Camera, Geometry, Mesh, Scene3d};
use serde::{Deserialize, Serialize};
pub use ui::{UiSystem, WidgetSpec};
pub use world::{EntitySpec, World, WorldCommand};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemsSpec {
    /// Initial client-owned lighting. Installed through the same validated
    /// command path as the Rust/JS API; later clients explicitly take ownership.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<view::Environment>,
    #[serde(default)]
    pub entities: Vec<EntitySpec>,
    /// Saved native-independent rigid-body settings, forces and joints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics: Option<physics::ClientPhysicsScene>,
    #[serde(default)]
    pub layers: Vec<LayerSpec>,
    #[serde(default)]
    pub widgets: Vec<WidgetSpec>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineSystems {
    #[serde(default)]
    pub canvases: canvas::CanvasSystem,
    pub world: World,
    pub layers: LayerSystem,
    pub ui: UiSystem,
}
impl EngineSystems {
    pub fn install(&mut self, client: ClientId, spec: &SystemsSpec) -> Result<()> {
        let mut staged = self.clone();
        staged.world.install(client, &spec.entities)?;
        if let Some(physics) = &spec.physics {
            staged.world.apply_batch(
                client,
                &[WorldCommand::SetPhysics {
                    physics: physics.clone(),
                }],
            )?;
        }
        if let Some(environment) = &spec.environment {
            staged.world.apply_batch(
                client,
                &[WorldCommand::SetEnvironment {
                    environment: environment.clone(),
                }],
            )?;
        }
        staged.layers.install(client, &spec.layers)?;
        staged.ui.install(client, &spec.widgets)?;
        staged.validate()?;
        for widget in &spec.widgets {
            let mut world = staged.world.clone();
            world.apply_batch(client, &widget.on_activate)?;
        }
        *self = staged;
        Ok(())
    }
    pub fn validate_clients(&self, allowed: &std::collections::BTreeSet<u32>) -> Result<()> {
        anyhow::ensure!(
            self.canvases
                .client_ids()
                .chain(self.world.client_ids())
                .chain(self.layers.client_ids())
                .chain(self.ui.client_ids())
                .all(|id| allowed.contains(&id)),
            "saved system has unregistered client"
        );
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.canvases.validate()?;
        self.world.validate()?;
        self.layers.validate()?;
        self.ui.validate()?;
        Ok(())
    }
    pub fn tick(&mut self, delta_ms: u64, input: InputFrame, canvas: [u32; 2]) -> Result<bool> {
        self.tick_with(delta_ms, input, canvas, |_, _| Ok(()))
    }
    pub fn tick_with(
        &mut self,
        delta_ms: u64,
        input: InputFrame,
        canvas: [u32; 2],
        step: impl FnMut(&mut World, f32) -> Result<()>,
    ) -> Result<bool> {
        let mut staged = self.clone();
        let (actions, consumed) = staged.ui.tick(input, canvas)?;
        for (client, commands) in actions {
            staged.world.apply_batch(ClientId(client), &commands)?;
        }
        staged.world.tick_with(delta_ms, step)?;
        staged.layers.tick(delta_ms);
        *self = staged;
        Ok(consumed)
    }
    pub fn frame(&self, canvas: [u32; 2]) -> Result<Vec<Primitive>> {
        let mut frame = self.layers.frame(canvas)?;
        frame.extend(self.ui.frame(canvas)?);
        Ok(frame)
    }
}

/// Typed engine operations carried through the dependency-free host contract.
/// The runtime decodes this data at the client boundary; no device handles/code.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceBatch {
    #[serde(default)]
    pub canvases: Vec<canvas::CanvasCommand>,
    #[serde(default)]
    pub world: Vec<world::WorldCommand>,
    #[serde(default)]
    pub layers: Vec<layer::LayerCommand>,
    #[serde(default)]
    pub ui: Vec<ui::UiCommand>,
}
impl EngineSystems {
    /// Apply one client batch atomically. Each subsystem's `apply_batch` is
    /// itself atomic and validates what it changed; the sections are staged
    /// without cloning the layer tree (one layer transaction) and canvas
    /// surfaces are copy-on-write, so a batch costs what it touches rather
    /// than the size of the whole engine state.
    pub fn submit(&mut self, client: ClientId, batch: &ServiceBatch) -> Result<()> {
        let mut canvases = (!batch.canvases.is_empty()).then(|| self.canvases.clone());
        let mut world = (!batch.world.is_empty()).then(|| self.world.clone());
        let mut ui = (!batch.ui.is_empty()).then(|| self.ui.clone());
        self.layers.transaction(|layers| {
            if let Some(canvases) = &mut canvases {
                canvases.apply_batch(client, &batch.canvases, layers)?;
            }
            if let Some(world) = &mut world {
                world.apply_batch(client, &batch.world)?;
            }
            if !batch.layers.is_empty() {
                layers.apply_batch(client, &batch.layers)?;
            }
            if let Some(ui) = &mut ui {
                ui.apply_batch(client, &batch.ui)?;
            }
            Ok(())
        })?;
        if let Some(canvases) = canvases {
            self.canvases = canvases;
        }
        if let Some(world) = world {
            self.world = world;
        }
        if let Some(ui) = ui {
            self.ui = ui;
        }
        Ok(())
    }
}

pub mod effect;
