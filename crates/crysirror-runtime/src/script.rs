// SPDX-License-Identifier: MPL-2.0
//! CrySirror's small data-program adapter. This is NOT an RFVP/Siglus VM.
use crate::model::Action;
use crysirror_core::{
    service::{HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
    Error, Result,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Waiting {
    Ready,
    Time { deadline: u64 },
    Input,
    Halted,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: u32,
    program_hash: String,
    pc: usize,
    waiting: Waiting,
}

pub struct ScriptVm {
    client: ClientId,
    program: Arc<Vec<Action>>,
    state: Snapshot,
}
impl ScriptVm {
    pub fn new(client: ClientId, program: Vec<Action>) -> anyhow::Result<Self> {
        for action in &program {
            action.command()?;
        }
        let program_hash = blake3::hash(&serde_json::to_vec(&program)?)
            .to_hex()
            .to_string();
        Ok(Self {
            client,
            program: Arc::new(program),
            state: Snapshot {
                schema: 1,
                program_hash,
                pc: 0,
                waiting: Waiting::Ready,
            },
        })
    }
}
impl VmAdapter for ScriptVm {
    fn engine_id(&self) -> &'static str {
        "crysirror-script-v1"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        instruction_budget: u32,
    ) -> Result<VmYield> {
        if instruction_budget == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        let mut state = self.state.clone();
        match state.waiting {
            Waiting::Time { deadline } if host.now_ms() < deadline => {
                return Ok(VmYield::WaitUntil(deadline))
            }
            Waiting::Input if input.buttons_pressed == 0 => return Ok(VmYield::WaitForInput),
            Waiting::Halted => return Ok(VmYield::Halted),
            _ => state.waiting = Waiting::Ready,
        }
        let mut commands = Vec::new();
        let mut yielded = VmYield::BudgetExhausted;
        for _ in 0..instruction_budget.min(4096) {
            let Some(action) = self.program.get(state.pc) else {
                state.waiting = Waiting::Halted;
                yielded = VmYield::Halted;
                break;
            };
            state.pc += 1;
            match action {
                Action::Wait { ms } => {
                    let deadline = host
                        .now_ms()
                        .checked_add(*ms)
                        .ok_or(Error::Limit("VM clock"))?;
                    state.waiting = Waiting::Time { deadline };
                    yielded = VmYield::WaitUntil(deadline);
                    break;
                }
                Action::WaitInput => {
                    state.waiting = Waiting::Input;
                    yielded = VmYield::WaitForInput;
                    break;
                }
                Action::Halt => {
                    state.waiting = Waiting::Halted;
                    yielded = VmYield::Halted;
                    break;
                }
                _ => commands.push(
                    action
                        .command()
                        .map_err(|_| Error::Invalid("script command"))?
                        .ok_or(Error::Invalid("missing script command"))?,
                ),
            }
        }
        if !commands.is_empty() {
            host.submit(self.client, &commands)?;
        }
        // Failed submission leaves both PC/wait state and host scene unchanged.
        self.state = state;
        Ok(yielded)
    }
    fn snapshot(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self.state).map_err(|_| Error::Invalid("script snapshot"))
    }
    fn restore(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > 64 * 1024 {
            return Err(Error::Limit("script snapshot"));
        }
        let next: Snapshot = serde_json::from_slice(bytes).map_err(|_| Error::CorruptSnapshot)?;
        if next.schema != 1
            || next.program_hash != self.state.program_hash
            || next.pc > self.program.len()
        {
            return Err(Error::SnapshotMismatch("script program"));
        }
        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{
        path::VfsPath,
        service::FileSystem,
        state::{Command, Limits, SceneState},
    };
    struct Host {
        scene: SceneState,
        time: u64,
        reject: bool,
    }
    impl FileSystem for Host {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> Result<Vec<u8>> {
            Err(Error::NotFound)
        }
        fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> Result<()> {
            Err(Error::ReadOnly)
        }
    }
    impl HostServices for Host {
        fn submit(&mut self, id: ClientId, c: &[Command]) -> Result<()> {
            if self.reject {
                return Err(Error::Unsupported("test"));
            }
            self.scene.apply_batch(id, c, Limits::default())
        }
        fn now_ms(&self) -> u64 {
            self.time
        }
    }
    fn host() -> Host {
        Host {
            scene: SceneState::default(),
            time: 0,
            reject: false,
        }
    }
    fn sprite() -> Action {
        Action::Sprite {
            id: 0,
            asset: "a.png".into(),
            rect: [0, 0, 10, 10],
            z: 0,
            tint: [255; 4],
        }
    }
    #[test]
    fn wait_is_driven_by_engine_clock() {
        let mut vm = ScriptVm::new(ClientId(1), vec![Action::Wait { ms: 100 }, sprite()]).unwrap();
        let mut h = host();
        assert_eq!(
            vm.step(&mut h, InputFrame::default(), 100).unwrap(),
            VmYield::WaitUntil(100)
        );
        h.time = 99;
        vm.step(&mut h, InputFrame::default(), 100).unwrap();
        assert!(h.scene.visuals().is_empty());
        h.time = 100;
        vm.step(&mut h, InputFrame::default(), 100).unwrap();
        assert_eq!(h.scene.visuals().len(), 1);
    }
    #[test]
    fn failed_batch_does_not_advance_vm() {
        let mut vm = ScriptVm::new(ClientId(1), vec![sprite()]).unwrap();
        let before = vm.snapshot().unwrap();
        let mut h = host();
        h.reject = true;
        assert!(vm.step(&mut h, InputFrame::default(), 100).is_err());
        assert_eq!(before, vm.snapshot().unwrap());
    }
    #[test]
    fn one_press_does_not_skip_two_waits() {
        let mut vm = ScriptVm::new(
            ClientId(1),
            vec![Action::WaitInput, Action::WaitInput, sprite()],
        )
        .unwrap();
        let mut h = host();
        vm.step(&mut h, InputFrame::default(), 100).unwrap();
        let press = InputFrame {
            buttons_pressed: 1,
            ..InputFrame::default()
        };
        vm.step(&mut h, press, 100).unwrap();
        assert!(h.scene.visuals().is_empty());
        vm.step(&mut h, InputFrame::default(), 100).unwrap();
        assert!(h.scene.visuals().is_empty());
        vm.step(&mut h, press, 100).unwrap();
        assert_eq!(h.scene.visuals().len(), 1);
    }
    #[test]
    fn changed_program_rejects_save() {
        let a = ScriptVm::new(ClientId(1), vec![Action::WaitInput]).unwrap();
        let mut b = ScriptVm::new(ClientId(1), vec![Action::Halt]).unwrap();
        assert!(b.restore(&a.snapshot().unwrap()).is_err());
    }
}
