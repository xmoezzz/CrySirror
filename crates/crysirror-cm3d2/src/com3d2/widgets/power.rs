// SPDX-License-Identifier: MPL-2.0
//! Unity 5.6 RectTransform and uGUI list templates on the facility upgrade canvas.
use super::rooms::anchored;
use super::*;
use std::collections::{BTreeMap, HashSet};

pub(in crate::com3d2) type Rect = [f32; 4];

pub(in crate::com3d2) const ROOM: &str = "/Window Facility List/Parent Button/temp Toggle";
pub(in crate::com3d2) const RECIPE: &str =
    "/Window PowerUp List/uGUI Scroll View/Viewport/Parent Button/temp Toggle";
pub(in crate::com3d2) const VIEW: &str = "/Window PowerUp List/uGUI Scroll View/Viewport";
pub(in crate::com3d2) const CONDITION: &str =
    "/Window PowerUp List/Window Condition List/Parent Conditions/temp item";
pub(in crate::com3d2) const MATERIAL: &str = "/Window Material List/Parent Dropdown/temp Dropdown";

pub(super) struct Grid {
    rect: Rect,
    padding: [i32; 4],
    align: i32,
    vertical: bool,
    fit_height: bool,
    cell: [f32; 2],
    gap: [f32; 2],
}

impl Grid {
    pub(super) fn read(r: &mut Reader, rect: Rect, path: &str) -> Result<Self> {
        r.seek(32)?;
        let padding = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
        let align = r.i32()?;
        let corner = r.i32()?;
        let axis = r.i32()?;
        let cell = r.f32s::<2>()?;
        let gap = r.f32s::<2>()?;
        ensure!(
            corner == 0
                && (0..=8).contains(&align)
                && (0..=1).contains(&axis)
                && r.i32()? == 0
                && cell.iter().all(|n| *n > 0.),
            "unknown facility grid {path}"
        );
        Ok(Self {
            rect,
            padding,
            align,
            vertical: axis == 1,
            fit_height: false,
            cell,
            gap,
        })
    }

    // The recipe content fitter grows its height; rooms and conditions fit their
    // vertical columns. Effect lists lay out horizontally then wrap by width.
    pub(super) fn position(&self, index: usize, count: usize) -> Rect {
        let [l, r, t, b] = self.padding.map(|x| x as f32);
        let available = self.rect[2] - l - r;
        let fit = |space: f32, size: f32, gap: f32| {
            (((space + gap + 0.001) / (size + gap)).floor().max(1.) as usize).min(count.max(1))
        };
        let (columns, rows, x_index, y_index) = if self.vertical && self.fit_height {
            (1, count, 0, index)
        } else if self.vertical {
            let rows = fit(self.rect[3] - t - b, self.cell[1], self.gap[1]);
            (count.div_ceil(rows), rows, index / rows, index % rows)
        } else {
            let cols = fit(available, self.cell[0], self.gap[0]);
            (cols, count.div_ceil(cols), index % cols, index / cols)
        };
        let used = self.cell[0] * columns as f32 + self.gap[0] * columns.saturating_sub(1) as f32;
        let used_height = self.cell[1] * rows as f32 + self.gap[1] * rows.saturating_sub(1) as f32;
        let top =
            self.rect[1] + t + (self.rect[3] - t - b - used_height) * (self.align / 3) as f32 / 2.;
        [
            self.rect[0]
                + l
                + (available - used) * (self.align % 3) as f32 / 2.
                + x_index as f32 * (self.cell[0] + self.gap[0]),
            top + y_index as f32 * (self.cell[1] + self.gap[1]),
            self.cell[0],
            self.cell[1],
        ]
    }
}

#[derive(Default)]
pub(in crate::com3d2) struct Layout {
    pub base: Vec<Widget>,
    rects: BTreeMap<String, Rect>,
    grids: BTreeMap<String, Grid>,
    pub list_spacing: f32,
    pub name_limit: usize,
}

impl Layout {
    pub fn rect(&self, path: &str) -> Result<Rect> {
        self.rects
            .get(path)
            .copied()
            .with_context(|| format!("missing facility RectTransform {path}"))
    }
    pub fn cell(&self, path: &str, index: usize, count: usize) -> Result<Rect> {
        let (parent, _) = path
            .rsplit_once('/')
            .context("facility template without parent")?;
        Ok(self
            .grids
            .get(parent)
            .with_context(|| format!("missing facility grid {parent}"))?
            .position(index, count))
    }
    pub fn instantiate(&self, path: &str, destination: Rect) -> Result<Vec<Widget>> {
        let source = self.rect(path)?;
        let mut out = Vec::new();
        for w in &self.base {
            if w.path != path
                && !w
                    .path
                    .strip_prefix(path)
                    .is_some_and(|p| p.starts_with('/'))
            {
                continue;
            }
            let mut w = w.clone();
            w.rect[0] += destination[0] - source[0];
            w.rect[1] += destination[1] - source[1];
            out.push(w);
        }
        Ok(out)
    }
}

impl Assets {
    pub(in crate::com3d2) fn upgrade_canvas(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
    ) -> Result<Layout> {
        let layout = self.facility_canvas(
            op,
            host,
            client,
            "SceneFacilityPowerUp",
            "Canvas Facility List",
        )?;
        for p in [
            ROOM,
            RECIPE,
            MATERIAL,
            CONDITION,
            "/Button EXIT",
            "/Button OK",
        ] {
            layout.rect(p)?;
        }
        Ok(layout)
    }
    pub(in crate::com3d2) fn facility_canvas(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        scene: &str,
        root_name: &str,
    ) -> Result<Layout> {
        self.filtered_canvas(op,host,client,scene,root_name,|_|true)
    }
    pub(in crate::com3d2) fn filtered_canvas(
        &mut self, op:&mut Opener,host:&mut dyn HostServices,client:ClientId,
        scene:&str,root_name:&str, include:impl Fn(&str)->bool,
    )->Result<Layout> {
        let file = self.scene_file(scene)?;
        let sf = self.unity.file(op, &file)?;
        let mut root = None;
        for obj in sf.objects.iter().filter(|o| o.class_id == c::GAME_OBJECT) {
            let bytes = sf.bytes(obj)?;
            let mut r = Reader::new(&bytes);
            let n = r.count(12)?;
            if n == 0 {
                continue;
            }
            let tr = r.pptr64()?.path_id;
            r.take((n - 1) * 12 + 4)?;
            if r.string()? == root_name {
                root = Some(tr);
                break;
            }
        }
        let root = root.with_context(|| format!("missing {root_name} in {scene}"))?;
        let screen = [0., 0., 1920., 1080.];
        let mut stack = vec![(root, String::new(), screen, 0, 1)];
        let mut layout = Layout::default();
        let mut seen = HashSet::new();
        while let Some((tr, parent, parent_rect, sibling, siblings)) = stack.pop() {
            ensure!(seen.insert(tr), "cycle in facility upgrade canvas");
            let bytes = sf.bytes_of(tr)?;
            let mut r = Reader::new(&bytes);
            let go = r.pptr64()?.path_id;
            r.seek(52)?;
            let n = r.count(12)?;
            let children = (0..n)
                .map(|_| r.pptr64().map(|p| p.path_id))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            r.pptr64()?;
            // Some levels group several full-screen Canvases below an ordinary Transform.
            let mut rect = if sf.object(tr).is_some_and(|o|o.class_id==224) {
                anchored(parent_rect,r.f32s::<10>()?)
            } else {parent_rect};
            let bytes = sf.bytes_of(go)?;
            let mut r = Reader::new(&bytes);
            let n = r.count(12)?;
            let components = (0..n)
                .map(|_| r.pptr64().map(|p| p.path_id))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            r.i32()?;
            let name = r.string()?;
            let path = if tr == root {
                rect = screen;
                String::new()
            } else {
                format!("{parent}/{name}")
            };
            if !include(&path) {continue;}
            if parent.is_empty() && sf.object(root).is_some_and(|o|o.class_id==c::TRANSFORM){rect=screen;}
            if parent == "/Window Main Menu/Button List" {
                rect = layout
                    .grids
                    .get(&parent)
                    .context("facility menu grid")?
                    .position(sibling, siblings);
            } else if let Some(grid) = layout.grids.get(&parent)
                && (name.starts_with("temp ") || name == "RawImage")
            {
                rect = grid.position(0, 1);
            }
            layout.rects.insert(path.clone(), rect);
            let count = children.len();
            for (index, child) in children.into_iter().enumerate().rev() {
                stack.push((child, path.clone(), rect, index, count));
            }
            let mut fit_height = false;
            for k in components.into_iter().filter(|&k| {
                sf.object(k)
                    .is_some_and(|o| o.class_id == c::MONO_BEHAVIOUR)
            }) {
                let bytes = sf.bytes_of(k)?;
                let mut r = Reader::new(&bytes);
                r.seek(16)?;
                let reference = self.reference(op, &file, &mut r)?;
                let (_, src) = self.unity.bytes(op, &reference)?;
                let class = Reader::new(&src).string()?;
                if class == "InputField"
                    && path.ends_with("/temp Image Facility Info/InputField Facility Name")
                {
                    r.seek(316)?;
                    layout.name_limit = usize::try_from(r.i32()?)?;
                }
                if class == "VerticalLayoutGroup" && path.ends_with("/Viewport/Parent Button") {
                    r.seek(52)?;
                    layout.list_spacing = r.f32()?;
                }
                if class == "ContentSizeFitter" {
                    r.seek(36)?;
                    fit_height = r.i32()? != 0;
                }
                if class == "GridLayoutGroup" {
                    layout
                        .grids
                        .insert(path.clone(), Grid::read(&mut r, rect, &path)?);
                    continue;
                }
                if !matches!(class.as_str(), "Image" | "Text" | "RawImage") {
                    continue;
                }
                let mut w = Widget {
                    path: path.clone(),
                    rect,
                    depth: 0,
                    asset: String::new(),
                    uv: [0., 0., 1., 1.],
                    label: None,
                    tint: [255; 4],
                    component: k,
                    toggle: None,
                    button: None,
                    collider: None,
                    slices: None,
                };
                r.seek(44)?;
                w.tint = r
                    .f32s::<4>()?
                    .map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
                r.seek(196)?;
                if class == "Text" {
                    let font = self.reference(op, &file, &mut r)?;
                    let size = r.i32()?.try_into()?;
                    r.seek(228)?;
                    let align = r.i32()?;
                    r.seek(252)?;
                    w.label = Some(Label {
                        font: self.font(op, host, client, font)?,
                        size,
                        align: (align % 3) as f32 / 2.,
                        vertical: (align / 3) as f32 / 2.,
                        clamp_lines: None,
                        spacing_y: 0.,
                        text: r.string()?,
                        measured_width: None,
                        ready: None,
                    });
                } else if class == "RawImage" {
                    if let Some(texture)=self.unity.resolve(op,&file,r.pptr64()?)? {w.asset=self.texture(op,host,client,texture)?.0;}
                    let [x, y, width, height] = r.f32s::<4>()?;
                    w.uv = [x, 1. - y - height, x + width, 1. - y];
                } else if let Some(sprite) = self.unity.resolve(op, &file, r.pptr64()?)? {
                    self.room_sprite(op, host, client, &mut w, sprite)?;
                }
                layout.base.push(w);
            }
            if let Some(g) = layout.grids.get_mut(&path) {
                g.fit_height = fit_height;
            }
        }
        Ok(layout)
    }
}
