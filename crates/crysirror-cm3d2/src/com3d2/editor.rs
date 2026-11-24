// SPDX-License-Identifier: MPL-2.0
//! SceneUserEditManager, SceneUserEditMain and PhotoManEditManager in the
//! standalone Unity 5.6 player. The stock man outlives the screen and view.
use super::*;
use crate::{
    chara::{Maid, View},
    menu::MenuFile,
    tjs::TjsState,
    unity_scene::UnityFiles,
};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{TextInputSessionHandle, TextInputSessionSpec},
};
use crysirror_unity::reader::Reader;
use unicode_segmentation::UnicodeSegmentation;

type Rect = [f32; 4];
const PANEL: &str = "UI Root/Parent/Set/ListParent/Contents";
const POSE: &str = "man_porse01.anm";

#[derive(Default)]
pub(super) struct Layout {
    pub bgm: String,
    panels: [Rect; 3],
    grids: [Option<(String, [f32; 2], usize, [f32; 2])>; 2],
}
fn group(p: &str) -> usize {
    if p.contains("/Head/ListParent/") {
        1
    } else if p.contains("/Body/ListParent/") {
        2
    } else {
        0
    }
}
impl Layout {
    pub fn read(
        &mut self,
        class: &str,
        path: &str,
        pos: [f32; 3],
        r: &mut Reader<'_>,
    ) -> Result<()> {
        let center = [960. + pos[0], 540. - pos[1]];
        if class == "SceneUserEditMain" {
            r.seek(44)?;
            self.bgm = r.string()?;
        } else if class == "UIPanel" && path.ends_with("/ListParent/Contents") {
            r.seek(172)?;
            let [x, y, w, h] = r.f32s::<4>()?;
            ensure!(w > 0. && h > 0., "empty editor scroll panel");
            self.panels[group(path)] = [center[0] + x - w / 2., center[1] - y - h / 2., w, h];
        } else if class == "UIGrid" && group(path) != 0 {
            r.seek(44)?;
            let cols = usize::try_from(r.i32()?)?;
            let step = r.f32s::<2>()?;
            ensure!(
                cols > 0 && step.iter().all(|&v| v > 0.),
                "invalid editor icon grid"
            );
            self.grids[group(path) - 1] = Some((path.to_owned(), center, cols, step));
        }
        Ok(())
    }
}
fn inside(r: Rect, p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]
}
fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    [
        x,
        y,
        ((a[0] + a[2]).min(b[0] + b[2]) - x).max(0.),
        ((a[1] + a[3]).min(b[1] + b[3]) - y).max(0.),
    ]
}
fn hsv([r, g, b, _]: [f32; 4]) -> [f32; 3] {
    let v = r.max(g).max(b);
    let d = v - r.min(g).min(b);
    let h = if d == 0. {
        0.
    } else if v == r {
        ((g - b) / d).rem_euclid(6.)
    } else if v == g {
        (b - r) / d + 2.
    } else {
        (r - g) / d + 4.
    };
    [h / 6., if v == 0. { 0. } else { d / v }, v]
}
fn rgb([h, s, v]: [f32; 3]) -> [f32; 4] {
    let channel = |n: f32| {
        let k = (n + h * 6.).rem_euclid(6.);
        v * (1. - s * k.min(4. - k).clamp(0., 1.))
    };
    [channel(5.), channel(3.), channel(1.), 1.]
}
fn player_name(s: &str) -> String {
    let clean = s.replace(['\r', '\n', '\t', '\\'], "");
    if clean.is_empty() {
        " ".into()
    } else {
        clean.graphemes(true).take(8).collect()
    }
}
struct Choice {
    menu: String,
    icon: String,
    size: [f32; 2],
    kind: usize,
}
#[derive(Default)]
pub(super) struct Editor {
    man: Option<Maid>,
    view: View,
    choices: Vec<Choice>,
    names: Vec<String>,
    name: String,
    first: bool,
    text: Option<TextInputSessionHandle>,
    select_all: bool,
    pressed: Option<usize>,
    scroll: [f32; 3],
    max_scroll: [f32; 3],
    controls: Vec<(String, Rect)>,
    hue: f32,
    pose: bool,
    tutorial: bool,
    help_closing: Option<u64>,
}
impl Editor {
    pub fn enter(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        files: &GameFiles,
        widgets: &mut Vec<Widget>,
        layout: &Layout,
        state: &mut TjsState,
        first: bool,
        now: u64,
    ) -> Result<()> {
        self.tutorial = false;
        self.help_closing = None;
        self.first = first;
        self.scroll = [0.; 3];
        self.max_scroll = [0.; 3];
        self.pressed = None;
        if self.man.is_none() {
            let mut man = Maid::new(true);
            // The men's initializer selects the default man preset, without
            // running the older standalone maid's default deletion menus.
            for p in &mut man.props {
                p.dut = false;
            }
            man.set_up_model(files, op.storage.as_ref(), "_I_man.menu")?;
            self.pose = files.exists(POSE);
            self.hue = hsv(man.man_color)[0];
            self.man = Some(man);
            let asm = crysirror_dotnet::Assembly::parse(crysirror_resource::read_all(
                op.open("Managed/Assembly-CSharp.dll")?.as_ref(),
                crysirror_dotnet::MAX_ASSEMBLY_BYTES as u64,
            )?)?;
            let st = asm.eval_cctor("MaidRandomName")?;
            self.names = st
                .items(
                    st.fields
                        .get("player_name_array")
                        .context("player name field")?,
                )
                .context("player names")?
                .iter()
                .filter_map(|v| v.str().map(str::to_owned))
                .collect();
            ensure!(!self.names.is_empty(), "empty player name list");
            if state.player.player_name.is_empty() {
                state.player.player_name = self.names[state.random(self.names.len())].clone();
            }
            for menu in files
                .names_with_ext(".menu")
                .into_iter()
                .filter(|n| n.contains("mhead") || n.contains("mbody"))
            {
                let item = MenuFile::parse(&files.read(op.storage.as_ref(), &menu)?)?;
                let Some(icon) = item.first("icons").or_else(|| item.first("icon")) else {
                    continue;
                };
                if !item
                    .first("メニューフォルダ")
                    .is_some_and(|s| s.eq_ignore_ascii_case("man"))
                    || !files.exists(icon)
                {
                    continue;
                }
                let kind = match item.category.as_str() {
                    "head" => 0,
                    "body" => 1,
                    _ => continue,
                };
                let b = files.read(op.storage.as_ref(), icon)?;
                let img =
                    crysirror_image::kiss_tex::decode_texture(&b)?;
                let id = 0x43000000 + u32::try_from(self.choices.len())?;
                let asset = host
                    .upload_rgba(client, id, img.width, img.height, &img.rgba)?
                    .relative()
                    .to_owned();
                self.choices.push(Choice {
                    menu,
                    icon: asset,
                    size: [img.width as f32, img.height as f32],
                    kind,
                });
            }
        }
        let man = self.man.as_mut().context("missing editor player")?;
        man.visible = true;
        man.chinko_visible = false;
        man.cross_fade(POSE, true, false, 0., now, f32::MAX);
        self.name = state.player.player_name.clone();
        for kind in 0..2 {
            let (path, origin, cols, step) = layout.grids[kind]
                .as_ref()
                .context("missing man editor grid")?;
            for (index, item) in self.choices.iter().filter(|i| i.kind == kind).enumerate() {
                let [w, h] = item.size;
                let rect = [
                    origin[0] + (index % cols) as f32 * step[0] - w / 2.,
                    origin[1] + (index / cols) as f32 * step[1] - h / 2.,
                    w,
                    h,
                ];
                widgets.push(Widget {
                    path: format!("{path}/{}", item.menu),
                    rect,
                    uv: [0., 0., 1., 1.],
                    asset: item.icon.clone(),
                    slices: None,
                    depth: 512,
                    tint: [255; 4],
                    label: None,
                    component: 0,
                    toggle: None,
                    collider: Some(rect),
                    button: None,
                });
                let panel = layout.panels[kind + 1];
                self.max_scroll[kind + 1] =
                    self.max_scroll[kind + 1].max(rect[1] + h - panel[1] - panel[3]);
            }
        }
        self.controls = widgets
            .iter()
            .filter_map(|w| w.collider.map(|r| (w.path.clone(), r)))
            .collect();
        let panel = layout.panels[0];
        self.max_scroll[0] = widgets
            .iter()
            .filter(|w| w.path.starts_with(PANEL))
            .map(|w| w.rect[1] + w.rect[3] - panel[1] - panel[3])
            .fold(0., f32::max);
        ensure!(
            self.controls.iter().any(|(k, _)| k.ends_with("/Parent/OK")),
            "missing editor OK"
        );
        Ok(())
    }
    pub fn help(
        &mut self,
        state: &mut TjsState,
        assets: &mut Assets,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        files: &GameFiles,
        widgets: &mut Vec<Widget>,
    ) -> Result<()> {
        let key = "__チュートリアルフラグ_SceneUserEdit";
        if state.player.flag(key) > 0 {
            return Ok(());
        }
        let t = state.retail.table("tutorial_list.nei")?;
        if let Some(row) = (1..t.rows).find(|&i| t.string(0, i) == "SceneUserEdit") {
            let pages: Vec<_> = (2..t.columns)
                .step_by(2)
                .filter(|&c| t.exists(c, row))
                .map(|c| t.string(c, row))
                .collect();
            ensure!(
                pages.len() == 1,
                "unsupported player editor tutorial page count {}",
                pages.len()
            );
            assets.editor_help(op, host, client, &pages[0], files, widgets)?;
            self.tutorial = true;
            state.player.generic_flag.insert(key.into(), 1);
        }
        Ok(())
    }
    pub fn help_alpha(&self, elapsed: u64, now: u64) -> u8 {
        let t = self.help_closing.map_or(elapsed as f32 / 500., |s| {
            1. - now.saturating_sub(s) as f32 / 500.
        });
        (255. * t.clamp(0., 1.)).round() as u8
    }
    fn ratio(&self, path: &str) -> Option<f32> {
        let m = self.man.as_ref()?;
        Some(if path.contains("/Alpha/") {
            m.man_alpha
        } else if path.contains("/Abdomen/") {
            m.value("Hara") as f32 / 100.
        } else if path.contains("/HueSlider") {
            self.hue
        } else if let Some(i) = channel(path) {
            m.man_color[i]
        } else {
            return None;
        })
    }
    pub fn geometry(&self, w: &Widget, layout: &Layout) -> (Rect, Option<Rect>, bool) {
        if w.path.starts_with("/Canvas Tutorial Panel/") {
            return (w.rect, None, self.tutorial);
        }
        let mut rect = w.rect;
        let mut clip = None;
        if w.path.starts_with(PANEL) {
            let g = group(&w.path);
            rect[1] -= self.scroll[0];
            clip = Some(layout.panels[0]);
            if g != 0 && w.path.ends_with(".menu") {
                rect[1] -= self.scroll[g];
                let mut inner = layout.panels[g];
                inner[1] -= self.scroll[0];
                clip = Some(intersect(layout.panels[0], inner));
            }
        }
        if let Some(parent) = w.path.strip_suffix("/Thumb")
            && let Some(v) = self.ratio(parent)
            && let Some((_, bar)) = self.controls.iter().find(|(k, _)| k == parent)
        {
            if parent.ends_with("/HueSlider") {
                rect[1] = bar[1] + bar[3] * (1. - v) - rect[3] / 2. - self.scroll[0];
            } else {
                rect[0] = bar[0] + bar[2] * v - rect[2] / 2.;
            }
        }
        if w.path.ends_with("/Palette/Selecter")
            && let Some((_, area)) = self
                .controls
                .iter()
                .find(|(p, _)| p.ends_with("/PalettePanel/HitArea"))
        {
            let [_, s, v] = hsv(self.man.as_ref().unwrap().man_color);
            rect[0] = area[0] + s * area[2] - rect[2] / 2.;
            rect[1] = area[1] + (1. - v) * area[3] - rect[3] / 2. - self.scroll[0];
        }
        (rect, clip, self.first || !w.path.contains("/Name/Random"))
    }
    pub fn decorate(&self, w: &mut Widget) {
        if let Some(label) = w.label.as_mut() {
            if w.path.ends_with("/Name/InputField/Label") {
                label.set_text(&self.name);
            } else if w.path.ends_with("/Value/Number")
                && let Some(v) = self.ratio(&w.path)
            {
                let n = if channel(&w.path).is_some() {
                    (v * 255.) as i32
                } else {
                    (v * 100.).round() as i32
                };
                label.set_text(&n.to_string());
            }
        } else if w.path.ends_with("/PalettePanel/Palette") {
            w.tint = rgb([self.hue, 1., 1.]).map(|v| (v * 255.).round() as u8);
        }
    }
    fn end_focus(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        if let Some(h) = self.text.take() {
            host.release_text_input_session(client, h)?;
        }
        Ok(())
    }
    pub fn update(
        &mut self,
        op: &Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        files: &GameFiles,
        widgets: &[Widget],
        layout: &Layout,
        state: &mut TjsState,
        input: InputFrame,
        released: u64,
        enabled: bool,
        now: u64,
    ) -> Result<bool> {
        let p = [input.pointer_x as f32, input.pointer_y as f32];
        if let Some(handle) = self.text {
            let committed = host.take_text_input_committed(client, handle)?;
            if !committed.is_empty() || input.buttons_pressed & buttons::BACKSPACE != 0 {
                let mut s = if self.select_all {
                    String::new()
                } else {
                    self.name.clone()
                };
                if input.buttons_pressed & buttons::BACKSPACE != 0
                    && let Some((i, _)) = s.grapheme_indices(true).last()
                {
                    s.truncate(i);
                }
                s.push_str(&committed);
                self.name = player_name(&s);
                self.select_all = false;
                state.player.player_name = self.name.clone();
            }
            if input.buttons_pressed & (buttons::ENTER | buttons::ESCAPE) != 0 {
                self.end_focus(host, client)?;
            }
        }
        if self
            .help_closing
            .is_some_and(|since| now.saturating_sub(since) >= 500)
        {
            self.help_closing = None;
            self.tutorial = false;
            self.pressed = None;
        }
        let mut done = false;
        if enabled && self.help_closing.is_none() {
            if !self.tutorial
                && input.buttons_pressed & (buttons::WHEEL_DOWN | buttons::WHEEL_UP) != 0
                && inside(layout.panels[0], p)
            {
                let mut g = 0;
                for i in 1..3 {
                    let mut r = layout.panels[i];
                    r[1] -= self.scroll[0];
                    if inside(r, p) && self.max_scroll[i] > 0. {
                        g = i;
                    }
                }
                let dy = if input.buttons_pressed & buttons::WHEEL_DOWN != 0 {
                    90.
                } else {
                    -90.
                };
                self.scroll[g] = (self.scroll[g] + dy).clamp(0., self.max_scroll[g]);
            }
            let hit = widgets.iter().enumerate().rev().find_map(|(i, w)| {
                if self.tutorial != w.path.starts_with("/Canvas Tutorial Panel/") {
                    return None;
                }
                let b = w.collider?;
                let (r, clip, visible) = self.geometry(w, layout);
                if !visible || clip.is_some_and(|c| !inside(c, p)) {
                    return None;
                }
                let bounds = [b[0] + r[0] - w.rect[0], b[1] + r[1] - w.rect[1], b[2], b[3]];
                inside(bounds, p).then_some(i)
            });
            if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
                self.pressed = hit;
                if !hit.is_some_and(|i| widgets[i].path.ends_with("/Name/InputField")) {
                    self.end_focus(host, client)?;
                }
            }
            if let Some(i) = self.pressed {
                let path = &widgets[i].path;
                let bar = path.strip_suffix("/Thumb").unwrap_or(path);
                if input.buttons_down & buttons::MOUSE_LEFT != 0
                    && (bar.ends_with("/Slider")
                        || bar.ends_with("/HueSlider")
                        || bar.ends_with("/HitArea"))
                    && let Some((_, r)) = self.controls.iter().find(|(k, _)| k == bar)
                {
                    let x = ((p[0] - r[0]) / r[2]).clamp(0., 1.);
                    let y = (1. - (p[1] + self.scroll[0] - r[1]) / r[3]).clamp(0., 1.);
                    let m = self.man.as_mut().unwrap();
                    if bar.contains("/Alpha/") {
                        let v = (x * 100.).ceil() / 100.;
                        if m.man_alpha != v {
                            m.man_alpha = v;
                            for s in m.slots.values_mut() {
                                s.dirty = true;
                            }
                        }
                    } else if bar.contains("/Abdomen/") {
                        m.set_value("Hara", (x * 100.).round() as i32)?;
                    } else if let Some(c) = channel(bar) {
                        let mut color = m.man_color;
                        color[c] = x;
                        m.set_man_color(color);
                        self.hue = hsv(color)[0];
                    } else if bar.ends_with("/HueSlider") {
                        self.hue = y;
                        let [_, s, v] = hsv(m.man_color);
                        m.set_man_color(rgb([y, s, v]));
                    } else if bar.ends_with("/HitArea") {
                        m.set_man_color(rgb([self.hue, x, y]));
                    }
                }
                if released & buttons::MOUSE_LEFT != 0 {
                    self.pressed = None;
                    if hit == Some(i) {
                        if path.ends_with("/Button OK") {
                            self.help_closing = Some(now);
                        } else if path.ends_with("/Parent/OK") {
                            done = true;
                        } else if path.ends_with("/Name/Random") && self.first {
                            self.name = self.names[state.random(self.names.len())].clone();
                            state.player.player_name = self.name.clone();
                        } else if path.ends_with("/Name/InputField")
                            && self.first
                            && self.text.is_none()
                        {
                            let (r, _, _) = self.geometry(&widgets[i], layout);
                            self.text = Some(host.open_text_input_session(
                                client,
                                TextInputSessionSpec {
                                    capture: true,
                                    cursor_area: Some(r.map(|v| v as i32)),
                                    ..Default::default()
                                },
                            )?);
                            self.select_all = true;
                        } else if let Some(item) = self
                            .choices
                            .iter()
                            .find(|item| path.ends_with(&format!("/{}", item.menu)))
                        {
                            let m = self.man.as_mut().unwrap();
                            m.set_prop_file(
                                if item.kind == 0 { "head" } else { "body" },
                                &item.menu,
                                false,
                            )?;
                            m.all_proc_prop(files, op.storage.as_ref())?;
                            if item.kind == 1 {
                                m.cross_fade(POSE, true, false, 0., now, f32::MAX);
                            }
                        } else if path.contains("/Scroll Bar/") {
                            let g = group(path);
                            let dy = if path.ends_with("/Up") {
                                -90.
                            } else if path.ends_with("/Down") {
                                90.
                            } else {
                                0.
                            };
                            self.scroll[g] = (self.scroll[g] + dy).clamp(0., self.max_scroll[g]);
                        }
                    }
                }
            }
        }
        let m = self.man.as_mut().context("editor player")?;
        m.body_morphs
            .insert("karadal".into(), m.value("Hara") as f32 / 100.);
        let cmds = self.view.sync(
            std::slice::from_mut(m),
            [0.; 3],
            [0.; 3],
            files,
            op.storage.as_ref(),
            now,
            &mut |id, glb| {
                Ok(host
                    .upload_model(client, 0x41000000 + id, glb)?
                    .relative()
                    .to_owned())
            },
            &Default::default(),
        )?;
        if !cmds.is_empty() {
            host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":cmds}))?)?;
        }
        self.release_models(host, client)?;
        Ok(done)
    }
    fn release_models(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        for asset in self.view.released.drain(..) {
            host.release_model(client, &VfsPath::new(Mount::Asset, &asset)?)?;
        }
        Ok(())
    }
    pub fn leave(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        self.end_focus(host, client)?;
        let cmds: Vec<_> = self
            .view
            .reset()
            .into_iter()
            .map(|id| json!({"op":"despawn","id":id}))
            .collect();
        if !cmds.is_empty() {
            host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":cmds}))?)?;
        }
        self.release_models(host, client)
    }
    pub fn hide_player(&mut self) {
        if let Some(m) = self.man.as_mut() {
            m.visible = false;
        }
    }
    pub fn diagnostics(&self) -> serde_json::Value {
        self.man.as_ref().map(|m|json!({"name":self.name,"first":self.first,"head":m.file("head"),"body":m.file("body"),"alpha":m.man_alpha,"abdomen":m.value("Hara"),"color":m.man_color,"tutorial":self.tutorial,"items":self.choices.len(),"pose_mounted":self.pose})).unwrap_or_default()
    }
}
fn channel(path: &str) -> Option<usize> {
    ["R", "G", "B"].iter().position(|c| {
        path.split_once("/SingleRGB/")
            .and_then(|(_, r)| r.split('/').next())
            == Some(*c)
    })
}
