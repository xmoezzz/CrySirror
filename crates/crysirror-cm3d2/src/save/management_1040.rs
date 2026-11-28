// SPDX-License-Identifier: MPL-2.0
//! Standalone COM3D2 1.0.4 records following CharacterMgr up to bookmark 1.
use super::facility_v5::Facilities;
use crate::binio::{Reader, Writer};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;

const SCRIPT: &str = "CM3D2_SCRIPT";
const KAG: &str = "CM3D2_KAG";
const DESK: &str = "CM3D2_DeskCustomize";
const SCENARIO: &str = "COM3D2_SCENARIO_SELECT";
const SCENARIO_IDS: [&str; 2] = ["CM3D2_SCENARIO_ID_START", "CM3D2_SCENARIO_ID_END"];
const SELECTABLE_IDS: [&str; 2] = ["CM3D2_SELECTABLE_START", "CM3D2_SELECTABLE_END"];
const VERSION: i32 = 1040;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Script<'a> {
    /// KagScript's private serialized checkpoint, framed by BaseKagManager.
    pub checkpoint: &'a [u8],
    pub fade_wait: bool,
    pub enabled: bool,
}

impl<'a> Script<'a> {
    fn read(r: &mut Reader<'a>) -> Result<Self> {
        read_versioned(r, SCRIPT)?;
        read_versioned(r, KAG)?;
        let len = r.count()?;
        Ok(Self {
            checkpoint: r.take(len)?,
            fade_wait: r.bool()?,
            enabled: r.bool()?,
        })
    }

    fn write(&self, w: &mut Writer) -> Result<()> {
        write_versioned(w, SCRIPT);
        write_versioned(w, KAG);
        w.i32(i32::try_from(self.checkpoint.len())?);
        w.bytes.extend_from_slice(self.checkpoint);
        w.bool(self.fade_wait);
        w.bool(self.enabled);
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct DeskItem {
    pub key: i32,
    pub id: i32,
    pub visible: bool,
    pub seasonal: bool,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

impl DeskItem {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let key = r.i32()?;
        let id = r.i32()?;
        let visible = r.bool()?;
        let seasonal = r.bool()?;
        let mut vector = || Ok::<_, anyhow::Error>([r.f32()?, r.f32()?, r.f32()?]);
        Ok(Self {
            key,
            id,
            visible,
            seasonal,
            position: vector()?,
            rotation: vector()?,
            scale: vector()?,
        })
    }

    fn write(&self, w: &mut Writer) {
        w.i32(self.key);
        w.i32(self.id);
        w.bool(self.visible);
        w.bool(self.seasonal);
        for value in self
            .position
            .into_iter()
            .chain(self.rotation)
            .chain(self.scale)
        {
            w.f32(value);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Scenario {
    pub played: Vec<(i32, bool)>,
    pub selectable: Vec<i32>,
}

impl Scenario {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        r.expect_str(SCENARIO)?;
        ensure!(r.f32()? == 1.3, "unsupported 1.0.4 scenario state version");
        let ids = read_ids(r, SCENARIO_IDS)?;
        let selectable = read_ids(r, SELECTABLE_IDS)?;
        let mut seen = BTreeSet::new();
        let played = ids
            .into_iter()
            .map(|id| {
                ensure!(seen.insert(id), "duplicate scenario ID {id}");
                Ok((id, r.bool()?))
            })
            .collect::<Result<_>>()?;
        Ok(Self { played, selectable })
    }

    fn write(&self, w: &mut Writer) {
        w.str(SCENARIO);
        w.f32(1.3);
        write_ids(w, SCENARIO_IDS, self.played.iter().map(|(id, _)| *id));
        write_ids(w, SELECTABLE_IDS, self.selectable.iter().copied());
        for (_, played) in &self.played {
            w.bool(*played);
        }
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct Management<'a> {
    pub script: Script<'a>,
    pub desk: Vec<DeskItem>,
    pub scenario: Scenario,
    pub facilities: Facilities,
}

impl<'a> Management<'a> {
    pub(super) fn read(bytes: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let script = Script::read(&mut r).context("1.0.4 ScriptManager")?;
        read_versioned(&mut r, DESK).context("1.0.4 DeskManager")?;
        let count = r.count()?;
        const DESK_ITEM_BYTES: usize = 2 * 4 + 2 + 9 * 4;
        ensure!(
            count <= (bytes.len() - r.at) / DESK_ITEM_BYTES,
            "1.0.4 desk items exceed the management section"
        );
        let desk = (0..count)
            .map(|_| DeskItem::read(&mut r))
            .collect::<Result<_>>()?;
        let scenario = Scenario::read(&mut r).context("1.0.4 ScenarioSelectMgr")?;
        let facilities = Facilities::read(&mut r)?;
        ensure!(
            r.at == bytes.len(),
            "facility section does not end at bookmark 1"
        );
        Ok(Self {
            script,
            desk,
            scenario,
            facilities,
        })
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let mut w = Writer::default();
        self.script.write(&mut w)?;
        write_versioned(&mut w, DESK);
        w.i32(i32::try_from(self.desk.len())?);
        for item in &self.desk {
            item.write(&mut w);
        }
        self.scenario.write(&mut w);
        w.bytes.extend(self.facilities.encode()?);
        Ok(w.bytes)
    }
}

pub(super) fn read_versioned(r: &mut Reader<'_>, tag: &str) -> Result<()> {
    r.expect_str(tag)?;
    ensure!(r.i32()? == VERSION, "unsupported standalone {tag} version");
    Ok(())
}

pub(super) fn write_versioned(w: &mut Writer, tag: &str) {
    w.str(tag);
    w.i32(VERSION);
}

pub(super) fn read_ids(r: &mut Reader<'_>, [start, end]: [&str; 2]) -> Result<Vec<i32>> {
    r.expect_str(start)?;
    let mut ids = Vec::new();
    loop {
        let text = r.str()?;
        if text == end {
            return Ok(ids);
        }
        ids.push(
            text.parse()
                .with_context(|| format!("invalid ID in {start}"))?,
        );
    }
}

pub(super) fn write_ids(w: &mut Writer, [start, end]: [&str; 2], ids: impl Iterator<Item = i32>) {
    w.str(start);
    for id in ids {
        w.str(&id.to_string());
    }
    w.str(end);
}
