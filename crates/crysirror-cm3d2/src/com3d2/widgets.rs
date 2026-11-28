// SPDX-License-Identifier: MPL-2.0
//! Serialized NGUI widget references of the Unity 5.6 player.
use super::*;
mod rooms;
pub(super) mod power;
use crate::unity_scene::{ObjectKey, UnityData, UnityFiles};
use crysirror_core::service::{FontJob, FontLoadSource, FontLoadSpec, FontLoadState};
use crysirror_unity::{PPtr, class_id as c, reader::Reader};
use std::collections::HashMap;

#[derive(Clone)]
enum LabelFont {
    Embedded(FontJob),
    System(String),
}

impl LabelFont {
    fn alias(&self, host: &mut dyn HostServices, client: ClientId) -> Result<Option<String>> {
        match self {
            Self::System(name) => Ok(Some(name.clone())),
            Self::Embedded(job) => match host.font_library(client)?.status(*job)? {
                FontLoadState::Ready(info) => Ok(Some(info.alias)),
                FontLoadState::Pending => Ok(None),
                FontLoadState::Failed(e) => bail!("UILabel font: {e}"),
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct Label {
    font: LabelFont,
    pub(super) text: String,
    size: u16,
    align: f32,
    vertical: f32,
    // ClampContent restricts the printed text to the authored region.
    clamp_lines: Option<usize>,
    spacing_y: f32,
    measured_width: Option<f32>,
    ready: Option<LabelLayers>,
}

#[derive(Clone)]
struct LabelLayers {
    rect: [f32; 4],
    tint: [u8; 4],
    lines: Vec<([f32; 4], serde_json::Value)>,
}

impl Label {
    pub fn set_size(&mut self,size:u16){if self.size!=size{self.size=size;self.ready=None;self.measured_width=None;}}
    pub fn set_text(&mut self, text: &str) {
        if self.text != text {
            self.text = text.to_owned();
            self.ready = None;
            self.measured_width = None;
        }
    }

    pub(super) fn title_width(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        text: &str,
    ) -> Result<Option<f32>> {
        if self.text != text {
            self.text = text.into();
            self.ready = None;
            self.measured_width = None;
        }
        if let Some(width) = self.measured_width {
            return Ok(Some(width));
        }
        let Some(alias) = self.font.alias(host, client)? else { return Ok(None) };
        let measure = host.text_measurement(client)?;
        let mut width = 2.0_f32;
        for line in text.lines() {
            width = width.max(
                measure
                    .measure_simple(&alias, f32::from(self.size), line)?
                    .advance
                    .round_ties_even(),
            );
        }
        self.measured_width = Some(width);
        Ok(Some(width))
    }
    pub(super) fn layers(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        rect: [f32; 4],
        tint: [u8; 4],
    ) -> Result<&[([f32; 4], serde_json::Value)]> {
        if self.ready.as_ref().is_none_or(|cached| cached.rect != rect || cached.tint != tint) {
            let Some(font) = self.font.alias(host, client)? else { return Ok(&[]) };
            let measure = host.text_measurement(client)?;
            let size = f32::from(self.size);
            let advance = |text: &str| {
                measure
                    .measure_simple(&font, size, text)
                    .map(|m| m.advance)
                    .unwrap_or(0.)
            };
            let mut lines = crate::ngui::wrap(&self.text, rect[2], advance);
            let step = (size + self.spacing_y).max(1.);
            if let Some(limit) = self.clamp_lines {
                let by_height = ((rect[3] + self.spacing_y + 0.5) / step).floor() as usize;
                lines.truncate(if limit == 0 { by_height } else { by_height.min(limit) });
            }
            let block = (step * lines.len() as f32 - self.spacing_y).max(0.);
            let y = rect[1] + (rect[3] - block) * self.vertical;
            let mut out = Vec::new();
            for (i, text) in lines.iter().enumerate() {
                if text.is_empty() {
                    continue;
                }
                let width = measure.measure_simple(&font, size, text)?.advance;
                out.push(([rect[0]+(rect[2]-width)*self.align, y + i as f32 * step, width.max(1.)+size, size*1.5],
                    json!({"kind":"text","font":font,"size":self.size,"text":text,"color":tint,"layout":crysirror_core::service::SIMPLE_TEXT_LAYOUT})));
            }
            self.ready = Some(LabelLayers { rect, tint, lines: out });
        }
        Ok(&self.ready.as_ref().unwrap().lines)
    }
}

fn ptr(r: &mut Reader<'_>) -> Result<PPtr> {
    r.pptr64()
}

#[derive(Clone)]
pub(super) struct Widget {
    pub path: String,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub asset: String,
    pub slices: Option<Slices>,
    pub depth: i32,
    pub tint: [u8; 4],
    pub label: Option<Label>,
    pub component: i64,
    pub toggle: Option<Toggle>,
    pub collider: Option<[f32; 4]>,
    pub button: Option<i64>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Slices {
    border: [f32; 4],
    texture: [f32; 2],
    center: bool,
}

impl Slices {
    pub fn draw(
        self,
        rect: [f32; 4],
        uv: [f32; 4],
        mut draw: impl FnMut([f32; 4], [f32; 4], [f32; 2]),
    ) {
        use crate::ngui::slice_boundaries;
        let [left, right, top, bottom] = self.border;
        let [w, h] = self.texture;
        let xs = slice_boundaries(rect[0], rect[0] + rect[2], left, right);
        let ys = slice_boundaries(rect[1], rect[1] + rect[3], top, bottom);
        let us = slice_boundaries(uv[0], uv[2], left / w, right / w);
        let vs = slice_boundaries(uv[1], uv[3], top / h, bottom / h);
        for col in 0..3 {
            for row in 0..3 {
                let width = xs[col + 1] - xs[col];
                let height = ys[row + 1] - ys[row];
                if width <= 0. || height <= 0. || (col == 1 && row == 1 && !self.center) {
                    continue;
                }
                // Some retail borders overlap by one source texel. Keep the
                // native reversed interpolation with a mirrored destination.
                let axis = |a: f32, b: f32, tex: f32| {
                    if a == b {
                        ((a - 0.5 / tex).max(0.), (b + 0.5 / tex).min(1.), 1.)
                    } else if a > b {
                        (b, a, -1.)
                    } else {
                        (a, b, 1.)
                    }
                };
                let (u0, u1, sx) = axis(us[col], us[col + 1], w);
                let (v0, v1, sy) = axis(vs[row], vs[row + 1], h);
                let x = if sx < 0. { xs[col + 1] } else { xs[col] };
                let y = if sy < 0. { ys[row + 1] } else { ys[row] };
                draw([x, y, width, height], [u0, v0, u1, v1], [sx, sy]);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Toggle {
    pub group: i32,
    pub active_sprite: i64,
    pub selected: bool,
}

pub(super) struct Assets {
    pub slot_table: super::slots::TableLayout,
    pub guest_slot_table: super::slots::TableLayout,
    pub guest_slot_comment_limit: usize,
    pub shop_layout: super::shop::Layout,
    pub operations_layout: super::business::Layout,
    schedule_icons: HashMap<String,String>,
    schedule_portraits: HashMap<usize, (std::sync::Weak<dyn crysirror_core::image_processing::ImageResource>, String)>,
    pub daily_buttons: super::daily::Bindings,
    pub profile_limits: [usize; 3],
    pub guest_comment_limit: usize,
    pub free_skill_layout: super::leisure::FreeSkillLayout,
    pub vip_layout: super::leisure::VipLayout,
    pub credits_pad_y: f32,
    pub costume_layout: super::stock_editor::costume::Layout,
    pub stock_grids: [super::stock_editor::Grid;3],
    pub editor_layout: super::editor::Layout,
    pub check_ok: Option<i64>,
    pub title_display_ms: u64,
    day_grid: Option<([f32; 2], f32)>,
    day_atlas: Option<ObjectKey>,
    pub desk_grid: super::shop::Grid,
    pub dance_layout: super::dance::Layout,
    pub personnel_layout: super::personnel::Layout,
    pub scenario_layout: super::events::Layout,
    pub trophy_layout: super::awards::Layout,
    pub trophy_timing: Option<(u64,f32,u64)>,
    pub choice_layout: super::choices::Layout,
    pub(super) unity: UnityData,
    levels: Vec<String>,
    fonts: HashMap<ObjectKey, LabelFont>,
    uploaded: HashMap<ObjectKey, (String, u32, u32)>,
}

impl Assets {
    pub fn load(op: &mut Opener) -> Result<Self> {
        let mut unity = UnityData::new()?;
        let main = unity.file(op, "globalgamemanagers")?;
        ensure!(
            main.format == 17 && main.unity_version == "5.6.4p2",
            "unsupported COM3D2 serialized ABI"
        );
        let settings = main
            .objects
            .iter()
            .find(|o| o.class_id == c::BUILD_SETTINGS)
            .context("BuildSettings")?;
        let levels = crysirror_unity::classes::build_levels(&main.bytes(settings)?)?;
        ensure!(
            levels
                .first()
                .is_some_and(|n| n.ends_with("/_cm3d21.unity")),
            "unexpected COM3D2 boot scene"
        );
        Ok(Self {
            unity,
            levels,
            daily_buttons: Default::default(),
            slot_table: Default::default(),
            guest_slot_table: Default::default(),
            guest_slot_comment_limit: 0,
            shop_layout: Default::default(),
            operations_layout: Default::default(),
            schedule_icons: Default::default(),
            schedule_portraits: Default::default(),
            profile_limits: [0; 3],
            guest_comment_limit: 0,
            free_skill_layout: Default::default(),
            vip_layout: Default::default(),
            stock_grids: Default::default(),
            costume_layout: Default::default(),
            credits_pad_y: 0.,
            editor_layout: Default::default(),
            check_ok: None,
            title_display_ms: 0,
            day_grid: None,
            day_atlas: None,
            trophy_layout: Default::default(),
            desk_grid: Default::default(),
            dance_layout: Default::default(),
            personnel_layout: Default::default(),
            scenario_layout: Default::default(),
            trophy_timing: None,
            choice_layout: Default::default(),
            fonts: HashMap::new(),
            uploaded: HashMap::new(),
        })
    }

    fn reference(&mut self, op: &mut Opener, file: &str, r: &mut Reader<'_>) -> Result<ObjectKey> {
        self.unity
            .resolve(op, file, ptr(r)?)?
            .context("null UI asset")
    }

    fn font(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        key: ObjectKey,
    ) -> Result<LabelFont> {
        if let Some(job) = self.fonts.get(&key) {
            return Ok(job.clone());
        }
        let (class, bytes) = self.unity.bytes(op, &key)?;
        ensure!(class == c::FONT, "UILabel references class {class}");
        let mut r = Reader::new(&bytes);
        let name = r.string()?;
        // Dynamic Unity 5.6 Font: metrics, material, character rect array,
        // texture, kerning pairs and pixel scale precede the encoded face.
        r.f32()?;
        r.pptr64()?;
        r.f32()?;
        r.pptr64()?;
        r.take(20)?;
        ensure!(r.i32()? == 0, "static Font character rects in {name}");
        let pairs = r.count(8)?;
        r.take(pairs * 8 + 4)?;
        let face = r.bytes()?;
        let font = if face.is_empty() {
            // The built-in Arial asset supplies host family names instead of a font program.
            r.take(12)?;
            let mut alias = None;
            let library = host.font_library(client)?;
            for _ in 0..r.count(4)? {
                let family = r.string()?;
                if alias.is_none() && !family.is_empty() { alias = library.resolve_alias(&family)?; }
            }
            if let Some(alias) = alias { LabelFont::System(alias) } else {
                // A scoped host may have no OS fonts. Use the installation's Japanese UI face.
                let fallback = self.unity.resource_of(op, "font/notosanscjkjp-hinted/notosanscjkjp-regular", c::FONT)?
                    .context("missing standalone UI fallback font")?;
                ensure!(fallback != key, "recursive Unity fallback font");
                self.font(op, host, client, fallback)?
            }
        } else {
            ensure!(
                face.starts_with(b"OTTO") || face.starts_with(&[0, 1, 0, 0]),
                "invalid embedded font {name}"
            );
            LabelFont::Embedded(host.font_library(client)?.begin(FontLoadSpec {
                name: format!("com3d2-font-{}", self.fonts.len()),
                source: FontLoadSource::Bytes(face.into()),
                face_index: 0,
            })?)
        };
        self.fonts.insert(key, font.clone());
        Ok(font)
    }

    pub(super) fn texture(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        key: ObjectKey,
    ) -> Result<(String, u32, u32)> {
        if let Some(texture) = self.uploaded.get(&key) {
            return Ok(texture.clone());
        }
        let (class, bytes) = self.unity.bytes(op, &key)?;
        ensure!(class == c::TEXTURE_2D, "widget texture class {class}");
        let image = crysirror_unity::texture::decode_56(&bytes, |path| {
            ensure!(
                !path.contains(['/', '\\']) && path.ends_with(".resS"),
                "invalid sibling texture stream"
            );
            op.open(path)
        })?;
        let id = u32::try_from(self.uploaded.len())? + 1;
        let asset = host
            .upload_rgba(client, id, image.width, image.height, &image.rgba)?
            .relative()
            .to_owned();
        let texture = (asset, image.width, image.height);
        self.uploaded.insert(key, texture.clone());
        Ok(texture)
    }

    pub fn costume_empty_icon(&mut self, op: &mut Opener, host: &mut dyn HostServices, client: ClientId) -> Result<String> {
        let keys = self.unity.resource(op, "sceneedit/randompreset/texture/_i_del")?;
        for key in keys {
            if self.unity.bytes(op, &key)?.0 == c::TEXTURE_2D {
                return Ok(self.texture(op, host, client, key)?.0);
            }
        }
        bail!("missing costume empty icon")
    }

    pub(super) fn sprite(&mut self, op: &mut Opener, key: ObjectKey) -> Result<(ObjectKey, [f32; 4])> {
        let (class, data) = self.unity.bytes(op, &key)?;
        ensure!(class == 213, "expected a Unity Sprite");
        let mut r = Reader::new(&data);
        r.string()?;
        let rect = r.f32s::<4>()?;
        r.take(44)?;
        Ok((self.reference(op, &key.file, &mut r)?, rect))
    }

    pub(super) fn resource_sprite(&mut self,w:&mut Widget,op:&mut Opener,host:&mut dyn HostServices,c:ClientId,name:&str)->Result<bool>{
        let Some(key)=self.unity.resource_of(op,name,213)? else{return Ok(false);};
        let (texture,[x,y,width,height])=self.sprite(op,key)?;
        let (asset,tw,th)=self.texture(op,host,c,texture)?;
        w.asset=asset;w.uv=[x/tw as f32,1.-(y+height)/th as f32,(x+width)/tw as f32,1.-y/th as f32];
        Ok(true)
    }
    fn atlas(
        &mut self,
        op: &mut Opener,
        key: ObjectKey,
        name: &str,
    ) -> Result<(ObjectKey, [f32; 4], [f32; 4])> {
        let (class, bytes) = self.unity.bytes(op, &key)?;
        ensure!(class == c::MONO_BEHAVIOUR, "expected UIAtlas");
        let mut r = Reader::new(&bytes);
        r.seek(32)?;
        let material = self.reference(op, &key.file, &mut r)?;
        let mut rect = None;
        for _ in 0..r.count(52)? {
            let entry = r.string()?;
            let pos = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
            let border = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
            r.take(16)?;
            if entry == name {
                rect = Some((pos.map(|v| v as f32), border.map(|v| v as f32)));
                break;
            }
        }
        let (rect, border) = rect.with_context(|| format!("missing UIAtlas sprite {name}"))?;
        let (class, mat) = self.unity.bytes(op, &material)?;
        ensure!(class == c::MATERIAL, "expected atlas material");
        let mat = crysirror_unity::classes::Material::read_56(&mat)?;
        let main = mat
            .texture("_MainTex")
            .context("atlas material main texture")?;
        Ok((
            self.unity
                .resolve(op, &material.file, main.texture)?
                .context("null atlas texture")?,
            rect,
            border,
        ))
    }

    pub fn scene_file(&self, scene: &str) -> Result<String> {
        let suffix = format!("/{scene}.unity");
        let index = self
            .levels
            .iter()
            .position(|p| p.ends_with(&suffix))
            .with_context(|| format!("missing {scene}"))?;
        Ok(format!("level{index}"))
    }

    pub fn scene(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        scene: &str,
    ) -> Result<Vec<Widget>> {
        let day_digit = scene.strip_prefix("DayNumber/");
        let address = match scene {
            "StandaloneDataUnit" => Some("SceneDaily/SaveAndLoad/Prefab/DataUnit"),
            "StandaloneEmptyUnit" => Some("SceneDaily/SaveAndLoad/Prefab/EmptyUnit"),
            "StockCategoryButton" => Some("SceneEdit/MainMenu/Prefab/ButtonCate"),
            "StaffPlate" | "PickerPlate" => Some("CharacterSelect/Prefab/MaidPlateSimple"),
            name if name.starts_with("TrophyInfo/") => Some("SceneTrophy/Prefab/TrophyInfo"),
            "DeskItemPlate" => Some("ScenePhotoMode/WindowListItemBig"),
            "DanceSongPlate" => Some("SceneDanceSelect/Prefab/SongPlate"),
            "RecollectionRow" => Some("SceneFreeModeSelect/FreeModeItemButton"),
            "VipTaskRow" => Some("SceneDaily/Schedule/Prefab/NightTaskUnit"),
            "VipCondition" => Some("SceneDaily/Schedule/Prefab/ConditionUnit"),
            "StageSelectionRow" => Some("SceneYotogi/StageSelect/Prefab/StageUnit"),
            "StageSelectionOldRow" => Some("SceneYotogi/StageSelect/Prefab/Old/StageUnit"),
            "FreeSkillOldPanel" => Some("SceneFreeModeSelect/Prefab/FreeSkillSelect"),
            "FreeSkillCategory" => Some("SceneYotogi/SkillSelect/Prefab/CategoryBtn"),
            "FreeSkillRow" => Some("SceneYotogi/FreeMode/FreeSkillButton"),
            "ScenarioPlate" => Some("SceneScenarioSelect/Prefab/ScenarioPlate"),
            "ScenarioCondition" => Some("SceneScenarioSelect/Prefab/PlayableText"),
            "ShopItem" => Some("SceneShop/Prefab/ShopItem"),
            "CasinoShopItem" => Some("SceneCasinoShop/Prefab/CasinoShopItem"),
            "CasinoShopCategory" => Some("SceneCasinoShop/Prefab/CategoryUI"),
            "TrainingCategory" => Some("SceneShop/Prefab/CategoryButton"),
            "WorkUnit" => Some("SceneDaily/Schedule/Prefab/WorkTaskUnit"),
            "TrainingUnit" => Some("SceneDaily/Schedule/Prefab/DaytimeTaskUnit"),
            "ResultMaidUnit" => Some("SceneDaily/ResultWork/Prefab/MaidStatusUnit"),
            "ResultStamp" => Some("SceneDaily/ResultWork/Prefab/ResultWorkIcon"),
            "StockSlider" => Some("SceneEdit/MainMenu/Prefab/Slider"),
            "SelectButton" => Some("SystemUI/MessageWindow/Main/Prefabs/SelectButton"),
            _ if day_digit.is_some() => Some("SceneStartDaily/Prefab/NumberUnit"),
            _ => None,
        };
        let prefab = if let Some(address) = address {
            Some(self.unity.resource_of(op, address, c::GAME_OBJECT)?.with_context(|| format!("missing {address}"))?)
        } else { None };
        let file = match &prefab {
            Some(key)=>key.file.to_string(),
            None=>self.scene_file(if matches!(scene, "SystemDialog" | "YesNoDialog" | "StandaloneConfig" | "StandaloneShortcuts") || scene.starts_with("Trophy/") { "_cm3d21" } else if scene=="StandaloneLoadPanel" { "SceneTitle" } else if scene=="GuestSlotPanel" { "SceneKasizukiMainMenu" } else if scene=="SlotHelpShortcut" { "SceneCasinoSlot" } else if matches!(scene,"DailySlotPanel"|"SchedulePanel"|"ResultWorkPanel"|"ResultIncomePanel") { "SceneDaily" } else { scene })?,
        };
        let sf = self.unity.file(op, &file)?;
        let mut pending:Vec<_>=match &prefab {
            Some(key)=>vec![key.path_id],
            None=>sf.objects.iter().filter(|o|o.class_id==c::GAME_OBJECT).map(|o|o.path_id).collect(),
        };
        let mut nodes = HashMap::new();
        while let Some(id)=pending.pop() {
            let data = sf.bytes_of(id)?;
            let mut r = Reader::new(&data);
            let mut comps = Vec::new();
            for _ in 0..r.count(12)? {
                comps.push(ptr(&mut r)?.path_id);
            }
            r.i32()?;
            let name = r.string()?;
            let tr = *comps.first().context("GameObject has no transform")?;
            let tr_data = sf.bytes_of(tr)?;
            let mut t = Reader::new(&tr_data);
            t.seek(28)?;
            let pos = t.f32s::<3>()?;
            t.take(12)?;
            let children = t.count(12)?;
            if prefab.is_some() {
                for _ in 0..children {
                    let child=ptr(&mut t)?;
                    ensure!(child.file_id==0,"external prefab transform");
                    let b=sf.bytes_of(child.path_id)?;
                    pending.push(ptr(&mut Reader::new(&b))?.path_id);
                }
            } else { t.take(12 * children)?; }
            ensure!(!nodes.contains_key(&tr),"duplicate Unity transform");
            let pos=if prefab.as_ref().is_some_and(|p|p.path_id==id) {[0.;3]}else{pos};
            nodes.insert(tr, (name, ptr(&mut t)?.path_id, pos, comps));
        }
        if scene == "SceneDaily" { self.daily_buttons.clear(); }
        if scene == "SceneNetorareCheck" { self.check_ok = None; }
        if scene == "SceneUserEdit" { self.editor_layout = Default::default(); }
        if matches!(scene, "SceneYotogi" | "SceneYotogiOld") {
            self.free_skill_layout = Default::default();
        }
        let mut widgets = Vec::new();
        let mut guest_page_step = 0.;
        let mut guest_page_sprites = Vec::new();
        for (name, parent, pos, comps) in nodes.values() {
            let mut path = name.clone();
            let mut offset = *pos;
            let mut p = *parent;
            for _ in 0..128 {
                let Some((ancestor, up, xyz, _)) = nodes.get(&p) else {
                    break;
                };
                path = format!("{ancestor}/{path}");
                if ancestor == "UI Root" || ancestor == "SystemUI Root" {
                    break;
                }
                offset[0] += xyz[0];
                offset[1] += xyz[1];
                p = *up;
            }
            let wanted = match scene {
                "SceneCasino"=>matches!(path.as_str(),"UI Root/CharacterSelectPanel/BG"|"UI Root/CharacterSelectPanel/Ok_Button"),
                "ScenePhotoMode" => super::leisure::photo_widget(&path),
                "SceneCompetitiveShow" => path.starts_with("UI Root/MaidExaminationPanel/Viewer/"),
                "SceneStaffRoll" => path == "UI Root/BG" || path.starts_with("UI Root/StaffRollPanel/StaffRollViewer/"),
                "SceneDeskCustomize" => path=="UI Root/MainScreen/Parent/OK" || (path.starts_with("UI Root/MainScreen/Parent/DeskCustomizeWindowManager/DeskItemWindow/Parent/") && !path.contains("/SelectPopupList") && !path.contains("/TransformParts") && !path.contains("/TitleBar/Btn/")),
                "DeskItemPlate" => true,
                "SceneDanceSelect" => path=="UI Root/Manager/Main" || path.starts_with("UI Root/Parent/ButtonParent/") || (path.starts_with("UI Root/Parent/MusicSelect/") && !path.contains("/DanceShowMusicInfo")) || (path.starts_with("UI Root/Parent/DanceModeSelect/") && !path.contains("/TouchMode")) || (path.starts_with("UI Root/Parent/CharaSelect/CharacterSelectPanel/") && !path.contains("/BigThumbnail")),
                "SceneMaidManagement" => path=="UI Root/Manager/Main" || (path.starts_with("UI Root/Parent/ButtonParent/") && !path.ends_with("/View")) || (path.starts_with("UI Root/Parent/CharacterSelectPanel/") && !path.contains("/BigThumbnail")),
                "DanceSongPlate" => true,
                "SceneFreeModeSelect" => ["MenuSelect", "MenuStory", "MenuEveryday", "MenuVip"].iter()
                    .any(|name| path.starts_with(&format!("UI Root/{name}/")))
                    && !path.contains("/FluctuationOfParameterParent/")
                    && (!path.contains("/TaskViewer/") || super::leisure::VipLayout::wanted(&path)),
                "SceneKasizukiMainMenu" => ["Window Main Menu", "Window Select Man", "Window Select Club", "MaidMiniManagement", "Window Play Info"].iter()
                    .any(|name| path.starts_with(&format!("UI Root/{name}/"))),
                "SceneYotogi" | "SceneYotogiOld" => (path.starts_with("UI Root/StageSelectPanel/")
                    || path.starts_with("UI Root/FreeSkillSelectPanel/")
                    || (path.starts_with("UI Root/SubCharacterSelectPanel/") && !path.contains("/BigThumbnail")))
                    && !path.ends_with("/Ok/SelectCursor"),
                "SceneScenarioSelect" => (path.starts_with("UI Root/Parent/") && !path.contains("/BigThumbnail")) || path=="UI Root/Manager/Main",
                "SceneTrophy" => path.starts_with("UI Root/Parent/"),
                "SceneShop" | "SceneCasinoShop" => path.starts_with("UI Root/Parent/"),
                "SceneADV" => path == "__SceneADV__" || path.starts_with("UI Root/Root/TitleBar/"),
                "StandaloneDataUnit" | "StandaloneEmptyUnit" => true,
                "GuestSlotPanel" => path.starts_with("UI Root/SaveAndLoadPanel/"),
                "DailySlotPanel" => path.starts_with("UI Root/SaveAndLoadPanel/") && !path.contains("/DataUnitParent/"),
                "StandaloneLoadPanel" => path.starts_with("UI Root/SaveAndLoadPanel/") && !path.contains("/TitleSave") && !path.contains("/DisplayButtonGroup") && !path.contains("/DataUnitParent/"),
                "SceneCharacterSelect" | "PickerPlate" | "StaffPlate" | "TrainingCategory" | "WorkUnit" | "TrainingUnit" | "ResultMaidUnit" | "SchedulePanel" | "ResultWorkPanel" | "ResultIncomePanel" => super::business::wanted(scene,&path),
                _ if prefab.is_some() => true,
                "SceneDaily" => (path == "UI Root/DailyPanel" || path.starts_with("UI Root/DailyPanel/"))
                    && !path.contains("/MainBusiness/ChangeBtn"),
                "SceneStartDaily" => path.starts_with("UI Root/StartDailyPanel/"),
                "_cm3d21" if path.ends_with("/SelectorViewer/Contents") || path.ends_with("/SelectorViewer/Contents/SelectButtonParent") => true,
                "StandaloneShortcuts" => matches!(path.as_str(),
                    "SystemUI Root/SystemShortcut/Gear" | "SystemUI Root/SystemShortcut/Base/Grid/Config"),
                "SlotHelpShortcut" => path == "SystemUI Root/SystemShortcut/Base/Grid/Help",
                "_cm3d21" if path.ends_with("/MsgParent/Buttons/Auto") || path.ends_with("/MsgParent/Buttons/Config") => true,
                "_cm3d21" => path
                    .strip_prefix("SystemUI Root/MessageWindowPanel/MessageViewer/MsgParent/")
                    .is_some_and(|child| {
                        matches!(
                            child,
                            "MessageBox"
                                | "Message"
                                | "SpeakerName"
                                | "SpeakerName/Name"
                                | "Hitret"
                        )
                    }),
                name if name.starts_with("Trophy/")=>path.starts_with("SystemUI Root/TrophyAchieveEffect"),
                "StandaloneConfig" => path.starts_with("SystemUI Root/ConfigPanel/") && !path.contains("/Title/PopupList") && !path.contains("/Device/"),
                "SystemDialog" | "YesNoDialog" => path == "SystemUI Root/SystemDialog/Base" || path.starts_with("SystemUI Root/SystemDialog/Base/"),
                "SceneLogo" => path == "UI Root/Logo" || path == "UI Root/Back",
                "SceneWarning" => path == "UI Root/Warning",
                "SceneTitle" => {
                    path.starts_with("UI Root/TitlePanel/")
                        && matches!(
                            name.as_str(),
                            "TitleLogo"
                                | "Start"
                                | "Continue"
                                | "Guest"
                                | "Config"
                                | "End"
                                | "Shop"
                                | "SelectCursor"
                        )
                }
                "SceneEdit" => super::stock_editor::costume::wanted(&path) || super::stock_profile_ui::uses(&path) || path == "UI Root/OkCancel/Ok" || path.starts_with("UI Root/FFNameDialog/")
                    || path.starts_with("UI Root/ScrollPanel-Category/") || path.starts_with("UI Root/ScrollPanel-PartsType/Scroll View") || path.starts_with("UI Root/ScrollPanel-Slider/Scroll View"),
                "SceneUserEdit" => {
                    path.starts_with("UI Root/Parent/") || path == "UI Root/Manager/Main"
                }
                "SceneNetorareCheck" => path.starts_with("UI Root/"),
                _ => false,
            };
            if !wanted {
                continue;
            }
            let begin = widgets.len();
            let mut toggle = None;
            let mut collider = None;
            let mut button = None;
            for &k in comps {
                if sf.object(k).is_some_and(|o| o.class_id == 65) {
                    let bytes = sf.bytes_of(k)?;
                    let mut r = Reader::new(&bytes);
                    r.seek(25)?;
                    if r.bool()? {
                        r.seek(28)?;
                        let [width, height, _] = r.f32s::<3>()?;
                        let [x, y, _] = r.f32s::<3>()?;
                        collider = Some([
                            960. + offset[0] + x - width * 0.5,
                            540. - offset[1] - y - height * 0.5,
                            width,
                            height,
                        ]);
                    }
                }
                if sf.object(k).is_none_or(|o| o.class_id != c::MONO_BEHAVIOUR) {
                    continue;
                }
                let body = sf.bytes_of(k)?;
                let mut r = Reader::new(&body);
                r.seek(16)?;
                let script = self.reference(op, &file, &mut r)?;
                let (_, bytes) = self.unity.bytes(op, &script)?;
                let class = Reader::new(&bytes).string()?;
                if matches!(scene,"SceneCharacterSelect"|"SchedulePanel"|"ResultWorkPanel"|"ResultMaidUnit") { self.operations_layout.read(&class,&path,offset,&body)?; }
                if matches!(scene,"StandaloneLoadPanel"|"DailySlotPanel") && path.ends_with("/DataViewer/DataUnitParent") && class=="UITable" {
                    self.slot_table.read(offset,&body)?;
                }
                if scene == "GuestSlotPanel" {
                    if class == "UITable" && path.ends_with("/DataViewer/DataUnitParent") {
                        self.guest_slot_table.read(offset, &body)?;
                    } else if class == "UIGrid" && path.ends_with("/PageButtonParent") {
                        r.seek(48)?;
                        guest_page_step = r.f32()?;
                    } else if class=="UIInput" && path.ends_with("/DataUnit/Content/Comment/InputField") {
                        r.seek(64)?;
                        self.guest_slot_comment_limit = usize::try_from(r.i32()?)?;
                    }
                }
                if scene == "SceneDaily" && class == "DailyCtrl" {
                    self.daily_buttons = super::daily::bindings(&body)?;
                }
                if scene=="_cm3d21" && path.contains("/SelectorViewer/Contents") {
                    self.choice_layout.read(&class,&body,offset)?;
                    continue;
                }
                if scene == "SceneStartDaily" && class == "UIGrid" && name == "DaysUnitParent" {
                    r.seek(32)?;
                    ensure!(r.i32()? == 0, "unsupported day digit grid arrangement");
                    r.seek(40)?;
                    ensure!(r.i32()? == 8, "unsupported day digit grid pivot");
                    r.seek(48)?;
                    self.day_grid = Some(([offset[0], offset[1]], r.f32()?));
                }
                if scene == "SceneEdit" { self.costume_layout.read(&class, &path, offset, &body)?; }
                if scene == "SceneEdit" && path.contains("/Scroll View") {
                    let group=if path.contains("ScrollPanel-Category") {0} else if path.contains("ScrollPanel-PartsType") {1} else {2};
                    if class=="UIGrid" {
                        r.seek(52)?;
                        self.stock_grids[group].step=r.f32()?;
                    } else if class=="UIPanel" && path.ends_with("/Scroll View") {
                        r.seek(172)?;
                        let [x,y,w,h]=r.f32s::<4>()?;
                        self.stock_grids[group].clip=[960.+offset[0]+x-w/2.,540.-offset[1]-y-h/2.,w,h];
                    }
                }
                if scene == "SceneEdit" && class == "UIInput"
                    && let Some(super::stock_profile_ui::Control::Input(field)) = super::stock_profile_ui::control(&path)
                {
                    r.seek(64)?;
                    let limit = r.i32()?;
                    ensure!((1..=4096).contains(&limit), "invalid stock profile character limit");
                    self.profile_limits[field.index()] = limit as usize;
                }
                if scene == "SceneKasizukiMainMenu" && class == "UIInput"
                    && path.ends_with("/MaidMiniStatusPanel/FreeCommentWindow/FreeComment")
                {
                    r.seek(64)?;
                    let limit = r.i32()?;
                    ensure!((1..=4096).contains(&limit), "invalid guest comment character limit");
                    self.guest_comment_limit = limit as usize;
                }
                if scene == "SceneUserEdit" {
                    self.editor_layout.read(&class, &path, offset, &mut r)?;
                }
                if scene == "SceneStaffRoll" && class == "UITable" && name=="StaffRollUnitParent" {
                    r.seek(32)?;
                    ensure!(r.i32()? == 1, "unsupported credits table layout");
                    r.seek(64)?;
                    self.credits_pad_y=r.f32()?;
                }
                if scene=="SceneDeskCustomize" && path.contains("/ListParent/") {self.desk_grid.read(&class,&path,offset,&body)?;}
                if scene=="SceneDanceSelect" {self.dance_layout.read(&class,&path,offset,&body)?;}
                if scene=="SceneFreeModeSelect" {self.vip_layout.read(&class,&path,offset,&body)?;}
                if scene=="SceneMaidManagement" {self.personnel_layout.read(&class,&path,offset,&body)?;}
                if scene=="SceneScenarioSelect" {self.scenario_layout.read(&class,&path,offset,&body)?;}
                if scene=="SceneTrophy" { self.trophy_layout.read(&class,&path,offset,&body)?; }
                if path.contains("FreeSkillSelect/") && matches!(scene, "SceneYotogi" | "FreeSkillOldPanel") {
                    self.free_skill_layout.read(&class, &path, offset, &mut r, scene == "FreeSkillOldPanel")?;
                }
                if class=="TrophyAchieveEffect" {
                    r.seek(32)?;
                    self.trophy_timing=Some(((r.f32()?*1000.).round() as u64,r.i32()? as f32,r.i32()?.try_into()?));
                }
                if class == "SceneADV" {
                    r.seek(32)?;
                    let bar = r.pptr64()?;
                    ensure!(
                        bar.file_id == 0 && bar.path_id != 0,
                        "invalid SceneADV.titleBar"
                    );
                    let duration = r.f32()?;
                    ensure!(
                        duration.is_finite() && duration >= 0.,
                        "invalid titleBarDisplayTime"
                    );
                    self.title_display_ms = (duration * 1000.).round() as u64;
                }
                if class == "UIToggle" {
                    r.seek(32)?;
                    let group = r.i32()?;
                    let sprite = r.pptr64()?;
                    ensure!(sprite.file_id == 0, "external toggle sprite");
                    r.seek(60)?;
                    toggle = Some(Toggle {
                        group,
                        active_sprite: sprite.path_id,
                        selected: r.bool()?,
                    });
                }
                if class == "SceneNetorareCheck" {
                    r.seek(44)?;
                    let ok = r.pptr64()?;
                    ensure!(
                        ok.file_id == 0 && ok.path_id != 0,
                        "invalid check button reference"
                    );
                    self.check_ok = Some(ok.path_id);
                }
                if scene == "SceneShop" { self.shop_layout.read(&class,&path,offset,&body)?; }
                if class == "UIButton" {
                    button = Some(k);
                }
                let hit_only=class=="UIWidget" && ((matches!(scene,"StaffPlate"|"PickerPlate") && name=="Button")
                    || (scene=="StandaloneConfig" && path.ends_with("/Slider/Foreground"))
                    || (scene=="SceneKasizukiMainMenu"
                        && (path.ends_with("/LockPlayCount/仕事回数") || path.ends_with("/LockLikability/好感度"))));
                let dynamic_texture = (class == "UI2DSprite" && ((scene == "CasinoShopItem" && name == "ItemIcon") || (scene == "SceneCasinoShop" && path.ends_with("/Window-ItemInfo/Base/Icon")))) || (scene=="SceneDanceSelect" && class=="UI2DSprite" && path.ends_with("/MujicInfo/BG/Image")) || (matches!(scene, "StageSelectionRow" | "StageSelectionOldRow" | "ScenarioPlate") && class=="UI2DSprite" && name=="Icon") || (class=="UI2DSprite" && ((scene.starts_with("TrophyInfo/") && name=="Tex") || (scene=="SceneTrophy" && name=="CartSprite"))) || (scene=="SceneStaffRoll" && class=="UITexture") || (scene=="SceneEdit" && class=="UITexture" && super::stock_editor::costume::wanted(&path)) || (scene=="SceneShop" && path.ends_with("/Window-ItemInfo/Base/Icon") && class=="UI2DSprite") || (scene=="ShopItem" && name=="ItemIcon" && class=="UI2DSprite") || (class=="UITexture" && matches!(scene,"SchedulePanel"|"ResultMaidUnit"|"ResultStamp"|"WorkUnit"|"TrainingUnit")) || hit_only || (matches!(scene,"StaffPlate"|"PickerPlate") && name=="Thum" && class=="UI2DSprite");
                let dynamic_texture = dynamic_texture || (scene == "VipTaskRow" && class == "UITexture" && name == "Main");
                if !dynamic_texture && !matches!(class.as_str(), "UISprite" | "UI2DSprite" | "UILabel") {
                    continue;
                }
                r.seek(116)?;
                let tint = r
                    .f32s::<4>()?
                    .map(|v| (255. * v.clamp(0., 1.)).round() as u8);
                let pivot = r.i32()?;
                ensure!((0..=8).contains(&pivot), "invalid UIWidget pivot");
                let (w, h, depth) = (r.i32()? as f32, r.i32()? as f32, r.i32()?);
                let left = 960. + offset[0] - (pivot % 3) as f32 * 0.5 * w;
                let top = 540. - offset[1] - (pivot / 3) as f32 * 0.5 * h;
                if dynamic_texture {
                    let mut asset = String::new();
                    if scene == "SceneStaffRoll" && !path.ends_with("/Texture") {
                        r.seek(220)?;
                        let key = self.reference(op,&file,&mut r)?;
                        asset=self.texture(op,host,client,key)?.0;
                    }
                    widgets.push(Widget { path:path.clone(), rect:[left,top,w,h], depth,
                        uv:[0.,0.,1.,1.],asset,slices:None,tint:if hit_only {[0;4]}else{tint},label:None,component:k,
                        toggle:None,button:None,collider:None });
                    continue;
                }
                if class == "UILabel" {
                    r.seek(168)?;
                    let font_key = self.reference(op, &file, &mut r)?;
                    r.pptr64()?;
                    let text = crate::ngui::strip_markup(&r.string()?).replace("\r", "");
                    let font_fields = r.position();
                    let size = r.i32()?.clamp(1, 512) as u16;
                    r.i32()?;
                    let align = match r.i32()? {
                        0 => (pivot % 3) as f32 * 0.5,
                        2 => 0.5,
                        3 => 1.,
                        _ => 0.,
                    };
                    r.seek(font_fields + 16)?;
                    let max_lines = r.i32()?.max(0) as usize;
                    r.seek(font_fields + 52)?;
                    let clamp_lines = (r.i32()? == 1).then_some(max_lines);
                    r.seek(font_fields + 108)?;
                    let integer_spacing = r.i32()? as f32;
                    let spacing_y = if r.bool()? {
                        r.seek(font_fields + 120)?;
                        r.f32()?
                    } else {
                        integer_spacing
                    };
                    ensure!(spacing_y.is_finite(), "invalid UILabel line spacing");
                    let label = Label {
                        font: self.font(op, host, client, font_key)?,
                        text,
                        size,
                        align,
                        vertical: (pivot / 3) as f32 * 0.5,
                        clamp_lines,
                        spacing_y,
                        measured_width: None,
                        ready: None,
                    };
                    widgets.push(Widget {
                        path: path.clone(),
                        rect: [left, top, w, h],
                        depth,
                        uv: [0.; 4],
                        asset: String::new(),
                        slices: None,
                        tint,
                        label: Some(label),
                        component: k,
                        toggle: None,
                        button: None,
                        collider: None,
                    });
                    continue;
                }
                r.seek(164)?;
                let sliced = r.i32()? == 1;
                r.seek(184)?;
                let center = r.i32()? != 0;
                r.seek(204)?;
                let reference = ptr(&mut r)?;
                let key = if day_digit.is_some() && class == "UISprite" {
                    self.day_atlas.clone()
                } else {
                    self.unity.resolve(op, &file, reference)?
                };
                let Some(key) = key else {
                    continue;
                };
                let (texture, rect, border) = if class == "UISprite" {
                    let serialized = r.string()?;
                    if scene == "SceneStartDaily" && name == "UnitDay" {
                        self.day_atlas = Some(key.clone());
                    }
                    let number_sprite;
                    let sprite_name = match (scene, name.as_str()) {
                        _ if day_digit.is_some() => {
                            number_sprite = format!("cmd2_eyecatch_number{}", day_digit.unwrap());
                            &number_sprite
                        }
                        (scene,"Image") if scene.starts_with("TrophyInfo/")=>{
                            number_sprite=format!("cm3d2_trophy_trophymark_{}", &scene["TrophyInfo/".len()..]);
                            &number_sprite
                        }
                        (scene,"Image") if scene.starts_with("Trophy/")=>{
                            number_sprite=format!("cm3d2_trophy_trophymark_{}", &scene[7..]);
                            &number_sprite
                        }
                        ("YesNoDialog", "Ok") => "cm3d2_dialog_buttom_yes",
                        ("YesNoDialog", "Cancel") => "cm3d2_dialog_buttom_no",
                        ("SystemDialog", "Ok") => "cm3d2_dialog_buttom_ok",
                        ("SceneYotogi" | "SceneYotogiOld", "Next")
                            if path.starts_with("UI Root/FreeSkillSelectPanel/") => "cm3d2_common_okbuttom",
                        ("SystemDialog", "Cancel") => "cm3d2_dialog_buttom_cancel",
                        _ => serialized.as_str(),
                    };
                    if sprite_name.is_empty() {
                        continue;
                    }
                    if scene == "GuestSlotPanel" && name == "Page_1" {
                        for page in 2..=5 {
                            let (texture, [x, y, w, h], _) = self.atlas(
                                op, key.clone(), &format!("cm3d2_save_load_page{page:02}")
                            )?;
                            let (asset, tw, th) = self.texture(op, host, client, texture)?;
                            guest_page_sprites.push((asset, [x / tw as f32, y / th as f32,
                                (x + w) / tw as f32, (y + h) / th as f32]));
                        }
                    }
                    self.atlas(op, key, &sprite_name)?
                } else {
                    let (texture, rect) = self.sprite(op, key)?;
                    (texture, rect, [0.; 4])
                };
                let (asset, tw, th) = self.texture(op, host, client, texture)?;
                let [x, y, rw, rh] = rect;
                let y = if class == "UI2DSprite" {
                    th as f32 - y - rh
                } else {
                    y
                };
                let uv = [
                    x / tw as f32,
                    y / th as f32,
                    (x + rw) / tw as f32,
                    (y + rh) / th as f32,
                ];
                widgets.push(Widget {
                    path: path.clone(),
                    rect: [left, top, w, h],
                    depth,
                    uv,
                    asset,
                    slices: (sliced && border.iter().any(|v| *v > 0.)).then_some(Slices {
                        border,
                        texture: [tw as f32, th as f32],
                        center,
                    }),
                    tint,
                    label: None,
                    component: k,
                    toggle: None,
                    button: None,
                    collider: None,
                });
            }
            for widget in &mut widgets[begin..] {
                widget.toggle = toggle;
                widget.collider = collider;
                widget.button = button;
            }
        }
        if scene == "GuestSlotPanel" {
            ensure!(guest_page_step.is_finite() && guest_page_step > 0.
                && guest_page_sprites.len() == 4, "invalid guest slot page list");
            let prefab: Vec<_> = widgets.iter().filter(|w| {
                w.path.ends_with("/Page_1") || w.path.contains("/Page_1/")
            }).cloned().collect();
            for (i, (asset, uv)) in guest_page_sprites.into_iter().enumerate() {
                for mut widget in prefab.iter().cloned() {
                    if widget.path.ends_with("/Page_1") {
                        widget.asset.clone_from(&asset);
                        widget.uv = uv;
                    }
                    widget.path = widget.path.replace("/Page_1", &format!("/Page_{}", i + 2));
                    let dx = (i + 1) as f32 * guest_page_step;
                    widget.rect[0] += dx;
                    if let Some(hit) = &mut widget.collider {
                        hit[0] += dx;
                    }
                    widgets.push(widget);
                }
            }
        }
        ensure!(!widgets.is_empty(), "no confirmed widgets in {scene}");
        widgets.sort_by(|a, b| (a.depth, &a.path).cmp(&(b.depth, &b.path)));
        Ok(widgets)
    }
}

impl Assets {
    pub fn schedule_portrait(&mut self, host:&mut dyn HostServices, client:ClientId, state:&crate::tjs::TjsState, id:usize)->Result<Option<String>> {
        let Some(icon)=crate::tjs::retail::operations::portrait(state,id) else {return Ok(None)};
        let source=std::sync::Arc::downgrade(icon);
        if let Some((old,asset))=self.schedule_portraits.get(&id) && old.ptr_eq(&source) {
            return Ok(Some(asset.clone()));
        }
        let upload_id=0x47000000u32.checked_add(id.try_into()?).context("schedule portrait index overflow")?;
        let [width,height]=icon.size();
        let asset=host.upload_rgba(client,upload_id,width,height,&icon.rgba())?.relative().to_owned();
        self.schedule_portraits.insert(id,(source,asset.clone()));
        Ok(Some(asset))
    }

    pub fn schedule_icon(&mut self, op:&Opener, host:&mut dyn HostServices, client:ClientId, files:&GameFiles, name:&str) -> Result<String> {
        self.archive_icon(op, host, client, files, name, false)
    }

    pub(super) fn archive_icon(&mut self, op:&Opener, host:&mut dyn HostServices, client:ClientId, files:&GameFiles, name:&str, legacy:bool) -> Result<String> {
        let file=format!("{}.tex", name.to_lowercase());
        let key=if legacy { format!("linked/{file}") } else { file.clone() };
        if let Some(asset)=self.schedule_icons.get(&key) { return Ok(asset.clone()); }
        let image=crysirror_image::kiss_tex::decode_texture(&files.read(op.storage.as_ref(),&file)?)?;
        let id=0x46000000u32.checked_add(self.schedule_icons.len().try_into()?).context("too many schedule icons")?;
        let asset=host.upload_rgba(client,id,image.width,image.height,&image.rgba)?.relative().to_owned();
        self.schedule_icons.insert(key,asset.clone());
        Ok(asset)
    }

    pub fn day_digits(&mut self, op: &mut Opener, host: &mut dyn HostServices, client: ClientId, day: i32, widgets: &mut Vec<Widget>) -> Result<()> {
        let ([x, y], width) = self.day_grid.context("missing DaysUnitParent grid")?;
        let digits = day.to_string();
        for (i, digit) in digits.chars().enumerate() {
            let mut pieces = self.scene(op, host, client, &format!("DayNumber/{digit}"))?;
            for widget in &mut pieces {
                widget.rect[0] += x + (i as f32 + 1. - digits.len() as f32) * width;
                widget.rect[1] -= y;
            }
            widgets.extend(pieces);
        }
        Ok(())
    }

    /// Instantiate the single-page mode of uGUITutorialPanel. RectTransforms
    /// are reconstructed from anchors, pivot, anchoredPosition and sizeDelta.
    pub fn editor_help(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        page: &str,
        files: &GameFiles,
        widgets: &mut Vec<Widget>,
    ) -> Result<()> {
        self.tutorial(op, host, client, &[(page.to_owned(), String::new())], 0, files, widgets)
    }

    pub fn tutorial(&mut self, op: &mut Opener, host: &mut dyn HostServices, client: ClientId,
        pages: &[(String,String)], current: usize, files: &GameFiles, widgets: &mut Vec<Widget>) -> Result<()> {
        let page = &pages.get(current).context("tutorial page index")?.0;
        let multi = pages.len() > 1;
        let mut toggle_step = None;
        let root = self
            .unity
            .resource_of(op, "SceneTutorial/Canvas Tutorial Panel", c::GAME_OBJECT)?
            .context("missing tutorial Canvas")?;
        let sf = self.unity.file(op, &root.file)?;
        let b = sf.bytes_of(root.path_id)?;
        let mut r = Reader::new(&b);
        ensure!(r.count(12)? > 0, "tutorial root");
        let transform = r.pptr64()?.path_id;
        let text_style = widgets
            .iter()
            .find_map(|w| w.label.clone())
            .context("editor font")?;
        let mut stack = vec![(transform, String::new(), [0., 0., 1920., 1080.])];
        let mut seen = std::collections::HashSet::new();
        while let Some((tr, parent, area)) = stack.pop() {
            ensure!(seen.insert(tr), "cyclic tutorial hierarchy");
            let b = sf.bytes_of(tr)?;
            let mut r = Reader::new(&b);
            let go = r.pptr64()?.path_id;
            r.seek(52)?;
            let n = r.count(12)?;
            let mut children = Vec::with_capacity(n);
            for _ in 0..n {
                children.push(r.pptr64()?.path_id);
            }
            r.pptr64()?;
            let [ax, ay, bx, by, dx, dy, dw, dh, px, py] = r.f32s::<10>()?;
            let rect = if tr == transform {
                area
            } else {
                let [x, y, w, h] = area;
                let (width, height) = (w * (bx - ax) + dw, h * (by - ay) + dh);
                [
                    x + w * (ax + (bx - ax) * px) + dx - width * px,
                    y + h * (1. - ay - (by - ay) * py) - dy - height * (1. - py),
                    width,
                    height,
                ]
            };
            let b = sf.bytes_of(go)?;
            let mut r = Reader::new(&b);
            let mut comps = vec![];
            for _ in 0..r.count(12)? {
                comps.push(r.pptr64()?.path_id);
            }
            r.i32()?;
            let name = r.string()?;
            let path = format!("{parent}/{name}");
            if (!multi && name == "parent multi mode") || (multi && name == "parent single mode")
                || name == "Slider Vertical" || name == "Text Page Number" {
                continue;
            }
            for child in children.into_iter().rev() {
                stack.push((child, path.clone(), rect));
            }
            if name == "raycast blocker" {
                continue;
            }
            for k in comps {
                if sf.object(k).is_none_or(|o| o.class_id != c::MONO_BEHAVIOUR) {
                    continue;
                }
                let b = sf.bytes_of(k)?;
                let mut r = Reader::new(&b);
                r.seek(16)?;
                let key = self.reference(op, &root.file, &mut r)?;
                let (_, bytes) = self.unity.bytes(op, &key)?;
                let class = Reader::new(&bytes).string()?;
                if class == "GridLayoutGroup" && path.contains("/Scroll View Buttons/") {
                    r.seek(64)?;
                    let [height, _, gap] = r.f32s::<3>()?;
                    ensure!(height > 0. && gap >= 0., "unsupported tutorial grid");
                    toggle_step = Some(height + gap);
                }
                if class != "Image" && class != "Text" {
                    continue;
                }
                if name == "Viewport" || name == "text title" {
                    continue;
                }
                let mut w = Widget {
                    path: path.clone(),
                    rect,
                    depth: 4000
                        + widgets
                            .iter()
                            .filter(|w| w.path.starts_with("/Canvas Tutorial Panel"))
                            .count() as i32,
                    uv: [0., 0., 1., 1.],
                    asset: String::new(),
                    slices: None,
                    label: None,
                    tint: [255; 4],
                    toggle: None,
                    collider: None,
                    button: None,
                    component: k,
                };
                r.seek(44)?;
                w.tint = r
                    .f32s::<4>()?
                    .map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
                if name == "Button OK" {
                    w.collider = Some(rect);
                }
                if name == "Content Image" {
                    let data = files.read(op.storage.as_ref(), &format!("{page}.tex"))?;
                    let img = crysirror_image::kiss_tex::decode_texture(&data)?;
                    w.asset = host
                        .upload_rgba(client, 0x44000000, img.width, img.height, &img.rgba)?
                        .relative()
                        .to_owned();
                } else if class == "Text" {
                    let mut l = text_style.clone();
                    l.set_text("OK");
                    l.size = 28;
                    l.align = 0.5;
                    l.vertical = 0.5;
                    w.label = Some(l);
                } else {
                    r.seek(196)?;
                    if let Some(sprite) = self.unity.resolve(op, &root.file, r.pptr64()?)? {
                        let (key, [x, y, sw, sh]) = self.sprite(op, sprite)?;
                        let (asset, tw, th) = self.texture(op, host, client, key)?;
                        let (tw, th) = (tw as f32, th as f32);
                        w.asset = asset;
                        w.uv = [x / tw, 1. - (y + sh) / th, (x + sw) / tw, 1. - y / th];
                    }
                }
                if path.contains("/temp toggle/") {
                    let step = toggle_step.context("missing tutorial buttons grid")?;
                    for (index, (_, title)) in pages.iter().enumerate() {
                        if name == "Checkmark" && index != current { continue; }
                        let mut row = w.clone();
                        row.rect[1] += index as f32 * step;
                        row.path = path.replace("/temp toggle/", &format!("/page-{index}/"));
                        if name == "Background" { row.collider = Some(row.rect); }
                        if let Some(label) = &mut row.label { label.set_text(title); }
                        widgets.push(row);
                    }
                } else {
                    widgets.push(w);
                }
            }
        }
        ensure!(
            widgets
                .iter()
                .any(|w| w.path.ends_with("/Button OK") && w.collider.is_some()),
            "missing tutorial confirmation"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Slices;

    #[test]
    fn sliced_frame_keeps_six_pixel_corners_and_overlapping_atlas_edges() {
        // The installed 11px line frame has six-pixel borders on both sides.
        let slices = Slices {
            border: [6.; 4],
            texture: [128.; 2],
            center: false,
        };
        let mut pieces = Vec::new();
        slices.draw(
            [10., 20., 200., 40.],
            [0., 0., 11. / 128., 11. / 128.],
            |r, u, s| pieces.push((r, u, s)),
        );
        assert_eq!(pieces.len(), 8);
        assert!(pieces.contains(&([10., 20., 6., 6.], [0., 0., 6. / 128., 6. / 128.], [1., 1.])));
        // Native NGUI traverses the one-texel middle UV backwards. The quad
        // spans the edge between the corners and uses the mirrored transform.
        assert!(pieces.contains(&(
            [204., 20., 188., 6.],
            [5. / 128., 0., 6. / 128., 6. / 128.],
            [-1., 1.]
        )));
    }
}
