// SPDX-License-Identifier: MPL-2.0
//! Static scene and Resources prefab geometry in the Unity 5.6 retail player.
use super::*;
use crate::unity_scene::{ObjectKey, UnityData, UnityFiles};
use crysirror_asset::pbr::{AlphaMode, PbrMaterial, ShaderEffect};
use crysirror_unity::{
    class_id as c,
    classes::{Material, Mesh},
    gltf::GlbBuilder,
    reader::Reader,
};
use std::collections::HashMap;

// Built-in 5.6 shaders can leave the outer Unity object name empty. The
// ShaderLab name in SerializedShader precedes its editor, fallback, dependency
// list and flag. The built-ins used here have empty values for all four.
pub(super) fn builtin_shader(bytes: &[u8], name: &str) -> bool {
    bytes.chunks_exact(4).enumerate().any(|(i, word)| {
        if u32::from_le_bytes(word.try_into().unwrap()) as usize != name.len() {
            return false;
        }
        let start = i * 4 + 4;
        let end = start + name.len();
        bytes.get(start..end) == Some(name.as_bytes())
            && bytes.get(end.next_multiple_of(4)..).is_some_and(|rest| rest.starts_with(&[0; 16]))
    })
}

fn texture(
    unity: &mut UnityData,
    op: &mut Opener,
    glb: &mut GlbBuilder,
    key: &ObjectKey,
    pointer: crysirror_unity::PPtr,
) -> Result<usize> {
    let tk = unity.resolve(op, &key.file, pointer)?.context("club texture")?;
    resolved_texture(unity, op, glb, &tk)
}

fn resolved_texture(
    unity: &mut UnityData,
    op: &mut Opener,
    glb: &mut GlbBuilder,
    tk: &ObjectKey,
) -> Result<usize> {
    glb.image(&format!("{}#{}", tk.file, tk.path_id), || {
        let (_, bytes) = unity.bytes(op, tk)?;
        let img = crysirror_unity::texture::decode_56(&bytes, |p| {
            ensure!(
                !p.contains(['/', '\\']) && p.ends_with(".resS"),
                "invalid sibling texture stream"
            );
            op.open(p)
        })?;
        Ok((crysirror_image::encode_png(&img)?, "image/png"))
    })
}

fn material(
    unity: &mut UnityData,
    op: &mut Opener,
    key: &ObjectKey,
    glb: &mut GlbBuilder,
    has_secondary_uvs: bool,
    main_texture: Option<(&str, &ObjectKey)>,
) -> Result<usize> {
    let (_, bytes) = unity.bytes(op, key)?;
    let m = Material::read_56(&bytes)?;
    let shader = unity
        .resolve(op, &key.file, m.shader)?
        .context("club material shader")?;
    let (_, sb) = unity.bytes(op, &shader)?;
    let object_name = Reader::new(&sb).string()?;
    let shader = if object_name.is_empty() {
        ["Unlit/Transparent", "Unlit/Texture", "Particles/Additive", "Particles/Additive (Soft)"]
            .into_iter()
            .find(|name| builtin_shader(&sb, name)).unwrap_or("")
    } else {
        object_name.as_str()
    };
    let mut out = PbrMaterial::default();
    // Unity leaves unused texture slots serialized with a null PPtr. In that
    // case the shader uses its default sampler, including for dance stages.
    if let Some(env) = m.texture("_MainTex").filter(|env| {
        env.texture.path_id != 0 || main_texture.is_some_and(|(name, _)| name == m.name)
    }) {
        out.base_color_texture = Some(match main_texture {
            Some((name, replacement)) if name == m.name => resolved_texture(unity, op, glb, replacement)?,
            _ => texture(unity, op, glb, key, env.texture)?,
        });
        out.uv_scale = env.scale;
        out.uv_offset = [env.offset[0], 1. - env.scale[1] - env.offset[1]];
    }
    out.base_color_factor = m.color("_Color").unwrap_or([1.; 4]);
    if matches!(shader, "Unlit/Transparent" | "Unlit/Texture") {
        out.unlit = true;
        // Both native fragment programs return the main texture directly.
        out.base_color_factor = [1.; 4];
        if shader == "Unlit/Transparent" {
            out.alpha_mode = AlphaMode::Blend;
        }
    } else if shader == "Particles/Additive" {
        let [r, g, b, a] = m.color("_TintColor").unwrap_or([0.5; 4]);
        out.unlit = true;
        out.double_sided = true;
        // Unity's additive pass multiplies texture × tint × 2.
        out.base_color_factor = [2. * r, 2. * g, 2. * b, 2. * a];
        out.alpha_mode = AlphaMode::Additive;
    } else if shader == "Particles/Additive (Soft)" {
        // Static glow geometry in the hall has no vertex color stream.
        out.unlit = true;
        out.double_sided = true;
        out.base_color_factor = [1.; 4];
        out.alpha_mode = AlphaMode::SoftAdditive;
    } else if shader == "CM3D2_Lighted_Lightmap" {
        // SATD-com3d2-standalone: shop curtains use this shader but omit UV1.
        if has_secondary_uvs && let Some(env) = m.texture("_LightMap").filter(|env| env.texture.path_id != 0) {
            out.emissive_texture = Some(texture(unity, op, glb, key, env.texture)?);
            let [r, g, b, _] = m.color("_ShadowColor").unwrap_or([0.5, 0.5, 0.5, 1.]);
            out.effect = Some(ShaderEffect::RgbLightmap {
                scale: env.scale,
                offset: [env.offset[0], 1. - env.scale[1] - env.offset[1]],
                shadow_color: [r, g, b],
            });
        }
    } else if shader.contains("Cutout") {
        out.alpha_mode = AlphaMode::Mask;
        out.alpha_cutoff = m.float("_Cutoff").unwrap_or(0.5);
    } else if shader.ends_with("_Trans") {
        out.alpha_mode = AlphaMode::Blend;
    }
    // Translate Unity HDR tint into the shared material's bounded albedo and
    // emissive range. The unlit path adds textured emission to the base, so
    // additive light quads retain their above-white RGB instead of clipping.
    let [r, g, b, a] = out.base_color_factor;
    let excess = [r, g, b].map(|v| (v - 1.).max(0.));
    if excess.iter().any(|&v| v > 0.) && out.emissive_texture.is_none() {
        out.emissive_texture = out.base_color_texture;
        out.emissive_factor = excess;
    }
    out.base_color_factor = [r, g, b, a].map(|v| v.clamp(0., 1.));
    out.validate().with_context(|| format!("Unity material {}", m.name))?;
    Ok(glb.material(&m.name, out))
}

pub(super) fn model(
    unity: &mut UnityData,
    op: &mut Opener,
    file: &str,
    root_name: &str,
) -> Result<(Vec<u8>, usize)> {
    model_variant(unity, op, file, root_name, None, &[])
}

pub(super) fn model_variant(
    unity: &mut UnityData,
    op: &mut Opener,
    file: &str,
    root_name: &str,
    main_texture: Option<(&str, &ObjectKey)>,
    separate_meshes: &[String],
) -> Result<(Vec<u8>, usize)> {
    let sf = unity.file(op, file)?;
    let mut root = None;
    for o in sf.objects.iter().filter(|o| o.class_id == c::GAME_OBJECT) {
        let b = sf.bytes(o)?;
        let mut r = Reader::new(&b);
        let n = r.count(12)?;
        if n == 0 {
            continue;
        }
        let tr = r.pptr64()?;
        r.take((n - 1) * 12 + 4)?;
        if r.string()? == root_name {
            root = Some(tr.path_id);
            break;
        }
    }
    build(
        unity,
        op,
        file,
        root.with_context(|| format!("scene root {root_name}"))?, false,
        main_texture, separate_meshes,
    )
}

pub(super) fn prefab(
    unity: &mut UnityData,
    op: &mut Opener,
    name: &str,
) -> Result<(Vec<u8>, usize)> {
    resource(unity,op,&format!("bg/{name}"),false)
}

pub(super) fn resource(unity:&mut UnityData,op:&mut Opener,path:&str,placed:bool)->Result<(Vec<u8>,usize)>{
    let key=unity.resource_of(op,path,c::GAME_OBJECT)?.with_context(||format!("missing model resource {path}"))?;
    let (_, data) = unity.bytes(op, &key)?;
    let mut r = Reader::new(&data);
    ensure!(r.count(12)? > 0, "background prefab has no transform");
    let root = unity
        .resolve(op, &key.file, r.pptr64()?)?
        .context("background transform")?;
    build(unity, op, &root.file, root.path_id,placed,None,&[])
}

fn build(
    unity: &mut UnityData,
    op: &mut Opener,
    file: &str,
    root: i64,
    placed:bool,
    main_texture: Option<(&str, &ObjectKey)>,
    separate_meshes: &[String],
) -> Result<(Vec<u8>, usize)> {
    let sf = unity.file(op, file)?;
    let mut stack = vec![(root, None)];
    let mut glb = GlbBuilder::new("CrySirror COM3D2");
    let mut materials = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    let mut count = 0;
    while let Some((transform, parent)) = stack.pop() {
        ensure!(seen.insert(transform), "cycle in title hierarchy");
        let b = sf.bytes_of(transform)?;
        let mut r = Reader::new(&b);
        let go = r.pptr64()?.path_id;
        let rot = r.f32s()?;
        let pos = r.f32s()?;
        let scale = r.f32s()?;
        let b = sf.bytes_of(go)?;
        let mut g = Reader::new(&b);
        let mut filter = None;
        let mut renderer = None;
        for _ in 0..g.count(12)? {
            let k = g.pptr64()?.path_id;
            let o = sf.object(k).context("title component")?;
            match o.class_id {
                c::MESH_FILTER => filter = Some(o),
                c::MESH_RENDERER => renderer = Some(o),
                _ => {}
            }
        }
        g.i32()?;
        let name = g.string()?;
        g.u16()?;
        if !g.bool()? {
            continue;
        }
        let index = if placed && parent.is_none(){
            glb.node(&name,None,[0.;3],[0.,0.,0.,1.],[1.;3])?
        }else{glb.node(&name, parent, pos, rot, scale)?};
        for _ in 0..r.count(12)? {
            stack.push((r.pptr64()?.path_id, Some(index)));
        }
        if let (Some(f), Some(re)) = (filter, renderer)
            && !separate_meshes.contains(&name)
        {
            let data = sf.bytes(f)?;
            let mut r = Reader::new(&data);
            r.seek(12)?;
            let Some(k) = unity.resolve(op, file, r.pptr64()?)? else {
                continue;
            };
            let data = sf.bytes(re)?;
            let mut r = Reader::new(&data);
            r.seek(12)?;
            if !r.bool()? {
                continue;
            }
            let (_, m) = unity.bytes(op, &k)?;
            let mesh = Mesh::read_56(&m).with_context(|| format!("club mesh {name}"))?;
            let has_secondary_uvs = !mesh.uv1.is_empty() && mesh.uv1.len() == mesh.positions.len();
            r.seek(56)?;
            let mut slots = vec![];
            for _ in 0..r.count(12)? {
                let key = unity
                    .resolve(op, file, r.pptr64()?)?
                    .context("club material")?;
                let cache_key = (key, has_secondary_uvs);
                let id = match materials.get(&cache_key) {
                    Some(&id) => id,
                    None => {
                        let id = material(unity, op, &cache_key.0, &mut glb, has_secondary_uvs, main_texture)
                            .with_context(|| format!("material for {name} in {file}"))?;
                        materials.insert(cache_key, id);
                        id
                    }
                };
                slots.push(id);
            }
            ensure!(
                mesh.indices
                    .iter()
                    .all(|&i| usize::from(i) < mesh.positions.len()),
                "club mesh {name} index exceeds vertex count"
            );
            glb.mesh(index, &mesh, &slots)?;
            count += 1;
        }
    }
    ensure!(count > 0, "empty title club");
    Ok((glb.finish()?, count))
}
