// SPDX-License-Identifier: MPL-2.0
//! Presentation updates controlled by late-resident action 271.
use crate::registers::{Registers, Value};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    image_processing::{ImageJob, ImageProcessState},
    service::HostServices,
    state::ClientId,
};
use serde_json::json;

const FROZEN_LAYER: u32 = 0x4558_0200;
const FROZEN_ORDER: [i32; 2] = [i32::MAX - 1, 0];

#[derive(Debug, PartialEq)]
pub(super) struct Command {
    destination: i32,
    arguments: [Value; 4],
}

impl Command {
    pub fn parse<'a>(mut fields: impl Iterator<Item = &'a [u8]>) -> Result<Self> {
        let destination = fields.next().map_or(0, crate::native_numbers::integer2011);
        let mut argument = || crate::late_numeric::scalar(fields.next().unwrap_or_default());
        Ok(Self {
            destination,
            arguments: [argument()?, argument()?, argument()?, argument()?],
        })
    }

    pub fn invoke(
        self,
        host: &mut dyn HostServices,
        client: ClientId,
        canvas: [u32; 2],
        registers: &mut Registers,
        updates: &mut Updates,
    ) -> Result<bool> {
        let [mode, second, third, fourth] = self.arguments;
        let mode = registers.read(mode)?;
        for arg in [second, third, fourth] {
            registers.read(arg)?;
        }
        updates.apply(host, client, canvas, mode, self.destination, registers)
    }
}

struct Capture {
    job: ImageJob,
    canvas: [u32; 2],
    destination: i32,
}

#[derive(Default)]
pub(super) struct Updates {
    counter: i32,
    image: Option<String>,
    capture: Option<Capture>,
}

impl Updates {
    pub fn preparing(&self) -> bool {
        self.capture.is_some()
    }

    pub fn counter(&self) -> i32 {
        self.counter
    }

    fn apply(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        canvas: [u32; 2],
        mode: i32,
        destination: i32,
        registers: &mut Registers,
    ) -> Result<bool> {
        let mode = if mode == -1 {
            i32::from(self.counter >= 0)
        } else {
            mode
        };
        let next = match mode {
            1 => self.counter.wrapping_sub(1),
            0 => self.counter.wrapping_add(1),
            -2 => self.counter.max(0),
            _ => self.counter,
        };
        if self.counter >= 0 && next < 0 {
            self.capture = Some(Capture {
                job: host.begin_layer_prefix_image_over(
                    client,
                    canvas,
                    FROZEN_ORDER,
                    [0, 0, 0, 255],
                )?,
                canvas,
                destination,
            });
            return Ok(false);
        }
        let refreshing = self.counter == -1 && next == 0 || mode == -2 && self.counter < 0;
        if refreshing {
            let path = self.image.as_ref().context("lost ExHibit frozen screen")?;
            send(host, client, json!({"op":"remove","id":FROZEN_LAYER}))?;
            self.counter = next;
            host.release_image_resource(client, path)?;
            self.image = None;
        } else {
            self.counter = next;
        }
        result(registers, destination)?;
        Ok(refreshing)
    }

    pub fn finish(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        registers: &mut Registers,
    ) -> Result<bool> {
        let Some(capture) = &mut self.capture else {
            return Ok(true);
        };
        let image = match capture.job.poll()? {
            ImageProcessState::Pending => return Ok(false),
            ImageProcessState::Ready(image) => image,
            ImageProcessState::Failed(error) => bail!("ExHibit screen capture: {error}"),
        };
        ensure!(
            image.client() == client && image.size() == capture.canvas,
            "invalid ExHibit screen capture"
        );
        let path = host
            .publish_image_resource(client, FROZEN_LAYER, image)?
            .path;
        let [width, height] = capture.canvas;
        if let Err(error) = send(
            host,
            client,
            json!({"op":"put","layer":{
                "id":FROZEN_LAYER,"rect":[0,0,width,height],"order":FROZEN_ORDER,
                "paint":{"kind":"image","asset":path.relative()},
            }}),
        ) {
            let _ = host.release_image_resource(client, path.relative());
            return Err(error);
        }
        self.counter = -1;
        self.image = Some(path.relative().to_owned());
        result(registers, capture.destination)?;
        self.capture = None;
        Ok(true)
    }
}

fn result(registers: &mut Registers, destination: i32) -> Result<()> {
    if destination >= 0 {
        registers.set_local(destination, 0)?;
    }
    Ok(())
}

fn send(host: &mut dyn HostServices, client: ClientId, layer: serde_json::Value) -> Result<()> {
    host.submit_engine(client, 1, &serde_json::to_vec(&json!({"layers":[layer]}))?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Script, Vm, profile::Resident, scenario_load_tests};
    use crysirror_core::{
        Error, Result as CoreResult,
        image_processing::{ImageProcess, ImageResource},
        path::{Mount, VfsPath},
        service::{FileSystem, InputFrame, UploadedImage},
        state::Command as EngineCommand,
        vm::VmAdapter,
    };
    use std::sync::Arc;

    #[derive(Debug)]
    struct Pixel(ClientId, [u8; 4]);
    impl ImageResource for Pixel {
        fn client(&self) -> ClientId {
            self.0
        }
        fn size(&self) -> [u32; 2] {
            [1, 1]
        }
        fn opaque(&self) -> bool {
            true
        }
        fn rgba(&self) -> Arc<[u8]> {
            self.1.into()
        }
    }

    struct Job(Arc<Pixel>, bool);
    impl ImageProcess for Job {
        fn poll(&mut self) -> CoreResult<ImageProcessState> {
            if std::mem::replace(&mut self.1, false) {
                return Ok(ImageProcessState::Pending);
            }
            Ok(ImageProcessState::Ready(self.0.clone()))
        }
    }

    #[derive(Default)]
    struct Presenter {
        live: [u8; 4],
        published: Option<Arc<dyn ImageResource>>,
        overlay_visible: bool,
        captures: usize,
        refreshes: usize,
    }
    impl Presenter {
        fn displayed(&self) -> [u8; 4] {
            if self.overlay_visible {
                return self
                    .published
                    .as_ref()
                    .unwrap()
                    .rgba()
                    .as_ref()
                    .try_into()
                    .unwrap();
            }
            self.live
        }
    }
    impl FileSystem for Presenter {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> CoreResult<Vec<u8>> {
            Err(Error::NotFound)
        }
        fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
            Err(Error::ReadOnly)
        }
    }
    impl HostServices for Presenter {
        fn now_ms(&self) -> u64 {
            0
        }
        fn submit(&mut self, _: ClientId, _: &[EngineCommand]) -> CoreResult<()> {
            Err(Error::Unsupported("fixture world commands"))
        }
        fn begin_layer_prefix_image_over(
            &mut self,
            client: ClientId,
            size: [u32; 2],
            order: [i32; 2],
            clear: [u8; 4],
        ) -> CoreResult<ImageJob> {
            assert_eq!((size, order, clear), ([1, 1], FROZEN_ORDER, [0, 0, 0, 255]));
            assert!(!self.overlay_visible);
            self.captures += 1;
            Ok(Box::new(Job(Arc::new(Pixel(client, self.live)), true)))
        }
        fn publish_image_resource(
            &mut self,
            client: ClientId,
            id: u32,
            image: Arc<dyn ImageResource>,
        ) -> CoreResult<UploadedImage> {
            assert_eq!((client, id), (image.client(), FROZEN_LAYER));
            self.published = Some(image);
            Ok(UploadedImage {
                path: VfsPath::new(Mount::Asset, "frozen-screen")?,
                size: [1, 1],
                opaque: true,
            })
        }
        fn submit_engine(&mut self, _: ClientId, schema: u32, bytes: &[u8]) -> CoreResult<()> {
            assert_eq!(schema, 1);
            let batch: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            let layer = &batch["layers"][0];
            match layer["op"].as_str() {
                Some("put") => {
                    assert_eq!(layer["layer"]["id"], FROZEN_LAYER);
                    assert_eq!(layer["layer"]["paint"]["asset"], "frozen-screen");
                    self.overlay_visible = true;
                }
                Some("remove") => {
                    assert_eq!(layer["id"], FROZEN_LAYER);
                    self.overlay_visible = false;
                    self.refreshes += 1;
                }
                _ => unreachable!(),
            }
            Ok(())
        }
        fn release_image_resource(&mut self, _: ClientId, path: &str) -> CoreResult<()> {
            assert_eq!(path, "frozen-screen");
            assert!(!self.overlay_visible);
            assert!(self.published.take().is_some());
            Ok(())
        }
    }

    fn step(vm: &mut Vm, host: &mut Presenter) -> Result<()> {
        vm.step(host, InputFrame::default(), 1)?;
        Ok(())
    }

    #[test]
    fn late_screen_271_keeps_the_displayed_frame_until_final_unlock() -> Result<()> {
        for resident in [Resident::LateVol1, Resident::LateVol2] {
            let modes = ["R11", "1", "0", "77", "-2", "0", "-1", "-1", "-1"];
            let mut bytes = Vec::new();
            for mode in modes {
                bytes.extend(0x0100_00bfu32.to_le_bytes());
                bytes.extend_from_slice(format!("271,10,{mode},0,0,0,*\0").as_bytes());
            }
            let mut vm = scenario_load_tests::vm(3)?;
            vm.canvas = [1, 1];
            let mut script = Script::new(
                scenario_load_tests::scenario(3, &bytes, modes.len() as u32),
                3,
            )?;
            script.resident = Some(resident);
            let first = script.pc;
            vm.script = Some(script);
            vm.registers.set_local(10, 99)?;
            vm.registers.set_local(11, -1)?;
            let mut host = Presenter {
                live: [215, 9, 1, 255],
                ..Default::default()
            };
            let before = host.displayed();

            step(&mut vm, &mut host)?; // Schedule the outer freeze.
            step(&mut vm, &mut host)?; // Capture pending: retain PC and result.
            assert_eq!(vm.script.as_ref().unwrap().pc, first);
            assert_eq!(vm.registers.read(Value::Local(10))?, 99);
            step(&mut vm, &mut host)?; // Present the frozen image and complete 271.
            assert!(vm.script.as_ref().unwrap().pc > first);
            assert_eq!(vm.registers.read(Value::Local(10))?, 0);
            assert_eq!(vm.late_screen.counter(), -1);
            assert_eq!(host.captures, 1);

            host.live = [12, 42, 91, 255];
            for count in [-2, -1, -1] {
                step(&mut vm, &mut host)?;
                assert_eq!(vm.late_screen.counter(), count);
                assert_eq!(host.displayed(), before);
                assert_eq!(host.captures, 1);
            }
            step(&mut vm, &mut host)?; // Unwind the remaining lock and refresh.
            assert_eq!(vm.late_screen.counter(), 0);
            assert_eq!(host.displayed(), host.live);
            assert_eq!(host.refreshes, 1);

            for count in [1, 0] {
                step(&mut vm, &mut host)?;
                assert_eq!(vm.late_screen.counter(), count);
                assert_eq!((host.captures, host.refreshes), (1, 1));
            }
            for _ in 0..3 {
                step(&mut vm, &mut host)?;
            }
            assert_eq!(vm.late_screen.counter(), -1);
            assert_eq!(host.captures, 2);
            step(&mut vm, &mut host)?;
            assert_eq!(vm.late_screen.counter(), 0);
            assert_eq!(host.refreshes, 2);
        }
        Ok(())
    }
}
