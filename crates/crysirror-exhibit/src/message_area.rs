// SPDX-License-Identifier: MPL-2.0
//! Authored v1 message-area catalog and inactive frame selection.
use crate::{
    Script,
    branch::number,
    cstring,
    registers::{Registers, Value},
    word,
};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::HostServices,
    state::ClientId,
};
use serde_json::json;
use std::collections::BTreeMap;

const BACKGROUND_SLOT: i32 = 51;
const BACKGROUND_LAYER: u32 = 256 + BACKGROUND_SLOT as u32;

#[derive(Debug, PartialEq)]
pub(super) struct Font {
    family: Vec<u8>, // Preserve the authored CP932 name.
    words: [i32; 8],
}
#[derive(Debug, PartialEq)]
pub(super) struct Area {
    id: u8,
    paint_mode: i32,
    asset: Option<String>,
    background: [i32; 4],
    text_rect: [i32; 4],
    name_rect: Option<[i32; 4]>,
    font: Font,
    name_font: Font,
}
impl Area {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        // The native legacy command keeps the two font specifications nested.
        // In the admitted authored files each has a family and eight numbers.
        let f: Vec<_> = raw.splitn(38, |&b| b == b',').collect();
        ensure!(f.len() == 37, "unsupported ExHibit message area layout");
        let n = |i: usize| number(f[i]);
        let id = u8::try_from(n(0)?)?;
        let mode = n(1)?;
        ensure!(
            matches!(mode, 1 | 2 | 258)
                && n(2)? == -1
                && n(3)? == 16
                && f[5].is_empty()
                && f[6].is_empty(),
            "unsupported ExHibit message frame mode"
        );
        let (asset, background) = if mode == 1 {
            let path = std::str::from_utf8(f[4])?.replace('\\', "/");
            ensure!(!path.is_empty(), "missing ExHibit message frame image");
            let path = VfsPath::new(Mount::Asset, &path)?.relative().to_owned();
            (Some(path), [n(7)?, n(8)?, n(9)?, n(10)?])
        } else {
            ensure!(f[4].is_empty(), "unexpected ExHibit solid frame image");
            // The null-path setFrame overload uses the text extent for
            // its transparent backing surface, not the legacy bg dimensions.
            (None, [n(7)?, n(8)?, n(13)?, n(14)?])
        };
        let text_rect = [n(11)?, n(12)?, n(13)?, n(14)?];
        let name_rect = [n(15)?, n(16)?, n(17)?, n(18)?];
        let font = |i| -> Result<Font> {
            let family: &[u8] = f[i];
            ensure!(
                !family.is_empty() && family.len() <= 128,
                "invalid ExHibit font name"
            );
            let mut words = [0; 8];
            for (j, value) in words.iter_mut().enumerate() {
                *value = n(i + 1 + j)?;
            }
            Ok(Font {
                family: family.to_vec(),
                words,
            })
        };
        for rect in [background, text_rect, name_rect] {
            ensure!(
                rect.iter().all(|v| (-8192..=8192).contains(v)) && rect[2] >= 0 && rect[3] >= 0,
                "invalid ExHibit message area rectangle"
            );
        }
        Ok(Self {
            id,
            paint_mode: mode,
            asset,
            background,
            text_rect,
            name_rect: (name_rect[2] != 0).then_some(name_rect),
            font: font(19)?,
            name_font: font(28)?,
        })
    }
}

#[derive(Default)]
pub(super) struct Catalog {
    areas: BTreeMap<u8, Area>,
}

impl Catalog {
    #[allow(dead_code)] // awaits the scoped character-say transaction
    pub fn opening_plain_style(&self) -> Result<crate::printer::Style> {
        let area = self
            .areas
            .get(&5)
            .context("undefined ExHibit opening printer")?;
        ensure!(
            area.paint_mode == 2
                && area.asset.is_none()
                && area.name_rect.is_none()
                && area.background == [0, 256, 728, 404]
                && area.text_rect == [32, 32, 728, 404]
                && area.font.words == [0x8600, 400, 30, 26, 128, 0, 0xffffff, 0],
            "unsupported ExHibit opening printer area profile"
        );
        let [x, y, w, h] = area.text_rect;
        Ok(crate::printer::Style {
            rect: [area.background[0] + x, area.background[1] + y, w, h],
            family: crysirror_resource::text_stream::decode_shift_jis(&area.font.family, 128)?,
        })
    }

    pub fn from_v1_definition(bytes: Vec<u8>) -> Result<Self> {
        let script = Script::new(bytes, 1)?;
        let mut at = script.pc;
        let mut definitions = BTreeMap::<u8, &[u8]>::new();
        let mut result = Self::default();
        for _ in 0..script.remaining {
            let header = word(&script.bytes, at)?;
            let arity = ((header >> 16) & 255) as usize;
            let flags = (header >> 24) as u8;
            at += 4 + arity * 4;
            ensure!(
                at <= script.bytes.len(),
                "truncated ExHibit definition operands"
            );
            let first = at;
            for _ in 0..(flags & 15) {
                at = cstring(&script.bytes, at)?.1;
            }
            if header as u16 == 0x49 {
                ensure!(
                    arity == 0 && flags == 1,
                    "unsupported ExHibit message area record"
                );
                let (raw, _) = cstring(&script.bytes, first)?;
                let area = Area::parse(raw)?;
                if let Some(prior) = definitions.insert(area.id, raw) {
                    ensure!(
                        prior == raw,
                        "branch-dependent ExHibit message area {}",
                        area.id
                    );
                }
                result.define(area);
            }
        }
        ensure!(at == script.bytes.len(), "trailing ExHibit definition data");
        ensure!(
            !result.areas.is_empty(),
            "no ExHibit message areas in local definitions"
        );
        Ok(result)
    }
    #[cfg(test)]
    pub fn audit_ids(&self) -> Vec<u8> {
        self.areas.keys().copied().collect()
    }
    pub fn merge_missing(&mut self, other: Self) {
        for (id, area) in other.areas {
            self.areas.entry(id).or_insert(area);
        }
    }
    pub fn has(&self, id: u8) -> bool {
        self.areas.contains_key(&id)
    }
    pub fn define(&mut self, area: Area) {
        self.areas.insert(area.id, area);
    }
    pub fn attach_inactive(
        &self,
        id: u8,
        h: &mut dyn HostServices,
        c: ClientId,
        regs: &mut Registers,
    ) -> Result<()> {
        let area = self
            .areas
            .get(&id)
            .context("undefined ExHibit message area")?;
        let old = regs.read(Value::System(14))?;
        let [x, y, mut w, mut height] = area.background;
        let paint = if let Some(asset) = area.asset.as_deref() {
            let [sw, sh] = h.image_size(c, asset)?;
            w = i32::try_from(sw)?;
            height = i32::try_from(sh)?;
            json!({"kind":"image","asset":asset})
        } else {
            json!({"kind":"solid","color":[0,0,0,0]})
        };
        h.submit_engine(
            c,
            1,
            &serde_json::to_vec(&json!({"layers":[{"op":"put","layer":{
                "id":BACKGROUND_LAYER,"order":[2,BACKGROUND_SLOT],
                "paint":paint,"rect":[x,y,w,height],"opacity":0,"visible":false
            }}]}))?,
        )?;
        regs.set_system(14, (old & !255) | i32::from(id))?;
        regs.set_normal(77, i32::from(id))?;
        Ok(())
    }
}

/// The 0x12 open path only. Printer text and active-area replacement are
/// deliberately separate from the panel's authored fade and visibility.
#[derive(Default)]
pub(super) struct Backdrop {
    pub opened: bool,
    fade: Option<(u64, u64, u8)>,
}

impl Catalog {
    pub fn open(
        &self,
        id: u8,
        state: &mut Backdrop,
        h: &mut dyn HostServices,
        c: ClientId,
        regs: &Registers,
        skip: bool,
    ) -> Result<()> {
        if state.opened {
            return Ok(());
        }
        let area = self
            .areas
            .get(&id)
            .context("undefined ExHibit message area")?;
        let ctrl_base = regs.read(Value::System(15))?;
        let mode = regs.read(Value::System(
            ctrl_base
                .checked_add(2)
                .context("ExHibit control base overflow")?,
        ))?;
        ensure!(
            (0..=2).contains(&mode),
            "unsupported ExHibit message open mode"
        );
        let target = if area.asset.is_some() { 16 } else { 0 };
        let duration = if target != 0 && mode != 0 && !skip {
            let ms = regs.read(Value::System(21))?;
            ensure!(
                (0..=600_000).contains(&ms),
                "invalid ExHibit message fade duration"
            );
            ms as u64
        } else {
            0
        };
        let initial = if duration == 0 { target } else { 0 };
        h.submit_engine(
            c,
            1,
            &serde_json::to_vec(&json!({"layers":[
                {"op":"set_opacity","id":BACKGROUND_LAYER,"opacity":level_opacity(initial)},
                {"op":"set_visible","id":BACKGROUND_LAYER,"visible":true}
            ]}))?,
        )?;
        state.opened = true;
        if duration > 0 {
            state.fade = Some((h.now_ms(), duration, 0));
        }
        Ok(())
    }
}

fn level_opacity(level: u8) -> u8 {
    level.saturating_mul(16)
}
impl Backdrop {
    pub fn waiting(&self) -> bool {
        self.fade.is_some()
    }
    pub fn present(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<bool> {
        let Some((start, duration, previous)) = self.fade else {
            return Ok(true);
        };
        let elapsed = h.now_ms().saturating_sub(start);
        let level = (16 * elapsed.min(duration) / duration) as u8;
        if level != previous {
            h.submit_engine(
                c,
                1,
                &serde_json::to_vec(&json!({"layers":[
                    {"op":"set_opacity","id":BACKGROUND_LAYER,"opacity":level_opacity(level)}
                ]}))?,
            )?;
        }
        self.fade = (level < 16).then_some((start, duration, level));
        Ok(self.fade.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crysirror_core::{Error, Result as CoreResult, service::FileSystem, state::Command};

    #[derive(Default)]
    struct Host {
        clock: u64,
        layers: Vec<serde_json::Value>,
    }
    impl FileSystem for Host {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> CoreResult<Vec<u8>> {
            Err(Error::NotFound)
        }
        fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
            Err(Error::ReadOnly)
        }
    }
    impl HostServices for Host {
        fn submit(&mut self, _: ClientId, _: &[Command]) -> CoreResult<()> {
            Ok(())
        }
        fn now_ms(&self) -> u64 {
            self.clock
        }
        fn image_size(&mut self, _: ClientId, _: &str) -> CoreResult<[u32; 2]> {
            Ok([800, 149])
        }
        fn submit_engine(&mut self, _: ClientId, _: u32, payload: &[u8]) -> CoreResult<()> {
            self.layers
                .push(serde_json::from_slice(payload).map_err(|e| Error::Io(e.to_string()))?);
            Ok(())
        }
    }

    fn def(x: i32) -> String {
        format!(
            "0,1,-1,16,ui\\frame.gyu,,,{x},440,800,151,32,32,728,100,16,6,172,20,JP,32768,400,28,24,128,0,0,16777215,JP,33537,400,20,18,128,0,0,16777215"
        )
    }
    fn fixture(items: &[String]) -> Vec<u8> {
        let mut b = b"\0DLR".to_vec();
        for n in [1, 16, items.len() as u32] {
            b.extend(n.to_le_bytes());
        }
        for text in items {
            b.extend([0x49, 0, 0, 1]);
            b.extend(text.as_bytes());
            b.push(0);
        }
        b
    }

    #[test]
    fn opening_printer_style_is_projected_from_exact_authored_fields() -> Result<()> {
        let record = "5,2,-1,16,,,,0,256,800,420,32,32,728,404,0,0,0,0,JP,34304,400,30,26,128,0,16777215,0,JP,34561,700,20,18,128,0,16777215,0";
        let mut catalog = Catalog::default();
        catalog.define(Area::parse(record.as_bytes())?);
        let style = catalog.opening_plain_style()?;
        assert_eq!(style.family, "JP");
        assert_eq!(style.rect, [32, 288, 728, 404]);
        catalog.define(Area::parse(
            record.replace(",30,26,", ",30,25,").as_bytes(),
        )?);
        assert!(catalog.opening_plain_style().is_err());
        Ok(())
    }

    #[test]
    fn inactive_binding_and_open_commit_scoped_layers_and_native_registers() -> Result<()> {
        let catalog = Catalog::from_v1_definition(fixture(&[def(0)]))?;
        let mut regs = crate::native_state_tests::fresh(1)?;
        regs.set_system(14, 0xa5ff)?;
        let (mut host, client) = (Host::default(), ClientId(1));
        catalog.attach_inactive(0, &mut host, client, &mut regs)?;
        assert_eq!(regs.read(Value::System(14))?, 0xa500);
        assert_eq!(regs.read(Value::Local(77))?, 0);
        assert_eq!(host.layers[0]["layers"][0]["layer"]["visible"], false);
        assert_eq!(
            host.layers[0]["layers"][0]["layer"]["rect"],
            json!([0, 440, 800, 149])
        );
        let mut backdrop = Backdrop::default();
        catalog.open(0, &mut backdrop, &mut host, client, &regs, false)?;
        assert!(backdrop.opened && backdrop.waiting());
        assert_eq!(host.layers[1]["layers"][1]["visible"], true);
        host.clock = 75;
        assert!(!backdrop.present(&mut host, client)?);
        assert_eq!(host.layers[2]["layers"][0]["opacity"], 128);
        host.clock = 150;
        assert!(backdrop.present(&mut host, client)?);
        assert_eq!(host.layers[3]["layers"][0]["opacity"], 255);
        Ok(())
    }

    #[test]
    fn authored_geometry_preserves_fonts_and_requires_identical_duplicate_definitions() -> Result<()>
    {
        let catalog = Catalog::from_v1_definition(fixture(&[def(0), def(0)]))?;
        let area = &catalog.areas[&0];
        assert_eq!(area.asset.as_deref(), Some("ui/frame.gyu"));
        assert_eq!(area.background, [0, 440, 800, 151]);
        assert_eq!(area.text_rect, [32, 32, 728, 100]);
        assert_eq!(area.name_rect, Some([16, 6, 172, 20]));
        assert_eq!(area.font.words[2], 28);
        assert_eq!(area.name_font.words[2], 20);
        assert!(Catalog::from_v1_definition(fixture(&[def(0), def(9)])).is_err());
        Ok(())
    }
}
