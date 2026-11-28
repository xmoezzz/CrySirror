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

#[derive(Debug, PartialEq, serde::Serialize)]
pub(super) struct Font {
    pub(super) family: Vec<u8>, // Preserve the authored CP932 name.
    pub(super) words: [i32; 8],
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
    fn page(&self) -> crate::v1_printer_page::Page {
        let position =
            |[x, y, w, h]: [i32; 4]| [self.background[0] + x, self.background[1] + y, w, h];
        crate::v1_printer_page::Page {
            body: position(self.text_rect),
            name: self.name_rect.map(position),
        }
    }
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
    pub fn plain_style(&self, id: u8) -> Result<crate::printer::PageStyle> {
        use crate::printer::{PageStyle, Plane, Style};
        let area = self.areas.get(&id).context("undefined ExHibit printer")?;
        let page = area.page();
        let style = |font: &Font, rect, plane| -> Result<Style> {
            let [
                flags,
                weight,
                line_px,
                font_px,
                charset,
                quality,
                color,
                outline,
            ] = font.words;
            let expected = match plane {
                Plane::Inline => [0x8600, 0xffffff, 0],
                Plane::FramedBody => [0x8000, 0, 0xffffff],
                Plane::FramedName => [0x8301, 0, 0xffffff],
            };
            ensure!(
                [weight, charset, quality] == [400, 128, 0]
                    && [flags, color, outline] == expected
                    && (1..=128).contains(&line_px)
                    && (1..=128).contains(&font_px),
                "unsupported ExHibit plain printer profile"
            );
            Ok(Style {
                rect,
                family: crysirror_resource::text_stream::decode_shift_jis(&font.family, 128)?,
                font_px,
                line_px,
                plane,
            })
        };
        if let Some(name) = page.name {
            ensure!(
                area.paint_mode == 1 && area.asset.is_some(),
                "unsupported ExHibit named printer frame"
            );
            Ok(PageStyle {
                body: style(&area.font, page.body, Plane::FramedBody)?,
                name: Some(style(&area.name_font, name, Plane::FramedName)?),
            })
        } else {
            ensure!(
                area.paint_mode == 2 && area.asset.is_none(),
                "unsupported ExHibit inline printer frame"
            );
            Ok(PageStyle {
                body: style(&area.font, page.body, Plane::Inline)?,
                name: None,
            })
        }
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
    #[cfg(test)]
    fn page(&self, id: u8) -> Result<crate::v1_printer_page::Page> {
        Ok(self
            .areas
            .get(&id)
            .context("undefined ExHibit message page")?
            .page())
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
    ) -> Result<crate::v1_printer_page::Page> {
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
        let mut layers = vec![json!({"op":"put","layer":{
            "id":BACKGROUND_LAYER,"order":[2,BACKGROUND_SLOT],
            "paint":paint,"rect":[x,y,w,height],"opacity":0,"visible":false
        }})];
        layers.extend(area.page().cleared(true));
        h.submit_engine(c, 1, &serde_json::to_vec(&json!({"layers": layers}))?)?;
        regs.set_system(14, (old & !255) | i32::from(id))?;
        regs.set_local(77, i32::from(id))?;
        Ok(area.page())
    }
}

/// Visibility and native 16-step fade of the message frame on slot 51.
#[derive(Default)]
pub(super) struct Backdrop {
    pub opened: bool,
    pub page: Option<crate::v1_printer_page::Page>,
    fade: Option<BackdropFade>,
}

struct BackdropFade {
    start: u64,
    duration: u32,
    previous: u8,
    from: u8,
    to: u8,
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
            state.fade = Some(BackdropFade {
                start: h.now_ms(),
                duration: duration as u32,
                previous: 0,
                from: 0,
                to: target,
            });
        }
        Ok(())
    }
}

impl Catalog {
    /// Forced close in liteExec. Empty both printers before fading the backdrop.
    pub fn close(
        &self,
        id: u8,
        state: &mut Backdrop,
        h: &mut dyn HostServices,
        c: ClientId,
        duration: u32,
    ) -> Result<()> {
        let area = self
            .areas
            .get(&id)
            .context("undefined ExHibit message area")?;
        state
            .page
            .context("no attached ExHibit message page")?
            .clear(h, c, true)?;
        let level = if area.asset.is_some() { 16 } else { 0 };
        if duration == 0 || level == 0 {
            h.submit_engine(
                c,
                1,
                &serde_json::to_vec(&json!({"layers":[
                    {"op":"set_visible","id":BACKGROUND_LAYER,"visible":false}
                ]}))?,
            )?;
            state.opened = false;
        } else {
            state.fade = Some(BackdropFade {
                start: h.now_ms(),
                duration,
                previous: level,
                from: level,
                to: 0,
            });
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
        let Some(fade) = &mut self.fade else {
            return Ok(true);
        };
        let age = u32::try_from(h.now_ms().saturating_sub(fade.start)).unwrap_or(u32::MAX);
        let level = crate::panel_animation::fade_level(
            i32::from(fade.from),
            i32::from(fade.to),
            fade.duration,
            age,
        ) as u8;
        if level == 0 && fade.to == 0 {
            h.submit_engine(
                c,
                1,
                &serde_json::to_vec(&json!({"layers":[
                    {"op":"set_visible","id":BACKGROUND_LAYER,"visible":false},
                    {"op":"set_opacity","id":BACKGROUND_LAYER,"opacity":level_opacity(fade.from)}
                ]}))?,
            )?;
            self.opened = false;
        } else if level != fade.previous {
            h.submit_engine(
                c,
                1,
                &serde_json::to_vec(&json!({"layers":[
                    {"op":"set_opacity","id":BACKGROUND_LAYER,"opacity":level_opacity(level)}
                ]}))?,
            )?;
        }
        fade.previous = level;
        if level == fade.to {
            self.fade = None;
        }
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
    fn plain_printer_style_is_projected_from_selected_authored_fields() -> Result<()> {
        let record = "5,2,-1,16,,,,0,256,800,420,32,32,728,404,0,0,0,0,JP,34304,400,30,26,128,0,16777215,0,JP,34561,700,20,18,128,0,16777215,0";
        let mut catalog = Catalog::default();
        catalog.define(Area::parse(record.as_bytes())?);
        let style = catalog.plain_style(5)?.body;
        assert_eq!(style.family, "JP");
        assert_eq!(style.rect, [32, 288, 728, 404]);
        assert_eq!((style.font_px, style.line_px), (26, 30));
        catalog.define(Area::parse(
            record
                .replacen('5', "6", 1)
                .replace(",30,26,", ",30,75,")
                .as_bytes(),
        )?);
        let large = catalog.plain_style(6)?.body;
        assert_eq!((large.font_px, large.line_px), (75, 30));
        assert_eq!(large.rect, style.rect);
        assert!(catalog.plain_style(0).is_err());
        catalog.define(Area::parse(
            b"0,1,-1,16,frame.gyu,,,0,440,800,151,32,32,728,100,16,6,172,20,JP,32768,400,28,24,128,0,0,16777215,JP,33537,400,20,18,128,0,0,16777215",
        )?);
        let framed = catalog.plain_style(0)?;
        let name = framed.name.context("missing separate name pane")?;
        assert_eq!(
            (framed.body.rect, framed.body.font_px),
            ([32, 472, 728, 100], 24)
        );
        assert_eq!((name.rect, name.font_px), ([16, 446, 172, 20], 18));
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
    fn v1_page_clear_preserves_name_when_immediate_and_fades_both_when_timed() -> Result<()> {
        let catalog = Catalog::from_v1_definition(fixture(&[def(0)]))?;
        let page = catalog.page(0)?;
        let (mut host, client) = (Host::default(), ClientId(1));
        page.clear(&mut host, client, false)?;
        let ops = host.layers[0]["layers"].as_array().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0]["layer"]["id"], 308);
        assert_eq!(ops[0]["layer"]["rect"], json!([32, 472, 728, 100]));
        page.clear(&mut host, client, true)?;
        assert_eq!(host.layers[1]["layers"][1]["layer"]["id"], 310);
        assert_eq!(
            host.layers[1]["layers"][1]["layer"]["rect"],
            json!([16, 446, 172, 20])
        );
        let mut fade = crate::v1_printer_page::ClearFade::new(page, 0, 400);
        host.clock = 200;
        assert!(!fade.present(&mut host, client)?);
        assert_eq!(
            host.layers[2]["layers"],
            json!([
                {"op":"set_opacity","id":308,"opacity":128},{"op":"set_opacity","id":310,"opacity":128}
            ])
        );
        host.clock = 375;
        assert!(!fade.present(&mut host, client)?);
        assert_eq!(host.layers[3]["layers"][0]["opacity"], 16);
        host.clock = 376;
        assert!(fade.present(&mut host, client)?);
        for layer in host.layers[4]["layers"].as_array().unwrap() {
            assert_eq!(layer["layer"]["paint"]["color"], json!([0, 0, 0, 0]));
            assert_eq!(layer["layer"]["opacity"], 255);
        }
        Ok(())
    }

    #[test]
    fn v1_close_clears_print_first_then_hides_and_restores_frame_opacity() -> Result<()> {
        let catalog = Catalog::from_v1_definition(fixture(&[def(0)]))?;
        let (mut host, client) = (Host::default(), ClientId(1));
        let mut backdrop = Backdrop {
            opened: true,
            page: Some(catalog.page(0)?),
            fade: None,
        };
        catalog.close(0, &mut backdrop, &mut host, client, 400)?;
        assert!(backdrop.opened && backdrop.waiting());
        assert_eq!(host.layers[0]["layers"].as_array().unwrap().len(), 2);
        assert_eq!(host.layers[0]["layers"][0]["layer"]["id"], 308);
        assert_eq!(host.layers[0]["layers"][1]["layer"]["id"], 310);
        host.clock = 200;
        assert!(!backdrop.present(&mut host, client)?);
        assert_eq!(
            host.layers[1]["layers"],
            json!([{"op":"set_opacity","id":307,"opacity":128}])
        );
        host.clock = 376;
        assert!(backdrop.present(&mut host, client)?);
        assert!(!backdrop.opened);
        assert_eq!(
            host.layers[2]["layers"],
            json!([
                {"op":"set_visible","id":307,"visible":false},
                {"op":"set_opacity","id":307,"opacity":255}
            ])
        );
        Ok(())
    }

    #[test]
    fn v1_vm_waits_for_a_bound_blank_page_and_preserves_dialogue_history() -> Result<()> {
        use crate::{
            VmYield, printer,
            scenario_load_tests::{scenario, vm},
        };
        use crysirror_core::service::InputFrame;

        let mut ops = vec![0x1e, 0, 2, 0];
        ops.extend(0_i32.to_le_bytes());
        ops.extend(400_i32.to_le_bytes());
        ops.extend([0x18, 0, 0, 0]);
        let script = scenario(1, &ops, 2);
        let mut detached = vm(1)?;
        detached.script = Some(Script::new(script.clone(), 1)?);
        let mut host = Host::default();
        assert_eq!(
            detached.tick(&mut host, InputFrame::default(), 16)?,
            VmYield::Halted
        );
        assert!(host.layers.is_empty());

        let mut bound = vm(1)?;
        bound.script = Some(Script::new(script, 1)?);
        bound.message_areas = Catalog::from_v1_definition(fixture(&[def(0)]))?;
        bound.message_backdrop.page = Some(bound.message_areas.attach_inactive(
            0,
            &mut host,
            bound.client,
            &mut bound.registers,
        )?);
        bound.selected_message_area = Some(0);
        bound.v1_history.push_back(printer::Line {
            character: 0,
            ordinal: 23,
            wait: false,
            say: printer::OrdinarySay::decode(b"", b"Saved line")?,
        });
        assert_eq!(
            bound.tick(&mut host, InputFrame::default(), 16)?,
            VmYield::WaitUntil(1)
        );
        host.clock = 200;
        assert_eq!(
            bound.tick(&mut host, InputFrame::default(), 16)?,
            VmYield::WaitUntil(201)
        );
        host.clock = 376;
        assert_eq!(
            bound.tick(&mut host, InputFrame::default(), 16)?,
            VmYield::Halted
        );
        assert_eq!(bound.v1_history.back().unwrap().ordinal, 23);
        assert!(!bound.message_backdrop.opened);
        Ok(())
    }

    #[test]
    fn v1_active_frame_rebinds_then_reopens_and_same_id_leaves_local_result_untouched() -> Result<()>
    {
        let mut vm = crate::scenario_load_tests::vm(1)?;
        let mut host = Host::default();
        let first = def(0);
        let second = first.replacen("0,1,-1", "1,1,-1", 1);
        vm.message_areas = Catalog::from_v1_definition(fixture(&[first, second]))?;
        vm.registers.set_system(14, 255)?;
        vm.registers.set_normal(77, 88)?;
        vm.bind_v1_message_frame(0, &mut host)?;
        vm.message_areas.open(
            0,
            &mut vm.message_backdrop,
            &mut host,
            vm.client,
            &vm.registers,
            true,
        )?;
        vm.registers.set_local(77, 19)?;
        let before = host.layers.len();
        vm.bind_v1_message_frame(0, &mut host)?;
        assert_eq!(host.layers.len(), before);
        assert_eq!(vm.registers.read(Value::Local(77))?, 19);
        vm.bind_v1_message_frame(1, &mut host)?;
        assert!(vm.message_backdrop.opened && vm.message_backdrop.waiting());
        assert_eq!(vm.registers.read(Value::Local(77))?, 1);
        assert_eq!(vm.registers.read_normal(77)?, 88);
        assert_eq!(vm.registers.read(Value::System(14))? & 255, 1);
        assert_eq!(host.layers[before]["layers"][0]["layer"]["id"], 308);
        assert_eq!(host.layers[before + 1]["layers"][0]["visible"], false);
        assert_eq!(host.layers[before + 3]["layers"][1]["visible"], true);
        host.clock = 150;
        assert!(vm.message_backdrop.present(&mut host, vm.client)?);
        Ok(())
    }

    #[test]
    fn v1_clear_and_close_decode_native_argument_offsets_and_arity() -> Result<()> {
        let mut bytes = b"\0DLR".to_vec();
        for n in [1_u32, 16, 5] {
            bytes.extend(n.to_le_bytes());
        }
        for (op, words) in [
            (0x1e, vec![0, 400]),
            (0x1e, vec![1, 0]),
            (0x12, vec![0, 800]),
            (0x12, vec![0, 500, 44]),
            (0x12, vec![7, -1]),
        ] {
            bytes.extend([op, 0, words.len() as u8, 0]);
            for value in words {
                bytes.extend(i32::to_le_bytes(value));
            }
        }
        let mut s = Script::new(bytes, 1)?;
        for action in [
            crate::Action::V1ClearPage(false, 400),
            crate::Action::V1ClearPage(true, 0),
            crate::Action::CloseV1MessageBackdrop(800),
            crate::Action::CloseV1MessageBackdrop(0),
            crate::Action::OpenMessageBackdrop,
        ] {
            assert_eq!(s.next()?, action);
        }
        assert_eq!(s.pc, s.bytes.len());
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
