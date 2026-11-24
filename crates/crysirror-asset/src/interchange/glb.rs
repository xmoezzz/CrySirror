// SPDX-License-Identifier: MPL-2.0
//! Minimal glTF 2.0 binary writer used by interchange importers. Importers
//! describe meshes, materials, nodes, skins and animations; this writes one
//! GLB with a single embedded buffer, so the engine's glTF path (PBR,
//! skinning, animation, streaming, ray tracing) consumes them unchanged.
use anyhow::{ensure, Result};
use serde_json::{json, Map, Value};

#[derive(Default)]
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    /// Optional per-vertex auxiliary vector, written as the custom glTF
    /// attribute `_AUX` (see `PbrPrimitive::aux`).
    pub aux: Vec<[f32; 4]>,
    pub material: Option<usize>,
    /// Morph targets: dense position deltas per vertex. Every primitive of a
    /// mesh carries the same targets; their names are the mesh's
    /// `targetNames`.
    pub targets: Vec<Vec<[f32; 3]>>,
    /// Dense normal deltas per morph target; empty, or one per target.
    pub target_normals: Vec<Vec<[f32; 3]>>,
    pub target_names: Vec<String>,
}

pub struct Material {
    pub name: String,
    pub base_color: [f32; 4],
    pub base_color_texture: Option<usize>,
    pub emissive: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub double_sided: bool,
    /// "OPAQUE", "MASK" or "BLEND".
    pub alpha_mode: &'static str,
    pub alpha_cutoff: f32,
    /// Complete engine material (`crysirror_material` extras, schema 1) whose
    /// texture fields are image indices; it supersedes the glTF fields above,
    /// which remain as the portable fallback.
    pub engine: Option<crate::pbr::PbrMaterial>,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color: [1.; 4],
            base_color_texture: None,
            emissive: [0.; 3],
            metallic: 0.,
            roughness: 0.8,
            double_sided: false,
            alpha_mode: "OPAQUE",
            alpha_cutoff: 0.5,
            engine: None,
        }
    }
}

#[derive(Default, Clone)]
pub struct Node {
    pub name: String,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub children: Vec<usize>,
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
}

impl Node {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rotation: [0., 0., 0., 1.],
            scale: [1.; 3],
            ..Default::default()
        }
    }
}

pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<[f32; 16]>,
    pub skeleton: Option<usize>,
}

pub enum Channel {
    Translation(Vec<[f32; 3]>),
    Rotation(Vec<[f32; 4]>),
    Scale(Vec<[f32; 3]>),
}

pub struct Track {
    pub node: usize,
    pub times: Vec<f32>,
    pub values: Channel,
    /// glTF CUBICSPLINE in/out tangents per key (xyz used for vectors);
    /// `None` samples linearly.
    pub tangents: Option<(Vec<[f32; 4]>, Vec<[f32; 4]>)>,
}

pub struct Animation {
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Default)]
pub struct Document {
    pub meshes: Vec<(String, Vec<Primitive>)>,
    pub materials: Vec<Material>,
    /// Encoded PNG or JPEG bytes and their MIME type.
    pub images: Vec<(Vec<u8>, &'static str)>,
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Animation>,
    /// Source features the conversion could not represent.
    pub dropped: Vec<String>,
    /// Spring-bone chains on document node indices.
    pub springs: Vec<crate::spring::SpringChain>,
    pub generator: String,
}

struct Writer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Writer {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let mut v = json!({"buffer":0,"byteOffset":self.bin.len(),"byteLength":bytes.len()});
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }

    fn floats<const N: usize>(
        &mut self,
        data: &[[f32; N]],
        kind: &str,
        bounds: bool,
        target: Option<u32>,
    ) -> usize {
        let bytes: Vec<u8> = data
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let view = self.view(&bytes, target);
        let mut a = json!({"bufferView":view,"componentType":5126,"count":data.len(),"type":kind});
        if bounds && !data.is_empty() {
            let mut min = [f32::MAX; N];
            let mut max = [f32::MIN; N];
            for v in data {
                for i in 0..N {
                    min[i] = min[i].min(v[i]);
                    max[i] = max[i].max(v[i]);
                }
            }
            a["min"] = json!(min.to_vec());
            a["max"] = json!(max.to_vec());
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn scalars(&mut self, data: &[f32]) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
        let view = self.view(&bytes, None);
        let min = data.iter().copied().fold(f32::MAX, f32::min);
        let max = data.iter().copied().fold(f32::MIN, f32::max);
        self.accessors.push(json!({"bufferView":view,"componentType":5126,"count":data.len(),"type":"SCALAR","min":[min],"max":[max]}));
        self.accessors.len() - 1
    }

    fn joints(&mut self, data: &[[u16; 4]]) -> usize {
        let bytes: Vec<u8> = data
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let view = self.view(&bytes, Some(34962));
        self.accessors
            .push(json!({"bufferView":view,"componentType":5123,"count":data.len(),"type":"VEC4"}));
        self.accessors.len() - 1
    }

    fn indices(&mut self, data: &[u32]) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34963));
        self.accessors.push(
            json!({"bufferView":view,"componentType":5125,"count":data.len(),"type":"SCALAR"}),
        );
        self.accessors.len() - 1
    }

    fn matrices(&mut self, data: &[[f32; 16]]) -> usize {
        let bytes: Vec<u8> = data
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let view = self.view(&bytes, None);
        self.accessors
            .push(json!({"bufferView":view,"componentType":5126,"count":data.len(),"type":"MAT4"}));
        self.accessors.len() - 1
    }
}

impl Document {
    pub fn to_glb(&self) -> Result<Vec<u8>> {
        let mut w = Writer {
            bin: Vec::new(),
            views: Vec::new(),
            accessors: Vec::new(),
        };
        let mut meshes = Vec::new();
        for (name, prims) in &self.meshes {
            let mut out = Vec::new();
            for p in prims {
                let n = p.positions.len();
                ensure!(
                    n > 0 && p.indices.len() % 3 == 0,
                    "mesh `{name}`: empty or non-triangle primitive"
                );
                ensure!(
                    p.indices.iter().all(|i| (*i as usize) < n),
                    "mesh `{name}`: index out of range"
                );
                ensure!(
                    p.normals.is_empty() || p.normals.len() == n,
                    "mesh `{name}`: normal count"
                );
                ensure!(
                    p.uvs.is_empty() || p.uvs.len() == n,
                    "mesh `{name}`: uv count"
                );
                ensure!(
                    p.joints.len() == p.weights.len()
                        && (p.joints.is_empty() || p.joints.len() == n),
                    "mesh `{name}`: skin attribute count"
                );
                let mut attributes = Map::new();
                attributes.insert(
                    "POSITION".into(),
                    json!(w.floats(&p.positions, "VEC3", true, Some(34962))),
                );
                if !p.normals.is_empty() {
                    attributes.insert(
                        "NORMAL".into(),
                        json!(w.floats(&p.normals, "VEC3", false, Some(34962))),
                    );
                }
                if !p.uvs.is_empty() {
                    attributes.insert(
                        "TEXCOORD_0".into(),
                        json!(w.floats(&p.uvs, "VEC2", false, Some(34962))),
                    );
                }
                if !p.aux.is_empty() {
                    ensure!(p.aux.len() == n, "mesh `{name}`: aux count");
                    attributes.insert("_AUX".into(), json!(w.floats(&p.aux, "VEC4", false, Some(34962))));
                }
                if !p.joints.is_empty() {
                    attributes.insert("JOINTS_0".into(), json!(w.joints(&p.joints)));
                    attributes.insert(
                        "WEIGHTS_0".into(),
                        json!(w.floats(&p.weights, "VEC4", false, Some(34962))),
                    );
                }
                let mut prim =
                    json!({"attributes":attributes,"indices":w.indices(&p.indices),"mode":4});
                ensure!(
                    p.targets.len() == prims[0].targets.len(),
                    "mesh `{name}`: primitives disagree on morph targets"
                );
                if !p.targets.is_empty() {
                    let mut targets = Vec::new();
                    ensure!(
                        p.target_normals.is_empty() || p.target_normals.len() == p.targets.len(),
                        "mesh `{name}`: morph normal count"
                    );
                    for (k, t) in p.targets.iter().enumerate() {
                        ensure!(t.len() == n, "mesh `{name}`: morph target vertex count");
                        let mut target = json!({"POSITION": w.floats(t, "VEC3", true, Some(34962))});
                        if let Some(normals) = p.target_normals.get(k) {
                            ensure!(normals.len() == n, "mesh `{name}`: morph normal vertex count");
                            target["NORMAL"] = json!(w.floats(normals, "VEC3", false, Some(34962)));
                        }
                        targets.push(target);
                    }
                    prim["targets"] = Value::Array(targets);
                }
                if let Some(m) = p.material {
                    ensure!(m < self.materials.len(), "material index");
                    prim["material"] = json!(m);
                }
                out.push(prim);
            }
            let mut mesh = json!({"name":name,"primitives":out});
            if let Some(first) = prims.first().filter(|p| !p.target_names.is_empty()) {
                ensure!(
                    first.target_names.len() == first.targets.len(),
                    "mesh `{name}`: morph target names"
                );
                mesh["extras"] = json!({ "targetNames": first.target_names });
            }
            meshes.push(mesh);
        }
        let mut images = Vec::new();
        let mut textures = Vec::new();
        for (i, (bytes, mime)) in self.images.iter().enumerate() {
            let view = w.view(bytes, None);
            images.push(json!({"bufferView":view,"mimeType":mime}));
            textures.push(json!({"source":i}));
        }
        let materials: Vec<Value> = self
            .materials
            .iter()
            .map(|m| {
                let mut pbr = json!({"baseColorFactor":m.base_color,"metallicFactor":m.metallic,"roughnessFactor":m.roughness});
                if let Some(t) = m.base_color_texture {
                    pbr["baseColorTexture"] = json!({"index":t});
                }
                let mut v = json!({"name":m.name,"pbrMetallicRoughness":pbr,"emissiveFactor":m.emissive,
                    "doubleSided":m.double_sided,"alphaMode":m.alpha_mode});
                if m.alpha_mode == "MASK" {
                    v["alphaCutoff"] = json!(m.alpha_cutoff);
                }
                if let Some(engine) = &m.engine {
                    v["extras"] = json!({"crysirror_material": {"schema": 1, "material": engine}});
                }
                v
            })
            .collect();
        for m in &self.materials {
            ensure!(
                m.base_color_texture.is_none_or(|t| t < self.images.len()),
                "texture index"
            );
            if let Some(engine) = &m.engine {
                engine.validate()?;
                ensure!(
                    engine.all_textures().into_iter().all(|t| *t < self.images.len()),
                    "engine material texture index"
                );
            }
        }
        let nodes: Vec<Value> = self
            .nodes
            .iter()
            .map(|n| {
                let mut v = json!({"name":n.name,"translation":n.translation,"rotation":n.rotation,"scale":n.scale});
                if !n.children.is_empty() {
                    v["children"] = json!(n.children);
                }
                if let Some(m) = n.mesh {
                    v["mesh"] = json!(m);
                }
                if let Some(s) = n.skin {
                    v["skin"] = json!(s);
                }
                v
            })
            .collect();
        let mut skins = Vec::new();
        for s in &self.skins {
            ensure!(
                s.joints.len() == s.inverse_bind.len() && !s.joints.is_empty(),
                "skin joint count"
            );
            let ibm = w.matrices(&s.inverse_bind);
            let mut v = json!({"joints":s.joints,"inverseBindMatrices":ibm});
            if let Some(root) = s.skeleton {
                v["skeleton"] = json!(root);
            }
            skins.push(v);
        }
        let mut animations = Vec::new();
        for a in &self.animations {
            let mut samplers = Vec::new();
            let mut channels = Vec::new();
            for t in &a.tracks {
                if t.times.is_empty() {
                    continue;
                }
                let input = w.scalars(&t.times);
                let (path, values): (&str, Vec<[f32; 4]>) = match &t.values {
                    Channel::Translation(v) => ("translation", v.iter().map(|x| [x[0], x[1], x[2], 0.]).collect()),
                    Channel::Rotation(v) => ("rotation", v.clone()),
                    Channel::Scale(v) => ("scale", v.iter().map(|x| [x[0], x[1], x[2], 0.]).collect()),
                };
                ensure!(values.len() == t.times.len(), "animation key count");
                let keys: Vec<[f32; 4]> = match &t.tangents {
                    None => values,
                    Some((a, b)) => {
                        ensure!(a.len() == values.len() && b.len() == values.len(), "animation tangent count");
                        (0..values.len()).flat_map(|k| [a[k], values[k], b[k]]).collect()
                    }
                };
                let output = if path == "rotation" {
                    w.floats(&keys, "VEC4", false, None)
                } else {
                    let v: Vec<[f32; 3]> = keys.iter().map(|x| [x[0], x[1], x[2]]).collect();
                    w.floats(&v, "VEC3", false, None)
                };
                let interpolation = if t.tangents.is_some() { "CUBICSPLINE" } else { "LINEAR" };
                samplers.push(json!({"input":input,"output":output,"interpolation":interpolation}));
                channels
                    .push(json!({"sampler":samplers.len()-1,"target":{"node":t.node,"path":path}}));
            }
            if !channels.is_empty() {
                animations.push(json!({"name":a.name,"samplers":samplers,"channels":channels}));
            }
        }
        let mut doc = json!({
            "asset": {"version":"2.0","generator":self.generator},
            "scene": 0,
            "scenes": [{"nodes": self.roots}],
            "nodes": nodes,
            "buffers": [{"byteLength": w.bin.len()}],
            "bufferViews": w.views,
            "accessors": w.accessors,
        });
        for (key, value) in [
            ("meshes", meshes),
            ("materials", materials),
            ("images", images),
            ("textures", textures),
            ("skins", skins),
            ("animations", animations),
        ] {
            if !value.is_empty() {
                doc[key] = Value::Array(value);
            }
        }
        if !self.dropped.is_empty() || !self.springs.is_empty() {
            let mut extras = serde_json::Map::new();
            if !self.dropped.is_empty() {
                extras.insert(crate::interchange::DROPPED_EXTRA.into(), json!(self.dropped));
            }
            if !self.springs.is_empty() {
                for c in &self.springs {
                    c.validate(self.nodes.len())?;
                }
                extras.insert(crate::interchange::SPRINGS_EXTRA.into(), json!(self.springs));
            }
            doc["extras"] = Value::Object(extras);
        }
        let mut json = serde_json::to_vec(&doc)?;
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let mut bin = w.bin;
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        ensure!(
            total <= crate::MAX_IMPORT_BYTES,
            "converted model exceeds the import limit"
        );
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        Ok(out)
    }
}
