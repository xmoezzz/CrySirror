// SPDX-License-Identifier: MPL-2.0
//! Unity scene/prefab object graphs of the player data, as VM data.
//!
//! A scene file (`mainData` for build index 0, `level<N-1>` for index N) is
//! loaded into GameObjects with their Transform hierarchy and components.
//! MonoBehaviour fields decode under the managed schema generated from the
//! game's script declarations (`data/unity_schema.json`). References into
//! shared asset files stay as `(file, path id)` keys; resolving them is the
//! caller's resource policy.
use anyhow::{Context, Result};
use crysirror_core::service::AssetReader;
use crysirror_unity::{
    class_id as c,
    classes::{Camera, GameObject, MonoBehaviour, MonoScript, Transform},
    managed::{Fields, Schema},
    PPtr, SerializedFile,
};
use std::{collections::HashMap, sync::Arc};

pub static SCHEMA_JSON: &str = include_str!("../data/unity_schema.json");

/// A global object key: (file name, path id).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectKey {
    pub file: Arc<str>,
    pub path_id: i64,
}

#[derive(Clone, Debug)]
pub enum Component {
    Transform,
    Camera(Camera),
    Script { class: String, enabled: bool, fields: Fields },
    /// A class without a reader here (colliders, lights, animations, ...).
    Other { class_id: i32 },
}

#[derive(Clone, Debug)]
pub struct Node {
    pub key: ObjectKey,
    pub name: String,
    pub active: bool,
    pub layer: u32,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub components: Vec<(ObjectKey, Component)>,
    /// `Object.Instantiate` copy this node belongs to (1 + the copy's root
    /// index), 0 for loaded nodes; copies keep the prefab's object keys.
    pub instance: usize,
}

/// Opens Unity files of the player data directory by name.
pub trait UnityFiles {
    fn open(&mut self, name: &str) -> Result<Arc<dyn AssetReader>>;
}

/// Loaded serialized files plus script-name resolution.
pub struct UnityData {
    files: HashMap<Arc<str>, Arc<SerializedFile>>,
    /// `Resources.Load` paths (lower case) of `mainData`'s ResourceManager.
    resources: Option<HashMap<String, Vec<ObjectKey>>>,
    scripts: HashMap<ObjectKey, String>,
    pub schema: Schema,
}

impl UnityData {
    pub fn new() -> Result<Self> {
        Ok(Self { files: HashMap::new(), resources: None, scripts: HashMap::new(), schema: Schema::parse(SCHEMA_JSON)? })
    }

    pub fn file(&mut self, opener: &mut dyn UnityFiles, name: &str) -> Result<Arc<SerializedFile>> {
        let name = name.rsplit('/').next().unwrap_or(name);
        if let Some(f) = self.files.get(name) {
            return Ok(f.clone());
        }
        let source = match name {
            "builtin:unity default resources" => "Resources/unity default resources",
            "builtin:unity_builtin_extra" => "Resources/unity_builtin_extra",
            _ => name,
        };
        let reader = opener.open(source).with_context(|| format!("open Unity file {source:?}"))?;
        let file = Arc::new(SerializedFile::open(reader).with_context(|| format!("Unity file {source}"))?);
        self.files.insert(Arc::from(name), file.clone());
        Ok(file)
    }

    /// `Resources.Load(path)`: objects registered under `path` (case-insensitive).
    pub fn resource(&mut self, opener: &mut dyn UnityFiles, path: &str) -> Result<Vec<ObjectKey>> {
        if self.resources.is_none() {
            let owner = if self.files.contains_key("globalgamemanagers") { "globalgamemanagers" } else { "mainData" };
            let main = self.file(opener, owner)?;
            let info = *main
                .objects
                .iter()
                .find(|o| o.class_id == c::RESOURCE_MANAGER)
                .context("mainData has no ResourceManager")?;
            let mut map: HashMap<String, Vec<ObjectKey>> = HashMap::new();
            let bytes = main.bytes(&info)?;
            let container = match main.format {
                9 => crysirror_unity::classes::resource_container(&bytes)?,
                17 => crysirror_unity::classes::resource_container_64(&bytes)?,
                other => anyhow::bail!("unsupported ResourceManager format {other}"),
            };
            for (name, p) in container {
                if let Some(k) = self.resolve(opener, owner, p)? {
                    map.entry(name.to_ascii_lowercase()).or_default().push(k);
                }
            }
            self.resources = Some(map);
        }
        Ok(self.resources.as_ref().unwrap().get(&path.to_ascii_lowercase()).cloned().unwrap_or_default())
    }

    /// `Resources.LoadAll(folder)` of `class`: objects registered below
    /// `folder`, in container (path) order.
    pub fn resources_under(&mut self, opener: &mut dyn UnityFiles, folder: &str, class: i32) -> Result<Vec<ObjectKey>> {
        self.resource(opener, "")?;
        let prefix = format!("{}/", folder.to_ascii_lowercase().trim_end_matches('/'));
        let mut names: Vec<String> = self.resources.as_ref().unwrap().keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        names.sort();
        let mut out = Vec::new();
        for name in names {
            if let Some(k) = self.resource_of(opener, &name, class)? {
                out.push(k);
            }
        }
        Ok(out)
    }

    /// The first object of `class` registered under `path`.
    pub fn resource_of(&mut self, opener: &mut dyn UnityFiles, path: &str, class: i32) -> Result<Option<ObjectKey>> {
        for k in self.resource(opener, path)? {
            if k.file.starts_with("builtin:") {
                continue;
            }
            let file = self.file(opener, &k.file)?;
            if file.object(k.path_id).is_some_and(|o| o.class_id == class) {
                return Ok(Some(k));
            }
        }
        Ok(None)
    }

    /// Instantiate a prefab GameObject as a new scene hierarchy.
    pub fn prefab(&mut self, opener: &mut dyn UnityFiles, game_object: &ObjectKey) -> Result<Scene> {
        let (_, bytes) = self.bytes(opener, game_object)?;
        let go = GameObject::read(&bytes)?;
        let t = go.component(c::TRANSFORM).context("prefab without Transform")?;
        let tk = self.resolve(opener, &game_object.file, t)?.context("prefab Transform")?;
        let mut scene = Scene::default();
        let root = scene.load(self, opener, &tk, None)?;
        scene.roots.push(root);
        Ok(scene)
    }

    /// Resolve a pointer found in `from` to a global key.
    pub fn resolve(&mut self, opener: &mut dyn UnityFiles, from: &str, p: PPtr) -> Result<Option<ObjectKey>> {
        if p.is_null() {
            return Ok(None);
        }
        let file = if p.file_id == 0 {
            Arc::from(from)
        } else {
            let owner = self.file(opener, from)?;
            let ext = owner.external(p.file_id)?;
            let base = ext.rsplit('/').next().unwrap_or(ext);
            // Built-in resource bundles are engine assets, not player data.
            if ext.starts_with("library/") || ext.starts_with("resources/") {
                return Ok(Some(ObjectKey { file: Arc::from(format!("builtin:{base}")), path_id: p.path_id }));
            }
            Arc::from(base)
        };
        Ok(Some(ObjectKey { file, path_id: p.path_id }))
    }

    /// Read the root scale track used by the roster plate's Unity 4 legacy Animation.
    /// The trailing component property curve is independent of this transform.
    pub fn root_scale_clip(&mut self, opener: &mut dyn UnityFiles, animation: &ObjectKey) -> Result<[Vec<[f32; 4]>; 3]> {
        use crysirror_unity::Reader;
        let (_, bytes) = self.bytes(opener, animation)?;
        let mut r = Reader::new(&bytes);
        let _owner = r.pptr()?;
        let _enabled = r.bool()?;
        r.align()?;
        let clip = self.resolve(opener, &animation.file, r.pptr()?)?.context("default AnimationClip")?;
        let (class, bytes) = self.bytes(opener, &clip)?;
        anyhow::ensure!(class == 74, "expected AnimationClip");
        let mut r = Reader::new(&bytes);
        let _name = r.string()?;
        anyhow::ensure!(r.i32()? == 1 && !r.bool()?, "unsupported roster clip encoding");
        let _high_quality = r.bool()?;
        r.align()?;
        anyhow::ensure!(r.i32()? == 0 && r.i32()? == 0 && r.i32()? == 0 && r.i32()? == 1,
            "expected a single roster scale curve");
        let count = r.count(40)?;
        anyhow::ensure!((1..=4096).contains(&count), "invalid roster scale key count");
        let mut axes = [Vec::new(), Vec::new(), Vec::new()];
        let mut last = -1.;
        for _ in 0..count {
            let t = r.f32()?;
            anyhow::ensure!(t.is_finite() && t >= 0. && t > last, "invalid scale key time");
            last = t;
            let (value, incoming, outgoing) = (r.f32s::<3>()?, r.f32s::<3>()?, r.f32s::<3>()?);
            for axis in 0..3 {
                anyhow::ensure!(value[axis].is_finite(), "invalid scale key value");
                axes[axis].push([t, value[axis], incoming[axis], outgoing[axis]]);
            }
        }
        let (_pre, _post) = (r.i32()?, r.i32()?);
        anyhow::ensure!(r.string()?.is_empty(), "expected a root scale track");
        Ok(axes)
    }

    pub fn bytes(&mut self, opener: &mut dyn UnityFiles, key: &ObjectKey) -> Result<(i32, Vec<u8>)> {
        let file = self.file(opener, &key.file)?;
        let info = *file.object(key.path_id).with_context(|| format!("Unity object {}:{} missing", key.file, key.path_id))?;
        Ok((info.class_id, file.bytes(&info)?))
    }

    pub fn script_name(&mut self, opener: &mut dyn UnityFiles, key: &ObjectKey) -> Result<String> {
        if let Some(n) = self.scripts.get(key) {
            return Ok(n.clone());
        }
        let (class, bytes) = self.bytes(opener, key)?;
        anyhow::ensure!(class == c::MONO_SCRIPT, "Unity script reference is not a MonoScript");
        let name = MonoScript::read(&bytes)?.full_name();
        self.scripts.insert(key.clone(), name.clone());
        Ok(name)
    }

    /// Decode a MonoBehaviour component.
    pub fn behaviour(&mut self, opener: &mut dyn UnityFiles, key: &ObjectKey) -> Result<(String, bool, PPtr, Fields)> {
        let (class, bytes) = self.bytes(opener, key)?;
        anyhow::ensure!(class == c::MONO_BEHAVIOUR, "not a MonoBehaviour");
        let mb = MonoBehaviour::read(&bytes)?;
        let script_key = self.resolve(opener, &key.file, mb.script)?.context("MonoBehaviour without script")?;
        let script = self.script_name(opener, &script_key)?;
        let fields = if self.schema.scripts.contains_key(&script) {
            self.schema.decode(&script, &bytes[mb.body..]).with_context(|| format!("{script} {}:{}", key.file, key.path_id))?
        } else {
            Fields::default()
        };
        Ok((script, mb.enabled, mb.game_object, fields))
    }

    /// Every root GameObject hierarchy of a scene file.
    pub fn scene(&mut self, opener: &mut dyn UnityFiles, name: &str) -> Result<Scene> {
        let file = self.file(opener, name)?;
        let roots: Vec<i64> = file
            .objects
            .iter()
            .filter(|o| o.class_id == c::TRANSFORM)
            .filter_map(|o| Transform::read(&file.bytes(o).ok()?).ok().filter(|t| t.parent.is_null()).map(|_| o.path_id))
            .collect();
        let mut scene = Scene { nodes: Vec::new(), roots: Vec::new() };
        for t in roots {
            let key = ObjectKey { file: Arc::from(name), path_id: t };
            let root = scene.load(self, opener, &key, None)?;
            scene.roots.push(root);
        }
        Ok(scene)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
}

/// Bound on a loaded hierarchy (the largest CM3D2 file has ~12.5k objects).
const MAX_NODES: usize = 65536;

impl Scene {
    /// Load the hierarchy under the Transform `transform`.
    pub fn load(&mut self, data: &mut UnityData, opener: &mut dyn UnityFiles, transform: &ObjectKey, parent: Option<usize>) -> Result<usize> {
        anyhow::ensure!(self.nodes.len() < MAX_NODES, "Unity hierarchy limit");
        let (_, bytes) = data.bytes(opener, transform)?;
        let t = Transform::read(&bytes)?;
        let go_key = data.resolve(opener, &transform.file, t.game_object)?.context("Transform without GameObject")?;
        let (_, go_bytes) = data.bytes(opener, &go_key)?;
        let go = GameObject::read(&go_bytes)?;
        let mut components = Vec::with_capacity(go.components.len());
        for (class_id, p) in &go.components {
            let Some(key) = data.resolve(opener, &go_key.file, *p)? else { continue };
            let component = match *class_id {
                c::TRANSFORM | c::RECT_TRANSFORM => Component::Transform,
                c::CAMERA => Component::Camera(Camera::read(&data.bytes(opener, &key)?.1)?),
                c::MONO_BEHAVIOUR => {
                    let (class, enabled, _, fields) = data.behaviour(opener, &key)?;
                    Component::Script { class, enabled, fields }
                }
                other => Component::Other { class_id: other },
            };
            components.push((key, component));
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            key: go_key,
            name: go.name,
            active: go.active,
            layer: go.layer,
            parent,
            children: Vec::new(),
            position: t.position,
            rotation: t.rotation,
            scale: t.scale,
            components,
            instance: 0,
        });
        for child in t.children {
            let Some(key) = data.resolve(opener, &transform.file, child)? else { continue };
            let i = self.load(data, opener, &key, Some(index))?;
            self.nodes[index].children.push(i);
        }
        Ok(index)
    }

    /// `Object.Instantiate(prefab)` parented to `parent`: copy the prefab
    /// hierarchy under `root` into this scene; returns the new root index.
    pub fn instantiate(&mut self, prefab: &Scene, root: usize, parent: Option<usize>) -> usize {
        let instance = self.nodes.len() + 1;
        self.instantiate_as(prefab, root, parent, instance)
    }

    fn instantiate_as(&mut self, prefab: &Scene, root: usize, parent: Option<usize>, instance: usize) -> usize {
        let index = self.nodes.len();
        let mut node = prefab.nodes[root].clone();
        node.parent = parent;
        node.children = Vec::new();
        node.instance = instance;
        self.nodes.push(node);
        match parent {
            Some(p) => self.nodes[p].children.push(index),
            None => self.roots.push(index),
        }
        for &child in &prefab.nodes[root].children {
            self.instantiate_as(prefab, child, Some(index), instance);
        }
        index
    }

    /// The node a serialized object reference of `from` points at (its
    /// GameObject or one of its components): Instantiate remaps references
    /// inside a copy to that copy, so a node of `from`'s copy wins.
    pub fn find_ref(&self, from: usize, path_id: i64) -> Option<usize> {
        let hit = |n: &Node| n.key.path_id == path_id || n.components.iter().any(|(k, _)| k.path_id == path_id);
        let instance = self.nodes[from].instance;
        let mut first = None;
        for (i, n) in self.nodes.iter().enumerate() {
            if hit(n) {
                if n.instance == instance {
                    return Some(i);
                }
                first.get_or_insert(i);
            }
        }
        first
    }

    /// `Transform.Find` below `node`.
    /// World transform of a node (Unity space): `parent · T · R · S`.
    pub fn global(&self, node: usize) -> glam::Mat4 {
        let n = &self.nodes[node];
        let local = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::from_array(n.scale),
            glam::Quat::from_array(n.rotation),
            glam::Vec3::from_array(n.position),
        );
        match n.parent {
            Some(p) => self.global(p) * local,
            None => local,
        }
    }

    /// The first node named `name` under `root` (`UTY.GetChildObject` by
    /// name, depth-first).
    pub fn descendant(&self, root: usize, name: &str) -> Option<usize> {
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            if self.nodes[n].name == name && n != root {
                return Some(n);
            }
            stack.extend(self.nodes[n].children.iter().rev());
        }
        None
    }

    pub fn child(&self, node: usize, path: &str) -> Option<usize> {
        let mut at = node;
        for part in path.split('/') {
            at = *self.nodes[at].children.iter().find(|c| self.nodes[**c].name == part)?;
        }
        Some(at)
    }

    pub fn find(&self, path: &str) -> Option<usize> {
        let mut parts = path.split('/');
        let first = parts.next()?;
        let mut at = *self.roots.iter().find(|r| self.nodes[**r].name == first)?;
        for part in parts {
            at = *self.nodes[at].children.iter().find(|c| self.nodes[**c].name == part)?;
        }
        Some(at)
    }

    pub fn path(&self, mut node: usize) -> String {
        let mut parts = vec![self.nodes[node].name.as_str()];
        while let Some(p) = self.nodes[node].parent {
            parts.push(self.nodes[p].name.as_str());
            node = p;
        }
        parts.reverse();
        parts.join("/")
    }

    pub fn script(&self, node: usize, class: &str) -> Option<&Fields> {
        self.nodes[node].components.iter().find_map(|(_, c)| match c {
            Component::Script { class: k, fields, .. } if k == class => Some(fields),
            _ => None,
        })
    }
}
