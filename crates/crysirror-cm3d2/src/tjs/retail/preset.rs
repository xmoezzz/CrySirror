// SPDX-License-Identifier: MPL-2.0
//! Retail CharacterMgr.PresetLoad / PresetSet and the 5.6 MaidProp layout.
use super::*;
use crate::parts_color::PartsColor;
use crysirror_asset::interchange::cm3d2::Reader;

#[derive(Clone, Debug, Default)]
pub(super) struct Appearance {
    pub slots: BTreeMap<&'static str, Slot>,
    pub colors: Option<[PartsColor; 9]>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Slot {
    pub file_id: i32,
    pub temp: Option<(String, i32)>,
    pub temp_value: i32,
    pub dirty: bool,
    pub subs: Vec<Option<(String, i32, f32)>>,
    pub adjustments: Adjustments,
    pub link_max: i32,
}

type Transform = [f32; 10];
#[derive(Clone, Debug, Default)]
pub(super) struct Adjustments {
    skin: BTreeMap<i32, (i32, bool, Transform)>,
    vertices: BTreeMap<i32, BTreeMap<String, (i32, bool, i32, i32, Transform)>>,
    materials: BTreeMap<i32, (i32, i32, String, String, String)>,
    bone_lengths: BTreeMap<i32, (i32, BTreeMap<String, f32>)>,
}
struct Prop {
    index: usize,
    kind: i32,
    value: i32,
    file: String,
    data: Slot,
}

fn transform(r: &mut Reader<'_>) -> Result<Transform> {
    let mut result = [0.; 10];
    for component in &mut result {
        *component = r.f32()?;
    }
    Ok(result)
}

fn adjustments(r: &mut Reader<'_>, version: i32) -> Result<Adjustments> {
    let mut data = Adjustments::default();
    for _ in 0..r.count(49)? {
        data.skin
            .insert(r.i32()?, (r.i32()?, r.bool()?, transform(r)?));
    }
    for _ in 0..r.count(8)? {
        let slot = r.i32()?;
        let mut vertices = BTreeMap::new();
        for _ in 0..r.count(54)? {
            vertices.insert(
                r.string()?,
                (r.i32()?, r.bool()?, r.i32()?, r.i32()?, transform(r)?),
            );
        }
        data.vertices.insert(slot, vertices);
    }
    for _ in 0..r.count(15)? {
        let (slot, item, material) = (r.i32()?, r.i32()?, r.i32()?);
        let (mut prop, ty, mut value) = (r.string()?, r.string()?, r.string()?);
        if version < 204
            && slot == 1
            && material == 3
            && matches!(prop.as_str(), "_ZTest" | "ZTest")
        {
            prop = "_ZTest2".into();
            value = if value == "8" { "7" } else { "1" }.into();
        }
        data.materials
            .insert(slot, (item, material, prop, ty, value));
    }
    if version >= 213 {
        for _ in 0..r.count(12)? {
            let (slot, item) = (r.i32()?, r.i32()?);
            let mut bones = BTreeMap::new();
            for _ in 0..r.count(5)? {
                bones.insert(r.string()?, r.f32()?);
            }
            data.bone_lengths.insert(slot, (item, bones));
        }
    }
    Ok(data)
}

pub(crate) fn delete_menu(name: &str) -> Option<&'static str> {
    match name {
        "accnail" => Some("_I_accnail_del.menu"),
        _ => crate::preset::del_item(name),
    }
}

fn file_id(name: &str, id: i32) -> i32 {
    if id == 0 {
        crate::save::mono_string_hash(&name.to_lowercase())
    } else {
        id
    }
}

fn read_prop(r: &mut Reader<'_>) -> Result<Prop> {
    let version = r.header("CM3D2_MPROP")?;
    let _old_index = r.i32()?;
    let name = r.string()?;
    let index = character::MPN
        .iter()
        .position(|n| n.eq_ignore_ascii_case(&name))
        .or_else(|| {
            name.trim()
                .parse()
                .ok()
                .filter(|i: &usize| *i < character::MPN.len())
        })
        .with_context(|| format!("unknown preset property {name}"))?;
    let (kind, _default, value) = (r.i32()?, r.i32()?, r.i32()?);
    if version >= 101 {
        r.i32()?;
    }
    let link_max = r.i32()?;
    let mut file = r.string()?;
    let mut data = Slot {
        file_id: r.i32()?,
        link_max,
        ..Default::default()
    };
    r.bool()?;
    r.i32()?;
    r.i32()?;
    if version >= 200 {
        for _ in 0..r.count(1)? {
            let sub = if r.bool()? {
                r.bool()?;
                Some((
                    r.string()?,
                    r.i32()?,
                    if version >= 211 { r.f32()? } else { 1.0 },
                ))
            } else {
                None
            };
            data.subs.push(sub);
        }
    } else if kind == 3
        && matches!(name.as_str(), "acctatoo" | "hokuro")
        && !file.is_empty()
        && !file.contains("_del")
    {
        data.subs.push(Some((file.clone(), data.file_id, 1.)));
        file = delete_menu(&name).context("missing delete menu")?.into();
        data.file_id = file_id(&file, 0);
    }
    let replacement = match name.as_str() {
        "eye_hi" if file.is_empty() => Some("_I_SkinHi.menu"),
        "mayu" if file.is_empty() => Some("_I_mayu_001_mugen.menu"),
        _ if version <= 208 && index == 44 && file.eq_ignore_ascii_case("_I_acctatoo_del.menu") => {
            delete_menu("accnail")
        }
        _ => None,
    };
    if let Some(replacement) = replacement {
        file = replacement.into();
        data.file_id = file_id(&file, 0);
    }
    if version >= 200 {
        data.adjustments = adjustments(r, version)?;
    }
    Ok(Prop {
        index,
        kind,
        value,
        file,
        data,
    })
}

fn numeric(index: usize, kind: i32, value: i32) -> Prop {
    Prop {
        index,
        kind,
        value,
        file: String::new(),
        data: Slot::default(),
    }
}

pub(crate) fn read_colors(r: &mut Reader<'_>) -> Result<[PartsColor; 9]> {
    let cv = r.header("CM3D2_MULTI_COL")?;
    let count = r.count(37)?;
    ensure!(
        count <= 9 && (cv < 200 || count == 9),
        "invalid retail parts color count {count}"
    );
    let mut c = [PartsColor::default(); 9];
    for p in &mut c[..count] {
        *p = PartsColor {
            used: r.bool()?,
            main_hue: r.i32()?,
            main_chroma: r.i32()?,
            main_brightness: r.i32()?,
            main_contrast: r.i32()?,
            shadow_rate: r.i32()?,
            shadow_hue: r.i32()?,
            shadow_chroma: r.i32()?,
            shadow_brightness: r.i32()?,
            shadow_contrast: r.i32()?,
        };
    }
    Ok(c)
}

/// CharacterMgr.Deserialize passes the outer key to GetProp and reads the
/// same managed MaidProp payload as a full-capacity preset. Keep slot bytes
/// available to the save loader, including adjustment dictionaries and flags
/// that PresetSet intentionally does not apply.
pub(crate) fn stock_props<'a>(r: &mut Reader<'a>, source: &'a [u8]) -> Result<Vec<(String, &'a [u8])>> {
    ensure!(r.header("CM3D2_MPROP_LIST")? == 1040, "unsupported stock property list version");
    let count = r.count(8)?;
    let mut props = Vec::new();
    for _ in 0..count {
        let key = r.string()?;
        ensure!(character::MPN.iter().any(|name| name.eq_ignore_ascii_case(&key)), "unknown stock property {key}");
        let start = r.position();
        read_prop(r).with_context(|| format!("stock property {key}"))?;
        props.push((key, source.get(start..r.position()).context("invalid stock property bounds")?));
    }
    Ok(props)
}

fn read_preset(bytes: &[u8]) -> Result<(i32, Vec<Prop>, Option<[PartsColor; 9]>)> {
    let mut r = Reader::new(bytes);
    let version = r.header("CM3D2_PRESET")?;
    let kind = r.i32()?;
    let size = r.count(1)?;
    r.take(size)?;
    let list = r.header("CM3D2_MPROP_LIST")?;
    let mut props = Vec::new();
    for _ in 0..r.count(8)? {
        if list >= 4 {
            r.string()?;
        }
        let p = read_prop(&mut r)?;
        if list <= 110 && p.index == 10 {
            props.push(numeric(11, p.kind, p.value));
            props.push(numeric(12, p.kind, p.value));
        }
        props.push(p);
    }
    if list < 200 {
        for (i, v) in [(15, 0), (20, 0), (21, 50), (22, 50)] {
            props.push(numeric(i, 2, v));
        }
    }
    if list < 210 {
        for i in 16..=19 {
            props.push(numeric(i, 2, 50));
        }
    }
    let colors = if version >= 2 {
        Some(read_colors(&mut r)?)
    } else {
        None
    };
    if version >= 200 {
        r.header("CM3D2_MAID_BODY")?;
    }
    Ok((kind, props, colors))
}

pub(super) fn load(s: &mut TjsState, a: &[Variant]) -> Result<Variant> {
    arity(a, 2)?;
    let idx = s
        .active_maids
        .get(arg_int(a, 0) as usize)
        .copied()
        .flatten()
        .context("empty preset slot")?;
    let source = s.retail.source()?.clone();
    let name = arg_string(a, 1);
    let bytes = if name == "pre_maid_secretary" {
        let mut op = crate::adapter::Opener {
            storage: source.storage.clone(),
            dir: source.dir.clone(),
        };
        let mut u = crate::unity_scene::UnityData::new()?;
        u.file(&mut op, "globalgamemanagers")?;
        let key = u
            .resource_of(
                &mut op,
                "Preset/pre_maid_secretary",
                crysirror_unity::class_id::TEXT_ASSET,
            )?
            .context("missing secretary preset resource")?;
        let (_, bytes) = u.bytes(&mut op, &key)?;
        crysirror_unity::classes::text_asset(&bytes)?.1
    } else {
        let key = format!("{name}.preset");
        ensure!(
            s.retail.files(&source.files).exists(&key),
            "missing preset {name}"
        );
        source.files.read(source.storage.as_ref(), &key)?
    };
    if let Some(model) = s.retail.stock_models.get(&idx).cloned() {
        let mut pending = s.clone();
        apply_bytes(&mut pending, idx, &bytes)?;
        let mut model = (*model).clone();
        stock::apply_body(&mut pending, idx, &mut model)?;
        pending.retail.stock_models.insert(idx, Arc::new(model));
        pending.requests.push(crate::tjs::Request::RefreshMaid(arg_int(a,0) as usize));
        *s = pending;
    } else {
        apply_bytes(s, idx, &bytes)?;
    }
    Ok(Variant::Void)
}

pub(crate) fn apply_bytes(s: &mut TjsState, idx: usize, bytes: &[u8]) -> Result<()> {
    let source = s.retail.source()?.clone();
    let (kind, props, colors) = read_preset(bytes)?;
    if s.retail.edit_owned_only { s.retail.index_parts()?; }
    let body = s.bodies.entry(idx).or_default();
    let appearance = s.retail.appearances.entry(idx).or_default();
    for p in props {
        let wear = (53..=82).contains(&p.index);
        let skin = (1..=52).contains(&p.index) || (87..=90).contains(&p.index);
        if !(wear && kind != 1 || skin && kind != 0) {
            continue;
        }
        let name = character::MPN[p.index];
        let slot = appearance.slots.entry(name).or_default();
        if p.kind != 3 {
            let (lo, hi) = match p.index {
                1 | 3 => (0, 130),
                8 | 25 | 26 => (20, 80),
                _ => (0, 100),
            };
            body.props
                .insert(name.into(), (p.kind as u8, p.value.clamp(lo, hi)));
            slot.dirty = true;
            continue;
        }
        body.props.entry(name.into()).or_insert((3, 0));
        let file = if p.file.is_empty() {
            delete_menu(name).unwrap_or("")
        } else {
            &p.file
        };
        let can_use = |file: &str| {
            source.files.exists(file)
                && (!s.retail.edit_owned_only
                    || s.retail
                        .owned_parts
                        .get(&file.to_lowercase())
                        .copied()
                        .unwrap_or(true))
        };
        let del = delete_menu(name);
        let subs = p.data.subs;
        if can_use(file) {
            if p.index != 33 {
                body.menu_files.insert(name.into(), file.into());
                *slot = Slot {
                    file_id: file_id(file, p.data.file_id),
                    ..Default::default()
                };
            }
            slot.adjustments = p.data.adjustments;
            slot.dirty = true;
        } else if let Some(del) = del {
            body.menu_files.insert(name.into(), del.into());
            *slot = Slot {
                file_id: file_id(del, 0),
                dirty: true,
                ..Default::default()
            };
        }
        if !subs.is_empty() {
            if let Some(del) = del {
                body.menu_files.insert(name.into(), del.into());
                *slot = Slot {
                    file_id: file_id(del, 0),
                    dirty: true,
                    ..Default::default()
                };
            }
            for (i, sub) in subs.into_iter().enumerate() {
                if let Some((file, rid, alpha)) = sub {
                    if can_use(&file) {
                        slot.subs.resize(slot.subs.len().max(i + 1), None);
                        let rid = file_id(&file, rid);
                        slot.subs[i] = Some((file, rid, alpha));
                    } else if let Some(existing) = slot.subs.get_mut(i) {
                        *existing = None;
                    }
                }
            }
        }
    }
    if matches!(kind, 1 | 2) && colors.is_some() {
        appearance.colors = colors;
    }
    Ok(())
}

/// CharacterMgr slot properties carry the complete v1040 adjustment maps.
pub(super) fn restore_model_props(
    model: &mut crate::chara::Maid,
    properties: &crate::save::stock_1040::Properties<'_>,
    colors: Option<[PartsColor; 9]>,
) -> Result<Appearance> {
    let mut appearance = Appearance { slots: BTreeMap::new(), colors };
    for (key, bytes) in properties {
        let mut r = Reader::new(bytes);
        let saved = read_prop(&mut r)?;
        ensure!(r.remaining() == 0, "unexpected bytes in stock prop {key}");
        let name = character::MPN[saved.index];
        ensure!(name.eq_ignore_ascii_case(key), "stock property key {key} differs from {name}");
        ensure!(!appearance.slots.contains_key(name), "duplicate saved property {name}");
        let prop = model.props.get_mut(saved.index).context("stock property index")?;
        if saved.kind == 3 {
            prop.file = saved.file;
            prop.dut = true;
        } else {
            prop.value = saved.value.clamp(prop.min, prop.max);
        }
        appearance.slots.insert(name, saved.data);
    }
    if let Some(colors) = colors {
        for (index, color) in colors.into_iter().enumerate() {
            model.set_parts_color(index, color);
        }
    }
    Ok(appearance)
}

pub(super) fn capture_model_props(
    model: &crate::chara::Maid,
    appearance: Option<&Appearance>,
) -> Result<crate::save::stock_1040::Properties<'static>> {
    use crate::binio::Writer;
    ensure!(model.prop_names == character::MPN, "nonretail character properties");
    model.prop_names.iter().zip(&model.props).enumerate().map(|(index, (&name, prop))| {
        let data = appearance.and_then(|look| look.slots.get(name));
        let mut w = Writer::default();
        w.str("CM3D2_MPROP");
        w.i32(1040);
        w.i32(i32::try_from(index)?);
        w.str(name);
        w.i32(i32::from(prop.kind));
        w.i32(prop.default);
        w.i32(prop.value);
        w.i32(prop.temporary_value.unwrap_or(prop.value));
        w.i32(data.map_or(0, |slot| slot.link_max));
        w.str(&prop.file);
        w.i32(file_id(&prop.file, data.map_or(0, |slot| slot.file_id)));
        w.bool(prop.dut);
        w.i32(prop.max);
        w.i32(prop.min);
        let subs = data.map(|slot| slot.subs.as_slice()).unwrap_or_default();
        w.i32(i32::try_from(subs.len())?);
        for sub in subs {
            w.bool(sub.is_some());
            if let Some((file, rid, alpha)) = sub {
                w.bool(prop.dut);
                w.str(file);
                w.i32(*rid);
                w.f32(*alpha);
            }
        }
        if let Some(adjustments) = data.map(|slot| &slot.adjustments) {
            write_adjustments(&mut w, adjustments)?;
        } else {
            for _ in 0..4 {
                w.i32(0);
            }
        }
        Ok((name.into(), std::borrow::Cow::Owned(w.bytes)))
    }).collect()
}

fn write_adjustments(w: &mut crate::binio::Writer, data: &Adjustments) -> Result<()> {
    w.i32(i32::try_from(data.skin.len())?);
    for (&slot, (key, enabled, transform)) in &data.skin {
        w.i32(slot);
        w.i32(*key);
        w.bool(*enabled);
        for &component in transform {
            w.f32(component);
        }
    }
    w.i32(i32::try_from(data.vertices.len())?);
    for (&slot, vertices) in &data.vertices {
        w.i32(slot);
        w.i32(i32::try_from(vertices.len())?);
        for (name, (key, enabled, count, index, transform)) in vertices {
            w.str(name);
            w.i32(*key);
            w.bool(*enabled);
            w.i32(*count);
            w.i32(*index);
            for &component in transform {
                w.f32(component);
            }
        }
    }
    w.i32(i32::try_from(data.materials.len())?);
    for (&slot, (key, index, name, ty, value)) in &data.materials {
        w.i32(slot);
        w.i32(*key);
        w.i32(*index);
        w.str(name);
        w.str(ty);
        w.str(value);
    }
    w.i32(i32::try_from(data.bone_lengths.len())?);
    for (&slot, (key, bones)) in &data.bone_lengths {
        w.i32(slot);
        w.i32(*key);
        w.i32(i32::try_from(bones.len())?);
        for (name, &value) in bones {
            w.str(name);
            w.f32(value);
        }
    }
    Ok(())
}
