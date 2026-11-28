// SPDX-License-Identifier: MPL-2.0
//! Numbered CG billboards, synchronous channel-8 fades and retirement.
use crate::{
    Vm, VmYield,
    branch::number,
    panel_animation::Kind,
    registers::{Bank, Change},
    surfaces::Surface,
    v1_services::decimal,
};
use anyhow::{Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{ClientStorage, HostServices},
};
use crysirror_tasks::Task;

#[derive(Debug, PartialEq)]
pub(super) struct Draw {
    cg: i32,
    surface: Surface,
    key: i32,
    fade: Option<(u8, u32)>,
}

impl Draw {
    pub fn parse(raw: &[u8], extended: bool) -> Result<Self> {
        let count = if extended { 27 } else { 26 };
        let f: Vec<_> = raw.splitn(count, |&b| b == b',').collect();
        ensure!(f.len() == count, "short ExHibit numbered billboard");
        let n = |i: usize| number(f[i]);
        let text_index = if extended { 21 } else { 20 };
        ensure!(
            n(0)? == 0
                && n(17)? == -1
                && n(18)? == 0
                && (!extended || n(19)? == 0)
                && (f[text_index].is_empty() || f[text_index] == b"-1"),
            "unsupported ExHibit numbered billboard property"
        );
        let slot = u8::try_from(n(1)?)?;
        ensure!(slot < 127, "unsupported ExHibit dynamic billboard slot");
        let origin = [n(4)?, n(5)?];
        ensure!(
            origin.iter().all(|v| (-32768..=32767).contains(v)),
            "unsupported ExHibit indirect billboard position"
        );
        let cg = n(2)?;
        let asset = format!("res/g/ev/{}/{}.gyu", decimal(cg / 100, 2), decimal(cg, 4));
        let effect = n(9)?;
        let level = u8::try_from(n(10)?)?;
        ensure!(level <= 16, "unsupported ExHibit billboard fade target");
        let key = n(12)?;
        ensure!(
            (-2..=255).contains(&key),
            "unsupported ExHibit billboard palette key"
        );
        // Numbered CG ignores the CSV rectangle size, path, work-surface parameters
        // and delay fields (those belong to other source types in cmdBillboard).
        let (initial, fade) = match effect {
            9 => (0, Some((level, n(8)? as u32))),
            519 => (0, None),
            _ => (level, None), // 255 shows directly; other values run AsyncShow once.
        };
        Ok(Self {
            cg,
            surface: Surface {
                slot,
                asset,
                origin,
                level: initial,
            },
            key,
            fade,
        })
    }

    fn size(&self, storage: &dyn ClientStorage) -> Result<[u32; 2]> {
        let path = VfsPath::new(Mount::Asset, &self.surface.asset)?;
        let reader = storage.open_read(&path)?;
        let mut b = [0; 36];
        crysirror_resource::read_exact_at(reader.as_ref(), 0, &mut b)?;
        self.size_from_header(&b)
    }

    fn size_from_header(&self, b: &[u8; 36]) -> Result<[u32; 2]> {
        ensure!(
            b.starts_with(b"GYU\x1a"),
            "unrecognized ExHibit CG billboard resource"
        );
        let word = |at| u32::from_le_bytes(b[at..at + 4].try_into().expect("GYU header"));
        let size = [word(16), word(20)];
        ensure!(
            size.iter().all(|n| (1..=8192).contains(n)),
            "invalid ExHibit CG billboard size"
        );
        if self.key != -2 {
            // With an alpha plane both keyed blitters branch to that plane
            // before comparing palette indices (0x958d0 and 0x94c00).
            ensure!(
                word(12) == 8 && word(28) != 0,
                "unsupported ExHibit palette-key billboard without indexed alpha"
            );
        }
        Ok(size)
    }
}

pub(super) enum Pending {
    Loading(Task<(Draw, [u32; 2])>),
    Mark(i32),
    Remove(u8),
}

impl Vm {
    pub(super) fn begin_cg_billboard(
        &mut self,
        h: &mut dyn HostServices,
        draw: Draw,
    ) -> Result<()> {
        ensure!(
            self.opening_dlt.as_ref().is_none_or(|delta| delta.slot() != draw.surface.slot),
            "ExHibit billboard conflicts with opening delta layer"
        );
        let storage = h.storage(self.client)?;
        self.v1_image_billboard = Some(Pending::Loading(self.worker.spawn(move |cancel| {
            cancel.check()?;
            let size = draw.size(storage.as_ref())?;
            cancel.check()?;
            Ok((draw, size))
        })?));
        Ok(())
    }

    pub(super) fn retire_billboard(
        &mut self,
        h: &mut dyn HostServices,
        slot: u8,
        ms: u32,
    ) -> Result<()> {
        if ms == u32::MAX {
            return self.panels.remove(h, self.client, slot);
        }
        if !self.panels.is_bound(slot) {
            self.animations.leave(Kind::Fade, 8, false);
            ensure!(
                ms == 0 || self.controls.skip,
                "unverified ExHibit inactive billboard fade level on slot {slot}"
            );
            return Ok(());
        }
        self.panel_fade(h, slot, 0, ms)?;
        self.v1_image_billboard = Some(Pending::Remove(slot));
        Ok(())
    }

    fn panel_fade(
        &mut self,
        h: &mut dyn HostServices,
        slot: u8,
        level: u8,
        duration: u32,
    ) -> Result<()> {
        let now = h.now_ms();
        self.animations.replace_billboard_fade(
            &self.panels,
            now,
            slot,
            level,
            duration,
            self.controls.skip,
        )?;
        self.animations
            .tick(&mut self.panels, h, self.client, now)?;
        Ok(())
    }

    pub(super) fn poll_cg_billboard(
        &mut self,
        h: &mut dyn HostServices,
    ) -> Result<Option<VmYield>> {
        let Some(mut work) = self.v1_image_billboard.take() else {
            return Ok(None);
        };
        if let Pending::Loading(job) = &mut work {
            let Some((draw, size)) = job.try_take()? else {
                self.v1_image_billboard = Some(work);
                return Ok(Some(VmYield::BudgetExhausted));
            };
            let slot = draw.surface.slot;
            self.panels
                .define_sized(h, self.client, draw.surface, size)?;
            if let Some((level, duration)) = draw.fade {
                self.panel_fade(h, slot, level, duration)?;
            }
            work = Pending::Mark(draw.cg);
        }
        if self.animations.waiting() {
            self.v1_image_billboard = Some(work);
            return Ok(Some(VmYield::WaitUntil(h.now_ms().saturating_add(1))));
        }
        match work {
            Pending::Mark(cg) => self
                .registers
                .change(Change::Experience(Bank::Cg, vec![cg]))?,
            Pending::Remove(slot) => self.panels.remove(h, self.client, slot)?,
            Pending::Loading(_) => unreachable!(),
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Script,
        native_state_tests::LIMITS,
        registers::{Registers, Value},
        scenario_load_tests::{scenario, vm},
        select_items::tests::bitmap,
    };
    use crysirror_core::{service::InputFrame, state::ClientId};
    use std::{
        fs,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    fn card(slot: u8, cg: u32, key: i32, effect: u32) -> String {
        format!(
            "0,{slot},{cg},0,0,192,800,130,400,{effect},16,0,{key},0,0,0,0,-1,0,0,*,-1,0,0,0,0,Font"
        )
    }

    #[test]
    fn source_alpha_supersedes_an_index_key_and_full_size_comes_from_the_bitmap() -> Result<()> {
        let keyed = Draw::parse(card(4, 1226, 0, 255).as_bytes(), true)?;
        let unkeyed = Draw::parse(card(4, 1226, -2, 519).as_bytes(), true)?;
        let alpha = bitmap(&[0, 1, 2, 3], &[255, 0, 32, 128]);
        let solid = bitmap(&[0, 1, 2, 3], &[]);
        let header = |b: &[u8]| <[u8; 36]>::try_from(&b[..36]).unwrap();
        assert_eq!(keyed.size_from_header(&header(&alpha))?, [4, 1]);
        assert!(keyed.size_from_header(&header(&solid)).is_err());
        assert_eq!(unkeyed.size_from_header(&header(&solid))?, [4, 1]);
        assert_eq!((keyed.surface.level, keyed.fade), (16, None));
        assert_eq!((unkeyed.surface.level, unkeyed.fade), (0, None));
        Ok(())
    }

    #[test]
    fn numerical_cards_wait_for_fade_before_counting_experience_and_remove_only_their_own_layer()
    -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "exhibit-numbered-cards-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir(&dir)?;
        let result = (|| -> Result<()> {
            let folder = dir.join("assets/res/g/ev/12");
            fs::create_dir_all(&folder)?;
            for id in [1226, 1227] {
                fs::write(
                    folder.join(format!("{id}.gyu")),
                    bitmap(&[0, 1, 2, 3], &[255, 0, 32, 128]),
                )?;
            }
            let project = dir.join("project.json");
            fs::write(&project, br#"{"schema":2,"name":"cards","content_revision":"1","canvas":[800,600],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
            let mut runtime = crysirror_runtime::Runtime::load(&project, &dir.join("state"))?;
            let input = InputFrame::default();
            let mut vm = vm(1)?;
            vm.client = ClientId(1);
            vm.registers = Registers::for_scenario(
                Some(crate::registers::Limits { cg: 1300, ..LIMITS }),
                1,
                [800, 600],
            )?;
            let mut body = Vec::new();
            for (slot, id, hold, key) in [(4u8, 1226, 2000i32, -2), (5, 1227, 5000, 0)] {
                body.extend([0x5e, 0, 0, 0x21]);
                body.extend(card(slot, id, key, 9).bytes());
                body.push(0);
                body.extend([0x0d, 0, 2, 0]);
                body.extend(hold.to_le_bytes());
                body.extend(0i32.to_le_bytes());
                body.extend([0x5f, 0, 2, 0]);
                body.extend(i32::from(slot).to_le_bytes());
                body.extend(800i32.to_le_bytes());
            }
            body.extend([0x18, 0, 0, 0]);
            vm.script = Some(Script::new(scenario(1, &body, 7), 1)?);
            let mut result = vm.tick(&mut runtime.host, input, 16)?;
            for (slot, cg, hold) in [("260", 1226, 2000u64), ("261", 1227, 5000)] {
                let deadline = Instant::now() + Duration::from_secs(5);
                while matches!(result, VmYield::BudgetExhausted) {
                    ensure!(
                        Instant::now() < deadline,
                        "background resource did not become ready"
                    );
                    std::thread::yield_now();
                    result = vm.tick(&mut runtime.host, input, 16)?;
                }
                let started = runtime.now_ms();
                assert_eq!(result, VmYield::WaitUntil(started + 1));
                let visible = |rt: &crysirror_runtime::Runtime| {
                    rt.inspect()["engine"]["layers"]["clients"]["1"][slot].clone()
                };
                let layer = visible(&runtime);
                assert_eq!(layer["opacity"], 0);
                assert_eq!(layer["rect"], serde_json::json!([0., 192., 4., 1.]));
                assert_eq!(layer["paint"]["asset"], format!("res/g/ev/12/{cg}.gyu"));
                assert_eq!(vm.registers.read(Value::Seen(Bank::Cg, cg))?, 0);
                runtime.tick(200, input)?;
                vm.tick(&mut runtime.host, input, 16)?;
                assert_eq!(visible(&runtime)["opacity"], 128);
                runtime.tick(200, input)?;
                assert_eq!(
                    vm.tick(&mut runtime.host, input, 16)?,
                    VmYield::WaitUntil(started + 400 + hold)
                );
                assert_eq!(visible(&runtime)["opacity"], 255);
                assert_eq!(vm.registers.read(Value::Seen(Bank::Cg, cg))?, 1);
                runtime.tick(hold, input)?;
                vm.tick(&mut runtime.host, input, 16)?;
                runtime.tick(400, input)?;
                vm.tick(&mut runtime.host, input, 16)?;
                assert_eq!(visible(&runtime)["opacity"], 128);
                runtime.tick(350, input)?;
                vm.tick(&mut runtime.host, input, 16)?;
                assert_eq!(visible(&runtime)["opacity"], 16);
                runtime.tick(1, input)?;
                result = vm.tick(&mut runtime.host, input, 16)?;
                assert!(visible(&runtime).is_null());
            }
            assert_eq!(result, VmYield::Halted);
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }
}
