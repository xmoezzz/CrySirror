// SPDX-License-Identifier: MPL-2.0
//! Level placement extraction: world transforms of scene components in a
//! cooked `.umap`, with static mesh references and light parameters.
//! Transforms stay in Unreal space (cm, left-handed, Z up); conversion to
//! CrySirror space happens in [`to_crysirror`].
use crate::graph::{Game, ObjRef};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;

pub type Mat = [[f64; 4]; 4];

fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut m = [[0.0; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

/// FRotator (pitch, yaw, roll in degrees) → quaternion (x, y, z, w), as UE's FRotator::Quaternion.
pub fn rotator_quat(pitch: f64, yaw: f64, roll: f64) -> [f64; 4] {
    let h = std::f64::consts::PI / 360.0;
    let (sp, cp) = (pitch * h).sin_cos();
    let (sy, cy) = (yaw * h).sin_cos();
    let (sr, cr) = (roll * h).sin_cos();
    [
        cr * sp * sy - sr * cp * cy,
        -cr * sp * cy - sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
        cr * cp * cy + sr * sp * sy,
    ]
}

/// Column-vector TRS matrix.
pub fn trs(t: [f64; 3], q: [f64; 4], s: [f64; 3]) -> Mat {
    let [x, y, z, w] = q;
    let r = [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ];
    let mut m = [[0.0; 4]; 4];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = r[i][j] * s[j];
        }
        m[i][3] = t[i];
    }
    m[3][3] = 1.0;
    m
}

/// Unreal (cm, LH, Z up) → CrySirror (m, RH, Y up): swap Y/Z and scale.
/// The swap is a reflection, so the result stays a proper rigid+scale
/// transform when applied as P·M·P to matrices built in UE space.
pub fn to_crysirror(m: &Mat) -> Mat {
    let p = |i: usize| [0usize, 2, 1, 3][i];
    let mut o = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            o[i][j] = m[p(i)][p(j)];
        }
    }
    for row in o.iter_mut().take(3) {
        row[3] *= 0.01;
    }
    o
}

fn vec3(v: Option<&Value>, default: [f64; 3]) -> [f64; 3] {
    match v {
        Some(Value::Array(a)) if a.len() == 3 => [0, 1, 2].map(|i| a[i].as_f64().unwrap_or(default[i])),
        Some(Value::Null) => [0.0; 3], // zero-mask: explicitly zero
        _ => default,
    }
}

fn obj_index(v: Option<&Value>) -> Option<i64> {
    v?.get("index")?.as_i64().filter(|i| *i != 0)
}

pub fn is_a(game: &Game, class: &str, base: &str) -> bool {
    let mut cur = Some(class.to_owned());
    while let Some(c) = cur {
        if c == base {
            return true;
        }
        cur = game.schema.schemas.get(&c).and_then(|s| s.super_name.clone());
    }
    false
}

/// Extract placements from the package whose path ends with `suffix`.
pub fn extract(game: &mut Game, suffix: &str) -> Result<Value> {
    let map = game.package_by_path(suffix)?;
    let mut comps = HashMap::new(); // export index → (class, props)
    let mut errors = HashMap::<String, usize>::new();
    for i in 0..map.pkg.exports.len() {
        let Ok(class) = game.native_class(map.id, i) else { continue };
        if !is_a(game, &class, "SceneComponent") {
            continue;
        }
        match game.properties(map.id, i) {
            Ok(v) => {
                comps.insert(i, (class, v));
            }
            Err(e) => *errors.entry(format!("{e:#}").chars().take(100).collect()).or_default() += 1,
        }
    }
    // World transforms through AttachParent (same-package exports only).
    let mut world: HashMap<usize, Mat> = HashMap::new();
    fn solve(i: usize, comps: &HashMap<usize, (String, Value)>, world: &mut HashMap<usize, Mat>, depth: usize, pkg: &str) -> Mat {
        if let Some(m) = world.get(&i) {
            return *m;
        }
        let (_, v) = &comps[&i];
        let t = vec3(v.get("RelativeLocation"), [0.0; 3]);
        let r = vec3(v.get("RelativeRotation"), [0.0; 3]);
        let s = vec3(v.get("RelativeScale3D"), [1.0; 3]);
        let local = trs(t, rotator_quat(r[0], r[1], r[2]), s);
        let parent = v
            .get("AttachParent")
            .filter(|a| a.get("package").and_then(Value::as_str) == Some(pkg))
            .and_then(|a| obj_index(Some(a)))
            .filter(|p| *p > 0 && depth < 64)
            .map(|p| (p - 1) as usize)
            .filter(|p| comps.contains_key(p))
            .map(|p| solve(p, comps, world, depth + 1, pkg));
        let m = parent.map_or(local, |p| mul(&p, &local));
        world.insert(i, m);
        m
    }
    let keys: Vec<usize> = comps.keys().copied().collect();
    let pkg = format!("{:016x}", map.id);
    for i in &keys {
        solve(*i, &comps, &mut world, 0, &pkg);
    }
    let mut meshes = Vec::new();
    let mut lights = Vec::new();
    let mut unresolved = 0usize;
    for i in keys {
        let (class, v) = &comps[&i];
        let m = world[&i];
        let visible = v.get("bVisible").is_none_or(|b| !b.is_null()) && v.get("bHiddenInGame").is_none_or(|b| b.is_null());
        if is_a(game, class, "StaticMeshComponent") {
            let Some(mv) = v.get("StaticMesh").filter(|m| obj_index(Some(m)).is_some()) else { continue };
            let r = game.resolve_value(mv)?;
            if !matches!(r, ObjRef::Export { .. }) {
                unresolved += 1;
                continue;
            }
            let mesh = game.describe(&r)?;
            let mut overrides = Vec::new();
            if let Some(Value::Array(mats)) = v.get("OverrideMaterials") {
                for mat in mats {
                    let d = game.resolve_value(mat).and_then(|r| game.describe(&r)).unwrap_or_default();
                    overrides.push(d);
                }
            }
            meshes.push(json!({
                "component": map.pkg.exports[i].name,
                "class": class,
                "mesh": mesh,
                "materials": overrides,
                "visible": visible,
                "matrix": m,
            }));
        } else if is_a(game, class, "LightComponentBase") {
            // Absent keys mean "class default"; zero-masked values become 0/false.
            let mut l = serde_json::Map::new();
            l.insert("component".into(), json!(map.pkg.exports[i].name));
            l.insert("class".into(), json!(class));
            l.insert("matrix".into(), json!(m));
            l.insert("visible".into(), json!(visible));
            for (key, prop) in [
                ("intensity", "Intensity"),
                ("color", "LightColor"),
                ("radius", "AttenuationRadius"),
                ("inner_cone", "InnerConeAngle"),
                ("outer_cone", "OuterConeAngle"),
                ("units", "IntensityUnits"),
                ("cast_shadows", "CastShadows"),
                ("inverse_squared", "bUseInverseSquaredFalloff"),
                ("falloff_exponent", "LightFalloffExponent"),
                ("source_radius", "SourceRadius"),
                ("source_width", "SourceWidth"),
                ("source_height", "SourceHeight"),
                ("source_type", "SourceType"),
                ("affects_world", "bAffectsWorld"),
            ] {
                match v.get(prop) {
                    None => {}
                    Some(Value::Null) => {
                        l.insert(key.into(), json!(0));
                    }
                    Some(x) => {
                        l.insert(key.into(), x.clone());
                    }
                }
            }
            if let Some(c) = v.get("Cubemap") {
                l.insert("cubemap".into(), json!(game.resolve_value(c).and_then(|r| game.describe(&r)).ok()));
            }
            lights.push(Value::Object(l));
        }
    }
    Ok(json!({
        "package": map.path,
        "scene_components": comps.len(),
        "static_meshes": meshes,
        "lights": lights,
        "unresolved_meshes": unresolved,
        "errors": errors,
    }))
}
