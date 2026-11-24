// SPDX-License-Identifier: MPL-2.0
//! Authored RLD v1 local controls; native layer/action identities stay scoped.
use crate::{Script, branch::number, cstring, word};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    image_processing::{
        ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageJob, ImageProcessState,
    },
    path::{Mount, VfsPath},
    region_map::contains_rect,
    service::{HostServices, ImageMetadata, InputFrame, buttons},
    state::ClientId,
};
use crysirror_tasks::{Task, TaskPool};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

const FIRST_SLOT: u8 = 61;
const FADE_MS: u64 = 300;
const COUNT: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Picture {
    asset: VfsPath,
    origin: [i32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Part {
    background: Picture,
    atlas: Picture,
    menu: Option<Picture>,
    header: Option<Picture>,
    options: [Option<Picture>; 6],
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Definition {
    Part(u8, Part),
    Sounds([Option<i32>; COUNT]),
    MenuBackgrounds([Option<Picture>; COUNT]),
}

impl Definition {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let fields: Vec<_> = raw.splitn(34, |&b| b == b',').collect();
        let n = |i| number(fields[i]);
        let picture = |i: usize| -> Result<Option<Picture>> {
            let raw: &[u8] = fields[i];
            if raw == b"-1" {
                return Ok(None);
            }
            let path = std::str::from_utf8(raw)?.replace('\\', "/");
            ensure!(!path.is_empty(), "empty ExHibit toolbar image path");
            let origin = [n(i + 1)?, n(i + 2)?];
            ensure!(
                origin.iter().all(|v| (-32768..=32767).contains(v)),
                "invalid ExHibit toolbar coordinates"
            );
            Ok(Some(Picture {
                asset: VfsPath::new(Mount::Asset, &path)?,
                origin,
            }))
        };
        match (n(0)?, fields.len()) {
            (0, 33) => {
                let part = u8::try_from(n(1)?)?;
                ensure!(
                    usize::from(part) < COUNT && fields[11] == b"-1",
                    "unsupported ExHibit toolbar part or second menu background"
                );
                let background = picture(2)?.context("missing ExHibit toolbar parent")?;
                let atlas = picture(5)?.context("missing ExHibit toolbar control atlas")?;
                ensure!(
                    atlas.origin == [0, 0],
                    "unsupported ExHibit control atlas origin"
                );
                let mut options: [Option<Picture>; 6] = Default::default();
                for (i, item) in options.iter_mut().enumerate() {
                    *item = picture(15 + i * 3)?;
                }
                Ok(Self::Part(
                    part,
                    Part {
                        background,
                        atlas,
                        menu: picture(8)?,
                        header: picture(12)?,
                        options,
                    },
                ))
            }
            (2, 9) => {
                let mut out = [None; COUNT];
                for (i, entry) in out.iter_mut().enumerate() {
                    let id = n(i + 1)?;
                    ensure!(id >= -1, "unsupported ExHibit control sound ID");
                    *entry = (id >= 0).then_some(id);
                }
                Ok(Self::Sounds(out))
            }
            (3, 25) => {
                let mut out: [Option<Picture>; COUNT] = Default::default();
                for (i, entry) in out.iter_mut().enumerate() {
                    *entry = picture(1 + i * 3)?;
                }
                Ok(Self::MenuBackgrounds(out))
            }
            _ => bail!("unsupported ExHibit toolbar definition layout"),
        }
    }
    fn identity(&self) -> u8 {
        match self {
            Self::Part(id, _) => *id,
            Self::Sounds(_) => 8,
            Self::MenuBackgrounds(_) => 9,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Catalog {
    definitions: BTreeMap<u8, Definition>,
}

impl Catalog {
    pub fn from_v1_definition(bytes: Vec<u8>) -> Result<Self> {
        let script = Script::new(bytes, 1)?;
        let mut at = script.pc;
        let mut first = BTreeMap::new();
        let mut result = Self::default();
        for _ in 0..script.remaining {
            let header = word(&script.bytes, at)?;
            let flags = (header >> 24) as u8;
            let arity = ((header >> 16) & 255) as usize;
            at += 4 + 4 * arity;
            ensure!(
                at <= script.bytes.len(),
                "truncated ExHibit toolbar definition"
            );
            let start = at;
            for _ in 0..(flags & 15) {
                at = cstring(&script.bytes, at)?.1;
            }
            if header as u16 == 0x2c {
                ensure!(
                    arity == 0 && flags == 1,
                    "unsupported ExHibit toolbar record"
                );
                let (raw, _) = cstring(&script.bytes, start)?;
                let definition = Definition::parse(raw)?;
                if let Some(previous) = first.insert(definition.identity(), raw) {
                    ensure!(
                        previous == raw,
                        "branch-dependent ExHibit toolbar definition"
                    );
                }
                result.define(definition);
            }
        }
        ensure!(
            at == script.bytes.len(),
            "trailing ExHibit toolbar definition bytes"
        );
        ensure!(result.has_parts(), "missing ExHibit authored toolbar parts");
        Ok(result)
    }
    fn merge_missing(&mut self, other: Self) {
        for (id, d) in other.definitions {
            self.definitions.entry(id).or_insert(d);
        }
    }
    fn define(&mut self, d: Definition) {
        self.definitions.insert(d.identity(), d);
    }
    fn has_parts(&self) -> bool {
        (0..COUNT as u8).all(|i| self.definitions.contains_key(&i))
    }
    fn parts(&self) -> Result<Vec<Part>> {
        (0..COUNT as u8)
            .map(|i| match self.definitions.get(&i) {
                Some(Definition::Part(_, p)) => Ok(p.clone()),
                _ => bail!("missing ExHibit toolbar part {i}"),
            })
            .collect()
    }
    #[cfg(test)]
    pub fn audit_ids(&self) -> Vec<u8> {
        self.definitions.keys().copied().collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolbarAction {
    Help,
    SystemDialog,
    Save,
    Load,
    Configure,
    Auto,
    Skip,
    History,
}
impl ToolbarAction {
    pub fn native_id(self) -> u8 {
        210 + self as u8
    }
    fn from_part(index: usize) -> Self {
        [
            Self::Help,
            Self::SystemDialog,
            Self::Save,
            Self::Load,
            Self::Configure,
            Self::Auto,
            Self::Skip,
            Self::History,
        ][index]
    }
}

struct PreparePart {
    origin: [i32; 2],
    size: [u32; 2],
    base: ImageInput,
    atlas: ImageInput,
    cells: u32,
}

struct Preparation {
    parts: Vec<PreparePart>,
    meta: Arc<dyn ImageMetadata>,
    images: Vec<PartImages>,
    active: Option<ImageJob>,
}

impl Preparation {
    fn advance(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<bool> {
        while self.images.len() < self.parts.len()
            || self
                .images
                .last()
                .zip(self.parts.last())
                .is_some_and(|(i, p)| i.paths.len() < p.cells as usize)
        {
            // Finish all cells of the last part before moving to the next.
            let index = if let Some(last) = self
                .images
                .len()
                .checked_sub(1)
                .filter(|&i| self.images[i].paths.len() < self.parts[i].cells as usize)
            {
                last
            } else {
                let index = self.images.len();
                let part = &self.parts[index];
                self.images.push(PartImages {
                    origin: part.origin,
                    size: part.size,
                    paths: Vec::with_capacity(part.cells as usize),
                });
                index
            };
            let part = &self.parts[index];
            let cell = self.images[index].paths.len() as u32;
            if self.active.is_none() {
                let size = part.size;
                let layer = ImageCompositeLayer {
                    input: Box::new(part.atlas.clone()),
                    source: [(cell * size[0]) as i32, 0, size[0] as i32, size[1] as i32],
                    destination: [0, 0],
                    clip: None,
                    opacity: 255,
                    math: ImageCompositeMath::CopyRgb,
                };
                // One scheduled image per VM, including decoder scratch; don't
                // let a 33-cell toolbar reserve all compressed-decode memory.
                self.active = Some(self.meta.begin_process(
                    part.base.clone().with_composite_layer(layer),
                    Arc::from(&b"[]"[..]),
                )?);
            }
            let image = match self
                .active
                .as_mut()
                .context("missing toolbar job")?
                .poll()?
            {
                ImageProcessState::Pending => return Ok(false),
                ImageProcessState::Ready(resource) => resource,
                ImageProcessState::Failed(error) => bail!("ExHibit toolbar image: {error}"),
            };
            self.active = None;
            let result = h.publish_image_resource(c, 4096 + index as u32 * 6 + cell, image)?;
            self.images[index]
                .paths
                .push(result.path.relative().to_owned());
        }
        Ok(true)
    }
}
struct PartImages {
    origin: [i32; 2],
    size: [u32; 2],
    paths: Vec<String>,
}

fn cell_count(part: usize) -> u32 {
    if part < 5 { 3 } else { 6 }
}
fn atlas_cell(part: usize, on: bool, hovered: bool) -> usize {
    usize::from(part >= 5 && on) * 3 + usize::from(hovered)
}
fn layer_index(part: usize) -> u32 {
    256 + u32::from(FIRST_SLOT) + part as u32
}
fn opacity(level: u8) -> u8 {
    level.saturating_mul(16)
}

#[derive(Default)]
struct Pointer {
    hovered: Option<usize>,
    pressed: Option<usize>,
}
impl Pointer {
    fn step(&mut self, input: InputFrame, hit: Option<usize>) -> (bool, Option<usize>) {
        let was_armed = self.pressed.is_some();
        let down = input.buttons_down & buttons::MOUSE_LEFT != 0;
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            self.pressed = hit;
        } else if self.pressed != hit {
            self.pressed = None;
        }
        self.hovered = hit;
        let action = if down { None } else { self.pressed.take() };
        (hit.is_some() || was_armed, action)
    }
}

struct Fade {
    start: u64,
    showing: bool,
}

pub(super) struct Toolbar {
    catalog: Catalog,
    prepare: Option<Task<(Catalog, Vec<PreparePart>)>>,
    jobs: Option<Preparation>,
    images: Vec<PartImages>,
    fade: Option<Fade>,
    depth: u32,
    level: u8,
    pointer: Pointer,
    bank: [bool; COUNT],
    pending_open: bool,
    render_skip: bool,
}

impl Default for Toolbar {
    fn default() -> Self {
        Self {
            catalog: Catalog::default(),
            prepare: None,
            jobs: None,
            images: vec![],
            fade: None,
            depth: 1,
            level: 0,
            pointer: Pointer::default(),
            bank: [false; COUNT],
            pending_open: false,
            render_skip: false,
        }
    }
}

impl Toolbar {
    pub fn define(&mut self, definition: Definition) -> Result<()> {
        ensure!(
            self.images.is_empty() && self.prepare.is_none() && self.jobs.is_none(),
            "unsupported ExHibit toolbar redefinition after preparation"
        );
        self.catalog.define(definition);
        Ok(())
    }

    pub fn change_scope(
        &mut self,
        open: bool,
        skip: bool,
        bank: [bool; COUNT],
        h: &mut dyn HostServices,
        c: ClientId,
        worker: &TaskPool,
    ) -> Result<()> {
        let was_open = self.depth == 0;
        self.depth = if open {
            self.depth.saturating_sub(1)
        } else {
            self.depth
                .checked_add(1)
                .context("ExHibit toolbar scope overflow")?
        };
        let showing = self.depth == 0;
        if was_open == showing {
            return Ok(());
        }
        self.pointer = Pointer::default();
        self.bank = bank;
        if showing && self.images.is_empty() {
            self.pending_open = true;
            self.render_skip = skip;
            let storage = h.storage(c)?;
            let authored = self.catalog.clone();
            self.prepare = Some(worker.spawn(move |cancel| {
                let mut catalog = authored;
                if !catalog.has_parts() {
                    let reader = storage.open_read(&crate::script_path("def")?)?;
                    let bytes = crysirror_resource::read_all(reader.as_ref(), 8 << 20)?;
                    catalog.merge_missing(Catalog::from_v1_definition(bytes)?);
                }
                let parts = catalog.parts()?;
                let mut out = Vec::with_capacity(COUNT);
                for (index, part) in parts.into_iter().enumerate() {
                    cancel.check()?;
                    let parent_reader = storage.open_read(&part.background.asset)?;
                    let atlas_reader = storage.open_read(&part.atlas.asset)?;
                    let (size, parent_palette) =
                        crate::select_items::palette(parent_reader.as_ref())?;
                    let (atlas_size, atlas_palette) =
                        crate::select_items::palette(atlas_reader.as_ref())?;
                    let cells = cell_count(index);
                    ensure!(
                        atlas_size[0] == size[0] * cells && atlas_size[1] == size[1],
                        "unsupported ExHibit toolbar parent/atlas geometry"
                    );
                    let source = if parent_palette == atlas_palette {
                        ImageInput::ScopedAsset {
                            path: part.atlas.asset,
                            size: atlas_size,
                            fingerprint: None,
                        }
                    } else {
                        let mut bytes =
                            crysirror_resource::read_all(atlas_reader.as_ref(), 64 << 20)?;
                        bytes
                            .get_mut(36..1060)
                            .context("short ExHibit toolbar atlas palette")?
                            .copy_from_slice(&parent_palette);
                        ImageInput::Encoded {
                            name: part.atlas.asset.relative().to_owned(),
                            bytes: bytes.into(),
                            size: atlas_size,
                        }
                    };
                    let base = ImageInput::ScopedAsset {
                        path: part.background.asset,
                        size,
                        fingerprint: None,
                    };
                    out.push(PreparePart {
                        origin: part.background.origin,
                        size,
                        base,
                        atlas: source,
                        cells,
                    });
                }
                Ok((catalog, out))
            })?);
        } else {
            self.transition(showing, skip, h, c)?;
        }
        Ok(())
    }

    fn put(&self, index: usize) -> Value {
        let part = &self.images[index];
        let cell = atlas_cell(index, self.bank[index], self.pointer.hovered == Some(index));
        json!({"op":"put","layer":{"id":layer_index(index),"order":[2,u32::from(FIRST_SLOT)+index as u32],
            "paint":{"kind":"image","asset":part.paths[cell]},
            "rect":[part.origin[0],part.origin[1],part.size[0],part.size[1]],
            "opacity":opacity(self.level)}})
    }

    fn submit(h: &mut dyn HostServices, c: ClientId, layers: Vec<Value>) -> Result<()> {
        if !layers.is_empty() {
            h.submit_engine(c, 1, &serde_json::to_vec(&json!({"layers":layers}))?)?;
        }
        Ok(())
    }

    fn transition(
        &mut self,
        showing: bool,
        skip: bool,
        h: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        self.level = if showing == skip { 16 } else { 0 };
        if showing {
            Self::submit(h, c, (0..self.images.len()).map(|i| self.put(i)).collect())?;
        } else if skip {
            Self::submit(
                h,
                c,
                (0..self.images.len())
                    .map(|i| json!({"op":"remove","id":layer_index(i)}))
                    .collect(),
            )?;
        } else {
            Self::submit(h, c, (0..self.images.len()).map(|i| self.put(i)).collect())?;
        }
        self.fade = (!skip).then_some(Fade {
            start: h.now_ms(),
            showing,
        });
        Ok(())
    }

    pub fn busy(&self) -> bool {
        self.pending_open || self.fade.is_some()
    }
    pub fn open(&self) -> bool {
        self.depth == 0 && !self.busy()
    }

    pub fn present(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<bool> {
        if let Some(task) = &mut self.prepare {
            let Some((catalog, parts)) = task.try_take()? else {
                return Ok(false);
            };
            self.prepare = None;
            self.catalog.merge_missing(catalog);
            self.jobs = Some(Preparation {
                parts,
                meta: h.image_metadata(c)?,
                images: Vec::with_capacity(COUNT),
                active: None,
            });
        }
        if let Some(prep) = &mut self.jobs {
            match prep.advance(h, c) {
                Ok(false) => return Ok(false),
                Ok(true) => self.images = std::mem::take(&mut prep.images),
                Err(error) => {
                    for part in std::mem::take(&mut prep.images) {
                        for path in part.paths {
                            let _ = h.release_image_resource(c, &path);
                        }
                    }
                    return Err(error);
                }
            }
            self.jobs = None;
            self.pending_open = false;
            self.transition(true, self.render_skip, h, c)?;
        }
        if let Some(fade) = &self.fade {
            let age = h.now_ms().saturating_sub(fade.start).min(FADE_MS);
            let level = fade_level(age, fade.showing);
            let complete = level == if fade.showing { 16 } else { 0 };
            if complete && !fade.showing {
                Self::submit(
                    h,
                    c,
                    (0..self.images.len())
                        .map(|i| json!({"op":"remove","id":layer_index(i)}))
                        .collect(),
                )?;
            } else if self.level != level {
                Self::submit(h, c, (0..self.images.len()).map(|i| json!({"op":"set_opacity","id":layer_index(i),"opacity":opacity(level)})).collect())?;
            }
            self.level = level;
            if complete {
                self.fade = None;
            }
        }
        Ok(!self.busy())
    }

    pub fn pointer(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        input: InputFrame,
        point: Option<[i32; 2]>,
        blocked: impl Fn(u8) -> bool,
    ) -> Result<(bool, Option<ToolbarAction>)> {
        if !self.open() {
            return Ok((false, None));
        }
        let hit = point.and_then(|p| {
            self.images
                .iter()
                .enumerate()
                .rev()
                .find(|(i, part)| {
                    if blocked(FIRST_SLOT + *i as u8) {
                        return false;
                    }
                    let [x, y] = part.origin.map(i64::from);
                    let [w, height] = part.size.map(i64::from);
                    contains_rect([x, y, w, height], p.map(i64::from))
                })
                .map(|(i, _)| i)
        });
        let old = self.pointer.hovered;
        let (consumed, action) = self.pointer.step(input, hit);
        if old != hit {
            Self::submit(
                h,
                c,
                [old, hit]
                    .into_iter()
                    .flatten()
                    .map(|i| self.put(i))
                    .collect(),
            )?;
        }
        Ok((consumed, action.map(ToolbarAction::from_part)))
    }

    pub fn set_bank(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        i: usize,
        active: bool,
    ) -> Result<()> {
        if self.bank[i] != active {
            self.bank[i] = active;
            if self.open() {
                Self::submit(h, c, vec![self.put(i)])?;
            }
        }
        Ok(())
    }
}

fn fade_level(age: u64, showing: bool) -> u8 {
    let age = age.min(FADE_MS);
    ((if showing { age } else { FADE_MS - age }) * 16 / FADE_MS) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{Error, Result as CoreResult, service::FileSystem, state::Command};

    #[derive(Default)]
    struct Host {
        clock: u64,
        submissions: Vec<Value>,
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
        fn submit_engine(&mut self, _: ClientId, _: u32, payload: &[u8]) -> CoreResult<()> {
            self.submissions
                .push(serde_json::from_slice(payload).map_err(|e| Error::Io(e.to_string()))?);
            Ok(())
        }
    }

    #[test]
    fn toolbar_definition_captures_each_authored_subtype() -> Result<()> {
        let mut fields = vec![
            "0",
            "2",
            "ui\\parent.gyu",
            "500",
            "560",
            "ui\\atlas.gyu",
            "0",
            "0",
            "ui\\menu.gyu",
            "300",
            "400",
            "-1",
            "ui\\header.gyu",
            "4",
            "18",
        ];
        for _ in 0..6 {
            fields.extend(["-1", "0", "0"]);
        }
        let Definition::Part(2, part) = Definition::parse(fields.join(",").as_bytes())? else {
            bail!("missing local control part");
        };
        assert_eq!(part.background.origin, [500, 560]);
        assert_eq!(part.atlas.asset.relative(), "ui/atlas.gyu");
        assert_eq!(part.menu.unwrap().origin, [300, 400]);
        assert_eq!(part.header.unwrap().origin, [4, 18]);
        assert!(part.options.iter().all(Option::is_none));
        let Definition::Sounds(sounds) = Definition::parse(b"2,999,998,999,-1,-1,-1,-1,-1")? else {
            bail!("missing sound definition");
        };
        assert_eq!(sounds[1], Some(998));
        assert_eq!(sounds[5], None);
        let inherited = format!("3,ui/menu.gyu,506,424{}", ",-1,0,0".repeat(7));
        let Definition::MenuBackgrounds(bg) = Definition::parse(inherited.as_bytes())? else {
            bail!("missing menu backgrounds");
        };
        assert_eq!(bg[0].as_ref().unwrap().origin, [506, 424]);
        assert!(bg[1..].iter().all(Option::is_none));
        assert!(Definition::parse(b"0,1,ui/part.gyu").is_err());
        Ok(())
    }

    #[test]
    fn toolbar_image_worker_copies_parent_palette_and_alpha_before_presentation() -> Result<()> {
        use crate::select_items::tests::bitmap;
        use std::{
            fs,
            time::{Duration, Instant, SystemTime, UNIX_EPOCH},
        };
        let dir = std::env::temp_dir().join(format!(
            "crysirror-toolbar-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        let assets = dir.join("assets");
        fs::create_dir_all(&assets)?;
        let run = (|| -> Result<()> {
            fs::write(assets.join("panel.gyu"), bitmap(&[1, 1], &[11, 97]))?;
            for (name, pixels) in [
                ("small.gyu", (2..8).collect::<Vec<u8>>()),
                ("large.gyu", (2..14).collect::<Vec<u8>>()),
            ] {
                let mut encoded = bitmap(&pixels, &[]);
                encoded[36 + 8..36 + 56].fill(171);
                fs::write(assets.join(name), encoded)?;
            }
            let project = dir.join("project.json");
            fs::write(&project, br#"{"schema":2,"name":"toolbar","content_revision":"1","canvas":[20,8],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
            let mut rt = crysirror_runtime::Runtime::load(&project, &dir.join("state"))?;
            let worker = TaskPool::new("toolbar-resource-test", 1, 1)?;
            let c = ClientId(1);
            let mut toolbar = Toolbar::default();
            for index in 0..COUNT as u8 {
                let asset = |name| -> Result<VfsPath> { Ok(VfsPath::new(Mount::Asset, name)?) };
                toolbar.define(Definition::Part(
                    index,
                    Part {
                        background: Picture {
                            asset: asset("panel.gyu")?,
                            origin: [2 * i32::from(index), 0],
                        },
                        atlas: Picture {
                            asset: asset(if index < 5 { "small.gyu" } else { "large.gyu" })?,
                            origin: [0, 0],
                        },
                        menu: None,
                        header: None,
                        options: Default::default(),
                    },
                ))?;
            }
            toolbar.change_scope(true, true, [false; COUNT], &mut rt.host, c, &worker)?;
            let until = Instant::now() + Duration::from_secs(10);
            while !toolbar.present(&mut rt.host, c)? {
                ensure!(Instant::now() < until, "toolbar image worker timed out");
                std::thread::yield_now();
            }
            let layer = &toolbar.images[5];
            let asset = rt
                .host
                .published_image_resource(c, &layer.paths[4])?
                .context("missing toggled toolbar image")?;
            assert_eq!(&*asset.rgba(), &[10, 2, 1, 11, 11, 2, 1, 97]);
            assert_eq!(
                toolbar.images.iter().map(|p| p.paths.len()).sum::<usize>(),
                33
            );
            assert!(toolbar.open());
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        run
    }

    #[test]
    fn toolbar_scope_fade_pointer_and_toggle_atlas_stay_separate() -> Result<()> {
        let (mut host, client) = (Host::default(), ClientId(1));
        let worker = TaskPool::new("v1-toolbar-test", 1, 1)?;
        let mut toolbar = Toolbar::default();
        toolbar.images = (0..COUNT)
            .map(|i| PartImages {
                origin: [(i * 40) as i32, 0],
                size: [40, 31],
                paths: (0..cell_count(i))
                    .map(|cell| format!("part{i}-cell{cell}"))
                    .collect(),
            })
            .collect();
        toolbar.change_scope(false, false, [false; COUNT], &mut host, client, &worker)?;
        assert!(!toolbar.busy());
        toolbar.change_scope(true, false, [false; COUNT], &mut host, client, &worker)?;
        assert!(!toolbar.busy());
        toolbar.change_scope(true, false, [false; COUNT], &mut host, client, &worker)?;
        assert!(toolbar.busy());
        host.clock = 150;
        assert!(!toolbar.present(&mut host, client)?);
        assert_eq!(
            host.submissions.last().unwrap()["layers"][0]["opacity"],
            128
        );
        host.clock = 300;
        assert!(toolbar.present(&mut host, client)?);
        assert!(toolbar.open());

        let press = InputFrame {
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        };
        let release = InputFrame::default();
        assert_eq!(
            toolbar.pointer(&mut host, client, press, Some([202, 0]), |_| false)?,
            (true, None)
        );
        assert_eq!(
            host.submissions.last().unwrap()["layers"][0]["layer"]["paint"]["asset"],
            "part5-cell1"
        );
        assert_eq!(
            toolbar.pointer(&mut host, client, release, Some([202, 0]), |_| false)?,
            (true, Some(ToolbarAction::Auto))
        );
        toolbar.set_bank(&mut host, client, 5, true)?;
        assert_eq!(
            host.submissions.last().unwrap()["layers"][0]["layer"]["paint"]["asset"],
            "part5-cell4"
        );
        assert_eq!(
            toolbar.pointer(&mut host, client, press, Some([242, 0]), |_| false)?,
            (true, None)
        );
        assert_eq!(
            toolbar.pointer(&mut host, client, release, Some([242, 31]), |_| false)?,
            (true, None)
        );
        assert_eq!(
            toolbar.pointer(&mut host, client, press, Some([242, 0]), |slot| slot == 67)?,
            (false, None)
        );
        assert_eq!(ToolbarAction::Skip.native_id(), 216);

        toolbar.change_scope(false, false, [false; COUNT], &mut host, client, &worker)?;
        host.clock = 581;
        assert!(!toolbar.present(&mut host, client)?);
        host.clock = 582;
        assert!(toolbar.present(&mut host, client)?);
        assert!(!toolbar.open());
        assert_eq!(
            host.submissions.last().unwrap()["layers"][7],
            json!({"op":"remove","id":324})
        );
        Ok(())
    }
}
