// SPDX-License-Identifier: MPL-2.0
//! Binary FBX (7.x) → glTF.
//!
//! The parser reads the documented binary node-record layout (32- and 64-bit
//! record headers, zlib-compressed arrays). The converter walks the object
//! graph through `Connections`:
//! - `Model` nodes with the full FBX local transform
//!   `T·Roff·Rp·Rpre·R·Rpost⁻¹·Rp⁻¹·Soff·Sp·S·Sp⁻¹`, all six Euler orders, and
//!   geometric transforms baked into unskinned vertices;
//! - `Geometry` polygons (fan-triangulated) with normal/UV/material layers in
//!   every mapping and reference mode;
//! - `Material` colours/opacity and `Texture`/`Video` files (embedded or
//!   external);
//! - `Skin`/`Cluster` deformers as glTF skins;
//! - `AnimationStack`/`Layer`/`CurveNode`/`Curve` as glTF animations, sampled
//!   at the union of key times and decomposed per node.
//! The file's axis system and unit scale become one root node.
use super::{glb, Textures};
use anyhow::{bail, ensure, Context, Result};
use glam::{DMat4, DQuat, DVec3};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_ARRAY: usize = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
/// FBX time units per second.
const KTIME: f64 = 46_186_158_000.0;
const MAX_SAMPLES: usize = 20_000;

#[derive(Clone, Debug)]
pub enum Prop {
    I16(i16),
    Bool(bool),
    I32(i32),
    F32(f32),
    F64(f64),
    I64(i64),
    F32s(Vec<f32>),
    F64s(Vec<f64>),
    I64s(Vec<i64>),
    I32s(Vec<i32>),
    Bools(Vec<u8>),
    Str(String),
    Raw(Vec<u8>),
}

impl Prop {
    pub fn as_f64(&self) -> Option<f64> {
        Some(match self {
            Prop::I16(v) => *v as f64,
            Prop::Bool(v) => *v as u8 as f64,
            Prop::I32(v) => *v as f64,
            Prop::F32(v) => *v as f64,
            Prop::F64(v) => *v,
            Prop::I64(v) => *v as f64,
            _ => return None,
        })
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Prop::I64(v) => Some(*v),
            Prop::I32(v) => Some(*v as i64),
            Prop::I16(v) => Some(*v as i64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn f64s(&self) -> Option<Vec<f64>> {
        Some(match self {
            Prop::F64s(v) => v.clone(),
            Prop::F32s(v) => v.iter().map(|x| *x as f64).collect(),
            _ => return None,
        })
    }
    pub fn i64s(&self) -> Option<Vec<i64>> {
        Some(match self {
            Prop::I32s(v) => v.iter().map(|x| *x as i64).collect(),
            Prop::I64s(v) => v.clone(),
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    fn array_f64(&self, name: &str) -> Option<Vec<f64>> {
        self.child(name)?.props.first()?.f64s()
    }
    fn array_i64(&self, name: &str) -> Option<Vec<i64>> {
        self.child(name)?.props.first()?.i64s()
    }
    fn string(&self, name: &str) -> Option<&str> {
        self.child(name)?.props.first()?.as_str()
    }
    /// `Properties70` value(s) by property name.
    fn p70(&self, name: &str) -> Option<&[Prop]> {
        self.child("Properties70")?
            .children_named("P")
            .find(|p| p.props.first().and_then(Prop::as_str) == Some(name))
            .map(|p| p.props.get(4..).unwrap_or(&[]))
    }
    fn p70_f64(&self, name: &str) -> Option<f64> {
        self.p70(name)?.first()?.as_f64()
    }
    fn p70_vec(&self, name: &str) -> Option<DVec3> {
        let v = self.p70(name)?;
        Some(DVec3::new(
            v.first()?.as_f64()?,
            v.get(1)?.as_f64()?,
            v.get(2)?.as_f64()?,
        ))
    }
}

// ------------------------------------------------------------------ parser

struct Parser<'a> {
    data: &'a [u8],
    wide: bool,
}

impl Parser<'_> {
    fn slice(&self, at: usize, n: usize) -> Result<&[u8]> {
        self.data
            .get(at..at.checked_add(n).context("FBX offset overflow")?)
            .with_context(|| format!("FBX truncated at byte {at}"))
    }
    fn u32(&self, at: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.slice(at, 4)?.try_into()?))
    }
    fn u64(&self, at: usize) -> Result<u64> {
        Ok(u64::from_le_bytes(self.slice(at, 8)?.try_into()?))
    }
    fn word(&self, at: usize) -> Result<(u64, usize)> {
        if self.wide {
            Ok((self.u64(at)?, 8))
        } else {
            Ok((self.u32(at)? as u64, 4))
        }
    }

    /// Parse one record at `at`; `None` for the null terminator record.
    fn record(&self, at: usize, depth: usize) -> Result<(Option<Node>, usize)> {
        ensure!(depth < MAX_DEPTH, "FBX nesting too deep");
        let (end, w) = self.word(at)?;
        let (count, _) = self.word(at + w)?;
        let (list_len, _) = self.word(at + 2 * w)?;
        let name_len = *self.slice(at + 3 * w, 1)?.first().unwrap() as usize;
        let header = 3 * w + 1;
        if end == 0 {
            return Ok((None, at + header + name_len));
        }
        let end = usize::try_from(end)?;
        ensure!(
            end > at && end <= self.data.len(),
            "FBX record end out of range"
        );
        let name = String::from_utf8_lossy(self.slice(at + header, name_len)?).into_owned();
        let mut cursor = at + header + name_len;
        let props_end = cursor
            .checked_add(usize::try_from(list_len)?)
            .context("FBX property list overflow")?;
        ensure!(props_end <= end, "FBX property list exceeds record");
        let mut props = Vec::with_capacity(count.min(1024) as usize);
        for _ in 0..count {
            let (p, next) = self.prop(cursor)?;
            props.push(p);
            cursor = next;
        }
        ensure!(
            cursor == props_end,
            "FBX property list length mismatch in `{name}`"
        );
        let mut children = Vec::new();
        while cursor < end {
            let (child, next) = self.record(cursor, depth + 1)?;
            ensure!(next > cursor, "FBX record did not advance");
            cursor = next;
            match child {
                Some(c) => children.push(c),
                None => break,
            }
        }
        Ok((
            Some(Node {
                name,
                props,
                children,
            }),
            end,
        ))
    }

    fn prop(&self, at: usize) -> Result<(Prop, usize)> {
        let kind = self.slice(at, 1)?[0];
        let at = at + 1;
        let scalar = |n: usize| self.slice(at, n);
        Ok(match kind {
            b'Y' => (
                Prop::I16(i16::from_le_bytes(scalar(2)?.try_into()?)),
                at + 2,
            ),
            b'C' => (Prop::Bool(scalar(1)?[0] != 0), at + 1),
            b'I' => (
                Prop::I32(i32::from_le_bytes(scalar(4)?.try_into()?)),
                at + 4,
            ),
            b'F' => (
                Prop::F32(f32::from_le_bytes(scalar(4)?.try_into()?)),
                at + 4,
            ),
            b'D' => (
                Prop::F64(f64::from_le_bytes(scalar(8)?.try_into()?)),
                at + 8,
            ),
            b'L' => (
                Prop::I64(i64::from_le_bytes(scalar(8)?.try_into()?)),
                at + 8,
            ),
            b'S' | b'R' => {
                let n = self.u32(at)? as usize;
                let bytes = self.slice(at + 4, n)?;
                let p = if kind == b'S' {
                    Prop::Str(String::from_utf8_lossy(bytes).into_owned())
                } else {
                    Prop::Raw(bytes.to_vec())
                };
                (p, at + 4 + n)
            }
            b'f' | b'd' | b'l' | b'i' | b'b' => {
                let len = self.u32(at)? as usize;
                let encoding = self.u32(at + 4)?;
                let stored = self.u32(at + 8)? as usize;
                let elem = match kind {
                    b'd' | b'l' => 8,
                    b'f' | b'i' => 4,
                    _ => 1,
                };
                let bytes = len.checked_mul(elem).context("FBX array size overflow")?;
                ensure!(bytes <= MAX_ARRAY, "FBX array exceeds {MAX_ARRAY} bytes");
                let raw = self.slice(at + 12, stored)?;
                let data = match encoding {
                    0 => raw.to_vec(),
                    1 => {
                        use std::io::Read;
                        let mut out = Vec::with_capacity(bytes);
                        flate2::read::ZlibDecoder::new(raw)
                            .take(bytes as u64 + 1)
                            .read_to_end(&mut out)?;
                        out
                    }
                    e => bail!("FBX array encoding {e}"),
                };
                ensure!(data.len() == bytes, "FBX array length mismatch");
                let p = match kind {
                    b'f' => Prop::F32s(
                        data.chunks_exact(4)
                            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                            .collect(),
                    ),
                    b'd' => Prop::F64s(
                        data.chunks_exact(8)
                            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                            .collect(),
                    ),
                    b'l' => Prop::I64s(
                        data.chunks_exact(8)
                            .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
                            .collect(),
                    ),
                    b'i' => Prop::I32s(
                        data.chunks_exact(4)
                            .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                            .collect(),
                    ),
                    _ => Prop::Bools(data),
                };
                (p, at + 12 + stored)
            }
            k => bail!("FBX property type {:?} at byte {}", k as char, at - 1),
        })
    }
}

/// Parse a binary FBX file into its top-level records.
pub fn parse(bytes: &[u8]) -> Result<(u32, Vec<Node>)> {
    ensure!(
        bytes.starts_with(b"Kaydara FBX Binary  \0"),
        "not a binary FBX file"
    );
    let version = u32::from_le_bytes(
        bytes
            .get(23..27)
            .context("FBX header truncated")?
            .try_into()?,
    );
    ensure!(
        (7000..8000).contains(&version),
        "FBX version {version} unsupported (7.x required)"
    );
    let p = Parser {
        data: bytes,
        wide: version >= 7500,
    };
    let mut at = 27;
    let mut nodes = Vec::new();
    while at < bytes.len() {
        let (node, next) = p.record(at, 0)?;
        match node {
            Some(n) => nodes.push(n),
            None => break,
        }
        at = next;
    }
    Ok((version, nodes))
}

// -------------------------------------------------------------- converter

fn object_name(node: &Node) -> String {
    let raw = node.props.get(1).and_then(Prop::as_str).unwrap_or("");
    raw.split("\u{0}\u{1}").next().unwrap_or(raw).to_owned()
}

fn object_class(node: &Node) -> &str {
    node.props.get(2).and_then(Prop::as_str).unwrap_or("")
}

fn euler(order: i64, degrees: DVec3) -> DQuat {
    let r = degrees * std::f64::consts::PI / 180.;
    // Order "ABC" applies A first, so the matrix is C·B·A.
    let (rx, ry, rz) = (
        DQuat::from_rotation_x(r.x),
        DQuat::from_rotation_y(r.y),
        DQuat::from_rotation_z(r.z),
    );
    match order {
        0 => rz * ry * rx,
        1 => ry * rz * rx,
        2 => rx * rz * ry,
        3 => rz * rx * ry,
        4 => ry * rx * rz,
        5 => rx * ry * rz,
        _ => rz * ry * rx,
    }
}

#[derive(Clone)]
struct Xform {
    t: DVec3,
    r: DVec3,
    s: DVec3,
    pre: DVec3,
    post: DVec3,
    roff: DVec3,
    rp: DVec3,
    soff: DVec3,
    sp: DVec3,
    order: i64,
}

impl Xform {
    fn of(model: &Node) -> Self {
        let v = |n: &str, d: DVec3| model.p70_vec(n).unwrap_or(d);
        Self {
            t: v("Lcl Translation", DVec3::ZERO),
            r: v("Lcl Rotation", DVec3::ZERO),
            s: v("Lcl Scaling", DVec3::ONE),
            pre: v("PreRotation", DVec3::ZERO),
            post: v("PostRotation", DVec3::ZERO),
            roff: v("RotationOffset", DVec3::ZERO),
            rp: v("RotationPivot", DVec3::ZERO),
            soff: v("ScalingOffset", DVec3::ZERO),
            sp: v("ScalingPivot", DVec3::ZERO),
            order: model.p70_f64("RotationOrder").unwrap_or(0.) as i64,
        }
    }
    fn matrix(&self) -> DMat4 {
        let t = DMat4::from_translation;
        t(self.t)
            * t(self.roff)
            * t(self.rp)
            * DMat4::from_quat(euler(0, self.pre))
            * DMat4::from_quat(euler(self.order, self.r))
            * DMat4::from_quat(euler(0, self.post)).inverse()
            * t(-self.rp)
            * t(self.soff)
            * t(self.sp)
            * DMat4::from_scale(self.s)
            * t(-self.sp)
    }
}

fn geometric(model: &Node) -> DMat4 {
    let t = model.p70_vec("GeometricTranslation").unwrap_or(DVec3::ZERO);
    let r = model.p70_vec("GeometricRotation").unwrap_or(DVec3::ZERO);
    let s = model.p70_vec("GeometricScaling").unwrap_or(DVec3::ONE);
    DMat4::from_scale_rotation_translation(s, euler(0, r), t)
}

fn trs(m: DMat4) -> ([f32; 3], [f32; 4], [f32; 3]) {
    let (s, r, t) = m.to_scale_rotation_translation();
    let r = r.normalize();
    (
        t.as_vec3().to_array(),
        r.as_quat().to_array(),
        s.as_vec3().to_array(),
    )
}

/// Map an FBX layer element value to polygon-vertex order.
fn layer_index(
    mapping: &str,
    reference: &str,
    index: Option<&[i64]>,
    polygon_vertex: usize,
    control_point: usize,
    polygon: usize,
) -> Result<usize> {
    let direct = match mapping {
        "ByPolygonVertex" => polygon_vertex,
        "ByVertice" | "ByVertex" | "ByControlPoint" => control_point,
        "ByPolygon" => polygon,
        "AllSame" => 0,
        m => bail!("FBX layer mapping `{m}` unsupported"),
    };
    match reference {
        "Direct" => Ok(direct),
        "IndexToDirect" | "Index" => {
            let i = *index
                .context("FBX IndexToDirect layer without index array")?
                .get(direct)
                .context("FBX layer index out of range")?;
            usize::try_from(i).context("negative FBX layer index")
        }
        r => bail!("FBX layer reference `{r}` unsupported"),
    }
}

struct Scene<'a> {
    objects: HashMap<i64, &'a Node>,
    /// child → [(parent, property)]
    parents: HashMap<i64, Vec<(i64, Option<String>)>>,
    /// parent → [(child, property)] in file order
    children: HashMap<i64, Vec<(i64, Option<String>)>>,
}

impl<'a> Scene<'a> {
    fn kids(&self, id: i64, kind: &str) -> Vec<i64> {
        self.children
            .get(&id)
            .into_iter()
            .flatten()
            .filter(|(c, _)| self.objects.get(c).is_some_and(|n| n.name == kind))
            .map(|(c, _)| *c)
            .collect()
    }
    fn kids_with(&self, id: i64, kind: &str) -> Vec<(i64, Option<String>)> {
        self.children
            .get(&id)
            .into_iter()
            .flatten()
            .filter(|(c, _)| self.objects.get(c).is_some_and(|n| n.name == kind))
            .cloned()
            .collect()
    }
    fn parent_of(&self, id: i64, kind: &str) -> Option<i64> {
        self.parents
            .get(&id)?
            .iter()
            .find(|(p, _)| self.objects.get(p).is_some_and(|n| n.name == kind))
            .map(|(p, _)| *p)
    }
}

/// Euler curves for one model property (T/R/S), per axis: (times, values).
type Curves = [Option<(Vec<i64>, Vec<f64>)>; 3];

fn sample(curve: &(Vec<i64>, Vec<f64>), t: i64) -> f64 {
    let (times, values) = curve;
    match times.binary_search(&t) {
        Ok(i) => values[i],
        Err(0) => values[0],
        Err(i) if i >= times.len() => values[times.len() - 1],
        Err(i) => {
            let (t0, t1) = (times[i - 1], times[i]);
            let f = (t - t0) as f64 / (t1 - t0) as f64;
            values[i - 1] + (values[i] - values[i - 1]) * f
        }
    }
}

pub fn convert(
    bytes: &[u8],
    external: &mut dyn FnMut(&str) -> Result<Vec<u8>>,
) -> Result<glb::Document> {
    let (version, top) = parse(bytes)?;
    let mut doc = glb::Document {
        generator: format!("CrySirror FBX {version} importer"),
        ..Default::default()
    };
    let mut textures = Textures::new(external);
    let objects_node = top
        .iter()
        .find(|n| n.name == "Objects")
        .context("FBX has no Objects")?;
    let mut scene = Scene {
        objects: HashMap::new(),
        parents: HashMap::new(),
        children: HashMap::new(),
    };
    for o in &objects_node.children {
        if let Some(id) = o.props.first().and_then(Prop::as_i64) {
            scene.objects.insert(id, o);
        }
    }
    if let Some(conns) = top.iter().find(|n| n.name == "Connections") {
        for c in conns.children_named("C") {
            let kind = c.props.first().and_then(Prop::as_str).unwrap_or("");
            let (Some(child), Some(parent)) = (
                c.props.get(1).and_then(Prop::as_i64),
                c.props.get(2).and_then(Prop::as_i64),
            ) else {
                continue;
            };
            let prop = (kind == "OP").then(|| {
                c.props
                    .get(3)
                    .and_then(Prop::as_str)
                    .unwrap_or("")
                    .to_owned()
            });
            scene
                .parents
                .entry(child)
                .or_default()
                .push((parent, prop.clone()));
            scene
                .children
                .entry(parent)
                .or_default()
                .push((child, prop));
        }
    }

    // Axis system and unit scale → one conversion root.
    let globals = top.iter().find(|n| n.name == "GlobalSettings");
    let gi = |name: &str, d: f64| globals.and_then(|g| g.p70_f64(name)).unwrap_or(d);
    let axis = |a: f64, s: f64| {
        let mut v = DVec3::ZERO;
        v[(a as usize).min(2)] = if s < 0. { -1. } else { 1. };
        v
    };
    let up = axis(gi("UpAxis", 1.), gi("UpAxisSign", 1.));
    let front = axis(gi("FrontAxis", 2.), gi("FrontAxisSign", 1.));
    let right = axis(gi("CoordAxis", 0.), gi("CoordAxisSign", 1.));
    let basis = glam::DMat3::from_cols(right, up, front).transpose();
    ensure!(
        basis.determinant().abs() > 0.5,
        "FBX axis system is degenerate"
    );
    let mut basis = basis;
    if basis.determinant() < 0. {
        // A left-handed declaration: mirror front so the result is a rotation.
        basis = glam::DMat3::from_cols(basis.x_axis, basis.y_axis, -basis.z_axis);
        doc.dropped
            .push("left-handed FBX axis declaration treated as right-handed".into());
    }
    let unit = gi("UnitScaleFactor", 1.) / 100.;
    let root_matrix = DMat4::from_mat3(basis) * DMat4::from_scale(DVec3::splat(unit));

    // Models in a stable order (object order in the file).
    let mut model_ids: Vec<i64> = objects_node
        .children
        .iter()
        .filter(|o| o.name == "Model")
        .filter_map(|o| o.props.first().and_then(Prop::as_i64))
        .collect();
    model_ids.dedup();
    let mut node_of: BTreeMap<i64, usize> = BTreeMap::new();
    let mut xforms: BTreeMap<i64, Xform> = BTreeMap::new();
    for id in &model_ids {
        let m = scene.objects[id];
        let x = Xform::of(m);
        let (t, r, s) = trs(x.matrix());
        let mut node = glb::Node::named(object_name(m));
        (node.translation, node.rotation, node.scale) = (t, r, s);
        doc.nodes.push(node);
        node_of.insert(*id, doc.nodes.len() - 1);
        xforms.insert(*id, x);
    }
    let root = doc.nodes.len();
    let (t, r, s) = trs(root_matrix);
    let mut root_node = glb::Node::named("fbx_root");
    (root_node.translation, root_node.rotation, root_node.scale) = (t, r, s);
    doc.nodes.push(root_node);
    doc.roots.push(root);
    for id in &model_ids {
        let parent = scene
            .parent_of(*id, "Model")
            .filter(|p| node_of.contains_key(p));
        let n = node_of[id];
        match parent {
            Some(p) => doc.nodes[node_of[&p]].children.push(n),
            None => doc.nodes[root].children.push(n),
        }
    }
    // Global rest matrices (in FBX space, without the conversion root).
    let mut global: BTreeMap<i64, DMat4> = BTreeMap::new();
    fn global_of(
        id: i64,
        scene: &Scene,
        xforms: &BTreeMap<i64, Xform>,
        cache: &mut BTreeMap<i64, DMat4>,
        depth: usize,
    ) -> DMat4 {
        if let Some(m) = cache.get(&id) {
            return *m;
        }
        let local = xforms
            .get(&id)
            .map(Xform::matrix)
            .unwrap_or(DMat4::IDENTITY);
        let m = match scene
            .parent_of(id, "Model")
            .filter(|p| xforms.contains_key(p) && depth < 256)
        {
            Some(p) => global_of(p, scene, xforms, cache, depth + 1) * local,
            None => local,
        };
        cache.insert(id, m);
        m
    }
    for id in &model_ids {
        global_of(*id, &scene, &xforms, &mut global, 0);
    }

    // Bind pose: node id → global bind matrix in FBX space.
    let mut bind_pose: BTreeMap<i64, DMat4> = BTreeMap::new();
    for pose in objects_node
        .children
        .iter()
        .filter(|o| o.name == "Pose" && object_class(o) == "BindPose")
    {
        for pn in pose.children_named("PoseNode") {
            let (Some(id), Some(m)) = (
                pn.child("Node")
                    .and_then(|n| n.props.first())
                    .and_then(Prop::as_i64),
                pn.array_f64("Matrix"),
            ) else {
                continue;
            };
            if m.len() == 16 {
                bind_pose.insert(id, DMat4::from_cols_slice(&m));
            }
        }
    }

    // Materials (shared across meshes).
    let mut material_index: BTreeMap<i64, usize> = BTreeMap::new();
    let mut material_for = |doc: &mut glb::Document, textures: &mut Textures, id: i64| -> usize {
        if let Some(i) = material_index.get(&id) {
            return *i;
        }
        let m = scene.objects[&id];
        let diffuse = m
            .p70_vec("DiffuseColor")
            .or_else(|| m.p70_vec("Diffuse"))
            .unwrap_or(DVec3::splat(0.8));
        let factor = m.p70_f64("DiffuseFactor").unwrap_or(1.);
        let opacity = m
            .p70_f64("Opacity")
            .or_else(|| m.p70_f64("TransparencyFactor").map(|t| 1. - t))
            .unwrap_or(1.)
            .clamp(0., 1.);
        let emissive = m.p70_vec("EmissiveColor").unwrap_or(DVec3::ZERO)
            * m.p70_f64("EmissiveFactor").unwrap_or(1.);
        let shininess = m
            .p70_f64("ShininessExponent")
            .or_else(|| m.p70_f64("Shininess"))
            .unwrap_or(20.)
            .max(0.);
        let mut texture = None;
        for (tex, prop) in scene.kids_with(id, "Texture") {
            let slot = prop.as_deref().unwrap_or("");
            let t = scene.objects[&tex];
            let path = t
                .string("RelativeFilename")
                .filter(|s| !s.is_empty())
                .or_else(|| t.string("FileName"))
                .unwrap_or("")
                .to_owned();
            let content = scene.kids(tex, "Video").into_iter().find_map(|v| {
                match scene.objects[&v].child("Content")?.props.first()? {
                    Prop::Raw(b) if !b.is_empty() => Some(b.clone()),
                    _ => None,
                }
            });
            match slot {
                "DiffuseColor"
                | "Maya|baseColor"
                | "3dsMax|Parameters|base_color_map"
                | "Diffuse" => {
                    texture = textures.embed(doc, &path, content.as_deref());
                }
                other => doc.dropped.push(format!(
                    "material `{}` texture slot `{other}`",
                    object_name(m)
                )),
            }
        }
        let alpha = opacity < 0.999;
        let cutout = !alpha && texture.is_some_and(|t| Textures::has_alpha(doc, t));
        let color = if texture.is_some() {
            DVec3::ONE
        } else {
            diffuse * factor
        };
        doc.materials.push(glb::Material {
            name: object_name(m),
            base_color: [
                color.x as f32,
                color.y as f32,
                color.z as f32,
                opacity as f32,
            ],
            base_color_texture: texture,
            emissive: emissive.as_vec3().min(glam::Vec3::ONE).to_array(),
            roughness: (2. / (shininess + 2.)).sqrt().clamp(0.05, 1.) as f32,
            alpha_mode: if alpha {
                "BLEND"
            } else if cutout {
                "MASK"
            } else {
                "OPAQUE"
            },
            ..Default::default()
        });
        let i = doc.materials.len() - 1;
        material_index.insert(id, i);
        i
    };

    // Meshes.
    let mut blend_shapes = 0usize;
    for model_id in &model_ids {
        let model = scene.objects[model_id];
        let Some(geom_id) = scene
            .kids(*model_id, "Geometry")
            .into_iter()
            .find(|g| object_class(scene.objects[g]) == "Mesh")
        else {
            let class = object_class(model);
            if matches!(class, "Camera" | "Light") {
                doc.dropped
                    .push(format!("{} `{}`", class.to_lowercase(), object_name(model)));
            }
            continue;
        };
        let g = scene.objects[&geom_id];
        let cps = g
            .array_f64("Vertices")
            .context("FBX mesh without Vertices")?;
        ensure!(cps.len() % 3 == 0, "FBX vertex array length");
        let cp: Vec<DVec3> = cps
            .chunks_exact(3)
            .map(|c| DVec3::new(c[0], c[1], c[2]))
            .collect();
        let pvi = g
            .array_i64("PolygonVertexIndex")
            .context("FBX mesh without PolygonVertexIndex")?;
        let normals = g.child("LayerElementNormal");
        let uvs = g.child("LayerElementUV");
        let mats = g.child("LayerElementMaterial");
        let layer = |l: Option<&Node>, data: &str, index: &str| {
            l.map(|l| {
                (
                    l.string("MappingInformationType")
                        .unwrap_or("ByPolygonVertex")
                        .to_owned(),
                    l.string("ReferenceInformationType")
                        .unwrap_or("Direct")
                        .to_owned(),
                    l.array_f64(data).unwrap_or_default(),
                    l.array_i64(index),
                )
            })
        };
        let normal_layer = layer(normals, "Normals", "NormalsIndex");
        let uv_layer = layer(uvs, "UV", "UVIndex");
        let material_layer = mats.map(|l| {
            (
                l.string("MappingInformationType")
                    .unwrap_or("AllSame")
                    .to_owned(),
                l.array_i64("Materials").unwrap_or_default(),
            )
        });
        let model_materials = scene.kids(*model_id, "Material");
        blend_shapes += scene
            .kids(geom_id, "Deformer")
            .iter()
            .filter(|d| object_class(scene.objects[d]) == "BlendShape")
            .count();

        // Skin: control point → (joint slot, weight).
        let skin = scene
            .kids(geom_id, "Deformer")
            .into_iter()
            .find(|d| object_class(scene.objects[d]) == "Skin");
        let mut cp_weights: Vec<Vec<(u16, f32)>> = vec![vec![]; cp.len()];
        let mut joints = Vec::new();
        let mut inverse_bind = Vec::new();
        let geo = geometric(model);
        if let Some(skin) = skin {
            for cluster in scene.kids(skin, "Deformer") {
                let c = scene.objects[&cluster];
                let Some(bone) = scene
                    .kids(cluster, "Model")
                    .into_iter()
                    .next()
                    .filter(|b| node_of.contains_key(b))
                else {
                    continue;
                };
                let (Some(idx), Some(w)) = (c.array_i64("Indexes"), c.array_f64("Weights")) else {
                    continue;
                };
                let mat = |name: &str| {
                    c.array_f64(name)
                        .filter(|v| v.len() == 16)
                        .map(|v| DMat4::from_cols_slice(&v))
                };
                // The cluster's `Transform` field is not written consistently
                // (Blender stores it bone-relative), so the mesh bind matrix
                // comes from the bind pose or the mesh's rest transform.
                let link = mat("TransformLink")
                    .or_else(|| bind_pose.get(&bone).copied())
                    .unwrap_or_else(|| global[&bone]);
                let mesh_bind = bind_pose
                    .get(model_id)
                    .copied()
                    .unwrap_or_else(|| global[model_id]);
                ensure!(
                    joints.len() < crate::anim::MAX_JOINTS,
                    "FBX skin exceeds {} joints",
                    crate::anim::MAX_JOINTS
                );
                let slot = joints.len() as u16;
                joints.push(node_of[&bone]);
                inverse_bind.push((link.inverse() * mesh_bind * geo).as_mat4().to_cols_array());
                for (i, w) in idx.iter().zip(w) {
                    if let Some(list) = usize::try_from(*i).ok().and_then(|i| cp_weights.get_mut(i))
                    {
                        if w > 0. {
                            list.push((slot, w as f32));
                        }
                    }
                }
            }
        }
        let skinned = !joints.is_empty();
        let mut dropped_influences = 0;

        // Triangulate polygons into per-material primitives with welded corners.
        let mut prims: BTreeMap<usize, (glb::Primitive, HashMap<(usize, [u32; 5]), u32>)> =
            BTreeMap::new();
        let mut polygon = 0usize;
        let mut pv = 0usize;
        let mut corners: Vec<(usize, usize)> = Vec::new(); // (control point, polygon-vertex index)
        while pv < pvi.len() {
            let raw = pvi[pv];
            let (index, last) = if raw < 0 {
                ((!raw) as usize, true)
            } else {
                (raw as usize, false)
            };
            ensure!(index < cp.len(), "FBX polygon vertex {index} out of range");
            corners.push((index, pv));
            pv += 1;
            if !last {
                continue;
            }
            let local_material = match &material_layer {
                Some((mapping, list)) if mapping == "ByPolygon" => {
                    list.get(polygon).copied().unwrap_or(0)
                }
                Some((_, list)) => list.first().copied().unwrap_or(0),
                None => 0,
            };
            let material = usize::try_from(local_material)
                .ok()
                .and_then(|i| model_materials.get(i))
                .map(|id| material_for(&mut doc, &mut textures, *id));
            let key = material.map_or(usize::MAX, |m| m);
            let (prim, weld) = prims.entry(key).or_insert_with(|| {
                (
                    glb::Primitive {
                        material,
                        ..Default::default()
                    },
                    HashMap::new(),
                )
            });
            let mut ids = Vec::with_capacity(corners.len());
            for &(c, pvx) in &corners {
                let normal = match &normal_layer {
                    Some((m, r, data, idx)) => {
                        let i = layer_index(m, r, idx.as_deref(), pvx, c, polygon)?;
                        let n = data
                            .get(i * 3..i * 3 + 3)
                            .context("FBX normal index out of range")?;
                        DVec3::new(n[0], n[1], n[2])
                    }
                    None => DVec3::ZERO,
                };
                let uv = match &uv_layer {
                    Some((m, r, data, idx)) => {
                        let i = layer_index(m, r, idx.as_deref(), pvx, c, polygon)?;
                        let t = data
                            .get(i * 2..i * 2 + 2)
                            .context("FBX UV index out of range")?;
                        [t[0] as f32, 1. - t[1] as f32]
                    }
                    None => [0., 0.],
                };
                let (pos, nrm) = if skinned {
                    (cp[c], normal)
                } else {
                    (geo.transform_point3(cp[c]), geo.transform_vector3(normal))
                };
                let nrm = if nrm.length_squared() > 1e-18 {
                    nrm.normalize()
                } else {
                    DVec3::Y
                };
                let key = (
                    c,
                    [nrm.x as f32, nrm.y as f32, nrm.z as f32, uv[0], uv[1]].map(f32::to_bits),
                );
                let next = prim.positions.len() as u32;
                let id = *weld.entry(key).or_insert_with(|| {
                    prim.positions.push(pos.as_vec3().to_array());
                    prim.normals.push(nrm.as_vec3().to_array());
                    prim.uvs.push(uv);
                    if skinned {
                        let mut list = cp_weights[c].clone();
                        list.sort_by(|a, b| b.1.total_cmp(&a.1));
                        if list.len() > 4 {
                            dropped_influences += 1;
                        }
                        let mut j = [0u16; 4];
                        let mut w = [0f32; 4];
                        for (k, (slot, weight)) in list.iter().take(4).enumerate() {
                            j[k] = *slot;
                            w[k] = *weight;
                        }
                        let sum: f32 = w.iter().sum();
                        if sum > 0. {
                            w.iter_mut().for_each(|x| *x /= sum);
                        } else {
                            w = [1., 0., 0., 0.];
                        }
                        prim.joints.push(j);
                        prim.weights.push(w);
                    }
                    next
                });
                ids.push(id);
            }
            for k in 1..ids.len().saturating_sub(1) {
                prim.indices
                    .extend_from_slice(&[ids[0], ids[k], ids[k + 1]]);
            }
            corners.clear();
            polygon += 1;
        }
        ensure!(
            corners.is_empty(),
            "FBX polygon list ends without a terminator"
        );
        let prims: Vec<glb::Primitive> = prims
            .into_values()
            .map(|(p, _)| p)
            .filter(|p| !p.indices.is_empty())
            .collect();
        if prims.is_empty() {
            continue;
        }
        if normal_layer.is_none() {
            doc.dropped.push(format!(
                "mesh `{}` has no normals (flat up normals used)",
                object_name(model)
            ));
        }
        if dropped_influences > 0 {
            doc.dropped.push(format!(
                "{dropped_influences} vertices of `{}` keep their 4 strongest bone influences",
                object_name(model)
            ));
        }
        doc.meshes.push((object_name(model), prims));
        let mesh = doc.meshes.len() - 1;
        if skinned {
            // glTF ignores a skinned mesh node's transform; attach it at the root.
            let mut n = glb::Node::named(format!("{}_skinned", object_name(model)));
            n.mesh = Some(mesh);
            doc.skins.push(glb::Skin {
                joints,
                inverse_bind,
                skeleton: None,
            });
            n.skin = Some(doc.skins.len() - 1);
            doc.nodes.push(n);
            let id = doc.nodes.len() - 1;
            doc.nodes[root].children.push(id);
        } else {
            doc.nodes[node_of[model_id]].mesh = Some(mesh);
        }
    }
    if blend_shapes > 0 {
        doc.dropped.push(format!(
            "{blend_shapes} blend shape deformer(s) (engine morph targets pending)"
        ));
    }

    // Animation stacks → glTF animations.
    for stack in objects_node
        .children
        .iter()
        .filter(|o| o.name == "AnimationStack")
    {
        let stack_id = stack.props.first().and_then(Prop::as_i64).unwrap_or(0);
        let layers = scene.kids(stack_id, "AnimationLayer");
        if layers.len() > 1 {
            doc.dropped.push(format!(
                "animation `{}`: {} extra layer(s)",
                object_name(stack),
                layers.len() - 1
            ));
        }
        let Some(layer) = layers.first() else {
            continue;
        };
        let mut per_model: BTreeMap<i64, [Curves; 3]> = BTreeMap::new();
        for curve_node in scene.kids(*layer, "AnimationCurveNode") {
            let Some((model, prop)) =
                scene
                    .parents
                    .get(&curve_node)
                    .into_iter()
                    .flatten()
                    .find(|(p, prop)| {
                        scene.objects.get(p).is_some_and(|n| n.name == "Model") && prop.is_some()
                    })
            else {
                continue;
            };
            let channel = match prop.as_deref() {
                Some("Lcl Translation") => 0,
                Some("Lcl Rotation") => 1,
                Some("Lcl Scaling") => 2,
                _ => continue,
            };
            let entry = per_model.entry(*model).or_insert_with(Default::default);
            for (curve, axis) in scene.kids_with(curve_node, "AnimationCurve") {
                let a = match axis.as_deref() {
                    Some("d|X") => 0,
                    Some("d|Y") => 1,
                    Some("d|Z") => 2,
                    _ => continue,
                };
                let c = scene.objects[&curve];
                let (Some(times), Some(values)) =
                    (c.array_i64("KeyTime"), c.array_f64("KeyValueFloat"))
                else {
                    continue;
                };
                ensure!(
                    times.len() == values.len() && !times.is_empty(),
                    "FBX curve key count"
                );
                ensure!(
                    times.windows(2).all(|w| w[0] < w[1]),
                    "FBX curve times not increasing"
                );
                entry[channel][a] = Some((times, values));
            }
        }
        let mut tracks = Vec::new();
        for (model, curves) in per_model {
            let Some(base) = xforms.get(&model) else {
                continue;
            };
            let mut times: BTreeSet<i64> = BTreeSet::new();
            for c in curves.iter().flatten().flatten() {
                times.extend(c.0.iter().copied());
            }
            if times.is_empty() {
                continue;
            }
            ensure!(
                times.len() <= MAX_SAMPLES,
                "FBX animation exceeds {MAX_SAMPLES} samples per node"
            );
            let origin = *times.first().unwrap();
            let mut tr = Vec::new();
            let mut rot: Vec<[f32; 4]> = Vec::new();
            let mut sc = Vec::new();
            let mut secs = Vec::new();
            for t in &times {
                let mut x = base.clone();
                for (channel, target) in [&mut x.t, &mut x.r, &mut x.s].into_iter().enumerate() {
                    for a in 0..3 {
                        if let Some(c) = &curves[channel][a] {
                            target[a] = sample(c, *t);
                        }
                    }
                }
                let (t3, mut r4, s3) = trs(x.matrix());
                if let Some(prev) = rot.last() {
                    if prev.iter().zip(&r4).map(|(a, b)| a * b).sum::<f32>() < 0. {
                        r4 = r4.map(|v| -v);
                    }
                }
                tr.push(t3);
                rot.push(r4);
                sc.push(s3);
                secs.push(((t - origin) as f64 / KTIME) as f32);
            }
            let node = node_of[&model];
            tracks.push(glb::Track {
                node,
                times: secs.clone(),
                values: glb::Channel::Translation(tr),
            });
            tracks.push(glb::Track {
                node,
                times: secs.clone(),
                values: glb::Channel::Rotation(rot),
            });
            tracks.push(glb::Track {
                node,
                times: secs,
                values: glb::Channel::Scale(sc),
            });
        }
        if !tracks.is_empty() {
            doc.animations.push(glb::Animation {
                name: object_name(stack),
                tracks,
            });
        }
    }
    if doc.meshes.is_empty() {
        bail!("FBX file contains no polygon meshes");
    }
    if !doc.animations.is_empty() {
        doc.dropped.push("FBX animation curves are sampled at their key times with linear interpolation (tangents ignored)".into());
    }
    Ok(doc)
}
