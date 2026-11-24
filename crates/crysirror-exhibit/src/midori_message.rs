// SPDX-License-Identifier: MPL-2.0
//! Midori 2011 path-backed printer area definitions (resident 0x10ea00).
//! The catalog also owns the closed printer's installed, empty glyph planes.
use crate::{
    branch::number,
    registers::{Registers, Value},
};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{HostServices, ImageMetadata, buttons},
    state::ClientId,
};
use crysirror_tasks::{Task, TaskPool};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
struct Font {
    family: Vec<u8>,
    words: [i32; 8],
}

#[derive(Clone, Debug, PartialEq)]
struct Shadow {
    color: u32,
    offset: [i32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct AreaDefinition {
    id: u8,
    asset: VfsPath,
    anchor: [i32; 2],
    text: [i32; 4],
    name: [i32; 4],
    fonts: [Font; 2],
    direction_steps: [u8; 2],
    mode: [i32; 3],
    colors: [[i32; 2]; 8],
    shadows: [Shadow; 2],
}

impl AreaDefinition {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        ensure!(raw.len() <= 4096, "oversized ExHibit 2011 area definition");
        // 42 top-level native fields: the last splits into two consecutive
        // nine-field CP932 fonts. The shadow and decoration lists are ';'
        // subfields, not additional comma arguments.
        let top: Vec<_> = raw.splitn(42, |&b| b == b',').collect();
        ensure!(top.len() == 42, "unsupported ExHibit 2011 area layout");
        let n = |i: usize| number(top[i]);
        let id = u8::try_from(n(0)?)?;
        let flags = n(1)?;
        ensure!(
            id < 3
                && flags == 0x0080_0001
                && n(2)? == -1
                && n(3)? == 16
                && top[5].is_empty()
                && top[6].is_empty(),
            "unsupported ExHibit 2011 area mode"
        );
        let asset = std::str::from_utf8(top[4])?.replace('\\', "/");
        let asset = VfsPath::new(Mount::Asset, &asset)?;
        ensure!(
            asset.relative().starts_with("res/g/") && asset.relative().ends_with(".gyu"),
            "unsupported ExHibit 2011 area asset"
        );
        // The file-backed native setFrame does not use the two nominal
        // extents at 9/10. Its metadata request supplies the actual extent.
        let anchor = [n(7)?, n(8)?];
        let nominal = [n(9)?, n(10)?];
        let text = [n(11)?, n(12)?, n(13)?, n(14)?];
        let name = [n(15)?, n(16)?, n(17)?, n(18)?];
        for rect in [text, name, [anchor[0], anchor[1], nominal[0], nominal[1]]] {
            ensure!(
                rect[..2].iter().all(|v| (-16384..=16384).contains(v))
                    && rect[2..].iter().all(|v| (1..=16384).contains(v)),
                "invalid ExHibit 2011 printer rectangle"
            );
        }
        let mode = [n(19)?, n(20)?, n(21)?];
        ensure!(
            mode == [4, 0, 0]
                && n(34)? == -1
                && n(35)? == 0
                && n(36)? == -1
                && n(37)? == 0
                && n(39)? == -1
                && top[40] == b"0;0;0;0",
            "unsupported ExHibit 2011 printer extensions"
        );
        let mut colors = [[-1_i32; 2]; 8];
        for (i, pair) in colors.iter_mut().enumerate().take(7).skip(1) {
            for (j, color) in pair.iter_mut().enumerate() {
                *color = n(20 + i * 2 + j)?;
                ensure!(
                    (-1..=0xffffff).contains(color),
                    "invalid printer section color"
                );
            }
        }
        let shadow: Vec<_> = top[38].split(|&b| b == b';').collect();
        ensure!(shadow.len() == 6, "invalid ExHibit printer shadows");
        let parse_shadow = |i: usize| -> Result<Shadow> {
            let color = std::str::from_utf8(shadow[i])?;
            let color = if let Some(hex) = color.strip_prefix("0x") {
                u32::from_str_radix(hex, 16)?
            } else {
                color.parse()?
            };
            let offset = [number(shadow[i + 1])?, number(shadow[i + 2])?];
            ensure!(
                offset.iter().all(|v| (-256..=256).contains(v)),
                "invalid printer shadow offset"
            );
            Ok(Shadow { color, offset })
        };
        let words: Vec<_> = top[41].splitn(19, |&b| b == b',').collect();
        ensure!(words.len() == 18, "invalid ExHibit 2011 font pair");
        let font = |i: usize| -> Result<Font> {
            let family = words[i];
            ensure!(
                !family.is_empty() && family.len() <= 128,
                "invalid ExHibit 2011 font family"
            );
            crysirror_resource::text_stream::decode_shift_jis(family, 128)?;
            let mut spec = [0; 8];
            for (j, dst) in spec.iter_mut().enumerate() {
                *dst = number(words[i + 1 + j])?;
            }
            ensure!(
                (1..=256).contains(&spec[2]) && (1..=256).contains(&spec[3]),
                "invalid ExHibit 2011 font extent"
            );
            Ok(Font {
                family: family.to_vec(),
                words: spec,
            })
        };
        Ok(Self {
            id,
            asset,
            anchor,
            text,
            name,
            fonts: [font(0)?, font(9)?],
            direction_steps: [1 << ((flags >> 26) & 3), 1 << ((flags >> 24) & 3)],
            mode,
            colors,
            shadows: [parse_shadow(0)?, parse_shadow(3)?],
        })
    }

    fn measure(self, image: &dyn ImageMetadata) -> Result<Area> {
        let [width, height] = image
            .size(self.asset.relative())
            .with_context(|| format!("resolving ExHibit 2011 printer area {}", self.id))?;
        let [width, height] = [i32::try_from(width)?, i32::try_from(height)?];
        ensure!(
            (1..=16384).contains(&width) && (1..=16384).contains(&height),
            "invalid ExHibit 2011 frame texture extent"
        );
        let [x, y] = self.anchor;
        Ok(Area {
            definition: self,
            backdrop: [x, y, width, height],
        })
    }
}

struct Area {
    definition: AreaDefinition,
    backdrop: [i32; 4],
}

const BACKDROP: u32 = 131;
const BODY: u32 = 132;
const NAME: u32 = 134;
const LAYER_BASE: u32 = 256;

#[derive(Debug, PartialEq)]
struct GlyphPlane {
    rect: [i32; 4],
    palette: [Option<[u8; 3]>; 256],
    // Newly allocated native pixels are uniform index zero; the descriptor
    // keys that index out. The renderer's solid paint is lossless here.
    fill_index: u8,
}

impl GlyphPlane {
    fn cleared(anchor: [i32; 2], [x, y, w, h]: [i32; 4], font: &Font) -> Self {
        let mut palette = std::array::from_fn(|i| Some([i as u8; 3]));
        let rgb = |v: i32| [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        palette[0] = Some([0, 255, 255]);
        palette[1] = Some(rgb(font.words[6]));
        palette[2] = Some(rgb(font.words[7]));
        palette[3] = Some([255; 3]);
        palette[4] = Some([0; 3]);
        // The two remaining optional overrides are not pinned. Neither can
        // be used by the closed, all-index-0 plane. Preserve the distinction
        // for any later glyph writer instead of inserting guessed colours.
        palette[6] = None;
        palette[7] = None;
        Self {
            rect: [anchor[0] + x, anchor[1] + y, w, h],
            palette,
            fill_index: 0,
        }
    }

    fn layer(&self, slot: u32) -> serde_json::Value {
        let [r, g, b] =
            self.palette[usize::from(self.fill_index)].expect("pinned closed glyph key");
        json!({"op":"put","layer":{
            "id":LAYER_BASE + slot, "order":[2,slot], "rect":self.rect,
            "visible":true, "opacity":255, "paint":{"kind":"solid","color":[r,g,b,0]}
        }})
    }
}

struct InstalledFrame {
    definition: AreaDefinition,
    backdrop: [i32; 4],
    body: GlyphPlane,
    name: GlyphPlane,
}

#[derive(Debug, PartialEq)]
pub(super) struct Visibility {
    opening: bool,
    duration: i32,
}

impl Visibility {
    pub fn parse(opening: i32, duration: i32) -> Result<Self> {
        ensure!(
            matches!(opening, 0 | 1) && matches!(duration, -1 | 0 | 400),
            "unsupported ExHibit 2011 printer visibility operands"
        );
        Ok(Self {
            opening: opening != 0,
            duration,
        })
    }
}

struct Fade {
    start: u64,
    duration: u64,
    from: u8,
    target: u8,
    opening: bool,
    next_key_check: u64,
}

impl Fade {
    fn new(start: u64, duration: u64, from: u8, target: u8, opening: bool) -> Self {
        Self {
            start,
            duration,
            from,
            target,
            opening,
            next_key_check: 0,
        }
    }

    fn begin(
        h: &dyn HostServices,
        c: ClientId,
        duration: u64,
        from: u8,
        target: u8,
        opening: bool,
    ) -> Option<Self> {
        if duration == 0 || from == target {
            return None;
        }
        let mut fade = Self::new(h.now_ms(), duration, from, target, opening);
        // waitWithKeyWait tests its first focused Ctrl before sleeping.
        (!fade.should_snap(h, c, 0, false)).then_some(fade)
    }

    fn level(&self, age: u64) -> u8 {
        // Native TmAsyncFade calculates the weighted unsigned level. A
        // descending fade can reach 0 one quantization interval before
        // its nominal duration and then the native worker retires.
        let t = age.min(self.duration);
        ((u64::from(self.from) * (self.duration - t) + u64::from(self.target) * t) / self.duration)
            as u8
    }

    fn should_snap(&mut self, h: &dyn HostServices, c: ClientId, age: u64, skip: bool) -> bool {
        if skip {
            return true;
        }
        if age < self.next_key_check {
            return false;
        }
        self.next_key_check = if age < 300 {
            300
        } else if age < 500 {
            500
        } else {
            500 + ((age - 500) / 200 + 1) * 200
        };
        if h.ui_input_owned(c).unwrap_or(true) {
            return false;
        }
        let Ok(input) = h.input_observation(c) else {
            return false;
        };
        let mut eligible = 0;
        if input.focused == Some(true) {
            eligible |= buttons::CTRL;
            if age >= 500 {
                eligible |= buttons::ENTER | buttons::SPACE;
            }
        }
        if age >= 500 && input.pointer_inside == Some(true) {
            eligible |= buttons::MOUSE_LEFT;
        }
        input.buttons_down & eligible != 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Socket {
    // A frame bind writes the live index even if it sees the same closed
    // area, but does not itself rewrite the serialized placement packet.
    index: Option<i32>,
    packet: Option<&'static str>,
}
impl Socket {
    const RESET_PACKET: &'static str = "0;0x40000000,-1,-1,0,0";

    fn select_closed(&mut self) {
        self.index = Some(0);
    }

    fn leave(&mut self) {
        self.index = Some(0);
        self.packet = Some(Self::RESET_PACKET);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SocketCapture {
    pub index: Option<(u32, i32)>,
    pub packet: Option<(u32, &'static str)>,
}

#[derive(Default)]
pub(super) struct Catalog {
    areas: BTreeMap<u8, Area>,
    task: Option<Task<Area>>,
    selected: Option<InstalledFrame>,
    socket: Socket,
    // For cmdFrameView only: enter is ignored while already positive,
    // and leave forces 0. Other native nesting operations are unadmitted.
    depth: u8,
    level: u8,
    fade: Option<Fade>,
}

impl Catalog {
    pub fn begin(
        &mut self,
        definition: AreaDefinition,
        h: &mut dyn HostServices,
        c: ClientId,
        pool: &TaskPool,
    ) -> Result<()> {
        self.begin_with(definition, h.image_metadata(c)?, pool)
    }

    fn begin_with(
        &mut self,
        definition: AreaDefinition,
        image: std::sync::Arc<dyn ImageMetadata>,
        pool: &TaskPool,
    ) -> Result<()> {
        ensure!(
            self.task.is_none(),
            "overlapping ExHibit 2011 area definition"
        );
        self.task = Some(pool.spawn(move |cancel| {
            cancel.check()?;
            let area = definition.measure(image.as_ref())?;
            cancel.check()?;
            Ok(area)
        })?);
        Ok(())
    }

    pub fn pending(&self) -> bool {
        self.task.is_some()
    }

    pub fn poll(&mut self) -> Result<bool> {
        let Some(job) = &mut self.task else {
            return Ok(true);
        };
        let result = job.try_take();
        match result {
            Ok(None) => Ok(false),
            Ok(Some(area)) => {
                // Native prepareMessageArea reuses an existing ID and
                // populates it with the branch that actually executed.
                self.areas.insert(area.definition.id, area);
                self.task = None;
                Ok(true)
            }
            Err(error) => {
                self.task = None;
                Err(error)
            }
        }
    }

    pub fn geometries(&self) -> Vec<(u8, [i32; 4])> {
        self.areas
            .iter()
            .map(|(&id, area)| (id, area.backdrop))
            .collect()
    }

    pub fn installed(&self) -> Option<u8> {
        self.selected.as_ref().map(|s| s.definition.id)
    }

    pub fn depth(&self) -> u8 {
        self.depth
    }

    pub fn checkpoint_open(&self) -> Option<bool> {
        // Native blockEntry tests RetouchSystem's positive message-open
        // depth, not opacity, the selected area ID or presence of text.
        (self.task.is_none() && self.fade.is_none()).then_some(self.depth > 0)
    }

    pub fn checkpoint_socket(&self) -> Option<SocketCapture> {
        // Neither the constructor's virgin socket nor native prelude are
        // inferred after an unobserved transition. Wait for real owners.
        let index = self.socket.index.map(|v| (78, v));
        let packet = self.socket.packet.map(|v| (6, v));
        (self.checkpoint_open().is_some() && (index.is_some() || packet.is_some()))
            .then_some(SocketCapture { index, packet })
    }

    #[cfg(test)]
    fn select_closed(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        regs: &mut Registers,
    ) -> Result<()> {
        ensure!(
            self.depth == 0 && self.fade.is_none(),
            "active ExHibit 2011 frame replacement is unsupported"
        );
        ensure!(
            !self.select_frame(h, c, regs, false)?,
            "unexpected closed selection fade"
        );
        Ok(())
    }

    pub fn select_frame(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        regs: &mut Registers,
        skip: bool,
    ) -> Result<bool> {
        ensure!(
            self.fade.is_none(),
            "overlapping ExHibit 2011 printer replacement"
        );
        let active = self.depth > 0;
        let requested = regs.read(Value::RelativeSystem(1_000_009))?; // L9
        let packed_style = regs.read(Value::System(14))?;
        let current = (packed_style & 0xff) as u8;
        if active {
            ensure!(
                requested >= -1,
                "unsupported ExHibit 2011 active style alias"
            );
            ensure!(
                self.installed() == Some(current),
                "active frame style does not match its owner"
            );
        }
        let requested_id = if requested < 0 {
            i32::from(current)
        } else {
            requested
        };
        if requested != -1 && requested_id == i32::from(current) {
            // When closed it calls messageFrame and writes its return.
            // The active outer guard instead skips both calls and R77.
            if !active {
                regs.set_local(77, i32::from(current))?;
                self.socket.select_closed();
            }
            return Ok(false);
        }
        let choice = u8::try_from(requested_id)
            .ok()
            .and_then(|id| self.areas.get(&id))
            .or_else(|| self.areas.get(&0))
            .context("undefined ExHibit 2011 printer area and fallback")?;
        let def = &choice.definition;
        let next = InstalledFrame {
            definition: def.clone(),
            backdrop: choice.backdrop,
            body: GlyphPlane::cleared(def.anchor, def.text, &def.fonts[0]),
            name: GlyphPlane::cleared(def.anchor, def.name, &def.fonts[1]),
        };
        let (target, duration) = if active {
            open_plan(
                regs,
                if skip {
                    0
                } else {
                    regs.read(Value::System(21))?
                },
            )?
        } else {
            (win_level(regs)?, 0)
        };
        let fade = Fade::begin(h, c, duration, 0, target, true);
        let level = if fade.is_some() { 0 } else { target };
        let backdrop = json!({"op":"put","layer":{
            "id":LAYER_BASE + BACKDROP, "order":[2,BACKDROP], "rect":next.backdrop,
            "visible":active, "opacity":level.saturating_mul(16),
            "paint":{"kind":"image","asset":def.asset.relative()}
        }});
        h.submit_engine(
            c,
            1,
            &serde_json::to_vec(&json!({
                "layers":[backdrop, next.body.layer(BODY), next.name.layer(NAME)]
            }))?,
        )?;
        regs.set_system(14, (packed_style & !255) | i32::from(def.id))?;
        regs.set_local(77, i32::from(def.id))?;
        self.selected = Some(next);
        if active {
            // The active native replacement first leaves (resetting the
            // socket) then creates the new ordinary no-socket window.
            self.socket.leave();
        } else {
            self.socket.select_closed();
        }
        self.level = level;
        self.fade = fade;
        Ok(self.fade.is_some())
    }

    pub fn change_visibility(
        &mut self,
        cmd: Visibility,
        h: &mut dyn HostServices,
        c: ClientId,
        regs: &Registers,
        skip: bool,
    ) -> Result<bool> {
        ensure!(
            self.fade.is_none(),
            "overlapping ExHibit 2011 printer transition"
        );
        if cmd.opening == (self.depth > 0) {
            return Ok(false);
        }
        let selected = self
            .selected
            .as_ref()
            .context("ExHibit 2011 printer has no installed frame")?;
        ensure!(
            regs.read(Value::System(14))? as u8 == selected.definition.id,
            "ExHibit 2011 printer style no longer matches its installed frame"
        );
        let ms = if skip {
            0
        } else if cmd.duration < 0 {
            regs.read(Value::System(if cmd.opening { 21 } else { 22 }))?
        } else {
            cmd.duration
        };
        if cmd.opening {
            let (target, duration) = open_plan(regs, ms)?;
            let fade = Fade::begin(h, c, duration, 0, target, true);
            let start = if fade.is_some() { 0 } else { target };
            self.apply_level(h, c, start, Some(true))?;
            self.depth = 1;
            self.fade = fade;
        } else if fade_duration(ms)? == 0 {
            self.apply_level(h, c, self.level, Some(false))?;
            self.depth = 0;
            self.socket.leave();
        } else {
            let fade = Fade::begin(h, c, ms as u64, self.level, 0, false);
            if fade.is_none() {
                // The lane was already zero or snapped to its endpoint.
                self.apply_level(h, c, 16, Some(false))?;
                self.depth = 0;
                self.socket.leave();
            }
            self.fade = fade;
        }
        Ok(self.fade.is_some())
    }

    fn apply_level(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        level: u8,
        visible: Option<bool>,
    ) -> Result<()> {
        let id = LAYER_BASE + BACKDROP;
        let mut batch =
            vec![json!({"op":"set_opacity","id":id,"opacity":level.saturating_mul(16)})];
        if let Some(visible) = visible {
            batch.push(json!({"op":"set_visible","id":id,"visible":visible}));
        }
        h.submit_engine(c, 1, &serde_json::to_vec(&json!({"layers":batch}))?)?;
        self.level = level;
        Ok(())
    }

    pub fn present(&mut self, h: &mut dyn HostServices, c: ClientId, skip: bool) -> Result<bool> {
        let Some(fade) = &mut self.fade else {
            return Ok(true);
        };
        let age = h.now_ms().saturating_sub(fade.start);
        let natural = fade.level(age);
        let level = if natural != fade.target && fade.should_snap(h, c, age, skip) {
            fade.target
        } else {
            natural
        };
        let end = level == fade.target;
        let closing = !fade.opening;
        if end && closing {
            // The native leave path keeps the glyph owners (all index 0
            // in this admitted profile), hides the frame, and resets its
            // authored opacity without leaving an on-screen sample.
            self.apply_level(h, c, 16, Some(false))?;
            self.depth = 0;
            self.socket.leave();
        } else if level != self.level {
            self.apply_level(h, c, level, None)?;
        }
        if end {
            self.fade = None;
        }
        Ok(end)
    }
}

fn fade_duration(ms: i32) -> Result<u64> {
    ensure!(
        (0..=600_000).contains(&ms),
        "invalid ExHibit 2011 printer fade duration"
    );
    Ok(ms as u64)
}

fn open_plan(regs: &Registers, ms: i32) -> Result<(u8, u64)> {
    let mode = regs.read(Value::RelativeSystem(1_000_002))?;
    ensure!(
        (0..=2).contains(&mode),
        "unsupported ExHibit 2011 printer opening mode"
    );
    let target = win_level(regs)?;
    let duration = fade_duration(ms)?;
    Ok((
        target,
        if mode != 0 && target != 0 {
            duration
        } else {
            0
        },
    ))
}

fn win_level(regs: &Registers) -> Result<u8> {
    let win = regs.read(Value::System(4))?;
    let win_fade = if win & 0x8000 == 0 { 255 } else { win & 255 };
    Ok((16 * win_fade / 255) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Script, select_items::tests::bitmap};
    use std::{
        fs,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    fn raw(id: u8, asset: &str, family: &str) -> String {
        let mut fields: Vec<String> = [
            "0", "8388609", "-1", "16", "", "", "", "4", "7", "39", "30", "2", "3", "15", "12",
            "6", "8", "9", "10", "4", "0", "0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        fields[0] = id.to_string();
        fields[4] = asset.to_owned();
        fields.extend((0..12).map(|n| {
            if n == 10 {
                "66051".to_owned()
            } else {
                "-1".to_owned()
            }
        }));
        fields.extend(
            [
                "-1",
                "0",
                "-1",
                "0",
                "0x00010203;1;-1;0x00fefdfc;2;3",
                "-1",
                "0;0;0;0",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        fields.extend(
            [family, "32768", "400", "32", "24", "128", "0", "1", "2"]
                .into_iter()
                .map(str::to_owned),
        );
        fields.extend(
            ["Name", "33025", "400", "26", "22", "128", "0", "3", "4"]
                .into_iter()
                .map(str::to_owned),
        );
        fields.join(",")
    }

    fn script_action(text: &str) -> Result<AreaDefinition> {
        let mut bytes = b"\0DLR".to_vec();
        for word in [3u32, 12, 1] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend([0x49, 0, 0, 0x91]);
        bytes.extend(text.as_bytes());
        bytes.push(0);
        let mut script = Script::new(bytes, 3)?;
        let action = script.next()?;
        ensure!(script.remaining == 0, "area did not consume its definition");
        let Action::DefineMidoriArea(def) = action else {
            anyhow::bail!("expected Midori area definition");
        };
        Ok(def)
    }

    #[derive(Default)]
    struct SelectionHost {
        batches: Vec<serde_json::Value>,
        reject: bool,
        now: u64,
        held: u64,
        focus: bool,
        pointer_inside: bool,
        ui_owns: bool,
    }
    impl crysirror_core::service::FileSystem for SelectionHost {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            Err(crysirror_core::Error::NotFound)
        }
        fn write_atomic(
            &mut self,
            _: ClientId,
            _: &VfsPath,
            _: &[u8],
        ) -> crysirror_core::Result<()> {
            Err(crysirror_core::Error::ReadOnly)
        }
    }
    impl HostServices for SelectionHost {
        fn now_ms(&self) -> u64 {
            self.now
        }
        fn ui_input_owned(&self, _: ClientId) -> crysirror_core::Result<bool> {
            Ok(self.ui_owns)
        }
        fn input_observation(
            &self,
            client: ClientId,
        ) -> crysirror_core::Result<crysirror_core::service::InputObservation> {
            Ok(crysirror_core::service::InputObservation {
                client,
                revision: 0,
                time_ms: self.now,
                buttons_down: self.held,
                buttons_pressed: 0,
                pointer: None,
                pointer_inside: Some(self.pointer_inside),
                focused: Some(self.focus),
            })
        }
        fn submit(
            &mut self,
            _: ClientId,
            _: &[crysirror_core::state::Command],
        ) -> crysirror_core::Result<()> {
            Ok(())
        }
        fn submit_engine(
            &mut self,
            c: ClientId,
            schema: u32,
            bytes: &[u8],
        ) -> crysirror_core::Result<()> {
            assert_eq!((c, schema), (ClientId(1), 1));
            if self.reject {
                return Err(crysirror_core::Error::Unsupported("test batch rejection"));
            }
            self.batches
                .push(serde_json::from_slice(bytes).expect("engine batch JSON"));
            Ok(())
        }
    }

    fn selection_fixture() -> Result<Catalog> {
        let mut result = Catalog::default();
        for (id, family) in [(0, "A"), (2, "B")] {
            let mut def = script_action(&raw(id, "res/g/frame.gyu", family))?;
            def.fonts[0].words[6] = 0x123456;
            def.fonts[1].words[6] = 0x876543;
            result.areas.insert(
                id,
                Area {
                    definition: def,
                    backdrop: [4, 7, 17, 19],
                },
            );
        }
        Ok(result)
    }

    #[test]
    fn socket_capture_tracks_closed_rebind_active_noop_and_transactional_leave() -> Result<()> {
        let (mut catalog, mut host, c) =
            (selection_fixture()?, SelectionHost::default(), ClientId(1));
        let mut regs = crate::native_state_tests::fresh(3)?;
        regs.set_normal(78, 106)?;
        let base = regs.read(Value::System(15))?;
        let names = crate::midori_state::EntryNames::default();
        let capture = |catalog: &Catalog, regs: &Registers| -> Result<Option<SocketCapture>> {
            Ok(
                crate::midori_status_capture::StatusCapture::from_owners(regs, catalog, &names)?
                    .printer_socket,
            )
        };
        let expected = |index, packet: Option<&'static str>| {
            Some(SocketCapture {
                index: Some((78, index)),
                packet: packet.map(|p| (6, p)),
            })
        };
        assert_eq!(capture(&catalog, &regs)?, None);
        regs.set_system(base + 9, 2)?;
        catalog.select_closed(&mut host, c, &mut regs)?;
        assert_eq!(capture(&catalog, &regs)?, expected(0, None));
        const PRIOR: &str = "0;1,3,4,5,6";
        // Test a seeded previous packet without admitting socket placements.
        // Native closed select writes only the ID; active same-area does not
        // call messageFrame or leave. Neither is equivalent to resetting T6.
        catalog.socket = Socket {
            index: Some(3),
            packet: Some(PRIOR),
        };
        assert!(!catalog.select_frame(&mut host, c, &mut regs, false)?);
        assert_eq!(capture(&catalog, &regs)?, expected(0, Some(PRIOR)));
        assert!(!catalog.change_visibility(
            Visibility::parse(1, 0)?,
            &mut host,
            c,
            &regs,
            false
        )?);
        catalog.socket.index = Some(3);
        let before = host.batches.len();
        assert!(!catalog.select_frame(&mut host, c, &mut regs, false)?);
        assert_eq!(host.batches.len(), before);
        assert_eq!(capture(&catalog, &regs)?, expected(3, Some(PRIOR)));
        regs.set_system(base + 9, 0)?;
        regs.set_system(21, 50)?;
        host.reject = true;
        assert!(
            catalog
                .select_frame(&mut host, c, &mut regs, false)
                .is_err()
        );
        assert_eq!(capture(&catalog, &regs)?, expected(3, Some(PRIOR)));
        host.reject = false;
        assert!(catalog.select_frame(&mut host, c, &mut regs, false)?);
        assert_eq!(capture(&catalog, &regs)?, None);
        host.now = 50;
        assert!(catalog.present(&mut host, c, false)?);
        assert_eq!(
            capture(&catalog, &regs)?,
            expected(0, Some(Socket::RESET_PACKET))
        );

        catalog.socket = Socket {
            index: Some(3),
            packet: Some(PRIOR),
        };
        regs.set_system(22, 100)?;
        assert!(catalog.change_visibility(
            Visibility::parse(0, -1)?,
            &mut host,
            c,
            &regs,
            false
        )?);
        host.now = 150;
        host.reject = true;
        assert!(catalog.present(&mut host, c, false).is_err());
        assert_eq!(catalog.socket.packet, Some(PRIOR));
        assert_eq!(capture(&catalog, &regs)?, None);
        host.reject = false;
        assert!(catalog.present(&mut host, c, false)?);
        assert_eq!(
            capture(&catalog, &regs)?,
            expected(0, Some(Socket::RESET_PACKET))
        );
        assert_eq!(regs.text(6)?, b"");
        assert_eq!(regs.read(Value::Local(78))?, 106);
        Ok(())
    }

    #[test]
    fn midori_printer_fade_preserves_depth_and_hides_at_native_integer_endpoint() -> Result<()> {
        let (mut catalog, mut h, c) = (selection_fixture()?, SelectionHost::default(), ClientId(1));
        let mut regs = crate::native_state_tests::fresh(3)?;
        let base = regs.read(Value::System(15))?;
        regs.set_system(base + 9, 2)?;
        regs.set_system(4, 0x8080)?;
        catalog.select_closed(&mut h, c, &mut regs)?;
        let names = crate::midori_state::EntryNames::default();
        let state = |catalog: &Catalog, regs: &Registers| -> Result<Option<i32>> {
            Ok(
                crate::midori_status_capture::StatusCapture::from_owners(regs, catalog, &names)?
                    .printer_open
                    .map(|write| write.bits),
            )
        };
        assert_eq!(state(&catalog, &regs)?, Some(0));
        assert!(catalog.change_visibility(Visibility::parse(1, -1)?, &mut h, c, &regs, false)?);
        assert_eq!(catalog.depth(), 1);
        assert_eq!(state(&catalog, &regs)?, None);
        h.now = 100;
        assert!(!catalog.present(&mut h, c, false)?);
        assert_eq!(catalog.level, 4);
        h.now = 200;
        assert!(catalog.present(&mut h, c, false)?);
        assert_eq!(catalog.level, 8);
        // Native captures the open scope, even at half bitmap opacity.
        assert_eq!(state(&catalog, &regs)?, Some(1));
        assert!(!catalog.change_visibility(Visibility::parse(1, 400)?, &mut h, c, &regs, false)?);
        assert_eq!(catalog.depth(), 1); // native cmdFrameView suppresses nested enter
        assert!(catalog.change_visibility(Visibility::parse(0, -1)?, &mut h, c, &regs, false)?);
        h.now = 375;
        assert!(!catalog.present(&mut h, c, false)?);
        assert_eq!(catalog.level, 1);
        h.now = 376;
        h.reject = true;
        assert!(catalog.present(&mut h, c, false).is_err());
        assert_eq!(catalog.depth(), 1);
        assert_eq!(state(&catalog, &regs)?, None);
        h.reject = false;
        assert!(catalog.present(&mut h, c, false)?);
        assert_eq!((catalog.depth(), catalog.level), (0, 16));
        assert_eq!(state(&catalog, &regs)?, Some(0));
        let batch = &h.batches.last().unwrap()["layers"];
        assert_eq!(batch[1]["visible"], false);
        assert_eq!(batch[1]["id"], 387);
        assert_eq!(catalog.installed(), Some(2));
        Ok(())
    }

    #[test]
    fn midori_active_replacement_reopens_and_respects_native_key_check_window() -> Result<()> {
        let (mut catalog, mut h, c) = (selection_fixture()?, SelectionHost::default(), ClientId(1));
        let mut regs = crate::native_state_tests::fresh(3)?;
        let base = regs.read(Value::System(15))?;
        regs.set_system(base + 9, 2)?;
        catalog.select_closed(&mut h, c, &mut regs)?;
        assert!(!catalog.change_visibility(Visibility::parse(1, 0)?, &mut h, c, &regs, false)?);
        regs.set_local(77, 88)?;
        assert!(!catalog.select_frame(&mut h, c, &mut regs, false)?);
        assert_eq!(regs.read(Value::Local(77))?, 88);
        regs.set_system(base + 9, 0)?;
        regs.set_system(21, 800)?;
        assert!(catalog.select_frame(&mut h, c, &mut regs, false)?);
        assert_eq!((catalog.installed(), catalog.depth()), (Some(0), 1));
        let batch = &h.batches.last().unwrap()["layers"];
        assert_eq!(batch[0]["layer"]["visible"], true);
        assert_eq!(batch[0]["layer"]["opacity"], 0);
        h.pointer_inside = true;
        h.held = buttons::MOUSE_LEFT;
        assert!(!catalog.present(&mut h, c, false)?);
        h.now = 300;
        assert!(!catalog.present(&mut h, c, false)?);
        h.now = 500;
        h.ui_owns = true;
        assert!(!catalog.present(&mut h, c, false)?);
        h.now = 699;
        h.ui_owns = false;
        assert!(!catalog.present(&mut h, c, false)?);
        h.now = 700;
        assert!(catalog.present(&mut h, c, false)?);
        assert_eq!((catalog.depth(), catalog.level), (1, 16));
        h.held = buttons::CTRL;
        h.focus = true;
        assert!(!catalog.change_visibility(Visibility::parse(0, -1)?, &mut h, c, &regs, false)?);
        assert_eq!(catalog.depth(), 0); // focused Ctrl snaps before the first sleep
        Ok(())
    }

    #[test]
    fn closed_selection_keeps_native_local_scope_palette_and_three_distinct_layers() -> Result<()> {
        let mut catalog = selection_fixture()?;
        let mut regs = crate::native_state_tests::fresh(3)?;
        let mut h = SelectionHost::default();
        regs.set_system(15, 151)?;
        regs.set_system(9, 1)?;
        regs.set_system(160, 0)?;
        regs.set_system(14, 0x7123_8400)?;
        regs.set_system(4, 0x8080)?;
        catalog.select_closed(&mut h, ClientId(1), &mut regs)?;
        assert!(h.batches.is_empty()); // equal-area native fast path
        assert_eq!(catalog.installed(), None);
        assert_eq!(regs.read(Value::Local(77))?, 0);

        regs.set_system(160, 2)?;
        catalog.select_closed(&mut h, ClientId(1), &mut regs)?;
        let layers = h.batches[0]["layers"]
            .as_array()
            .context("expected layer transaction")?;
        let s = catalog
            .selected
            .as_ref()
            .context("expected retained frame")?;
        assert_eq!(s.definition.fonts[0].family, b"B");
        assert_eq!(s.body.palette[1], Some([0x12, 0x34, 0x56]));
        assert_eq!(s.name.palette[1], Some([0x87, 0x65, 0x43]));
        assert_eq!(s.name.palette[6], None);
        assert_eq!(s.body.fill_index, 0);
        assert_eq!(layers[0]["layer"]["id"], 387);
        assert_eq!(layers[0]["layer"]["rect"], json!([4, 7, 17, 19]));
        assert_eq!(layers[0]["layer"]["opacity"], 128);
        assert_eq!(layers[0]["layer"]["visible"], false);
        assert_eq!(layers[1]["layer"]["id"], 388);
        assert_eq!(layers[2]["layer"]["id"], 390);
        assert_eq!(layers[1]["layer"]["rect"], json!([6, 10, 15, 12]));
        assert_eq!(layers[2]["layer"]["rect"], json!([10, 15, 9, 10]));
        assert_eq!(layers[1]["layer"]["visible"], true);
        assert_eq!(
            layers[1]["layer"]["paint"]["color"],
            json!([0, 255, 255, 0])
        );
        assert_eq!(regs.read(Value::System(14))?, 0x7123_8402);
        regs.set_normal(77, 99)?;
        assert_eq!(regs.read(Value::Local(77))?, 2); // native LocalHolder wins
        Ok(())
    }

    #[test]
    fn closed_selection_forced_refresh_fallback_and_rejected_transaction_are_distinct() -> Result<()>
    {
        let mut catalog = selection_fixture()?;
        let mut regs = crate::native_state_tests::fresh(3)?;
        let mut h = SelectionHost::default();
        let rel = regs.read(Value::System(15))? + 9;
        regs.set_system(rel, 2)?;
        catalog.select_closed(&mut h, ClientId(1), &mut regs)?;
        catalog.areas.get_mut(&2).unwrap().definition.fonts[0].family =
            b"Retained alternate".to_vec();
        catalog.select_closed(&mut h, ClientId(1), &mut regs)?;
        assert_eq!(h.batches.len(), 1);
        assert_eq!(
            catalog.selected.as_ref().unwrap().definition.fonts[0].family,
            b"B"
        );
        regs.set_system(rel, -1)?;
        catalog.select_closed(&mut h, ClientId(1), &mut regs)?;
        assert_eq!(h.batches.len(), 2);
        assert_eq!(
            catalog.selected.as_ref().unwrap().definition.fonts[0].family,
            b"Retained alternate"
        );

        regs.set_system(rel, 99)?;
        regs.set_local(77, -77)?;
        h.reject = true;
        assert!(
            catalog
                .select_closed(&mut h, ClientId(1), &mut regs)
                .is_err()
        );
        assert_eq!(regs.read(Value::System(14))?, 2);
        assert_eq!(regs.read(Value::Local(77))?, -77);
        assert_eq!(catalog.installed(), Some(2));
        h.reject = false;
        catalog.depth = 1;
        assert!(
            catalog
                .select_closed(&mut h, ClientId(1), &mut regs)
                .is_err()
        );
        assert_eq!(h.batches.len(), 2);
        catalog.depth = 0;
        catalog.select_closed(&mut h, ClientId(1), &mut regs)?;
        assert_eq!(catalog.installed(), Some(0));
        assert_eq!(regs.read(Value::Local(77))?, 0);
        assert_eq!(h.batches.len(), 3);
        Ok(())
    }

    #[test]
    fn midori_area_retains_native_nested_font_and_direction_fields() -> Result<()> {
        let def = script_action(&raw(2, "res\\g\\frame.gyu", "Body"))?;
        assert_eq!(def.asset.relative(), "res/g/frame.gyu");
        assert_eq!(def.text, [2, 3, 15, 12]);
        assert_eq!(def.name, [6, 8, 9, 10]);
        assert_eq!(def.fonts[0].family, b"Body");
        assert_eq!(&def.fonts[1].words[2..4], &[26, 22]);
        assert_eq!(def.colors[6], [66051, -1]);
        assert_eq!(def.colors[7], [-1, -1]);
        assert_eq!(
            def.shadows[0],
            Shadow {
                color: 0x10203,
                offset: [1, -1]
            }
        );
        assert_eq!(def.direction_steps, [1, 1]);
        assert_eq!(def.mode, [4, 0, 0]);
        assert!(script_action(&raw(2, "../frame.gyu", "Body")).is_err());
        assert!(script_action(&raw(3, "res/g/frame.gyu", "Body")).is_err());
        assert!(
            script_action(&raw(2, "res/g/frame.gyu", "Body").replace(";0;0;0,", ";1;0;0,"))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn midori_area_uses_decoded_size_and_only_commits_successful_branch() -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "crysirror-midori-area-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir_all(dir.join("assets/res/g"))?;
        let result = (|| -> Result<()> {
            fs::write(
                dir.join("assets/res/g/frame.gyu"),
                bitmap(&[1, 1, 1], &[48, 58, 68]),
            )?;
            let project = dir.join("project.json");
            fs::write(&project, br#"{"schema":2,"name":"message-area","content_revision":"1","canvas":[20,8],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
            let mut rt = crysirror_runtime::Runtime::load(&project, &dir.join("state"))?;
            let worker = TaskPool::new("midori-area-test", 1, 1)?;
            let mut catalog = Catalog::default();
            let wait = |catalog: &mut Catalog| -> Result<()> {
                let until = Instant::now() + Duration::from_secs(5);
                while !catalog.poll()? {
                    ensure!(
                        Instant::now() < until,
                        "Midori image metadata test timed out"
                    );
                    std::thread::yield_now();
                }
                Ok(())
            };
            for family in ["Initial", "Alternate"] {
                catalog.begin(
                    script_action(&raw(1, "res/g/frame.gyu", family))?,
                    &mut rt.host,
                    ClientId(1),
                    &worker,
                )?;
                assert!(catalog.pending());
                wait(&mut catalog)?;
                assert_eq!(catalog.geometries(), [(1, [4, 7, 3, 1])]);
                assert_eq!(
                    catalog.areas[&1].definition.fonts[0].family,
                    family.as_bytes()
                );
            }
            catalog.begin(
                script_action(&raw(1, "res/g/missing.gyu", "WouldOverwrite"))?,
                &mut rt.host,
                ClientId(1),
                &worker,
            )?;
            assert!(wait(&mut catalog).is_err());
            assert!(!catalog.pending());
            assert_eq!(catalog.areas[&1].definition.fonts[0].family, b"Alternate");
            assert_eq!(catalog.geometries(), [(1, [4, 7, 3, 1])]);
            let mut regs = crate::native_state_tests::fresh(3)?;
            let local = regs.read(Value::System(15))? + 9;
            regs.set_system(local, 1)?;
            catalog.select_closed(&mut rt.host, ClientId(1), &mut regs)?;
            for (slot, rect, visible) in [
                (BACKDROP, [4., 7., 3., 1.], false),
                (BODY, [6., 10., 15., 12.], true),
                (NAME, [10., 15., 9., 10.], true),
            ] {
                let layer = rt
                    .host
                    .engine
                    .layers
                    .get(ClientId(1), LAYER_BASE + slot)
                    .context("missing shared retained Midori layer")?;
                assert_eq!((layer.rect, layer.visible), (rect, visible));
            }
            assert!(
                rt.host
                    .engine
                    .layers
                    .get(ClientId(1), LAYER_BASE + 51)
                    .is_none()
            );
            assert_eq!(catalog.installed(), Some(1));
            assert!(!catalog.change_visibility(
                Visibility::parse(1, 0)?,
                &mut rt.host,
                ClientId(1),
                &regs,
                false,
            )?);
            assert!(
                rt.host
                    .engine
                    .layers
                    .get(ClientId(1), LAYER_BASE + BACKDROP)
                    .context("visible area backdrop")?
                    .visible
            );
            assert!(!catalog.change_visibility(
                Visibility::parse(0, 0)?,
                &mut rt.host,
                ClientId(1),
                &regs,
                false,
            )?);
            assert!(
                !rt.host
                    .engine
                    .layers
                    .get(ClientId(1), LAYER_BASE + BACKDROP)
                    .context("retained hidden area backdrop")?
                    .visible
            );
            for slot in [BODY, NAME] {
                assert!(
                    rt.host
                        .engine
                        .layers
                        .get(ClientId(1), LAYER_BASE + slot)
                        .is_some()
                );
            }
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }
}
