// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Legacy message records, retained shared GlyphRuns and native click waits.
//! Shared Text owns cell layout, font metrics and reveal; workers never borrow
//! HostServices. Native encoded rows, names and mode policy remain here.
use crate::{
    config::{integer, Names},
    machine::{Machine, NativeCall, Payload},
};
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    service::{EngineQuery, HostServices, InputFrame, TextMeasurement},
    state::ClientId,
};
use crysirror_scene::layer::LayerSpec;
use crysirror_tasks::{Task, TaskPool};
use crysirror_text::grid::{layout_cells, CellCursor, CellLayoutOptions};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub struct TextFace {
    pub font: String,
    pub size: u16,
    pub spacing: i32,
    pub color: [u8; 4],
}
#[derive(Clone, Debug)]
pub struct MessageWindow {
    pub id: u32,
    pub asset: String,
    pub rect: [i32; 4],
    pub source_size: [u32; 2],
    /// Native percent0..100; exact /256 profile is processed by engine jobs.
    pub percent: u16,
    /// Engine source whose alpha is the original mask, or shared CornerKey
    /// extraction when native DWQ metadata proves the image has no mask.
    pub resource: Option<Arc<dyn crysirror_core::image_processing::ImageResource>>,
}
#[derive(Clone, Debug)]
pub struct MessageBindings {
    pub client: ClientId,
    pub root: u32,
    pub body_layer: u32,
    pub name_layer: u32,
    pub order: i32,
    pub canvas: [u32; 2],
    pub window: Option<MessageWindow>,
}
#[derive(Clone, Debug)]
pub struct MessageProfile {
    pub client: ClientId,
    pub root: u32,
    pub body_layer: u32,
    pub name_layer: u32,
    pub order: i32,
    pub canvas: [u32; 2],
    pub body: TextFace,
    pub name: TextFace,
    pub print_origin: [i32; 2],
    pub print_next: [i32; 2],
    pub lprint_origin: [i32; 2],
    pub lprint_next: [i32; 2],
    pub name_origin: [i32; 2],
    pub name_enabled: bool,
    pub auto_name: bool,
    pub name_center: i32,
    pub name_length: i32,
    pub window_rows: usize,
    pub full_rows: usize,
    pub reveal_per_second: u32,
    pub name_colors: BTreeMap<Vec<u8>, [u8; 4]>,
    pub window: Option<MessageWindow>,
    pub name_window: Option<MessageWindow>,
    /// Original MyMessage one-byte-to-two-byte resource table, supplied by a
    /// reviewed module/config worker. No source/font table is bundled here.
    pub single_byte_map: Option<Arc<[char]>>,
    pub advance_buttons: u64,
    pub cannot_click: bool,
}
impl MessageProfile {
    /// Defaults and key bindings are verified against complete legacy native
    /// CCommonPrintMessage constructor0x500fa0, not a modern struct layout.
    pub fn from_legacy_setup(
        setup: &Names,
        bindings: MessageBindings,
        font: &str,
        speed: usize,
        name_colors: Option<&Names>,
        single_byte_map: Option<Arc<[char]>>,
        advance_buttons: u64,
    ) -> Result<Self> {
        ensure!(speed < 5, "NNN native message speed index");
        let n = |key: &str, default: i32| -> Result<i32> {
            Ok(setup.integer(key.as_bytes())?.unwrap_or(default))
        };
        let size = n("fontSize", 24)?;
        let x = n("printX", 100)?;
        let y = n("printY", 100)?;
        let dx = n("nextX", 0)?;
        let dy = n("nextY", size + 6)?;
        let lx = n("LPrintX", 100)?;
        let ly = n("LPrintY", 100)?;
        let ldx = n("LNextX", dx)?;
        let ldy = n("LNextY", dy)?;
        ensure!(ldy > 0, "NNN native full message line step");
        let custom = n("colorCustomFlag", 0)? != 0;
        let color = |prefix: &str| -> Result<[u8; 4]> {
            Ok(if custom {
                [
                    n(&format!("{prefix}ColorR"), 255)? as u8,
                    n(&format!("{prefix}ColorG"), 255)? as u8,
                    n(&format!("{prefix}ColorB"), 255)? as u8,
                    255,
                ]
            } else {
                [255; 4]
            })
        };
        let body_color = if custom {
            [
                n("colorR", 255)? as u8,
                n("colorG", 255)? as u8,
                n("colorB", 255)? as u8,
                255,
            ]
        } else {
            [255; 4]
        };
        let use_colors = n("nameColorChangeFlag", 1)? != 0;
        let auto = n("autoNameWindowFlag", 0)? != 0;
        let colors = if use_colors || auto {
            Self::parse_name_colors(
                name_colors.context("NNN original name-color file not prepared")?,
            )?
        } else {
            BTreeMap::new()
        };
        let rates = [20, 40, 60, 120, 10000];
        let rate = if n("customMessageSpeed", 0)? != 0 {
            n(&format!("messageSpeed{}", speed + 1), rates[speed])?
        } else {
            rates[speed]
        };
        let profile = Self {
            client: bindings.client,
            root: bindings.root,
            body_layer: bindings.body_layer,
            name_layer: bindings.name_layer,
            order: bindings.order,
            canvas: bindings.canvas,
            body: TextFace {
                font: font.into(),
                size: u16::try_from(size)?,
                spacing: n("space", 1)?,
                color: body_color,
            },
            name: TextFace {
                font: font.into(),
                size: u16::try_from(n("nameFontSize", 20)?)?,
                spacing: n("nameSpace", 1)?,
                color: color("name")?,
            },
            print_origin: [x, y],
            print_next: [dx, dy],
            lprint_origin: [lx, ly],
            lprint_next: [ldx, ldy],
            name_origin: [n("nameX", x - dx)?, n("nameY", y - dy)?],
            name_enabled: n("nameFlag", 1)? != 0,
            auto_name: auto,
            name_center: n("nameCenterFlag", 1)?,
            name_length: n("nameLengthMax", 5)?,
            window_rows: usize::try_from(n("windowMessageNumber", 4)?)?,
            full_rows: usize::try_from(n(
                "fullMessageNumber",
                (bindings.canvas[1] as i32 - ly) / ldy - 1,
            )?)?,
            reveal_per_second: u32::try_from(rate)?,
            name_colors: colors,
            window: bindings.window,
            name_window: None,
            single_byte_map,
            advance_buttons,
            cannot_click: false,
        };
        profile.validate()?;
        Ok(profile)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.client.0 != 0
                && self.root != 0
                && self.body_layer != self.name_layer
                && self.root != self.body_layer
                && self.root != self.name_layer,
            "NNN text identities"
        );
        ensure!(
            self.canvas.iter().all(|n| (1..=8192).contains(n))
                && self.window_rows > 0
                && self.window_rows <= 64
                && self.full_rows > 0
                && self.full_rows <= 64,
            "NNN text canvas/row limits"
        );
        ensure!(
            (1..=100000).contains(&self.reveal_per_second) && self.advance_buttons != 0,
            "NNN reveal/input profile"
        );
        ensure!(
            (0..=2).contains(&self.name_center) && (0..=256).contains(&self.name_length),
            "NNN name alignment"
        );
        for face in [&self.body, &self.name] {
            ensure!(
                !face.font.is_empty()
                    && (1..=512).contains(&face.size)
                    && (-511..=4096).contains(&face.spacing),
                "NNN text face"
            );
        }
        for p in [
            self.print_origin,
            self.print_next,
            self.lprint_origin,
            self.lprint_next,
            self.name_origin,
        ] {
            ensure!(
                p.iter().all(|v| (-32767..=32767).contains(v)),
                "NNN text position"
            );
        }
        for w in self.window.iter().chain(self.name_window.iter()) {
            ensure!(w.percent <= 100, "NNN message window percent bounds");
            if let Some(image) = &w.resource {
                ensure!(
                    image.client() == self.client && image.size() == w.source_size,
                    "NNN scoped message-window resource"
                );
            } else {
                ensure!(
                    w.percent == 100,
                    "NNN translucent window requires engine resource"
                );
            }
            ensure!(
                w.id != self.root
                    && w.id != self.body_layer
                    && w.id != self.name_layer
                    && w.rect[2] > 0
                    && w.rect[3] > 0
                    && !w.asset.is_empty()
                    && w.source_size[0] >= w.rect[2] as u32
                    && w.source_size[1] >= w.rect[3] as u32,
                "NNN message window binding"
            );
        }
        ensure!(
            self.window
                .as_ref()
                .zip(self.name_window.as_ref())
                .is_none_or(|(a, b)| a.id != b.id),
            "NNN window layer collision"
        );
        if let Some(map) = &self.single_byte_map {
            ensure!(map.len() == 192, "NNN original single-byte mapping size");
        }
        Ok(())
    }
    /// The name-color file is groups of original encoded name,R,G,B tokens.
    pub fn parse_name_colors(names: &Names) -> Result<BTreeMap<Vec<u8>, [u8; 4]>> {
        ensure!(names.tokens.len() % 4 == 0, "NNN name color tuple size");
        let mut out = BTreeMap::new();
        for row in names.tokens.chunks_exact(4) {
            out.entry(row[0].clone()).or_insert([
                integer(&row[1])? as u8,
                integer(&row[2])? as u8,
                integer(&row[3])? as u8,
                255,
            ]);
        }
        Ok(out)
    }
}
/// Read original character-mapping data from the reviewed owner module. This
/// never executes PE code, bundles the table or infers a foreign module ABI.
pub fn mugen2_single_byte_map(module: &[u8]) -> Result<Arc<[char]>> {
    let digest = crysirror_resource::pe::PeData::parse(module)?
        .sha256()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    ensure!(
        digest == crate::native_bootstrap::MUGEN2_MODULE_SHA256,
        "NNN unreviewed character map module"
    );
    let table = module
        .get(0x173c70..0x173c70 + 384)
        .context("NNN original character map bounds")?;
    ensure!(
        module.get(0x175108..0x175108 + 384) == Some(table),
        "NNN original character map copies differ"
    );
    let mut out = Vec::with_capacity(192);
    for pair in table.chunks_exact(2) {
        let text = encoding_rs::SHIFT_JIS
            .decode_without_bom_handling_and_without_replacement(pair)
            .context("NNN original character map encoding")?;
        let mut cells = text.chars();
        let c = cells
            .next()
            .context("NNN original character map empty cell")?;
        ensure!(
            cells.next().is_none(),
            "NNN original character map scalar width"
        );
        out.push(c);
    }
    Ok(Arc::from(out))
}
/// Decode exactly one native display cell at a time. Unknown markup fails
/// before publication; encoded rows never become silently replaced Unicode.
pub fn plain_cells(raw: &[u8], single_byte_map: Option<&[char]>) -> Result<String> {
    ensure!(
        raw.len() <= 254 && !raw.contains(&0),
        "NNN message row bytes"
    );
    let mut out = String::new();
    let mut at = 0;
    while at < raw.len() {
        let b = raw[at];
        ensure!(
            b != b'#',
            "NNN message inline markup requires native token semantics"
        );
        if b < 0x20 {
            at += 1;
            continue;
        }
        if (0x20..=0x7f).contains(&b) || (0xa0..=0xdf).contains(&b) {
            let map = single_byte_map.context("NNN original one-byte cell mapping not prepared")?;
            out.push(
                *map.get((b - 0x20) as usize)
                    .context("NNN single-byte cell mapping bounds")?,
            );
            at += 1;
        } else {
            let pair = raw
                .get(at..at + 2)
                .context("NNN incomplete Shift_JIS display cell")?;
            let text = encoding_rs::SHIFT_JIS
                .decode_without_bom_handling_and_without_replacement(pair)
                .context("NNN invalid Shift_JIS display cell")?;
            let mut chars = text.chars();
            let c = chars.next().context("NNN empty encoded display cell")?;
            ensure!(chars.next().is_none(), "NNN encoded cell is not one scalar");
            out.push(c);
            at += 2;
        }
    }
    Ok(out)
}
#[derive(Clone, Debug)]
pub struct NativeMessageState {
    pub mode: u32,
    pub serial: u32,
    pub rows: Vec<Vec<u8>>,
    pub window_visible: bool,
    pub body_cells: usize,
    pub next_window_override: Option<bool>,
}
impl Default for NativeMessageState {
    fn default() -> Self {
        Self {
            mode: 0x22220001,
            serial: u32::MAX,
            rows: Vec::new(),
            window_visible: false,
            body_cells: 0,
            next_window_override: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct MessagePlan {
    pub next: NativeMessageState,
    pub batch: Value,
}
pub(crate) fn positioned(
    text: &str,
    face: &TextFace,
    origin: [i32; 2],
    metrics: &dyn TextMeasurement,
) -> Result<Vec<Value>> {
    let extent = metrics.measure_simple(&face.font, f32::from(face.size), "")?;
    let options = CellLayoutOptions {
        bounds: [32767, 32767],
        size: i32::from(face.size),
        spacing: [face.spacing, 0],
        half_width: false,
        half_spacing: false,
        hanging: 0,
        forbidden_line_start: String::new(),
        indent_pairs: vec![],
    };
    let result = layout_cells(text, &options, &mut CellCursor::at_start())?;
    ensure!(
        result.consumed == text.len(),
        "NNN native row exceeded shared cell layout"
    );
    Ok(result.glyphs.into_iter().map(|g|json!({"character":g.character,"baseline":[origin[0] as f32+g.position[0] as f32,origin[1] as f32+g.position[1] as f32+extent.ascent]})).collect())
}
impl NativeMessageState {
    /// CPU-only preparation for a scoped worker. Native PRINT/LPRINT replace
    /// rows, APPEND retains mode and rows; legacy APPEND hides the window.
    pub fn prepare(
        &self,
        call: &NativeCall,
        profile: &MessageProfile,
        metrics: &dyn TextMeasurement,
    ) -> Result<MessagePlan> {
        profile.validate()?;
        ensure!(call.identify == 0x66660006, "NNN message callback identify");
        let Payload::Message {
            kind,
            serial,
            cutin,
            text,
        } = &call.payload
        else {
            anyhow::bail!("NNN message payload")
        };
        ensure!(
            (0x22220001..=0x22220003).contains(kind) && call.code == *kind,
            "NNN message native code"
        );
        ensure!(
            *cutin == 0,
            "NNN cutin message policy requires native cutin layout"
        );
        ensure!(
            text.len() <= 1024 * 1024 && !text.contains(&0),
            "NNN native message byte limit"
        );
        let mut next = self.clone();
        let append = *kind == 0x22220003;
        if !append {
            next.mode = *kind;
            next.rows.clear();
        }
        next.window_visible = self.next_window_override.unwrap_or(*kind == 0x22220001);
        next.next_window_override = None;
        next.serial = *serial;
        let mut previous = if append { self.body_cells } else { 0 };
        let mut removed = 0usize;
        let mut at = 0;
        while at < text.len() {
            let end = text[at..]
                .iter()
                .position(|b| *b == 13)
                .map_or(text.len(), |n| at + n);
            let bytes = &text[at..end];
            next.rows.push(bytes[..bytes.len().min(254)].to_vec());
            if next.rows.len() > 64 {
                next.rows.remove(0);
                removed += 1;
            }
            if end == text.len() {
                break;
            }
            ensure!(
                text.get(end + 1) == Some(&10),
                "NNN message requires native CRLF row separator"
            );
            at = end + 2;
        }
        let max = if next.mode == 0x22220001 {
            profile.window_rows
        } else {
            profile.full_rows
        };
        if next.rows.len() > max {
            if next.mode == 0x22220001 {
                next.rows.truncate(max);
            } else {
                let drop = next.rows.len() - max;
                next.rows.drain(..drop);
                removed += drop;
            }
        }
        if append && removed > 0 {
            for row in self.rows.iter().take(removed) {
                previous = previous.saturating_sub(
                    plain_cells(row, profile.single_byte_map.as_deref())?
                        .chars()
                        .count(),
                );
            }
        }
        let has_name = next.mode == 0x22220001
            && profile.name_enabled
            && !next.rows.is_empty()
            && (!profile.auto_name || profile.name_colors.contains_key(&next.rows[0]));
        let mut name_glyphs = Vec::new();
        let mut name_color = profile.name.color;
        if has_name {
            let raw = &next.rows[0];
            let name = plain_cells(raw, profile.single_byte_map.as_deref())?;
            let mut origin = profile.name_origin;
            if profile.name_center != 0 {
                let spare = (profile.name_length - name.chars().count() as i32).max(0);
                let divisor = if profile.name_center == 1 { 2 } else { 1 };
                origin[0] +=
                    (spare * (i32::from(profile.name.size) + profile.name.spacing)) / divisor;
            }
            name_color = profile
                .name_colors
                .get(raw)
                .copied()
                .unwrap_or(profile.name.color);
            name_glyphs = positioned(&name, &profile.name, origin, metrics)?;
        }
        let mut body = Vec::new();
        let offset = usize::from(has_name);
        let (origin, step) = if next.mode == 0x22220001 {
            (profile.print_origin, profile.print_next)
        } else {
            (profile.lprint_origin, profile.lprint_next)
        };
        for (row, raw) in next.rows.iter().skip(offset).enumerate() {
            let cells = plain_cells(raw, profile.single_byte_map.as_deref())?;
            body.extend(positioned(
                &cells,
                &profile.body,
                [
                    origin[0] + row as i32 * step[0],
                    origin[1] + row as i32 * step[1],
                ],
                metrics,
            )?);
        }
        next.body_cells = body.len();
        let reveal = json!({"interval_ms":0,"initial_visible":previous.min(body.len()),"rate_per_second":profile.reveal_per_second});
        let mut layers = vec![
            json!({"op":"put","layer":{"id":profile.body_layer,"parent":profile.root,"rect":[0,0,profile.canvas[0],profile.canvas[1]],"order":[profile.order,1],"paint":{"kind":"glyph_run","font":profile.body.font,"size":profile.body.size,"color":profile.body.color,"glyphs":body,"reveal":reveal}}}),
            json!({"op":"put","layer":{"id":profile.name_layer,"parent":profile.root,"rect":[0,0,profile.canvas[0],profile.canvas[1]],"order":[profile.order,2],"paint":{"kind":"glyph_run","font":profile.name.font,"size":profile.name.size,"color":name_color,"glyphs":name_glyphs}}}),
        ];
        for (w, visible) in profile
            .window
            .iter()
            .map(|w| (w, next.window_visible))
            .chain(
                profile
                    .name_window
                    .iter()
                    .map(|w| (w, next.window_visible && has_name)),
            )
        {
            let uv = [
                0.,
                0.,
                w.rect[2] as f32 / w.source_size[0] as f32,
                w.rect[3] as f32 / w.source_size[1] as f32,
            ];
            layers.insert(0,json!({"op":"put","layer":{"id":w.id,"parent":profile.root,"rect":w.rect,"uv":uv,"visible":visible&&w.percent>0,"order":[profile.order,0],"paint":{"kind":"image","asset":w.asset}}}));
        }
        Ok(MessagePlan {
            next,
            batch: json!({"layers":layers}),
        })
    }
}
/// Pending native message owns its one worker and publication receipt. Failed
/// preparation/publication never acknowledges the original VM instruction.
pub struct NativeTextBridge {
    pub profile: MessageProfile,
    pub state: NativeMessageState,
    pub api_calls: u64,
    pool: TaskPool,
    worker: Option<Task<MessagePlan>>,
    ready: Option<MessagePlan>,
    pending: Option<NativeCall>,
    published: bool,
    blocked_click_at: Option<u64>,
    layers_present: bool,
    background: Option<Arc<dyn crysirror_core::image_processing::ImageResource>>,
    window_job: Option<crysirror_core::image_processing::ImageJob>,
    window_prepared: bool,
    window_path: Option<String>,
    window_candidate_path: Option<String>,
}
impl NativeTextBridge {
    pub fn new(profile: MessageProfile) -> Result<Self> {
        profile.validate()?;
        Ok(Self {
            profile,
            state: NativeMessageState::default(),
            api_calls: 0,
            pool: TaskPool::new("nnn-native-text", 1, 1)?,
            worker: None,
            ready: None,
            pending: None,
            published: false,
            blocked_click_at: None,
            layers_present: false,
            background: None,
            window_job: None,
            window_prepared: false,
            window_path: None,
            window_candidate_path: None,
        })
    }
    pub fn pending_message(&self)->bool {self.pending.is_some()}
    pub fn dependency_pending(&self)->bool {self.worker.is_some()||self.window_job.is_some()}
    pub fn set_next_window_visibility(&mut self,visible:bool)->Result<()> {
        ensure!(self.pending.is_none(),"NNN cannot override a pending native message");
        self.state.next_window_override=Some(visible);
        Ok(())
    }
    /// Stage with the parent's effect operations so one original ClearAll
    /// callback can commit one atomic shared engine batch.
    pub fn set_background(
        &mut self,
        image: Arc<dyn crysirror_core::image_processing::ImageResource>,
    ) -> Result<()> {
        ensure!(
            image.client() == self.profile.client
                && image.size() == self.profile.canvas
                && image.opaque(),
            "NNN message background scope/opaque XRGB dimensions"
        );
        if let Some(previous) = &self.background {
            if Arc::ptr_eq(previous, &image) {
                return Ok(());
            }
            ensure!(self.pending.is_none(),"NNN changing message background needs native frame/window dependency recomposition");
        }
        self.background = Some(image);
        Ok(())
    }
    fn prepare_windows(&mut self, host: &mut dyn HostServices) -> Result<bool> {
        use crysirror_core::image_processing::{
            ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageProcessState,
        };
        if self.window_prepared {
            return Ok(true);
        }
        let plan = self
            .ready
            .as_ref()
            .context("NNN window composition without prepared message")?;
        let mut windows = Vec::new();
        for window in self
            .profile
            .window
            .iter()
            .chain(self.profile.name_window.iter())
        {
            let node = plan.batch["layers"]
                .as_array()
                .context("NNN message layer batch")?
                .iter()
                .find(|node| node["layer"]["id"].as_u64() == Some(u64::from(window.id)))
                .context("NNN message window layer missing")?;
            if node["layer"]["visible"].as_bool() == Some(true) && window.percent > 0 {
                windows.push(window.clone());
            }
        }
        let has_resources = windows.iter().any(|window| window.resource.is_some());
        if !has_resources {
            self.window_prepared = true;
            return Ok(true);
        }
        ensure!(
            windows.iter().all(|window| window.resource.is_some()),
            "NNN mixed resolved/unresolved message window resources"
        );
        if self.window_job.is_none() {
            let background = self
                .background
                .clone()
                .context("NNN native window requires completed engine background frame")?;
            let layers = windows
                .iter()
                .map(|window| ImageCompositeLayer {
                    input: Box::new(ImageInput::Resource(window.resource.clone().unwrap())),
                    source: [0, 0, window.rect[2], window.rect[3]],
                    destination: [window.rect[0], window.rect[1]],
                    clip: None,
                    opacity: 255,
                    math: ImageCompositeMath::MaskPiecewise256 {
                        weight: window.percent * 256 / 100,
                    },
                })
                .collect();
            self.window_job = Some(host.image_metadata(self.profile.client)?.begin_process(
                ImageInput::Composite {
                    base: Box::new(ImageInput::Resource(background)),
                    layers,
                },
                b"[]".as_slice().into(),
            )?);
            self.api_calls += 1;
        }
        match self.window_job.as_mut().unwrap().poll()? {
            ImageProcessState::Pending => Ok(false),
            ImageProcessState::Failed(error) => {
                anyhow::bail!("NNN shared native window composition: {error}")
            }
            ImageProcessState::Ready(image) => {
                ensure!(
                    image.client() == self.profile.client && image.size() == self.profile.canvas,
                    "NNN window composition resource scope"
                );
                let published =
                    host.publish_image_resource(self.profile.client, windows[0].id, image)?;
                self.api_calls += 1;
                let path = published.path.relative().to_owned();
                let plan = self.ready.as_mut().unwrap();
                for node in plan.batch["layers"].as_array_mut().unwrap() {
                    let Some(window) = windows
                        .iter()
                        .find(|window| node["layer"]["id"].as_u64() == Some(u64::from(window.id)))
                    else {
                        continue;
                    };
                    let [x, y, w, h] = window.rect;
                    let [cw, ch] = self.profile.canvas;
                    node["layer"]["uv"] = json!([
                        x as f32 / cw as f32,
                        y as f32 / ch as f32,
                        (x + w) as f32 / cw as f32,
                        (y + h) as f32 / ch as f32
                    ]);
                    node["layer"]["paint"] = json!({"kind":"image","asset":path});
                }
                self.window_candidate_path = Some(path);
                self.window_prepared = true;
                self.window_job = None;
                Ok(true)
            }
        }
    }
    pub fn clear_operations(&self) -> Result<Vec<Value>> {
        ensure!(
            self.pending.is_none() && self.worker.is_none() && self.ready.is_none(),
            "NNN cannot clear a pending native message"
        );
        Ok(if self.layers_present {
            vec![
                json!({"op":"remove","id":self.profile.body_layer}),
                json!({"op":"remove","id":self.profile.name_layer}),
            ]
        } else {
            vec![]
        })
    }
    /// Call only after the staged shared clear batch successfully publishes.
    pub fn accept_clear(&mut self) -> Result<()> {
        let _ = self.clear_operations()?;
        self.state.rows.clear();
        self.state.body_cells = 0;
        self.layers_present = false;
        Ok(())
    }
    /// Original message Clear discards rows and glyphs, preserving mode and
    /// window policy. This standalone form owns its one shared batch.
    pub fn clear(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let layers = self.clear_operations()?;
        if !layers.is_empty() {
            host.submit_engine(
                self.profile.client,
                1,
                &serde_json::to_vec(&json!({"layers":layers}))?,
            )?;
            self.api_calls += 1;
        }
        self.accept_clear()
    }
    pub fn try_commit(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
        input: InputFrame,
    ) -> Result<bool> {
        machine.validate_receipt(call)?;
        if self.pending.is_none() {
            ensure!(
                matches!(call.payload, Payload::Message { .. }),
                "NNN text bridge requires message"
            );
            let metrics = host.text_measurement(self.profile.client)?;
            let permit = self.pool.reserve()?;
            let profile = self.profile.clone();
            let state = self.state.clone();
            let record = call.clone();
            self.worker = Some(permit.spawn(move |cancel| {
                cancel.check()?;
                let plan = state.prepare(&record, &profile, metrics.as_ref())?;
                cancel.check()?;
                Ok(plan)
            })?);
            self.pending = Some(call.clone());
            return Ok(false);
        }
        ensure!(
            self.pending.as_ref() == Some(call),
            "NNN stale text pending receipt"
        );
        if let Some(worker) = &mut self.worker {
            let Some(plan) = worker.try_take()? else {
                return Ok(false);
            };
            self.ready = Some(plan);
            self.worker = None;
        }
        if self.ready.is_some() && !self.prepare_windows(host)? {
            return Ok(false);
        }
        if let Some(plan) = &self.ready {
            // Retain a completed plan until the whole engine batch commits.
            host.submit_engine(self.profile.client, 1, &serde_json::to_vec(&plan.batch)?)?;
            self.api_calls += 1;
            self.state = plan.next.clone();
            if let Some(path) = self.window_candidate_path.take() {
                if let Some(previous) = self.window_path.replace(path) {
                    host.release_image_resource(self.profile.client, &previous)?;
                }
            }
            self.ready = None;
            self.published = true;
            self.layers_present = true;
            self.blocked_click_at = Some(host.now_ms());
        }
        ensure!(self.published, "NNN message publication absent");
        let bytes = host.query_engine(
            self.profile.client,
            EngineQuery::Layer(self.profile.body_layer),
        )?;
        let layer: LayerSpec =
            serde_json::from_slice(&bytes).context("NNN owned message layer disappeared")?;
        let complete = layer
            .paint
            .glyph_reveal_status()
            .context("NNN message layer has no shared reveal")?
            .complete;
        let click = !self.profile.cannot_click
            && input.buttons_pressed & self.profile.advance_buttons != 0
            && self.blocked_click_at != Some(host.now_ms());
        if !click {
            return Ok(false);
        }
        if !complete {
            host.submit_engine(
                self.profile.client,
                1,
                &serde_json::to_vec(
                    &json!({"layers":[{"op":"complete_reveal","id":self.profile.body_layer}]}),
                )?,
            )?;
            self.api_calls += 1;
            self.blocked_click_at = Some(host.now_ms());
            return Ok(false);
        }
        machine.acknowledge(call)?;
        self.pending = None;
        self.published = false;
        self.window_prepared = false;
        self.blocked_click_at = Some(host.now_ms());
        Ok(true)
    }
}
