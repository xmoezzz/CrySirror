// SPDX-License-Identifier: GPL-2.0-or-later
//! NScripter declarations and native policy. Drawing/decoding/reveal are engine APIs.
use super::*;
use crysirror_core::image_processing::{ImageInput, ImageJob, ImageProcessState};

const WINDOW: u32 = TEXT + 2;
const SHADOW: u32 = TEXT + 3;
const CURSOR: u32 = TEXT + 4;
const MASK: u32 = TEXT + 32;
const BUTTONS: u32 = CHOICES + 256;
const TACHI: u32 = SPRITE + 1000;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Effect {
    kind: i32,
    duration: u64,
    mask: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct NativeState {
    pub declarations: BTreeMap<String, Vec<Value>>,
    pub effects: BTreeMap<i32, Effect>,
    pub window: Option<[i32; 4]>,
    pub size: Option<u16>,
    pub spacing: [i32; 2],
    pub color: Option<[u8; 4]>,
    pub bold: bool,
    pub shadow: bool,
    pub window_color: Option<[u8; 4]>,
    pub window_rect: Option<[i32; 4]>,
    pub window_published: bool,
    pub shadow_published: bool,
    pub text_visible: bool,
    pub autoclick: Option<u64>,
    pub trap: Option<String>,
    pub timer: u64,
    pub button_atlas: Option<String>,
    pub buttons: Vec<NativeButton>,
    pub transition_pending: Option<i32>,
    pub cursors: BTreeMap<u32, NativeCursor>,
    pub render_products: bool,
    pub image_sources: BTreeMap<u32, String>,
    pub reveal_masks: BTreeMap<i32, String>,
    pub globals_enabled: bool,
    pub filelog_enabled: bool,
    pub files: BTreeSet<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct NativeCursor {
    pub sequence: crysirror_scene::sprite::SheetSequence,
    pub offset: [i32; 2],
    pub absolute: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct NativeButton {
    number: i32,
    rect: [i32; 4],
    source: [i32; 2],
}
pub(super) struct PendingImage {
    pub job: ImageJob,
    pub id: u32,
    pub layer: Json,
    pub present: bool,
    pub effect: i32,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Persistent {
    pub integers: BTreeMap<u16, i32>,
    pub strings: BTreeMap<u16, String>,
    pub files: BTreeSet<String>,
}
impl Persistent {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.integers.len() <= 9800
                && self.strings.len() <= 9800
                && self
                    .integers
                    .keys()
                    .chain(self.strings.keys())
                    .all(|n| (200..10000).contains(n))
                && self.strings.values().all(|s| s.len() <= 65536)
                && self.files.len() <= 65536,
            "native persistent metadata bounds"
        );
        for path in &self.files {
            asset(path)?;
        }
        Ok(())
    }
}
impl NscripterVm {
    pub(super) fn window_rect(&self) -> [i32; 4] {
        self.state.native.window.unwrap_or(self.config.text.rect)
    }
    pub(super) fn text_size(&self) -> u16 {
        self.state.native.size.unwrap_or(self.config.text.size)
    }
    fn resolve_asset(&self, name: &str, host: &mut dyn HostServices) -> Result<String> {
        let path = asset(name)?;
        let storage = host.storage(self.client)?;
        if let Some(found) = case_path(storage.as_ref(), &path)? {
            return Ok(found);
        }
        for prefix in &self.config.archives {
            let candidate = format!("{prefix}/{path}");
            if let Some(found) = case_path(storage.as_ref(), &candidate)? {
                return Ok(found);
            }
        }
        bail!("native resource `{path}` not found in loose files or mounted archives")
    }
    pub(super) fn publish_text(&mut self, layer: Json, host: &mut dyn HostServices) -> Result<()> {
        let mut ops = vec![];
        if let (Some(rect), Some(color)) = (
            self.state.native.window_rect,
            self.state.native.window_color,
        ) {
            ops.push(json!({"op":"put","layer":{"id":WINDOW,"rect":rect,"order":[9998,0],"paint":{"kind":"solid","color":color},"blend":"multiply"}}));
            self.state.native.window_published = true;
        }
        if self.state.native.shadow {
            let mut shadow = layer.clone();
            shadow["id"] = json!(SHADOW);
            shadow["order"] = json!([9999, 0]);
            shadow["paint"]["color"] = json!([0, 0, 0, 255]);
            if let Some(glyphs) = shadow["paint"]["glyphs"].as_array_mut() {
                for glyph in glyphs {
                    glyph["color"] = json!([0, 0, 0, 255]);
                }
            }
            let rect = self.window_rect();
            shadow["rect"] = json!([rect[0] + 1, rect[1] + 1, rect[2], rect[3]]);
            ops.push(json!({"op":"put","layer":shadow}));
            self.state.native.shadow_published = true;
        }
        ops.push(json!({"op":"put","layer":layer}));
        self.submit(host, ops, vec![])?;
        self.state.native.text_visible = true;
        Ok(())
    }
    fn hide_text(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let mut ops = vec![];
        if self.state.text_published {
            ops.push(json!({"op":"set_visible","id":TEXT,"visible":false}));
        }
        if self.state.native.shadow_published {
            ops.push(json!({"op":"set_visible","id":SHADOW,"visible":false}));
        }
        if self.state.native.window_published {
            ops.push(json!({"op":"set_visible","id":WINDOW,"visible":false}));
        }
        self.submit(host, ops, vec![])?;
        self.state.native.text_visible = false;
        Ok(())
    }
    fn effect(&mut self, id: i32, host: &mut dyn HostServices) -> Result<()> {
        if id == 0 {
            return Ok(());
        }
        self.flush(host)?;
        if id == 1 {
            return Ok(());
        }
        let effect = self
            .state
            .native
            .effects
            .get(&id)
            .cloned()
            .with_context(|| format!("native effect {id} not defined"))?;
        let kind = match effect.kind {
            2..=9 => {
                json!({"kind":"mask","asset":self.state.native.reveal_masks.get(&effect.kind).context("native stripe rule dependency missing")?,"fade":0})
            }
            10 => json!({"kind":"crossfade"}),
            11 => json!({"kind":"slide","direction":2}),
            12 => json!({"kind":"slide","direction":3}),
            13 => json!({"kind":"slide","direction":0}),
            14 => json!({"kind":"slide","direction":1}),
            15 | 18 => {
                json!({"kind":"mask","asset":self.resolve_asset(effect.mask.as_deref().context("native mask effect resource missing")?,host)?,"fade":if effect.kind==18 {1}else{0}})
            }
            other => bail!("native effect kind {other} needs exact shared transition mapping"),
        };
        if effect.duration == 0 {
            return Ok(());
        }
        self.submit(host,vec![json!({"op":"transition","transition":{"effect":kind,"duration_ms":effect.duration}})],vec![])?;
        self.state.wait = Wait::Time(host.now_ms().saturating_add(effect.duration));
        Ok(())
    }
    fn start_image(
        &mut self,
        id: u32,
        name: &str,
        mut layer: Json,
        present: bool,
        effect: i32,
        host: &mut dyn HostServices,
    ) -> Result<()> {
        ensure!(
            self.pending_image.is_none(),
            "native image dependency already pending"
        );
        let (tag, path) = if let Some(tagged) = name.strip_prefix(':') {
            let (tag, path) = tagged
                .split_once(';')
                .context("native image tag delimiter")?;
            (tag, path)
        } else {
            ("l", name)
        };
        let path = self.resolve_asset(path, host)?;
        if self.state.native.filelog_enabled {
            self.persistent_dirty |= self.state.native.files.insert(path.clone());
        }
        let metadata = host.image_metadata(self.client)?;
        let size = metadata.size(&path)?;
        ensure!(
            !tag.contains('/') || self.state.native.cursors.contains_key(&id),
            "native animated sprite resource needs shared layer animation mapping"
        );
        let tag = tag.split('/').next().unwrap_or(tag);
        let mode = match tag {
            "c" => json!({"mode":"opaque"}),
            "l" => json!({"mode":"corner_key","right":false}),
            "r" => json!({"mode":"corner_key","right":true}),
            "a" if path.to_ascii_lowercase().ends_with(".png") => json!({"mode":"preserve"}),
            "a" => {
                json!({"mode":"side_mask","cells":self.state.native.cursors.get(&id).map(|c|c.sequence.cells).unwrap_or(1),"channel":2})
            }
            _ => bail!("native image tag `{tag}` needs shared sprite-sheet mapping"),
        };
        let job = metadata.begin_process(
            ImageInput::Asset {
                path: path.clone(),
                size,
                fingerprint: Some(metadata.fingerprint(&path)?),
            },
            Arc::from(serde_json::to_vec(
                &json!([{"kind":"sprite_alpha","spec":mode}]),
            )?),
        )?;
        self.state.native.image_sources.insert(id, path.clone());
        layer["id"] = json!(id);
        self.pending_image = Some(PendingImage {
            job,
            id,
            layer,
            present,
            effect,
        });
        self.state.wait = Wait::Image {
            next: self.state.pc,
        };
        Ok(())
    }
    pub(super) fn poll_image(
        &mut self,
        host: &mut dyn HostServices,
        next: usize,
    ) -> Result<Option<VmYield>> {
        let mut pending = self
            .pending_image
            .take()
            .context("native image job missing")?;
        match pending.job.poll()? {
            ImageProcessState::Pending => {
                self.pending_image = Some(pending);
                Ok(Some(VmYield::BudgetExhausted))
            }
            ImageProcessState::Failed(error) => bail!("native image preparation: {error}"),
            ImageProcessState::Ready(resource) => {
                ensure!(
                    resource.client() == self.client,
                    "foreign native image result"
                );
                self.state.native.render_products = true;
                let [w, h] = resource.size();
                let path = host.present_surface(self.client, pending.id, w, h, &resource.rgba())?;
                if (MASK + 2..=MASK + 9).contains(&pending.id) {
                    self.state
                        .native
                        .reveal_masks
                        .insert((pending.id - MASK) as i32, path.relative().into());
                    self.state.pc = next;
                    self.state.wait = Wait::None;
                    return Ok(None);
                }
                let layer = &mut pending.layer;
                if layer["rect"].is_null() {
                    if pending.id >= TACHI && pending.id < TACHI + 3 {
                        let no = pending.id - TACHI;
                        layer["rect"] = json!([
                            self.config.canvas[0] as i32 * (no as i32 + 1) / 4 - w as i32 / 2,
                            self.config.canvas[1] as i32 - h as i32,
                            w,
                            h
                        ]);
                    } else {
                        layer["rect"] = json!([0, 0, w, h]);
                    }
                } else if layer["rect"][2].is_null() {
                    layer["rect"][2] = json!(w);
                    layer["rect"][3] = json!(h);
                }
                if let Some(cursor) = self.state.native.cursors.get(&pending.id) {
                    layer["rect"] = json!([0, 0, w / u32::from(cursor.sequence.cells), h]);
                    layer["uv"] = json!(cursor.sequence.uv(host.now_ms())?);
                    layer["visible"] = json!(false);
                }
                layer["paint"] = json!({"kind":"image","asset":path.relative()});
                self.layer(pending.id, pending.layer);
                self.state.pc = next;
                self.state.wait = Wait::None;
                if pending.present {
                    self.effect(pending.effect, host)?;
                }
                Ok(if matches!(self.state.wait, Wait::None) {
                    None
                } else {
                    Some(VmYield::BudgetExhausted)
                })
            }
        }
    }
    fn play(
        &mut self,
        path: &str,
        channel: u32,
        looping: bool,
        host: &mut dyn HostServices,
    ) -> Result<()> {
        let path = if let Some(track) = path.strip_prefix('*') {
            let track: u16 = track.parse()?;
            self.resolve_asset(&format!("CD/track{track:02}.ogg"), host)?
        } else {
            self.resolve_asset(path, host)?
        };
        if self.state.native.filelog_enabled {
            self.persistent_dirty |= self.state.native.files.insert(path.clone());
        }
        let ticket = host.begin_audio(
            self.client,
            AudioRequest::PlayAsset {
                channel,
                source: AudioAsset {
                    path: VfsPath::new(Mount::Asset, &path)?,
                    frame_limit: None,
                },
                intro: None,
                parameters: AudioParameters {
                    looping,
                    bus: if channel == 100 {
                        PcmBus::Music
                    } else {
                        PcmBus::Sfx
                    },
                    ..Default::default()
                },
            },
        )?;
        self.state.wait = Wait::Audio {
            ticket,
            next: self.state.pc,
        };
        Ok(())
    }
    fn args(t: &mut Tokens, vars: &Variables) -> Result<Vec<Value>> {
        let mut args = vec![];
        while t.peek().is_some() {
            if !args.is_empty() {
                t.comma()?;
            }
            args.push(t.expr(vars)?);
            ensure!(args.len() <= 128, "native declaration arguments");
        }
        Ok(args)
    }
    pub(super) fn native_command(
        &mut self,
        command: &str,
        t: &mut Tokens,
        host: &mut dyn HostServices,
    ) -> Result<bool> {
        match command {
            "versionstr" | "clickstr" | "selectcolor" | "menusetwindow" | "menuselectcolor"
            | "rmenu" | "savenumber" | "savename" | "lookbackbutton" | "lookbackcolor"
            | "effectblank" | "windoweffect" => {
                // Definitions are native data, not calls to unimplemented UI. A
                // request to use original menus/saves remains an explicit failure.
                let mut values = vec![];
                while t.peek().is_some() {
                    if !values.is_empty() {
                        t.comma()?;
                    }
                    let value = if let Some(Token::Word(w)) = t.peek() {
                        if self.state.vars.aliases.contains_key(w) {
                            t.expr(&self.state.vars)?
                        } else {
                            let w = w.clone();
                            t.at += 1;
                            Value::Str(w)
                        }
                    } else {
                        t.expr(&self.state.vars)?
                    };
                    values.push(value);
                }
                self.state
                    .native
                    .declarations
                    .insert(command.into(), values);
            }
            "globalon" => {
                t.end()?;
                self.state.native.globals_enabled = true;
                if let Some(p) = self.loaded_persistent.as_ref() {
                    self.state.vars.ints.extend(p.integers.clone());
                    self.state.vars.strings.extend(p.strings.clone());
                }
            }
            "filelog" => {
                t.end()?;
                self.state.native.filelog_enabled = true;
                if let Some(p) = self.loaded_persistent.as_ref() {
                    self.state.native.files = p.files.clone();
                }
            }
            "nsa" | "sar" => {
                t.end()?;
                let storage = host.storage(self.client)?;
                ensure!(
                    !self.config.archives.is_empty(),
                    "native archive declaration needs engine auto-detection/bootstrap mounts"
                );
                for prefix in &self.config.archives {
                    ensure!(
                        storage
                            .metadata(Mount::Asset, prefix)?
                            .is_some_and(|m| m.is_directory),
                        "native archive prefix `{prefix}` not mounted"
                    );
                }
            }
            "effect" => {
                let args = Self::args(t, &self.state.vars)?;
                ensure!(
                    (2..=4).contains(&args.len()),
                    "native effect definition args"
                );
                let id = args[0].int()?;
                let kind = args[1].int()?;
                ensure!(
                    (2..=255).contains(&id) && (1..=18).contains(&kind),
                    "native effect id/type"
                );
                let duration = if args.len() > 2 { args[2].int()? } else { 0 };
                ensure!((0..=600000).contains(&duration), "native effect duration");
                self.state.native.effects.insert(
                    id,
                    Effect {
                        kind,
                        duration: duration as u64,
                        mask: args.get(3).map(Value::string).transpose()?,
                    },
                );
                if (2..=9).contains(&kind) && !self.state.native.reveal_masks.contains_key(&kind) {
                    let pattern = if kind >= 6 { kind - 6 } else { kind - 2 };
                    let effects = json!([{"kind":"stripe_reveal","spec":{"vertical":pattern>=2,"reverse":pattern%2==1,"period":if kind>=6 {24}else{16},"stagger":kind>=6}}]);
                    let job = host.image_metadata(self.client)?.begin_process(
                        ImageInput::Solid {
                            size: self.config.canvas,
                            color: [0, 0, 0, 255],
                        },
                        Arc::from(serde_json::to_vec(&effects)?),
                    )?;
                    self.pending_image = Some(PendingImage {
                        job,
                        id: MASK + kind as u32,
                        layer: Json::Null,
                        present: false,
                        effect: 0,
                    });
                    self.state.wait = Wait::Image {
                        next: self.state.pc,
                    };
                }
            }
            "setwindow" => {
                let a = Self::args(t, &self.state.vars)?;
                ensure!(a.len() == 16, "native setwindow requires 16 arguments");
                let n: Vec<i32> = a[..11].iter().map(Value::int).collect::<Result<_>>()?;
                ensure!(
                    (1..=512).contains(&n[4]) && n[4] == n[5] && n[2] > 0 && n[3] > 0,
                    "native square horizontal font window required"
                );
                self.clear_text(host)?;
                self.state.native.window =
                    Some([n[0], n[1], n[2] * (n[4] + n[6]), n[3] * (n[5] + n[7])]);
                self.state.native.size = Some(n[4] as u16);
                self.state.native.spacing = [n[6], n[7]];
                self.state.speed = n[8].clamp(0, 1000) as u64;
                self.state.native.bold = n[9] != 0;
                self.state.native.shadow = n[10] != 0;
                self.state.native.window_color = Some(color(&a[11].string()?)?);
                let r = a[12..].iter().map(Value::int).collect::<Result<Vec<_>>>()?;
                ensure!(
                    r[2] >= r[0] && r[3] >= r[1],
                    "native message backing rectangle"
                );
                self.state.native.window_rect =
                    Some([r[0], r[1], r[2] - r[0] + 1, r[3] - r[1] + 1]);
            }
            "setcursor" | "abssetcursor" => {
                let n = number(t, &self.state.vars, 0, 1)? as u32;
                t.comma()?;
                let image = t.expr(&self.state.vars)?.string()?;
                t.comma()?;
                let x = number(t, &self.state.vars, -32767, 32767)?;
                t.comma()?;
                let y = number(t, &self.state.vars, -32767, 32767)?;
                t.end()?;
                let animation = image
                    .strip_prefix(':')
                    .and_then(|s| s.split_once(';'))
                    .and_then(|(tag, _)| tag.split_once('/').map(|(_, a)| a));
                let mut sequence = crysirror_scene::sprite::SheetSequence {
                    cells: 1,
                    frame_ms: 1,
                    mode: crysirror_scene::sprite::SheetLoop::Still,
                    started_ms: host.now_ms(),
                };
                if let Some(animation) = animation {
                    let a = animation
                        .split(',')
                        .map(str::parse::<u32>)
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    ensure!(a.len() == 3 && a[0] <= 1024, "native cursor animation tag");
                    sequence.cells = a[0] as u16;
                    sequence.frame_ms = a[1];
                    sequence.mode = match a[2] {
                        0 => crysirror_scene::sprite::SheetLoop::Repeat,
                        1 => crysirror_scene::sprite::SheetLoop::Once,
                        2 => crysirror_scene::sprite::SheetLoop::PingPong,
                        3 => crysirror_scene::sprite::SheetLoop::Still,
                        _ => bail!("native cursor loop mode"),
                    };
                }
                sequence.validate()?;
                self.state.native.cursors.insert(
                    CURSOR + n,
                    NativeCursor {
                        sequence,
                        offset: [x, y],
                        absolute: command == "abssetcursor",
                    },
                );
                self.start_image(
                    CURSOR + n,
                    &image,
                    json!({"order":[10001,n],"visible":false}),
                    true,
                    1,
                    host,
                )?;
            }
            "date" => {
                use chrono::Datelike;
                let now = chrono::Local::now();
                let values = [now.year() % 100, now.month() as i32, now.day() as i32];
                for (i, value) in values.into_iter().enumerate() {
                    if i > 0 {
                        t.comma()?;
                    }
                    let variable = t.variable()?;
                    self.state.vars.set(variable, Value::Int(value))?;
                }
                t.end()?;
            }
            "newpage" => {
                t.end()?;
                self.state.wait = Wait::Click { page: true };
            }
            "textcolor" => {
                let c = t.expr(&self.state.vars)?.string()?;
                t.end()?;
                self.state.native.color = Some(color(&c)?);
            }
            "delay" => {
                let ms = number(t, &self.state.vars, 0, 600000)?;
                t.end()?;
                self.state.wait = Wait::Delay(host.now_ms().saturating_add(ms as u64));
            }
            "autoclick" => {
                let ms = number(t, &self.state.vars, 0, 600000)?;
                t.end()?;
                self.state.native.autoclick = Some(ms as u64);
            }
            "trap" => {
                let target = if matches!(t.peek(),Some(Token::Word(w)) if w=="off") {
                    t.at += 1;
                    None
                } else {
                    let name = t.expr(&self.state.vars)?.string()?;
                    self.program.as_ref().unwrap().label(&name)?;
                    Some(name)
                };
                t.end()?;
                self.state.native.trap = target;
            }
            "resettimer" => {
                t.end()?;
                self.state.native.timer = host.now_ms();
            }
            "waittimer" => {
                let ms = number(t, &self.state.vars, 0, 600000)?;
                t.end()?;
                self.state.wait = Wait::Time(self.state.native.timer.saturating_add(ms as u64));
            }
            "skip" => {
                let n = t.expr(&self.state.vars)?.int()?;
                t.end()?;
                let p = self.program.as_ref().unwrap();
                let line = p.statements[self.state.pc - 1].line;
                let target = (line as i64 + if n == 0 { 1 } else { n } as i64).max(1) as usize;
                self.state.pc = p.statements.partition_point(|s| s.line < target);
            }
            "textoff" => {
                t.end()?;
                self.hide_text(host)?;
            }
            "texton" => {
                t.end()?;
                let mut ops = vec![];
                if self.state.text_published {
                    ops.push(json!({"op":"set_visible","id":TEXT,"visible":true}));
                }
                if self.state.native.window_published {
                    ops.push(json!({"op":"set_visible","id":WINDOW,"visible":true}));
                }
                if self.state.native.shadow_published {
                    ops.push(json!({"op":"set_visible","id":SHADOW,"visible":true}));
                }
                self.submit(host, ops, vec![])?;
                self.state.native.text_visible = true;
            }
            "bg" => {
                let value = if let Some(Token::Word(w)) = t.peek() {
                    if matches!(w.as_str(), "black" | "white") {
                        let w = w.clone();
                        t.at += 1;
                        w
                    } else {
                        t.expr(&self.state.vars)?.string()?
                    }
                } else {
                    t.expr(&self.state.vars)?.string()?
                };
                t.comma()?;
                let effect = t.expr(&self.state.vars)?.int()?;
                t.end()?;
                // Pinned ONScripter bgCommand clears all three character
                // slots. Ordinary lsp sprites are independent and survive.
                // Remove both retained declarations and unpublished staging,
                // otherwise a later print could resurrect an old character.
                for id in TACHI..TACHI + 3 {
                    self.state.layers.remove(&id);
                    self.state.dirty.remove(&id);
                    self.state.native.image_sources.remove(&id);
                    if self.state.published.contains(&id) {
                        self.state.removed.insert(id);
                    }
                }
                self.hide_text(host)?;
                let color = match value.as_str() {
                    "white" => Some([255; 4]),
                    "black" => Some([0, 0, 0, 255]),
                    s if s.starts_with('#') => Some(color(s)?),
                    _ => None,
                };
                if let Some(color) = color {
                    self.layer(BG,json!({"id":BG,"rect":[0,0,self.config.canvas[0],self.config.canvas[1]],"order":[-10000,0],"paint":{"kind":"solid","color":color}}));
                    self.effect(effect, host)?;
                } else {
                    self.start_image(BG,&format!(":c;{value}"),json!({"rect":[0,0,self.config.canvas[0],self.config.canvas[1]],"order":[-10000,0]}),true,effect,host)?;
                }
            }
            "lsp" | "lsph" => {
                let n = number(t, &self.state.vars, 0, 999)?;
                t.comma()?;
                let name = t.expr(&self.state.vars)?.string()?;
                t.comma()?;
                let x = number(t, &self.state.vars, -32767, 32767)?;
                t.comma()?;
                let y = number(t, &self.state.vars, -32767, 32767)?;
                let opacity = if t.peek().is_some() {
                    t.comma()?;
                    number(t, &self.state.vars, 0, 255)?
                } else {
                    255
                };
                t.end()?;
                self.start_image(SPRITE+n as u32,&name,json!({"rect":[x,y,null,null],"order":[1000-n,0],"opacity":opacity,"visible":command=="lsp"}),false,0,host)?;
            }
            "ld" => {
                let position = t.word()?;
                let no = match position.as_str() {
                    "l" => 0,
                    "c" => 1,
                    "r" => 2,
                    _ => bail!("native character position"),
                };
                t.comma()?;
                let name = t.expr(&self.state.vars)?.string()?;
                t.comma()?;
                let effect = t.expr(&self.state.vars)?.int()?;
                t.end()?;
                self.hide_text(host)?;
                self.start_image(
                    TACHI + no,
                    &name,
                    json!({"order":[100+no,0]}),
                    true,
                    effect,
                    host,
                )?;
            }
            "cl" => {
                let position = t.word()?;
                let ids = match position.as_str() {
                    "l" => vec![TACHI],
                    "c" => vec![TACHI + 1],
                    "r" => vec![TACHI + 2],
                    "a" => vec![TACHI, TACHI + 1, TACHI + 2],
                    _ => bail!("native character clear position"),
                };
                t.comma()?;
                let effect = t.expr(&self.state.vars)?.int()?;
                t.end()?;
                for id in ids {
                    self.state.layers.remove(&id);
                    self.state.dirty.remove(&id);
                    if self.state.published.contains(&id) {
                        self.state.removed.insert(id);
                    }
                }
                self.effect(effect, host)?;
            }
            "msp" => {
                let n = number(t, &self.state.vars, 0, 999)? as u32;
                t.comma()?;
                let x = number(t, &self.state.vars, -32767, 32767)?;
                t.comma()?;
                let y = number(t, &self.state.vars, -32767, 32767)?;
                let opacity = if t.peek().is_some() {
                    t.comma()?;
                    Some(number(t, &self.state.vars, -256, 256)?)
                } else {
                    None
                };
                t.end()?;
                let id = SPRITE + n;
                let layer = self
                    .state
                    .layers
                    .get_mut(&id)
                    .context("msp sprite missing")?;
                layer["rect"][0] =
                    json!(layer["rect"][0].as_i64().context("native sprite x")? + i64::from(x));
                layer["rect"][1] =
                    json!(layer["rect"][1].as_i64().context("native sprite y")? + i64::from(y));
                if let Some(delta) = opacity {
                    let before = layer["opacity"].as_i64().unwrap_or(255);
                    layer["opacity"] = json!((before + i64::from(delta)).clamp(0, 255));
                }
                self.state.dirty.insert(id);
            }
            "cmp" => {
                let dest = t.variable()?;
                t.comma()?;
                let left = t.expr(&self.state.vars)?.string()?;
                t.comma()?;
                let right = t.expr(&self.state.vars)?.string()?;
                t.end()?;
                let value = match left.cmp(&right) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                };
                self.state.vars.set(dest, Value::Int(value))?;
            }
            "print" => {
                let id = t.expr(&self.state.vars)?.int()?;
                t.end()?;
                self.effect(id, host)?;
            }
            "repaint" => {
                t.end()?;
                self.flush(host)?;
            }
            "play" | "playonce" | "wave" | "waveloop" => {
                let path = t.expr(&self.state.vars)?.string()?;
                t.end()?;
                self.play(
                    &path,
                    if command.starts_with("play") { 100 } else { 0 },
                    matches!(command, "play" | "waveloop"),
                    host,
                )?;
            }
            "playstop" | "wavestop" | "stop" => {
                t.end()?;
                for channel in if command == "stop" {
                    vec![0, 100]
                } else if command == "playstop" {
                    vec![100]
                } else {
                    vec![0]
                } {
                    host.submit_audio(
                        self.client,
                        AudioRequest::Stop {
                            channel,
                            fade_ms: 0.,
                        },
                    )?;
                }
            }
            "btndef" => {
                let name = t.expr(&self.state.vars)?.string()?;
                t.end()?;
                self.state.native.button_atlas = Some(self.resolve_asset(&name, host)?);
                self.state.native.buttons.clear();
            }
            "btn" => {
                let a = Self::args(t, &self.state.vars)?
                    .iter()
                    .map(Value::int)
                    .collect::<Result<Vec<_>>>()?;
                ensure!(
                    a.len() == 7 && a[0] > 0 && a[3] > 0 && a[4] > 0,
                    "native btn arguments"
                );
                ensure!(self.state.native.buttons.len() < 256, "native button count");
                self.state.native.buttons.push(NativeButton {
                    number: a[0],
                    rect: [a[1], a[2], a[3], a[4]],
                    source: [a[5], a[6]],
                });
            }
            "btnwait" => {
                let Token::IntVar(variable) = self.state.vars.resolve(t.variable()?)? else {
                    bail!("native btnwait integer destination")
                };
                t.end()?;
                self.hide_text(host)?;
                self.show_buttons(host)?;
                self.state.wait = Wait::Buttons { variable };
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    pub(super) fn update_cursors(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let active = match self.state.wait {
            Wait::Click { page } | Wait::AutoClick { page, .. } => Some(CURSOR + u32::from(page)),
            _ => None,
        };
        let mut ops = vec![];
        for (&id, cursor) in &self.state.native.cursors {
            let Some(layer) = self.state.layers.get(&id) else {
                continue;
            };
            if active == Some(id) {
                let mut layer = layer.clone();
                let origin = if cursor.absolute {
                    [0, 0]
                } else {
                    let r = self.window_rect();
                    [
                        r[0] + self.state.cursor.position[0],
                        r[1] + self.state.cursor.position[1],
                    ]
                };
                layer["rect"][0] = json!(origin[0] + cursor.offset[0]);
                layer["rect"][1] = json!(origin[1] + cursor.offset[1]);
                layer["uv"] = json!(cursor.sequence.uv(host.now_ms())?);
                layer["visible"] = json!(true);
                ops.push(json!({"op":"put","layer":layer}));
            } else if self.state.published.contains(&id) {
                ops.push(json!({"op":"set_visible","id":id,"visible":false}));
            }
        }
        if !ops.is_empty() {
            self.submit(host, ops, vec![])?;
        }
        Ok(())
    }
    pub(super) fn poll_persistent(&mut self, host: &mut dyn HostServices) -> Result<()> {
        use crysirror_core::service::StorageWriteState;
        if let Some(job) = self.persistent_write.as_mut() {
            match job.poll()? {
                StorageWriteState::Pending => return Ok(()),
                StorageWriteState::Ready => self.persistent_write = None,
                StorageWriteState::Failed(error) => {
                    bail!("native persistent metadata write failed: {error}")
                }
            }
        }
        if self.persistent_dirty {
            let data = Persistent {
                integers: if self.state.native.globals_enabled {
                    self.state
                        .vars
                        .ints
                        .iter()
                        .filter(|(n, _)| **n >= 200)
                        .map(|(n, v)| (*n, *v))
                        .collect()
                } else {
                    BTreeMap::new()
                },
                strings: if self.state.native.globals_enabled {
                    self.state
                        .vars
                        .strings
                        .iter()
                        .filter(|(n, _)| **n >= 200)
                        .map(|(n, v)| (*n, v.clone()))
                        .collect()
                } else {
                    BTreeMap::new()
                },
                files: self.state.native.files.clone(),
            };
            data.validate()?;
            let bytes = serde_json::to_vec(&data)?;
            ensure!(
                bytes.len() <= 4 * 1024 * 1024,
                "native persistent metadata byte budget"
            );
            self.persistent_write = Some(host.storage(self.client)?.begin_write_atomic(
                &VfsPath::new(Mount::Save, "nscripter-state.json")?,
                Arc::from(bytes),
            )?);
            self.persistent_dirty = false;
        }
        Ok(())
    }
    fn show_buttons(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let path = self
            .state
            .native
            .button_atlas
            .as_ref()
            .context("btnwait requires btndef")?;
        let [w, h] = host.image_size(self.client, path)?;
        let mut ui = vec![
            json!({"op":"put","widget":{"id":BUTTONS,"rect":[0,0,self.config.canvas[0],self.config.canvas[1]],"kind":"panel","modal":true,"z":30000,"color":[0,0,0,0]}}),
        ];
        let mut layers = vec![];
        for (index, button) in self.state.native.buttons.iter().enumerate() {
            ensure!(
                button.source[0] >= 0
                    && button.source[1] >= 0
                    && button.source[0] + button.rect[2] <= w as i32
                    && button.source[1] + button.rect[3] <= h as i32,
                "native button atlas bounds"
            );
            let id = BUTTONS + 1 + index as u32;
            ui.push(json!({"op":"put","widget":{"id":id,"parent":BUTTONS,"rect":button.rect,"kind":"button","color":[0,0,0,0]}}));
            layers.push(json!({"op":"put","layer":{"id":id,"rect":button.rect,"order":[20000,index],"visible":false,"uv":[button.source[0] as f32/w as f32,button.source[1] as f32/h as f32,(button.source[0]+button.rect[2]) as f32/w as f32,(button.source[1]+button.rect[3]) as f32/h as f32],"paint":{"kind":"image","asset":path}}}));
        }
        self.submit(host, layers, ui)
    }
    pub(super) fn poll_buttons(
        &mut self,
        host: &mut dyn HostServices,
        input: &mut InputFrame,
        variable: u16,
    ) -> Result<Option<VmYield>> {
        let events: Vec<crysirror_scene::ui::UiEvent> =
            serde_json::from_slice(&host.query_engine(self.client, EngineQuery::UiEvents)?)?;
        let selected = events.iter().find(|e| {
            e.kind == "activate"
                && e.widget.owner == self.client.0
                && (BUTTONS + 1..BUTTONS + 1 + self.state.native.buttons.len() as u32)
                    .contains(&e.widget.id)
        });
        let number = if let Some(event) = selected {
            Some(self.state.native.buttons[(event.widget.id - BUTTONS - 1) as usize].number)
        } else if input.buttons_pressed & buttons::MOUSE_RIGHT != 0 {
            Some(-1)
        } else {
            None
        };
        let Some(number) = number else {
            let mut layers = vec![];
            for index in 0..self.state.native.buttons.len() {
                let id = BUTTONS + 1 + index as u32;
                let interaction: Option<crysirror_scene::ui::WidgetInteraction> =
                    serde_json::from_slice(
                        &host.query_engine(self.client, EngineQuery::UiInteraction(id))?,
                    )?;
                let visible = interaction.is_some_and(|s| s.hovered || s.focused);
                layers.push(json!({"op":"set_visible","id":id,"visible":visible}));
            }
            self.submit(host, layers, vec![])?;
            return Ok(Some(VmYield::WaitForInput));
        };
        let layers = (0..self.state.native.buttons.len())
            .map(|i| json!({"op":"remove","id":BUTTONS+1+i as u32}))
            .collect();
        self.submit(host, layers, vec![json!({"op":"remove","id":BUTTONS})])?;
        self.state.native.buttons.clear();
        self.state.vars.ints.insert(variable, number);
        self.state.wait = Wait::None;
        input.buttons_pressed = 0;
        Ok(None)
    }
}
fn color(value: &str) -> Result<[u8; 4]> {
    ensure!(
        value.len() == 7 && value.starts_with('#'),
        "native #RRGGBB required"
    );
    Ok([
        u8::from_str_radix(&value[1..3], 16)?,
        u8::from_str_radix(&value[3..5], 16)?,
        u8::from_str_radix(&value[5..7], 16)?,
        255,
    ])
}

fn case_path(
    storage: &dyn crysirror_core::service::ClientStorage,
    path: &str,
) -> Result<Option<String>> {
    if storage
        .metadata(Mount::Asset, path)?
        .is_some_and(|m| !m.is_directory)
    {
        return Ok(Some(path.into()));
    }
    let mut found = String::new();
    for part in path.split('/') {
        let names = match storage.list(Mount::Asset, &found) {
            Ok(v) => v,
            Err(Error::NotFound) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let matching = names
            .iter()
            .filter(|name| name.eq_ignore_ascii_case(part))
            .collect::<Vec<_>>();
        ensure!(
            matching.len() <= 1,
            "ambiguous native case-insensitive resource"
        );
        let Some(name) = matching.first() else {
            return Ok(None);
        };
        if !found.is_empty() {
            found.push('/');
        }
        found.push_str(name);
    }
    if storage
        .metadata(Mount::Asset, &found)?
        .is_some_and(|m| !m.is_directory)
    {
        Ok(Some(found))
    } else {
        Ok(None)
    }
}
