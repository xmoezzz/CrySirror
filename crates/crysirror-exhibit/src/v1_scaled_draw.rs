// SPDX-License-Identifier: MPL-2.0
//! RLD v1 0x4a's 0x8c580 RGB resampling branch.
use crate::{
    Vm, VmYield, clip,
    registers::{Bank, Change},
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState, ImageResource},
    path::{Mount, VfsPath},
    service::{ClientStorage, HostServices},
};
use crysirror_tasks::Task;
use serde_json::json;
use std::sync::Arc;

pub(super) fn supports(crop: [i32; 4], canvas: [u32; 2]) -> bool {
    let [w, h] = [crop[2] as u32, crop[3] as u32];
    let [width, height] = canvas;
    crop.iter().all(|n| *n >= 0)
        && (w < width || h < height)
        && w != width
        && h != height
        && w > width / 4
        && h > height / 4
}

struct Draw {
    id: i32,
    crop: [i32; 4],
    retain_clip: bool,
    duration: u64,
    continuation: usize,
}

enum Stage {
    Reading(Task<ImageInput>),
    Sampling(ImageJob),
    Presenting(u64),
}

pub(super) struct Pending {
    draw: Draw,
    stage: Stage,
}

impl Pending {
    pub(super) fn preparing(&self) -> bool {
        !matches!(self.stage, Stage::Presenting(_))
    }
}

fn read_input(storage: &dyn ClientStorage, path: VfsPath) -> Result<ImageInput> {
    let input = storage.open_read(&path)?;
    let mut header = [0; 36];
    crysirror_resource::read_exact_at(input.as_ref(), 0, &mut header)?;
    let word = |index| u32::from_le_bytes(header[index..index + 4].try_into().unwrap());
    ensure!(
        header.starts_with(b"GYU\x1a") && word(12) == 24 && word(28) == 0 && word(32) == 0,
        "unsupported ExHibit scaled CG source layout"
    );
    Ok(ImageInput::ScopedAsset {
        path,
        size: [word(16), word(20)],
        fingerprint: None,
    })
}

impl Vm {
    pub(super) fn store_v1_draw(&mut self, id: i32, crop: [i32; 4], clip_flag: i32) -> Result<()> {
        let [width, height] = self.canvas;
        self.clip = None;
        self.magnify_target = None;
        self.draw_scene[0] = id;
        self.draw_scene[1] = clip_flag;
        // 0x32580 stores the authored rectangle even when the backup is released.
        self.draw_scene[2..6].copy_from_slice(&crop);
        self.draw_scene[8..10].fill(-1);
        self.draw_scene[11] = (width | (height << 16)) as i32;
        if id >= 0 && self.registers.configured() {
            self.registers
                .change(Change::Experience(Bank::Cg, vec![id]))?;
        }
        Ok(())
    }

    pub(super) fn begin_v1_scaled_draw(
        &mut self,
        host: &mut dyn HostServices,
        id: i32,
        asset: &str,
        crop: [i32; 4],
        retain_clip: bool,
        duration: u64,
    ) -> Result<()> {
        ensure!(
            self.v1_scaled_draw.is_none() && self.clip.is_none() && self.movie.is_none(),
            "unsupported ExHibit scaled CG with an active background effect"
        );
        let path = VfsPath::new(Mount::Asset, asset)?;
        let storage = host.storage(self.client)?;
        let script = self
            .script
            .as_mut()
            .context("missing ExHibit scaled draw script")?;
        let continuation = script.pc;
        let load = self.worker.spawn(move |cancel| {
            cancel.check()?;
            read_input(storage.as_ref(), path)
        })?;
        let draw = Draw {
            id,
            crop,
            retain_clip,
            duration,
            continuation,
        };
        self.v1_scaled_draw = Some(Pending {
            draw,
            stage: Stage::Reading(load),
        });
        Ok(())
    }

    pub(super) fn poll_v1_scaled_draw(
        &mut self,
        host: &mut dyn HostServices,
        now: u64,
    ) -> Result<Option<VmYield>> {
        let Some(Pending { draw, stage }) = self.v1_scaled_draw.take() else {
            return Ok(None);
        };
        let (stage, reason) = match stage {
            Stage::Reading(mut read) => {
                let next = if let Some(input) = read.try_take()? {
                    let effects = serde_json::to_vec(&json!([{
                        "kind": "rect_resample_byte255",
                        "spec": {"source": draw.crop, "size": self.canvas},
                    }]))?;
                    Stage::Sampling(
                        host.image_metadata(self.client)?
                            .begin_process(input, effects.into())?,
                    )
                } else {
                    Stage::Reading(read)
                };
                (next, VmYield::BudgetExhausted)
            }
            Stage::Sampling(mut job) => match job.poll()? {
                ImageProcessState::Pending => (Stage::Sampling(job), VmYield::BudgetExhausted),
                ImageProcessState::Failed(error) => bail!("{error}"),
                ImageProcessState::Ready(image) => {
                    let deadline = self.present_v1_scaled_draw(host, &draw, image)?;
                    (Stage::Presenting(deadline), VmYield::WaitUntil(deadline))
                }
            },
            Stage::Presenting(deadline) if now < deadline => {
                (Stage::Presenting(deadline), VmYield::WaitUntil(deadline))
            }
            Stage::Presenting(_) => {
                self.store_v1_draw(
                    draw.id,
                    draw.crop,
                    if draw.retain_clip { 0x10000 } else { 0 },
                )?;
                let script = self
                    .script
                    .as_mut()
                    .context("missing ExHibit scaled draw continuation")?;
                script.pc = draw.continuation;
                script.remaining -= 1;
                return Ok(None);
            }
        };
        self.v1_scaled_draw = Some(Pending { draw, stage });
        Ok(Some(reason))
    }

    fn present_v1_scaled_draw(
        &mut self,
        host: &mut dyn HostServices,
        draw: &Draw,
        image: Arc<dyn ImageResource>,
    ) -> Result<u64> {
        let path = host.publish_image_resource(self.client, 1, image)?.path;
        let relative = path.relative();
        let paint = json!({"kind": "image", "asset": relative});
        let command = clip::draw_commands(paint, self.canvas, None, draw.duration);
        let published = host.submit_engine(self.client, 1, &serde_json::to_vec(&command)?);
        if let Err(error) = published {
            let _ = host.release_image_resource(self.client, relative);
            return Err(error.into());
        }
        let old = self.v1_scaled_image.replace(relative.to_owned());
        if let Some(old) = old {
            host.release_image_resource(self.client, &old)?;
        }
        Ok(host.now_ms().saturating_add(draw.duration))
    }

    pub(super) fn release_v1_scaled_image(&mut self, host: &mut dyn HostServices) -> Result<()> {
        if let Some(old) = &self.v1_scaled_image {
            host.release_image_resource(self.client, old)?;
            self.v1_scaled_image = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Script,
        registers::Value,
        scenario_load_tests::{scenario, vm},
    };
    use crysirror_core::{service::InputFrame, state::ClientId};
    use std::{
        fs,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    fn record(id: u32, mode: u32, ms: u32, crop: [u32; 4], flags: u32) -> Vec<u8> {
        let [x, y, w, h] = crop;
        [
            0x0010_004a,
            id,
            mode,
            0,
            ms,
            0,
            0,
            0,
            x,
            y,
            w,
            h,
            u32::MAX,
            u32::MAX,
            flags,
            0,
            0,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
    }

    #[test]
    fn scaled_background_preserves_the_scoped_scene_until_publication_and_resumes_after_fade()
    -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "exhibit-scaled-draw-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir(&dir)?;
        let result = (|| -> Result<()> {
            let assets = dir.join("assets/res/g/ev/00");
            fs::create_dir_all(&assets)?;
            let mut gyu = b"GYU\x1a\x00\x00\x00\x01".to_vec();
            gyu.extend(
                [u32::MAX, 24, 4, 3, 36, 0, 0]
                    .into_iter()
                    .flat_map(u32::to_le_bytes),
            );
            for row in [[40, 80, 120, 160], [20, 60, 100, 140], [0, 40, 80, 120]] {
                gyu.extend(row.into_iter().flat_map(|r| [0, 255, r]));
            }
            fs::write(assets.join("0005.gyu"), gyu)?;
            let project = dir.join("project.json");
            fs::write(&project, br#"{"schema":2,"name":"scale","content_revision":"1","canvas":[4,4],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
            let mut runtime = crysirror_runtime::Runtime::load(&project, &dir.join("state"))?;
            let mut vm = vm(1)?;
            vm.canvas = [4, 4];
            vm.client = ClientId(1);
            let black = 0x8000_0000;
            let mut records = record(black, 255, 0, [0; 4], 0);
            let offset = 16 + records.len();
            records.extend(record(5, 9, 500, [1, 0, 2, 2], 65536));
            records.extend(record(black, 255, 0, [0; 4], 0));
            vm.script = Some(Script::new(scenario(1, &records, 3), 1)?);
            let input = InputFrame::default();
            let first = vm.tick(&mut runtime.host, input, 2)?;
            assert_eq!(first, VmYield::BudgetExhausted);
            assert_eq!(vm.script.as_ref().unwrap().pc, offset);
            assert!(vm.v1_scaled_image.is_none());
            assert_eq!(vm.draw_scene[0], black as i32);
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                let state = vm.tick(&mut runtime.host, input, 2)?;
                assert_eq!(vm.script.as_ref().unwrap().pc, offset);
                if state == VmYield::WaitUntil(500) {
                    break;
                }
                ensure!(
                    state == VmYield::BudgetExhausted && Instant::now() < until,
                    "scaled CG preparation did not complete"
                );
                std::thread::yield_now();
            }
            assert_eq!(vm.registers.read(Value::Seen(Bank::Cg, 5))?, 0);
            let asset = vm
                .v1_scaled_image
                .clone()
                .context("missing published scaled CG")?;
            let image = runtime
                .host
                .published_image_resource(vm.client, &asset)?
                .unwrap();
            assert_eq!(image.size(), [4, 4]);
            assert_eq!(image.pixel_at([3, 3])?, Some([129, 253, 0, 255]));
            runtime.tick(500, input)?;
            vm.tick(&mut runtime.host, input, 0)?;
            assert_eq!(vm.draw_scene[..6], [5, 65536, 1, 0, 2, 2]);
            assert_eq!(vm.registers.read(Value::Seen(Bank::Cg, 5))?, 1);
            vm.tick(&mut runtime.host, input, 1)?;
            assert!(vm.v1_scaled_image.is_none());
            assert_eq!(vm.draw_scene[0], black as i32);
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }
}
