// SPDX-License-Identifier: MPL-2.0
//! The original BGI interpreter as a dormant, capability-scoped VM adapter.
//! Native draw plans reach shared CrySirror image/text services; this crate
//! does not invoke the original engine's whole-frame renderer or own devices.
#![forbid(unsafe_code)]
mod audio_bridge;
mod config;
mod frame_bridge;
use anyhow::{ensure, Context, Result};
pub use config::Config;
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{buttons, HostServices, InputFrame, StorageLease},
    state::ClientId,
    vm::{VmAdapter, VmYield},
    Error,
};
use ethornell_app::host_runtime::{HostConfig, HostInput, HostRuntime};
use std::{collections::VecDeque, sync::Arc};

pub const ENGINE_ID: &str = "bgi";

pub struct BgiVm {
    client: ClientId,
    config: Config,
    machine: Option<HostRuntime>,
    bridge: Option<frame_bridge::FrameBridge>,
    last_ms: u64,
    last_buttons: u64,
    last_pointer: Option<(i32, i32)>,
    frames: u64,
    draws: serde_json::Value,
    discarded_audio_commands: u64,
    events: VecDeque<serde_json::Value>,
    _storage_lease: Option<Box<dyn StorageLease>>,
}
impl BgiVm {
    /// No filesystem, process-global state, threads or device side effects.
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            machine: None,
            bridge: None,
            last_ms: 0,
            last_buttons: 0,
            last_pointer: None,
            frames: 0,
            draws: serde_json::Value::Null,
            discarded_audio_commands: 0,
            events: VecDeque::new(),
            _storage_lease: None,
        })
    }
    fn boot(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let storage = host
            .storage(self.client)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let lease = storage
            .acquire_lease("bgi-session")
            .map_err(|e| anyhow::anyhow!("BGI save writer lease: {e}"))?;
        let font_path =
            VfsPath::new(Mount::Asset, &self.config.font).map_err(|e| anyhow::anyhow!("{e}"))?;
        let font = storage
            .read(&font_path)
            .map_err(|e| anyhow::anyhow!("BGI font '{}': {e}", self.config.font))?;
        let bridge = frame_bridge::FrameBridge::new(font.clone(), self.config.font_face_index)?;
        let machine = HostRuntime::create(HostConfig {
            storage,
            asset_root: self.config.root.clone(),
            font_data: font,
            font_face_index: self.config.font_face_index,
            gbk: self.config.gbk,
            strict: self.config.strict,
        })
        .context("boot original BGI interpreter")?;
        self.machine = Some(machine);
        self.bridge = Some(bridge);
        self._storage_lease = Some(lease);
        Ok(())
    }
    fn inputs(&mut self, input: InputFrame) -> Vec<HostInput> {
        let mut events = Vec::new();
        let pointer = (input.pointer_x, input.pointer_y);
        let (x, y) = (pointer.0 as f32, pointer.1 as f32);
        if self.last_pointer != Some(pointer) {
            events.push(HostInput::MouseMove { x, y });
            self.last_pointer = Some(pointer);
        }
        for (bit, code) in [
            (buttons::ENTER, 13),
            (buttons::SPACE, 32),
            (buttons::LEFT, 37),
            (buttons::UP, 38),
            (buttons::RIGHT, 39),
            (buttons::DOWN, 40),
            (buttons::TAB, 9),
            (buttons::SHIFT, 16),
            (buttons::CTRL, 17),
            (buttons::KEY_W, 87),
            (buttons::KEY_A, 65),
            (buttons::KEY_S, 83),
            (buttons::KEY_D, 68),
            (buttons::KEY_E, 69),
            (buttons::KEY_Q, 81),
            (buttons::KEY_F, 70),
            (buttons::KEY_R, 82),
            (buttons::MOUSE_LEFT, 1),
            (buttons::MOUSE_RIGHT, 2),
        ] {
            let was = self.last_buttons & bit != 0;
            let pressed = input.buttons_pressed & bit != 0;
            let down = input.buttons_down & bit != 0;
            let mouse = bit == buttons::MOUSE_LEFT || bit == buttons::MOUSE_RIGHT;
            if (down || pressed) && !was {
                events.push(if mouse {
                    HostInput::ButtonPress { button: code, x, y }
                } else {
                    HostInput::KeyPress { descriptor: code }
                });
            }
            if !down && (was || pressed) {
                events.push(if mouse {
                    HostInput::ButtonRelease { button: code, x, y }
                } else {
                    HostInput::KeyRelease { descriptor: code }
                });
            }
        }
        self.last_buttons = input.buttons_down;
        events
    }
    fn execute(&mut self, host: &mut dyn HostServices, input: InputFrame) -> Result<VmYield> {
        let now = host.now_ms();
        if self.machine.is_none() {
            self.boot(host)?;
            self.last_ms = now;
        }
        let inputs = self.inputs(input);
        let delta = now.saturating_sub(self.last_ms);
        self.last_ms = now;
        let machine = self.machine.as_mut().expect("BGI booted above");
        let plan = machine
            .frame(delta, &inputs)
            .context("original BGI frame")?;
        let audio = machine.take_audio_commands();
        for command in audio {
            if self.config.silent {
                self.discarded_audio_commands += 1;
            } else {
                audio_bridge::submit(self.client, command, host)?;
            }
        }
        while let Some(event) = machine.poll_event() {
            ensure!(
                event.request_id == 0 && event.kind != 2 && event.kind != 5,
                "BGI requires a host interaction response: {}",
                event.text
            );
            if self.events.len() == 64 {
                self.events.pop_front();
            }
            self.events
                .push_back(serde_json::json!({"kind":event.kind,"code":event.code,
                "text":event.text.chars().take(1024).collect::<String>()}));
        }
        if let Some(plan) = plan {
            let stats = self
                .bridge
                .as_mut()
                .expect("BGI bridge booted above")
                .publish(self.client, &plan, host)?;
            self.draws = serde_json::json!({"screen":[stats.width,stats.height],
                "image_draws":stats.image_draws,"glyph_draws":stats.glyph_draws,
                "framebuffer_hash":stats.framebuffer_hash});
        }
        self.frames += 1;
        Ok(if machine.is_halted() {
            VmYield::Halted
        } else {
            VmYield::BudgetExhausted
        })
    }
}
impl VmAdapter for BgiVm {
    fn engine_id(&self) -> &'static str {
        ENGINE_ID
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        if budget == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        // BGI schedules its original VM threads to their frame boundary.
        self.execute(host, input)
            .map_err(|e| Error::Io(format!("{e:#}")))
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported(
            "BGI runtime snapshot; use original game save slots",
        ))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported(
            "BGI runtime snapshot; use original game save slots",
        ))
    }
    fn diagnostics(&self) -> Vec<u8> {
        let native = self.machine.as_ref().map(|machine| {
            let text = machine.diagnostics();
            if text.len() > 16384 {
                return serde_json::json!({"message":text.chars().take(4096).collect::<String>(),"truncated":true});
            }
            serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(
                |_| serde_json::json!({"message":text.chars().take(16384).collect::<String>()}),
            )
        });
        serde_json::to_vec(
            &serde_json::json!({"vm":"original-bgi","booted":self.machine.is_some(),
            "frames":self.frames,"strict":self.config.strict,"silent":self.config.silent,
            "discarded_audio_commands":self.discarded_audio_commands,"draws":self.draws,
            "events":self.events,"native":native}),
        )
        .unwrap_or_default()
    }
}
pub struct BgiFactory;
impl crysirror_runtime::runtime::VmFactory for BgiFactory {
    fn create(
        &self,
        client: ClientId,
        spec: &crysirror_runtime::model::ClientSpec,
    ) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "BGI executes original bytecode; data VM program must be empty"
        );
        let config = serde_json::from_value(spec.config.clone()).context("native BGI config")?;
        Ok(Box::new(BgiVm::new(client, config)?))
    }
}
pub fn register(registry: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    registry.register(ENGINE_ID, Arc::new(BgiFactory))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::service::FileSystem;
    use crysirror_runtime::runtime::VmFactory;
    fn config() -> Config {
        serde_json::from_value(serde_json::json!({"font":"missing/font.ttf"})).unwrap()
    }
    struct NoAccess;
    impl FileSystem for NoAccess {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            panic!("dormant BGI must not read files")
        }
        fn write_atomic(
            &mut self,
            _: ClientId,
            _: &VfsPath,
            _: &[u8],
        ) -> crysirror_core::Result<()> {
            panic!("dormant BGI must not write files")
        }
    }
    impl HostServices for NoAccess {
        fn now_ms(&self) -> u64 {
            panic!("zero-budget BGI must not boot")
        }
        fn submit(
            &mut self,
            _: ClientId,
            _: &[crysirror_core::state::Command],
        ) -> crysirror_core::Result<()> {
            panic!("dormant BGI must not publish")
        }
    }
    #[test]
    fn construction_and_factory_are_dormant_and_snapshots_fail_explicitly() {
        let mut vm = BgiVm::new(ClientId(3), config()).unwrap();
        let diagnostics: serde_json::Value = serde_json::from_slice(&vm.diagnostics()).unwrap();
        assert_eq!(diagnostics["booted"], false);
        assert_eq!(diagnostics["strict"], true);
        assert!(vm
            .snapshot()
            .unwrap_err()
            .to_string()
            .contains("original game save slots"));
        assert!(vm.restore(b"anything").is_err());
        assert_eq!(
            vm.step(&mut NoAccess, InputFrame::default(), 0).unwrap(),
            VmYield::BudgetExhausted
        );
        let mut spec: crysirror_runtime::model::ClientSpec = serde_json::from_value(
            serde_json::json!({"name":"bgi","adapter":"bgi","game":"test","config":{"font":"missing/font.ttf"}})
        ).unwrap();
        assert_eq!(
            BgiFactory.create(ClientId(3), &spec).unwrap().engine_id(),
            "bgi"
        );
        spec.program.push(crysirror_runtime::model::Action::Halt);
        assert!(BgiFactory.create(ClientId(3), &spec).is_err());
        let mut registry = crysirror_runtime::AdapterRegistry::default();
        register(&mut registry).unwrap();
        assert!(registry.names().contains(&"bgi"));
        assert!(register(&mut registry).is_err());
    }
    #[test]
    fn input_pulses_and_held_edges_reach_original_native_descriptors() {
        let mut vm = BgiVm::new(ClientId(1), config()).unwrap();
        let events = vm.inputs(InputFrame {
            buttons_pressed: buttons::ENTER | buttons::MOUSE_RIGHT,
            pointer_x: 30,
            pointer_y: 40,
            ..Default::default()
        });
        assert!(matches!(events[0], HostInput::MouseMove { x: 30., y: 40. }));
        assert!(matches!(events[1], HostInput::KeyPress { descriptor: 13 }));
        assert!(matches!(
            events[2],
            HostInput::KeyRelease { descriptor: 13 }
        ));
        assert!(matches!(
            events[3],
            HostInput::ButtonPress {
                button: 2,
                x: 30.,
                y: 40.
            }
        ));
        assert!(matches!(
            events[4],
            HostInput::ButtonRelease {
                button: 2,
                x: 30.,
                y: 40.
            }
        ));
        let held = InputFrame {
            buttons_down: buttons::SPACE,
            pointer_x: 30,
            pointer_y: 40,
            ..Default::default()
        };
        assert!(matches!(
            vm.inputs(held)[0],
            HostInput::KeyPress { descriptor: 32 }
        ));
        assert!(vm.inputs(held).is_empty());
        let events = vm.inputs(InputFrame {
            pointer_x: 30,
            pointer_y: 40,
            ..Default::default()
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            HostInput::KeyRelease { descriptor: 32 }
        ));
        let events = vm.inputs(InputFrame {
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            pointer_x: 77,
            pointer_y: 88,
            ..Default::default()
        });
        assert!(matches!(events[0], HostInput::MouseMove { x: 77., y: 88. }));
        assert!(matches!(
            events[1],
            HostInput::ButtonPress {
                button: 1,
                x: 77.,
                y: 88.
            }
        ));
        let events = vm.inputs(InputFrame {
            pointer_x: 90,
            pointer_y: 91,
            ..Default::default()
        });
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], HostInput::MouseMove { x: 90., y: 91. }));
        assert!(matches!(
            events[1],
            HostInput::ButtonRelease {
                button: 1,
                x: 90.,
                y: 91.
            }
        ));
    }
}
