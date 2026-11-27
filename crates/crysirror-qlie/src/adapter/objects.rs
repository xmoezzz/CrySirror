use super::*;
mod bar;
mod strings;
mod native_log;
/// 5D3DE0 (log\\ct): message text without tags. A token is one character
/// or a `[`..`]` run (5D3C98). Inside a tag, Copy(Trim(token), 2, len-2) is
/// split on ','; `lim,n,text` keeps the first n code points of text,
/// `pc,text` and `pr,text` keep text and `rb,base,ruby` keeps base. Other
/// tags produce nothing.
fn log_plain_text(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '[' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < chars.len() {
            i += 1;
            if chars[i - 1] == ']' {
                break;
            }
        }
        let token: String = chars[start..i].iter().collect();
        let length = i - start;
        let trimmed: Vec<char> = token.trim_matches(|c: char| c <= ' ').chars().collect();
        let inner: String = trimmed.iter().skip(1).take(length.saturating_sub(2)).collect();
        let parts = crate::program::split_native(&inner, ",", false, false);
        let name = parts.first().map(String::as_str).unwrap_or("");
        match (name, parts.len()) {
            ("lim", 3) => {
                let n = crate::expression::parse_integer(&parts[1]).unwrap_or(0).max(0) as usize;
                out.extend(parts[2].chars().take(n));
            }
            ("pc" | "pr", 2) | ("rb", 3) => out.push_str(&parts[1]),
            _ => {}
        }
    }
    out
}
pub(super) fn strings_text(object: &Object) -> String {
    strings::text(object)
}
pub(super) fn strings_install(object: &mut Object, text: &str) -> Result<()> {
    strings::install_text(object, text)
}
use crysirror_core::service::{
    AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, EngineQuery, MediaPhase,
    MediaRequest, PcmBus,
};
fn reindex(o: &mut Object) {
    o.table = o
        .lines
        .iter()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.to_owned()))
        .collect();
}
fn set_name_value(o: &mut Object, name: &str, value: &str) {
    // Native fast-table Add ignores an empty name (572EA4); placeholders
    // from a short split must not become executable callback labels.
    if name.is_empty() {
        return;
    }
    if let Some(line) = o.lines.iter_mut().find(|l| {
        l.split_once('=')
            .is_some_and(|p| p.0.eq_ignore_ascii_case(name))
    }) {
        *line = format!("{name}={value}");
    } else {
        o.lines.push(format!("{name}={value}"));
    }
    o.table.insert(name.to_ascii_lowercase(), value.into());
}
pub(super) fn native_key(code: i32) -> Result<u64> {
    use crysirror_core::service::buttons as B;
    match code & 0xffff {
        0xf0 => Ok(B::WHEEL_UP),
        0xf1 => Ok(B::WHEEL_DOWN),
        value => B::windows_virtual_key(value as u32)
            .with_context(|| format!("unsupported QLIE input key {code:#x}")),
    }
}
impl QlieVm {
    pub(super) fn copy_native_value_object(
        &mut self,
        _h: &mut dyn HostServices,
        m: &Machine,
        destination: &str,
        source: &str,
    ) -> Result<()> {
        let destination_kind = m
            .object_types
            .get(destination)
            .context("QLIE vget/vset destination object")?;
        let source_kind = m
            .object_types
            .get(source)
            .context("QLIE vget/vset source object")?;
        ensure!(
            destination_kind == source_kind && matches!(source_kind.as_str(), "list" | "table"),
            "unsupported QLIE vget/vset object copy {source_kind} to {destination_kind}"
        );
        let (lines, table, properties) = if let Some(source) = self.objects.get(source) {
            (
                source.lines.clone(),
                source.table.clone(),
                source.properties.clone(),
            )
        } else {
            (Vec::new(), BTreeMap::new(), BTreeMap::new())
        };
        if !self.objects.contains_key(destination) {
            let layer = self.next_layer;
            self.next_layer = self
                .next_layer
                .checked_add(1)
                .context("QLIE object layer identity")?;
            self.objects.insert(
                destination.into(),
                Object {
                    kind: destination_kind.clone(),
                    thread: if destination.starts_with("system/") {
                        ThreadKind::System
                    } else if destination.starts_with("object/") {
                        m.thread_kind()
                    } else {
                        ThreadKind::Game
                    },
                    layer,
                    gain: 1.,
                    ..Default::default()
                },
            );
        }
        let object = self.objects.get_mut(destination).unwrap();
        object.lines = lines;
        object.table = table;
        object.properties = properties;
        Ok(())
    }
    pub(super) fn collect_objects(
        &mut self,
        h: &mut dyn HostServices,
        m: &mut Machine,
    ) -> Result<()> {
        let mut live = BTreeSet::new();
        for value in m
            .global_object_roots()
            .chain(m.execution_frames().flat_map(|f| {
                f.locals
                    .values()
                    .chain(f.call_results.iter().flat_map(|values| values.values()))
            }))
        {
            if let Value::Text(id) = value {
                if m.object_types.contains_key(id) {
                    live.insert(id.clone());
                }
            }
        }
        for frame in m.execution_frames() {
            live.extend(frame.selection.iter().cloned());
        }
        live.extend(self.host_object_roots());
        live.insert(m.scope.clone());
        if let Some(game) = m.suspended_game() {
            live.insert(game.scope.clone());
        }
        // Contextual host objects remain roots while the other thread is paused.
        for name in ["sys", "eff", "inp", "skey", "cam", "camex", "rep", "path"] {
            live.insert(format!("system/{name}"));
        }
        let mut pending = live.iter().cloned().collect::<Vec<_>>();
        while let Some(id) = pending.pop() {
            if let Some(o) = self.objects.get(&id) {
                for text in o
                    .properties
                    .values()
                    .flatten()
                    .map(Value::text)
                    .chain(o.table.values().cloned())
                {
                    if m.object_types.contains_key(&text) && live.insert(text.clone()) {
                        pending.push(text);
                    }
                }
            }
        }
        let dead = m
            .object_types
            .keys()
            .filter(|id| !live.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in dead {
            if let Some(o) = self.objects.remove(&id) {
                self.free_object(h, o)?;
            }
            m.object_types.remove(&id);
        }
        Ok(())
    }
    pub(super) fn delete(
        &mut self,
        h: &mut dyn HostServices,
        m: &mut Machine,
        args: &[String],
    ) -> Result<()> {
        for name in args {
            let key = m.key(name)?;
            let count = if m.frame.locals.contains_key(&key) {
                None
            } else {
                m.frame
                    .arrays
                    .get(&key)
                    .copied()
                    .or_else(|| m.global_array_extent(&key))
            };
            let local = m.frame.locals.contains_key(&key) || m.frame.arrays.contains_key(&key);
            for i in 0..count.unwrap_or(1) {
                let name = if count.is_some() {
                    format!("{key}[{i}]")
                } else {
                    key.clone()
                };
                if local {
                    m.frame.locals.remove(&name);
                } else {
                    m.remove_global_value(&name);
                }
            }
            if local {
                m.frame.arrays.remove(&key);
                m.frame.array_types.remove(&key);
                m.frame.array_shapes.remove(&key);
            } else {
                m.remove_global_array(&key);
            }
        }
        // Object variables are references. Deleting a temporary alias must not
        // remove a shared native sound channel or another live object's layer.
        self.collect_objects(h, m)
    }
    pub(super) fn free_object(&mut self, h: &mut dyn HostServices, o: Object) -> Result<()> {
        super::render::release_draw_images(h, self.client, &o)?;
        if o.kind == "bar" {
            for part in ["thumb", "track", "fill"] {
                if let Some(child) = o
                    .properties
                    .get(&format!("bar/{part}_id"))
                    .and_then(|v| v.first())
                    .map(Value::text)
                {
                    if let Some(child) = self.objects.remove(&child) {
                        self.free_object(h, child)?;
                    }
                }
            }
        }
        if matches!(o.kind.as_str(), "cam" | "camex") && o.properties.contains_key("col") {
            self.submit(
                h,
                json!({"layers":[{"op":"remove","id":o.layer+0x20000000}]}),
            )?;
        }
        if o.ui_registered {
            self.submit(h, json!({"ui":[{"op":"remove","id":o.layer}]}))?;
        }
        if o.created {
            self.submit(h, json!({"layers":[{"op":"remove","id":o.layer}]}))?;
        }
        if o.kind == "snd" && o.channel != 0 {
            h.submit_audio(
                self.client,
                AudioRequest::Stop {
                    channel: o.channel,
                    fade_ms: 0.,
                },
            )?;
        }
        if let Some(id) = o.media {
            h.media_request(self.client, MediaRequest::Remove { id })?;
            self.media_ids.remove(&id);
        }
        for (_, media) in o.embedded_media {
            let id = media.id;
            h.media_request(self.client, MediaRequest::Remove { id })?;
            self.media_ids.remove(&id);
        }
        for (ticket, _) in o.audio_tickets {
            h.release_audio(self.client, ticket)?;
        }
        for channel in o.audio_channels {
            h.submit_audio(
                self.client,
                AudioRequest::Stop {
                    channel,
                    fade_ms: 0.,
                },
            )?;
        }
        let mut released = BTreeSet::new();
        for sheet in o.message.iter().flat_map(|m| &m.pictures.sheets).filter(|s| s.live) {
            if let Some((path, _)) = &sheet.image {
                if released.insert(path.clone()) {
                    h.release_image_resource(self.client, path)?;
                }
            }
        }
        for image in o
            .images
            .into_iter()
            .flatten()
            .chain(o.originals.into_values())
        {
            if !released.insert(image.path.relative().to_owned()) {
                continue;
            }
            h.release_image_resource(self.client, image.path.relative())?;
        }
        Ok(())
    }
    fn object(&mut self, m: &Machine) -> Result<String> {
        let selected = m.scope.clone();
        let kind = m
            .object_types
            .get(&selected)
            .context("QLIE method without object scope")?
            .clone();
        //6E1A00 deliberately reads the reader's +296 replacement controller;
        //a newly declared rep is another wrapper, not an independent manager.
        let id = if matches!(kind.as_str(), "rep" | "path") {
            Self::builtin(&kind, m.source_thread_kind())
        } else {
            selected
        };
        if !self.objects.contains_key(&id) {
            ensure!(self.objects.len() < 4096, "QLIE object budget");
            let layer = self.next_layer;
            self.next_layer += 1;
            self.objects.insert(
                id.clone(),
                Object {
                    thread: if id.starts_with("system/") {
                        crate::machine::ThreadKind::System
                    } else if id.starts_with("object/") {
                        m.thread_kind()
                    } else {
                        crate::machine::ThreadKind::Game
                    },
                    kind,
                    layer,
                    gain: 1.,
                    ..Default::default()
                },
            );
        }
        Ok(id)
    }
    pub(super) fn object_method(
        &mut self,
        h: &mut dyn HostServices,
        m: &mut Machine,
        ins: &Instruction,
    ) -> Result<bool> {
        let id = self.object(m)?;
        // 6D1B24: dsnd channels share the snd sound-manager calls; cursor
        // images (csr/scsr) use the image dispatcher 6F1370.
        let kind = match self.objects[&id].kind.as_str() {
            "dsnd" => "snd".to_owned(),
            "csr" => "img".to_owned(),
            "scsr" => "simg".to_owned(),
            other => other.to_owned(),
        };
        let a = ins
            .args
            .iter()
            .map(|s| m.eval(s))
            .collect::<Result<Vec<_>>>()?;
        let n = |i: usize| {
            a.get(i)
                .map(Value::integer)
                .transpose()
                .map(|n| n.unwrap_or(0))
        };
        let s = |i: usize| a.get(i).map(Value::text).unwrap_or_default();
        let op = ins.op.as_str();
        if matches!(kind.as_str(), "file" | "path") {
            return self.file_method(h, m, &id, &kind, op, &a);
        }
        if matches!(kind.as_str(), "table" | "list") && matches!(op, "lff" | "stf") {
            return self.list_file_method(h, m, &id, op, &a);
        }
        if matches!(kind.as_str(), "table" | "list") {
            let object = self.objects.get_mut(&id).unwrap();
            match op {
                "i" => {
                    object.lines.clear();
                    object.table.clear();
                }
                "l" => {
                    let path = self.resolve(m, &s(0))?;
                    let text = self
                        .text
                        .get(&crate::program::canonical(&path)?)
                        .context("QLIE text resource not prepared")?
                        .clone();
                    let o = self.objects.get_mut(&id).unwrap();
                    strings::install_text(o, &text)?;
                }
                "?v" => {
                    ensure!((1..=2).contains(&a.len()), "QLIE named string query ABI");
                    let key = s(0).to_ascii_lowercase();
                    let value = object
                        .table
                        .get(&key)
                        .filter(|value| !value.is_empty())
                        .cloned()
                        .unwrap_or_else(|| s(1));
                    let default = a.get(1).and_then(|value| value.integer().ok()).unwrap_or(0);
                    m.result(
                        0,
                        Value::Int(crate::expression::parse_integer(&value).unwrap_or(default)),
                    );
                    m.result_text(0, value);
                }
                "v" => {
                    set_name_value(object, &s(0), &s(1));
                }
                // 6FE1D0 sw,i,j: TStrings.Exchange; an index outside the
                // list raises the native list-index error.
                "sw" => {
                    ensure!(a.len() == 2, "QLIE string exchange ABI");
                    let (i, j) = (usize::try_from(n(0)?), usize::try_from(n(1)?));
                    let (Ok(i), Ok(j)) = (i, j) else {
                        bail!("QLIE string list index out of bounds")
                    };
                    ensure!(
                        i < object.lines.len() && j < object.lines.len(),
                        "QLIE string list index out of bounds"
                    );
                    object.lines.swap(i, j);
                }
                "?c" => m.result(0, Value::Int(object.lines.len() as i32)),
                "?s" => {
                    ensure!(a.len() == 1, "QLIE indexed string query ABI");
                    let index = usize::try_from(n(0)?)?;
                    let line = object.lines.get(index).context("QLIE list index")?;
                    let value = if kind == "table" {
                        line.split_once('=').map(|p| p.1).unwrap_or("")
                    } else {
                        line
                    };
                    //Both original string containers return text and the low
                    //32 bits of Delphi StrToInt64Def's numeric observation.
                    m.result(
                        0,
                        Value::Int(value.trim().parse::<i64>().unwrap_or(0) as i32),
                    );
                    m.result_text(0, value);
                }
                "?n" => {
                    let index = usize::try_from(n(0)?)?;
                    let line = object.lines.get(index).context("QLIE table name index")?;
                    m.result_text(0, line.split_once('=').map(|p| p.0).unwrap_or(""));
                }
                "s" => {
                    ensure!(a.len() == 2, "QLIE indexed string replacement ABI");
                    let index = usize::try_from(n(0)?)?;
                    ensure!(index < object.lines.len(), "QLIE line replacement index");
                    object.lines[index] = if kind == "table" {
                        let name = object.lines[index]
                            .split_once('=')
                            .map(|p| p.0)
                            .unwrap_or("");
                        format!("{name}={}", s(1))
                    } else {
                        s(1)
                    };
                    reindex(object);
                }
                "txt" => {
                    ensure!((1..=2).contains(&a.len()), "QLIE string container text ABI");
                    let separator = s(1);
                    let text = if separator.is_empty() {
                        s(0)
                    } else {
                        s(0).replace(&separator, "\r\n")
                    };
                    strings::install_text(object, &text)?;
                }
                "?txt" => {
                    ensure!(a.len() <= 1, "QLIE string container text query ABI");
                    let text = strings::text(object);
                    let separator = s(0);
                    m.result_text(
                        0,
                        if separator.is_empty() {
                            text
                        } else {
                            text.replace("\r\n", &separator)
                        },
                    );
                }
                "ad" => {
                    let separator = s(0);
                    ensure!(!separator.is_empty(), "QLIE list delimiter");
                    let text = s(1);
                    if !text.is_empty() {
                        object.lines.extend(crate::program::split_native(
                            &text,
                            &separator,
                            n(2)? == 1,
                            n(3)? == 1,
                        ));
                        reindex(object);
                    }
                }
                "?i" => m.result(
                    0,
                    Value::Int(
                        object
                            .lines
                            .iter()
                            .position(|v| v == &s(0))
                            .map(|n| n as i32)
                            .unwrap_or(-1),
                    ),
                ),
                "d" => {
                    let index = usize::try_from(n(0)?)?;
                    ensure!(index < object.lines.len(), "QLIE line deletion index");
                    object.lines.remove(index);
                    reindex(object);
                }
                "a" => object.lines.push(s(0)),
                "asn" => {
                    let source = self
                        .objects
                        .get(&s(0))
                        .context("QLIE table/list assignment source")?;
                    ensure!(source.kind == kind, "QLIE table/list assignment type");
                    let (lines, table) = (source.lines.clone(), source.table.clone());
                    let target = self.objects.get_mut(&id).unwrap();
                    target.lines = lines;
                    target.table = table;
                }
                "sort" => {
                    object.lines.sort_by_key(|v| v.to_lowercase());
                    reindex(object);
                }
                "bpos" => {
                    let text = s(0).to_lowercase();
                    let found = object.lines.iter().enumerate().rev().find_map(|(i, line)| {
                        let (key, value) = line.split_once('=')?;
                        (!key.is_empty() && text.contains(&key.to_lowercase()))
                            .then(|| (i, key.to_owned(), value.to_owned()))
                    });
                    m.result_text(0, "");
                    m.result_text(1, "");
                    m.result(0, Value::Int(0));
                    m.result(1, Value::Int(-1));
                    if let Some((i, key, value)) = found {
                        m.result_text(0, key);
                        m.result_text(1, value.clone());
                        m.result(
                            0,
                            Value::Int(crate::expression::parse_integer(&value).unwrap_or(0)),
                        );
                        m.result(1, Value::Int(i as i32));
                    }
                }
                "brep" => {
                    let mut text = s(0);
                    for line in object.lines.iter().rev() {
                        if let Some((key, value)) = line.split_once('=') {
                            if !key.is_empty() {
                                text = text.replace(key, value);
                                ensure!(text.len() <= 1024 * 1024, "QLIE table replacement output");
                            }
                        }
                    }
                    m.result_text(0, text);
                }
                "idx" | "idxn" => {
                    ensure!(a.len() == 1, "QLIE string index query ABI");
                    let key = s(0);
                    let index = object
                        .lines
                        .iter()
                        .position(|line| {
                            if op == "idx" {
                                line.eq_ignore_ascii_case(&key)
                            } else {
                                line.split_once('=')
                                    .is_some_and(|(name, _)| name.eq_ignore_ascii_case(&key))
                            }
                        })
                        .map(|v| v as i32)
                        .unwrap_or(-1);
                    m.result(0, Value::Int(index));
                }

                "comb" => {
                    let other = self
                        .objects
                        .get(&s(0))
                        .context("QLIE table merge source")?
                        .table
                        .clone();
                    let object = self.objects.get_mut(&id).unwrap();
                    for (key, value) in other {
                        set_name_value(object, &key, &value);
                    }
                }
                _ => bail!("unsupported QLIE {kind} method \\{op}"),
            }
            return Ok(true);
        }
        if matches!(kind.as_str(), "ext" | "ppngpool" | "twitter") {
            if let Some(done) = self.minor_kind_method(h, &id, &kind, m, op, &a)? {
                return Ok(done);
            }
            bail!("unsupported QLIE {kind} method \\{op}");
        }
        if kind == "reg" {
            return self.reg_method(h, m, op, &a);
        }
        if kind == "clip" {
            return self.clip_method(h, m, op, &a);
        }
        if kind == "sdat" {
            return self.sdat_method(h, m, op, &a);
        }
        if kind == "dlgs" {
            return self.dialog_method(h, m, op, &a);
        }
        if kind == "cfg" {
            if !self.objects[&id]
                .properties
                .contains_key("native/config_loaded")
            {
                let settings = self.config_settings_map();
                let o = self.objects.get_mut(&id).unwrap();
                for (native, key, default) in [
                    ("fs", "fullscreen", 0),
                    ("il", "interlace", 0),
                    ("fps", "fpslimit", 60),
                    ("fdm", "fullscreendispmode", 0),
                    ("sb", "streamlength", 65536),
                    ("cs", "cachesize", 67108864),
                    ("blt", "bitbltmode", 0),
                    ("gim", "graphicintegermode", 0),
                    ("sfs", "scriptframeskip", 0),
                    ("ms", "messagespeed", 10),
                    ("mtw", "autoplaywaittime", 30),
                    ("mcw", "autoplayclicktime", 1500),
                ] {
                    let value = settings
                        .get(key)
                        .and_then(|v| crate::expression::parse_integer(v))
                        .unwrap_or(default);
                    // 505FE8 bounds CacheSize to 0..=0x10000000.
                    let value = if native == "cs" { value.clamp(0, 0x10000000) } else { value };
                    o.properties.insert(native.into(), vec![Value::Int(value)]);
                }
                super::cfg_settings::load(o, &settings);
                for index in 0..4 {
                    let key = if index == 0 {
                        "messagealpha".to_owned()
                    } else {
                        format!("messagealpha{index}")
                    };
                    let value = settings
                        .get(&key)
                        .and_then(|v| crate::expression::parse_integer(v))
                        .unwrap_or(255)
                        .clamp(0, 255);
                    o.properties
                        .insert(format!("ma/{index}"), vec![Value::Int(value)]);
                }
                for index in 0..16 {
                    for (suffix, native, default) in [
                        ("style", "fst2", 0),
                        ("stylecolor", "fsc2", 0),
                        ("aa", "faa2", 1),
                    ] {
                        let value = settings
                            .get(&format!("font[{index}].{suffix}"))
                            .and_then(|v| crate::expression::parse_integer(v))
                            .unwrap_or(default);
                        o.properties
                            .insert(format!("{native}/{index}"), vec![Value::Int(value)]);
                    }
                    if let Some(name) = settings.get(&format!("font[{index}].name")) {
                        o.properties
                            .insert(format!("fn2/{index}"), vec![Value::Text(name.clone())]);
                    }
                }
                o.properties
                    .insert("native/config_loaded".into(), vec![Value::Int(1)]);
            }
            if self.cfg_setting_method(h, m, &id, op, &a)? {
                return Ok(true);
            }
            match op {
                "?fdm" | "?fps" | "?sb" | "?cs" | "?blt" | "?gim" | "?sfs" => {
                    ensure!(
                        a.is_empty(),
                        "QLIE configured presentation/buffer query ABI"
                    );
                    m.result(
                        0,
                        self.objects[&id].properties[op.trim_start_matches('?')][0].clone(),
                    );
                }
                "fs" | "smr" => {
                    //6EA71C updates the retained fullscreen setting; smr
                    //dispatches the pending settings to the physical owner.
                    let enabled = if op == "fs" {
                        ensure!(a.len() == 1, "QLIE fullscreen setting ABI");
                        n(0)? != 0
                    } else {
                        ensure!(a.is_empty(), "QLIE screen refresh ABI");
                        self.objects[&id]
                            .properties
                            .get("fs")
                            .and_then(|v| v.first())
                            .map(Value::integer)
                            .transpose()?
                            .map(|v| v != 0)
                            .unwrap_or(h.window_fullscreen(self.client)?.unwrap_or(false))
                    };
                    self.submit(h, json!({"ui":[{"op":"set_fullscreen","enabled":enabled}]}))?;
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert("fs".into(), vec![Value::Int(i32::from(enabled))]);
                }
                "?fs" => {
                    ensure!(a.is_empty(), "QLIE fullscreen query ABI");
                    let observed = h.window_fullscreen(self.client)?;
                    let stored = self.objects[&id]
                        .properties
                        .get("fs")
                        .and_then(|v| v.first())
                        .map(Value::integer)
                        .transpose()?;
                    m.result(
                        0,
                        Value::Int(stored.unwrap_or(i32::from(observed.unwrap_or(false)))),
                    );
                }
                "il" => {
                    ensure!(a.len() == 1, "QLIE interlace hint ABI");
                    //6B9F44 pushes the quality hint to5688B8 arg0, and
                    //6B9F88 to568B84 arg8. Neither shipped presenter reads
                    //that argument: both modes use the same full redraw.
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert("il".into(), vec![Value::Int(i32::from(n(0)? != 0))]);
                }
                "?il" => {
                    ensure!(a.is_empty(), "QLIE interlace query ABI");
                    m.result(
                        0,
                        self.objects[&id]
                            .properties
                            .get("il")
                            .and_then(|v| v.first())
                            .cloned()
                            .unwrap_or(Value::Int(0)),
                    );
                }
                "ma" | "ma2" => {
                    //6EA71C ->705A7C ->6E2274 publishes four alpha slots.
                    //ABMP special tracks multiply these slots in the shared
                    //Layer projection; no private compositor is introduced.
                    if op == "ma2" {
                        ensure!(a.len() == 2, "QLIE indexed message alpha ABI");
                        let index = n(0)?.clamp(0, 3);
                        self.objects
                            .get_mut(&id)
                            .unwrap()
                            .properties
                            .insert(format!("ma/{index}"), vec![Value::Int(n(1)?)]);
                    } else {
                        ensure!(a.len() <= 4, "QLIE message alpha bank ABI");
                        for (index, value) in a.iter().enumerate() {
                            self.objects.get_mut(&id).unwrap().properties.insert(
                                format!("ma/{index}"),
                                vec![Value::Int(value.integer()?.clamp(0, 255))],
                            );
                        }
                    }
                }
                "?ma" | "?ma2" => {
                    if op == "?ma2" {
                        ensure!(a.len() == 1, "QLIE indexed message alpha query ABI");
                        let index = n(0)?.clamp(0, 3);
                        m.result(
                            0,
                            self.objects[&id]
                                .properties
                                .get(&format!("ma/{index}"))
                                .and_then(|v| v.first())
                                .cloned()
                                .unwrap_or(Value::Int(255)),
                        );
                    } else {
                        ensure!(a.is_empty(), "QLIE message alpha bank query ABI");
                        for index in 0..4 {
                            m.result(
                                index,
                                self.objects[&id]
                                    .properties
                                    .get(&format!("ma/{index}"))
                                    .and_then(|v| v.first())
                                    .cloned()
                                    .unwrap_or(Value::Int(255)),
                            );
                        }
                    }
                }
                "?vtv" | "?vtm" => {
                    ensure!(
                        a.len() == 1 && (0..32).contains(&n(0)?),
                        "QLIE audio mix group query ABI"
                    );
                    let current = h.audio_mix_group(self.client, &format!("qlie/{}", n(0)?))?;
                    let value = if op == "?vtm" {
                        i32::from(current.is_some_and(|v| v.settings.muted))
                    } else {
                        current
                            .map(|v| (v.settings.gain * 256.).round() as i32)
                            .unwrap_or(256)
                    };
                    m.result(0, Value::Int(value));
                }
                "vtv" => {
                    ensure!(
                        a.len() == 2 && (0..32).contains(&n(0)?),
                        "QLIE audio volume type"
                    );
                    let gain = n(1)? as f32 / 256.;
                    ensure!((0.0..=4.).contains(&gain), "QLIE audio volume range");
                    h.submit_audio(
                        self.client,
                        AudioRequest::SetMixGroupGain {
                            name: format!("qlie/{}", n(0)?),
                            gain,
                            fade_ms: 0.,
                        },
                    )?;
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(format!("vtv/{}", n(0)?), vec![Value::Int(n(1)?)]);
                }
                "vtm" => {
                    ensure!(
                        a.len() == 2 && (0..32).contains(&n(0)?),
                        "QLIE audio mute type"
                    );
                    let group = format!("qlie/{}", n(0)?);
                    let gain = self.objects[&id]
                        .properties
                        .get(&format!("vtv/{}", n(0)?))
                        .and_then(|v| v.first())
                        .map(Value::integer)
                        .transpose()?
                        .unwrap_or(256) as f32
                        / 256.;
                    h.submit_audio(
                        self.client,
                        AudioRequest::SetMixGroup {
                            name: group,
                            settings: crysirror_core::service::AudioMixGroupSettings {
                                muted: n(1)? != 0,
                                gain,
                                parent: None,
                            },
                            fade_ms: 0.,
                        },
                    )?;
                }
                // +4B8/+5B8/+7B8 per-voice volume, mute and the third bank.
                "?cv" | "?cvv" | "?cvm" | "?cvi" | "cvv" | "cvm" | "cvi" => {
                    let name = s(0).to_lowercase();
                    let source = self
                        .text
                        .get("system/voice/voicedata.txt")
                        .or_else(|| self.text.get("system/voice/voicedata.dat"))
                        .context("QLIE character voice catalog missing")?;
                    let mut aliases = BTreeMap::<usize, Vec<String>>::new();
                    for line in source
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.starts_with(';') && !l.starts_with("//"))
                    {
                        if let Some((key, value)) = line.split_once('=') {
                            if let Some(index) = key
                                .trim()
                                .to_ascii_lowercase()
                                .strip_prefix("act[")
                                .and_then(|s| s.strip_suffix(']'))
                                .and_then(|s| s.parse::<usize>().ok())
                                .filter(|i| *i < 64)
                            {
                                aliases.insert(
                                    index,
                                    value
                                        .split(',')
                                        .map(|s| s.trim().trim_matches('"').to_lowercase())
                                        .collect(),
                                );
                            }
                        }
                    }
                    // Native 6EA5F8 scans groups in index order and accepts
                    // either an exact alias or the group's wildcard entry.
                    let index = aliases
                        .iter()
                        .find(|(_, v)| v.iter().any(|s| s == &name || s == "*"))
                        .map(|(i, _)| *i);
                    if op == "?cv" {
                        m.result(0, Value::Int(index.map(|i| i as i32).unwrap_or(-1)));
                    } else {
                        let index = index.context("QLIE unknown character voice alias")?;
                        let key = format!("{}/{}", op.trim_start_matches('?'), index);
                        if op.starts_with('?') {
                            m.result(
                                0,
                                self.objects[&id]
                                    .properties
                                    .get(&key)
                                    .and_then(|v| v.first())
                                    .cloned()
                                    .unwrap_or(Value::Int(if op == "?cvv" { 256 } else { 0 })),
                            );
                        } else {
                            ensure!(a.len() == 2, "QLIE character voice setting ABI");
                            let value = n(1)?;
                            ensure!(
                                match op {
                                    "cvv" => (0..=256).contains(&value),
                                    "cvm" => (0..=1).contains(&value),
                                    _ => true,
                                },
                                "QLIE character voice setting range"
                            );
                            self.objects
                                .get_mut(&id)
                                .unwrap()
                                .properties
                                .insert(key, vec![Value::Int(value)]);
                        }
                    }
                }
                "ms" | "mtw" | "mcw" => {
                    ensure!(n(0)? >= 0 && n(0)? <= 60000, "QLIE message timing range");
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(op.into(), a);
                }
                "?ms" | "?mtw" | "?mcw" => {
                    let key = op.trim_start_matches('?');
                    let default = match key {
                        "ms" => 30,
                        "mtw" => 40,
                        _ => 1200,
                    };
                    m.result(
                        0,
                        self.objects[&id]
                            .properties
                            .get(key)
                            .and_then(|v| v.first())
                            .cloned()
                            .unwrap_or(Value::Int(default)),
                    );
                }
                "fn" | "fn2" => {
                    ensure!(
                        a.len() == 2 && (0..16).contains(&n(0)?),
                        "QLIE font slot setting ABI"
                    );
                    let fonts = h.font_library(self.client)?;
                    let alias = fonts
                        .resolve_alias(&s(1))?
                        .or(fonts.resolve_alias("default")?)
                        .context("QLIE configured font and fallback are unavailable")?;
                    let o = self.objects.get_mut(&id).unwrap();
                    o.properties
                        .insert(format!("font_alias/{}", n(0)?), vec![Value::Text(alias)]);
                    o.properties
                        .insert(format!("fn2/{}", n(0)?), a[1..].to_vec());
                }
                "fst" | "faa" | "fsc" | "fst2" | "faa2" | "fsc2" => {
                    ensure!(
                        a.len() == 2 && (0..16).contains(&n(0)?),
                        "QLIE font style slot setting ABI"
                    );
                    self.objects.get_mut(&id).unwrap().properties.insert(
                        format!("{}2/{}", op.trim_end_matches('2'), n(0)?),
                        vec![Value::Int(if op.starts_with("fsc") {
                            n(1)? & 0xffffff
                        } else {
                            n(1)?
                        })],
                    );
                }
                "?fn" | "?faa" | "?fst" | "?fsc" => {
                    ensure!(a.len() <= 1, "QLIE font slot query ABI");
                    let slots = if a.is_empty() {
                        (0..16).collect::<Vec<_>>()
                    } else {
                        let index = n(0)?;
                        ensure!((0..16).contains(&index), "QLIE font slot query index");
                        vec![index]
                    };
                    for (result, index) in slots.into_iter().enumerate() {
                        let key = format!("{}2/{index}", op.trim_start_matches('?'));
                        let value = self.objects[&id]
                            .properties
                            .get(&key)
                            .and_then(|v| v.first())
                            .cloned();
                        if op == "?fn" {
                            m.result_text(
                                result,
                                value.map(|v| v.text()).unwrap_or_else(|| "default".into()),
                            );
                        } else {
                            m.result(
                                result,
                                value.unwrap_or(Value::Int(match op {
                                    "?faa" => 1,
                                    _ => 0,
                                })),
                            );
                        }
                    }
                }
                // fnl,slot,list (6EDE35) assigns font slot `slot`'s candidate
                // list to a list object. 504BB0 builds the 16 lists from
                // system/font/fontdata(.dat|.txt) keys font[n].list[m] =
                // name[:charset] after the engine default font; the system,
                // system_fix, system_pro and system_ext entries enumerate the
                // installed families, which here are the host's font aliases.
                "fnl" => {
                    ensure!(a.len() == 2, "QLIE font list ABI");
                    let slot = n(0)?;
                    let target = s(1);
                    ensure!(
                        m.object_types.get(&target).is_some_and(|k| k == "list"),
                        "QLIE fnl target is not a list object: {target}"
                    );
                    if !(0..16).contains(&slot) {
                        return Ok(true);
                    }
                    let fonts = h.font_library(self.client)?;
                    let installed = fonts.aliases()?;
                    let mut names = vec![fonts.resolve_alias("default")?.unwrap_or_default()];
                    if let Some(text) = self.text.get("system/font/fontdata.txt")
                        .or_else(|| self.text.get("system/font/fontdata.dat")) {
                        let entries: BTreeMap<String, String> = text
                            .lines()
                            .filter_map(|l| l.split_once('='))
                            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
                            .collect();
                        for i in 0..32 {
                            let Some(value) = entries.get(&format!("font[{slot}].list[{i}]")) else {
                                continue;
                            };
                            let name = value.split(':').next().unwrap_or_default().trim();
                            if name.is_empty() {
                                continue;
                            }
                            if matches!(
                                name.to_ascii_lowercase().as_str(),
                                "system" | "system_fix" | "system_pro" | "system_ext"
                            ) {
                                names.extend(installed.iter().cloned());
                            } else {
                                names.push(name.to_owned());
                            }
                        }
                    }
                    let mut seen = BTreeSet::new();
                    names.retain(|n| seen.insert(n.to_ascii_lowercase()));
                    let text = names.join("\r\n");
                    if !self.objects.contains_key(&target) {
                        let layer = self.next_layer;
                        self.next_layer = self.next_layer.checked_add(1).context("QLIE object layer identity")?;
                        self.objects.insert(
                            target.clone(),
                            Object { kind: "list".into(), thread: m.thread_kind(), layer, gain: 1., ..Default::default() },
                        );
                    }
                    strings_install(self.objects.get_mut(&target).unwrap(), &text)?;
                }
                _ => bail!("unsupported QLIE cfg method \\{op}"),
            }
            return Ok(true);
        }
        if matches!(kind.as_str(), "log" | "llog" | "flog") {
            let o = self.objects.get_mut(&id).unwrap();
            if kind == "log" && native_log::method(o, m, op, &a)? {
                return Ok(true);
            }
            match op {
                "lk" => o.locked = n(0)? != 0,
                "?lk" => m.result(0, Value::Int(i32::from(o.locked))),
                "i" | "cc" => {
                    o.lines.clear();
                    o.table.clear();
                    o.properties.clear();
                }
                "a" => {
                    if !o.locked {
                        if kind == "log" {
                            // TImoObjectDataLog / 5D39A0 uses a 256-entry ring.
                            // Four strings and four integers are independent
                            // result banks, including omitted argument defaults.
                            ensure!(a.len() <= 8, "QLIE message log append ABI");
                            let mut entry = Vec::with_capacity(8);
                            for i in 0..4 {
                                entry.push(Value::Text(
                                    a.get(i).map(Value::text).unwrap_or_default(),
                                ));
                            }
                            for i in 4..8 {
                                entry.push(Value::Int(
                                    a.get(i).map(Value::integer).transpose()?.unwrap_or(0),
                                ));
                            }
                            if o.lines.len() == 256 {
                                o.lines.remove(0);
                            }
                            o.lines.push(serde_json::to_string(&entry)?);
                        } else {
                            ensure!(o.lines.len() < 8192, "QLIE native log limit");
                            o.lines.push(serde_json::to_string(&a)?);
                        }
                    }
                }
                "s" if kind == "log" => {
                    ensure!((1..=9).contains(&a.len()), "QLIE message log replace ABI");
                    // 6DF3A8 / 5D3C30: index zero is the newest entry;
                    // an index outside the ring leaves it unchanged.
                    if let Ok(index) = usize::try_from(n(0)?) {
                        if let Some(index) = o.lines.len().checked_sub(index.saturating_add(1)) {
                            let mut entry = Vec::with_capacity(8);
                            for i in 1..5 {
                                entry.push(Value::Text(
                                    a.get(i).map(Value::text).unwrap_or_default(),
                                ));
                            }
                            for i in 5..9 {
                                entry.push(Value::Int(
                                    a.get(i).map(Value::integer).transpose()?.unwrap_or(0),
                                ));
                            }
                            o.lines[index] = serde_json::to_string(&entry)?;
                        }
                    }
                }
                "?c" => m.result(0, Value::Int(o.lines.len() as i32)),
                "ct" if kind == "log" => {
                    ensure!(a.len() == 1, "QLIE log text conversion ABI");
                    m.result_text(0, log_plain_text(&s(0)));
                }
                "?g" if kind == "log" => {
                    let index = usize::try_from(n(0)?)?;
                    let entry = o
                        .lines
                        .len()
                        .checked_sub(index + 1)
                        .and_then(|i| o.lines.get(i))
                        .map(|s| serde_json::from_str::<Vec<Value>>(s))
                        .transpose()?
                        .unwrap_or_default();
                    for i in 0..4 {
                        m.result_text(i, entry.get(i).map(Value::text).unwrap_or_default());
                        m.result(
                            i,
                            Value::Int(
                                entry
                                    .get(i + 4)
                                    .map(Value::integer)
                                    .transpose()?
                                    .unwrap_or(0),
                            ),
                        );
                    }
                }
                "?g" | "g" if kind != "log" => {
                    let key = s(0).to_ascii_lowercase();
                    let value = o.table.get(&key).cloned().unwrap_or_default();
                    m.result(
                        0,
                        Value::Int(crate::expression::parse_integer(&value).unwrap_or(0)),
                    );
                    m.result_text(0, value);
                }
                "s" if kind != "log" => {
                    o.table.insert(s(0).to_ascii_lowercase(), s(1));
                }
                // flog d,name (5D50F8) deletes one named entry.
                "d" if kind == "flog" => {
                    o.table.remove(&s(0).to_ascii_lowercase());
                }
                // ?k answers the raw 6E9520 read state, k its ==1 test.
                "?k" | "?k2" | "k2" | "k" if kind == "llog" => {
                    m.result(0, Value::Int(i32::from(self.last_read)))
                }
                _ => bail!("unsupported QLIE native {kind} method \\{op}"),
            }
            return Ok(true);
        }
        if matches!(kind.as_str(), "tdat" | "mdat") {
            // 6FD388 TImoObjectDataStackSave: named reader saves kept in
            // memory in insertion order; mdat is embedded by dat\s flag 1.
            let store = kind.clone();
            let name = || crate::program::canonical(&s(0));
            match op {
                "lim" => {
                    ensure!(
                        a.len() == 1 && (0..=256).contains(&n(0)?),
                        "QLIE checkpoint count limit"
                    );
                    let stack = self.stacks.entry(store).or_default();
                    stack.limit = Some(n(0)? as usize);
                    stack.trim();
                }
                "?lim" => {
                    let limit = self.stacks.get(&store).and_then(|s| s.limit).unwrap_or(16);
                    m.result(0, Value::Int(limit as i32));
                }
                "i" => {
                    self.stacks.entry(store).or_default().entries.clear();
                }
                "g" | "?g" => {
                    ensure!(a.len() == 1, "QLIE checkpoint lookup ABI");
                    let name = name()?;
                    if self.stacks.get(&store).and_then(|s| s.get(&name)).is_some() {
                        m.result(0, Value::Int(1));
                    }
                }
                "d" => {
                    ensure!(a.len() == 1, "QLIE checkpoint remove ABI");
                    let name = name()?;
                    let stack = self.stacks.entry(store).or_default();
                    let before = stack.entries.len();
                    stack.entries.retain(|e| e.name != name);
                    if stack.entries.len() != before {
                        m.result(0, Value::Int(1));
                    }
                }
                "s" => {
                    ensure!((1..=3).contains(&a.len()), "QLIE checkpoint save ABI");
                    let flags = if a.len() >= 2 { n(1)? } else { 0 };
                    let mode = if a.len() >= 3 { n(2)? } else { 2 };
                    return self.save_reader(
                        h,
                        m,
                        save_metadata::SaveTarget::Memory { store, name: name()? },
                        flags,
                        mode,
                    );
                }
                "l" => {
                    ensure!(a.len() == 1, "QLIE checkpoint load ABI");
                    return self.load_reader(
                        h,
                        m,
                        save_metadata::SaveTarget::Memory { store, name: name()? },
                    );
                }
                "c" => {
                    ensure!((1..=9).contains(&a.len()), "QLIE checkpoint metadata ABI");
                    let strings = a[1..].iter().map(Value::text).collect::<Vec<_>>();
                    let done = self.memory_set_metadata(&store, &name()?, &strings)?;
                    m.result(0, Value::Int(i32::from(done)));
                }
                "cp" => {
                    ensure!(a.len() == 2, "QLIE checkpoint copy ABI");
                    let source = name()?;
                    let destination = crate::program::canonical(&s(1))?;
                    let stack = self.stacks.entry(store).or_default();
                    if let Some(entry) = stack.get(&source).cloned() {
                        stack.put(save_state::StackEntry {
                            name: destination,
                            envelope: entry.envelope,
                        });
                        m.result(0, Value::Int(1));
                    }
                }
                "gd" | "?gd" => {
                    ensure!(a.len() == 1, "QLIE checkpoint metadata query ABI");
                    let meta = self.memory_metadata(&store, &name()?);
                    self.report_metadata(m, meta)?;
                }
                _ => bail!("unsupported QLIE native checkpoint method \\{op}"),
            }
            return Ok(true);
        }
        if kind == "timer" {
            // 6D3644 TImoObjectDataTimer; 6AA42C defaults, 6AA598 ticking.
            let o = self.objects.get_mut(&id).unwrap();
            let int = |o: &Object, key: &str, default: i32| {
                o.properties
                    .get(key)
                    .and_then(|v| v.first())
                    .and_then(|v| v.integer().ok())
                    .unwrap_or(default)
            };
            match op {
                "i" => {
                    o.properties.retain(|k, _| !k.starts_with("timer/"));
                }
                "l" => {
                    ensure!((1..=2).contains(&a.len()), "QLIE timer label ABI");
                    o.properties
                        .insert("timer/label".into(), vec![Value::Text(s(0))]);
                    o.properties
                        .insert("timer/file".into(), vec![Value::Text(s(1))]);
                }
                "?l" => {
                    for (i, key) in ["timer/label", "timer/file"].into_iter().enumerate() {
                        let value = o
                            .properties
                            .get(key)
                            .and_then(|v| v.first())
                            .map(Value::text)
                            .unwrap_or_default();
                        m.result_text(i, value);
                    }
                }
                "t" => {
                    ensure!(a.len() == 1, "QLIE timer interval ABI");
                    o.properties
                        .insert("timer/interval".into(), vec![Value::Int(n(0)?.max(0))]);
                }
                "?t" => m.result(0, Value::Int(int(o, "timer/interval", 0))),
                "e" => {
                    o.properties.insert("timer/enabled".into(), vec![Value::Int(1)]);
                }
                "d" => {
                    o.properties.remove("timer/enabled");
                    o.properties.remove("timer/acc");
                }
                "?e" => m.result(
                    0,
                    Value::Int(i32::from(o.properties.contains_key("timer/enabled"))),
                ),
                "tt" => {
                    ensure!(a.len() == 1, "QLIE timer clock ABI");
                    o.properties.insert("timer/tt".into(), vec![Value::Int(n(0)?)]);
                }
                "?tt" => m.result(0, Value::Int(int(o, "timer/tt", 0))),
                "in" => {
                    ensure!(a.len() == 1, "QLIE timer input mask ABI");
                    o.properties.insert("timer/in".into(), vec![Value::Int(n(0)?)]);
                }
                "?in" => m.result(0, Value::Int(int(o, "timer/in", 255))),
                // +29 output group mask (6AA42C default 0xFF).
                "out" => {
                    ensure!(a.len() == 1, "QLIE timer output mask ABI");
                    o.properties.insert("timer/out".into(), vec![Value::Int(n(0)?)]);
                }
                "?out" => m.result(0, Value::Int(int(o, "timer/out", 255))),
                _ => bail!("unsupported QLIE timer method \\{op}"),
            }
            return Ok(true);
        }
        if kind == "dat" {
            match op {
                "s" => {
                    ensure!((1..=3).contains(&a.len()), "QLIE original-save ABI");
                    let flags = if a.len() >= 2 { n(1)? } else { 0 };
                    let mode = if a.len() >= 3 { n(2)? } else { 2 };
                    let name = crate::program::canonical(&s(0))?;
                    return self.save_reader(
                        h,
                        m,
                        save_metadata::SaveTarget::File(name),
                        flags,
                        mode,
                    );
                }
                "l" => {
                    ensure!(a.len() == 1, "QLIE original-load ABI");
                    let name = crate::program::canonical(&s(0))?;
                    return self.load_reader(h, m, save_metadata::SaveTarget::File(name));
                }
                "d" => {
                    ensure!(a.len() == 1, "QLIE original-save delete ABI");
                    return self.mutate_save(h, m, &s(0), save_metadata::Mutation::Delete);
                }
                "cp" => {
                    ensure!(a.len() == 2, "QLIE original-save copy ABI");
                    let destination = crate::program::canonical(&s(1))?;
                    return self.mutate_save(
                        h,
                        m,
                        &s(0),
                        save_metadata::Mutation::Copy { destination },
                    );
                }
                "c" => {
                    ensure!(
                        (1..=9).contains(&a.len()),
                        "QLIE original-save metadata update ABI"
                    );
                    return self.mutate_save(
                        h,
                        m,
                        &s(0),
                        save_metadata::Mutation::SetMetadata {
                            strings: a[1..].iter().map(Value::text).collect(),
                        },
                    );
                }
                "g" | "?g" | "gd" | "?gd" => {
                    //6DB24C: g/?g validates slot presence; gd/?gd emits
                    //seven calendar integers and eight metadata strings.
                    ensure!(a.len() == 1, "QLIE save query ABI");
                    return self.query_save(h, m, &s(0), matches!(op, "gd" | "?gd"));
                }
                _ => bail!("unsupported QLIE original-save method \\{op}"),
            }
        }
        if kind == "rep" {
            if op == "?r" {
                ensure!(a.len() == 1, "QLIE replacement query ABI");
                m.result_text(0, self.rewrite_line(&s(0), &id)?);
                return Ok(true);
            }
            let o = self.objects.get_mut(&id).unwrap();
            match op {
                "lk" => o.locked = n(0)? != 0,
                "a" => {
                    ensure!(
                        (2..=3).contains(&a.len()) && o.lines.len() < 4096,
                        "QLIE replacement rule ABI/budget"
                    );
                    let mode = n(2)?;
                    ensure!(
                        (0..=3).contains(&mode),
                        "unsupported QLIE replacement mode {mode}"
                    );
                    let find = s(0);
                    let pattern = if mode == 0 {
                        regex::escape(&find)
                    } else {
                        find.clone()
                    };
                    o.regex_rules.insert(
                        find.clone(),
                        regex::RegexBuilder::new(&pattern)
                            .case_insensitive(mode == 0)
                            .size_limit(1024 * 1024)
                            .build()?,
                    );
                    if !o.lines.contains(&find) {
                        o.lines.push(find.clone());
                    }
                    // 6BCC44 replaces an existing rule in place; a new rule is
                    // appended and the list re-sorted with CompareStr (6BCF80),
                    // so 6BC3E0's last-to-first pass prefers longer prefixes.
                    o.lines.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                    o.table.insert(find.clone(), s(1));
                    o.properties
                        .insert(format!("rule:{find}"), vec![Value::Int(mode)]);
                }
                "d" => {
                    let find = s(0);
                    o.lines.retain(|name| name != &find);
                    o.table.remove(&find);
                    o.regex_rules.remove(&find);
                    o.properties.remove(&format!("rule:{find}"));
                }
                "i" => {
                    o.lines.clear();
                    o.table.clear();
                    o.properties.clear();
                    o.regex_rules.clear();
                    o.locked = false;
                }
                "et" | "nt" => {
                    o.properties.insert(op.into(), a);
                }
                "?c" => m.result(0, Value::Int(o.lines.len() as i32)),
                //Native exposes the enabled byte here, despite the name ?lk.
                "?lk" => m.result(0, Value::Int(i32::from(!o.locked))),
                "?g" => {
                    ensure!(a.len() == 1, "QLIE replacement entry query ABI");
                    let index = n(0)?;
                    m.result_text(0, "");
                    m.result_text(1, "");
                    m.result(0, Value::Int(0));
                    if let Some(find) = usize::try_from(index)
                        .ok()
                        .and_then(|index| o.lines.get(index))
                    {
                        m.result_text(0, find);
                        m.result_text(1, o.table.get(find).cloned().unwrap_or_default());
                        m.result(
                            0,
                            o.properties
                                .get(&format!("rule:{find}"))
                                .and_then(|values| values.first())
                                .cloned()
                                .unwrap_or(Value::Int(0)),
                        );
                    }
                }
                _ => bail!("unsupported QLIE replacement method \\{op}"),
            }
            return Ok(true);
        }
        if kind == "math" {
            return self.math_method(m, op, &a);
        }
        if kind == "misc" {
            if let Some(done) = self.misc_native_method(h, m, op, &a)? {
                return Ok(done);
            }
        }
        if kind == "misc" && op != "prm" {
            return self.misc_method(h, m, op, &a);
        }
        if kind == "misc" && op == "prm" {
            ensure!(a.is_empty(), "QLIE command-line parameter ABI");
            // Native 6EF018 / prm returns ParamCount and at most 16 strings.
            // Host renderer/input options are never exposed as game arguments.
            m.result(0, Value::Int(self.config.parameters.len() as i32));
            for i in 0..16 {
                m.result_text(
                    i,
                    self.config.parameters.get(i).cloned().unwrap_or_default(),
                );
            }
            return Ok(true);
        }
        if kind == "skey" {
            if self.gesture_guide_method(h, m, op, &a)? {
                return Ok(true);
            }
            let o = self.objects.get_mut(&id).unwrap();
            match op {
                "i" => {
                    o.properties.clear();
                    o.locked = false;
                }
                "lk" => o.locked = n(0)? != 0,
                "?lk" => m.result(0, Value::Int(i32::from(o.locked))),
                "a" => {
                    ensure!(
                        (2..=5).contains(&a.len()) && o.properties.len() < 512,
                        "QLIE key binding"
                    );
                    let code = n(0)?;
                    o.properties.insert(code.to_string(), a[1..].to_vec());
                    o.properties.insert(
                        format!("source/{code}"),
                        vec![Value::Text(m.reader_path().to_owned())],
                    );
                }
                // d,code (6CA898) removes one binding.
                "d" => {
                    let code = n(0)?;
                    o.properties.remove(&code.to_string());
                    o.properties.remove(&format!("source/{code}"));
                }
                // dc,n (+30): the sound channel of the key-click effect,
                // clamped to the manager's channel range (64 slots here).
                "dc" => {
                    let channel = n(0)?.clamp(0, 63);
                    o.properties.insert("native/dc".into(), vec![Value::Int(channel)]);
                }
                _ => bail!("unsupported QLIE system key method \\{op}"),
            };
            return Ok(true);
        }
        if kind == "sys" {
            if self.sys_native_method(h, m, &id, op, &a)? {
                return Ok(true);
            }
            let o = self.objects.get_mut(&id).unwrap();
            match op {
                "dskp" => {
                    m.result(0, Value::Int(i32::from(h.now_ms() < self.skip_until)));
                }
                // skp and ?skp both answer the skip flag (6D9434 LABEL_48).
                "?skp" | "skp" => {
                    let mode = o
                        .properties
                        .get("p")
                        .and_then(|v| v.first())
                        .map(Value::integer)
                        .transpose()?
                        .unwrap_or(0);
                    m.result(
                        0,
                        Value::Int(i32::from(
                            mode >= 3
                                || self.input.buttons_down & crysirror_core::service::buttons::CTRL
                                    != 0,
                        )),
                    );
                }
                "?mrd" => {
                    let frame = std::iter::once(&m.frame)
                        .chain(m.stack.iter().rev())
                        .find(|f| {
                            f.path.starts_with("script/") && !f.path.starts_with("script/title/")
                        })
                        .unwrap_or(&m.frame);
                    let label = m.programs[&frame.path]
                        .labels
                        .iter()
                        .filter(|(_, pc)| **pc <= frame.pc)
                        .max_by_key(|(_, pc)| **pc)
                        .map(|(s, _)| s.clone())
                        .unwrap_or_default();
                    m.result_text(0, frame.path.clone());
                    m.result_text(1, label);
                    m.result(0, Value::Int(i32::try_from(m.frame.pc)?));
                }
                "eve" => {
                    ensure!(a.len() == 2, "QLIE system event ABI");
                    ensure!(
                        s(0).eq_ignore_ascii_case("OnFrame"),
                        "unsupported QLIE system event {}",
                        s(0)
                    );
                    o.properties.insert(
                        format!("eve:{}", s(0).to_ascii_lowercase()),
                        vec![a[1].clone(), Value::Text(m.reader_path().to_owned())],
                    );
                }
                "ap" => {
                    o.properties.insert("ap".into(), a);
                }
                "?mt" => {
                    m.result(0, Value::Int(i32::try_from(self.tick_ms)?));
                }
                "?ap" => {
                    m.result(
                        0,
                        o.properties
                            .get("ap")
                            .and_then(|v| v.first())
                            .cloned()
                            .unwrap_or(Value::Int(0)),
                    );
                }
                // 6D9434: n stores seven save-metadata strings with [...]
                // tags removed (6D9158); n2,i,s stores one raw (i <= 6).
                "n" => {
                    for i in 0..7 {
                        let mut depth = 0i32;
                        let plain: String = a
                            .get(i)
                            .map(Value::text)
                            .unwrap_or_default()
                            .chars()
                            .filter(|c| match c {
                                '[' => {
                                    depth += 1;
                                    false
                                }
                                ']' => {
                                    depth -= 1;
                                    false
                                }
                                _ => depth == 0,
                            })
                            .collect();
                        o.properties
                            .insert(format!("meta/{i}"), vec![Value::Text(plain)]);
                    }
                }
                "n2" => {
                    ensure!(a.len() == 2, "QLIE save metadata string ABI");
                    if let Ok(index) = u32::try_from(n(0)?) {
                        if index <= 6 {
                            o.properties
                                .insert(format!("meta/{index}"), vec![Value::Text(s(1))]);
                        }
                    }
                }
                "?n" => {
                    for i in 0..8 {
                        let value = o
                            .properties
                            .get(&format!("meta/{i}"))
                            .and_then(|v| v.first())
                            .map(Value::text)
                            .unwrap_or_default();
                        m.result_text(i, value);
                    }
                }
                "lk" | "lkc" | "p" | "tz" => {
                    o.properties.insert(op.into(), a);
                }
                // 6D9434: sr stores a raw @@label (6E9000) at system+28;
                // fcs stores the form-close save filename at system+32.
                "sr" | "fcs" => {
                    ensure!(a.len() == 1, "QLIE system string field ABI");
                    o.properties.insert(op.into(), vec![Value::Text(s(0))]);
                }
                "?sr" | "?fcs" => {
                    let value = o
                        .properties
                        .get(op.trim_start_matches('?'))
                        .and_then(|v| v.first())
                        .map(Value::text)
                        .unwrap_or_default();
                    m.result_text(0, value);
                }
                // tef: thumbnail effect mode/x/y at system+412..420, applied
                // by 6D5218 to the save thumbnail.
                "tef" => {
                    ensure!((1..=3).contains(&a.len()), "QLIE thumbnail effect ABI");
                    let values = (0..3)
                        .map(|i| a.get(i).map(Value::integer).transpose())
                        .collect::<Result<Vec<_>>>()?
                        .into_iter()
                        .map(|v| Value::Int(v.unwrap_or(0)))
                        .collect();
                    o.properties.insert("tef".into(), values);
                }
                "?tef" => {
                    let values = o.properties.get("tef").cloned().unwrap_or_default();
                    for i in 0..3 {
                        m.result(i, values.get(i).cloned().unwrap_or(Value::Int(0)));
                    }
                }
                // 6D9434 in/?in: the global input mask at system+364. The
                // constructor default is 0xFFFFFFF; an omitted argument 0xFFFFFF.
                "in" => {
                    let mask = if a.is_empty() { 0xFFFFFF } else { n(0)? };
                    o.properties.insert("in".into(), vec![Value::Int(mask)]);
                }
                "?in" => {
                    let mask = o
                        .properties
                        .get("in")
                        .and_then(|v| v.first())
                        .cloned()
                        .unwrap_or(Value::Int(0xFFFFFFF));
                    m.result(0, mask);
                }
                // ?ver returns the build string literal and 1 (6DB154).
                "?ver" => {
                    m.result_text(0, "Release");
                    m.result(0, Value::Int(1));
                }
                // ?gs returns the configured game screen width/height.
                "?gs" => {
                    m.result(0, Value::Int(i32::try_from(self.config.canvas[0])?));
                    m.result(1, Value::Int(i32::try_from(self.config.canvas[1])?));
                }
                "?lk" | "?lkc" | "?p" => {
                    m.result(
                        0,
                        o.properties
                            .get(op.trim_start_matches('?'))
                            .and_then(|v| v.first())
                            .cloned()
                            .unwrap_or(Value::Int(0)),
                    );
                }
                _ => bail!("unsupported QLIE system method \\{op}"),
            }
            return Ok(true);
        }
        if matches!(kind.as_str(), "cam" | "camex") {
            return self.camera_method(h, m, &id, op, &a);
        }
        if matches!(kind.as_str(), "msg" | "btn" | "sbtn") {
            if let Some(done) = self.glyph_picture_method(h, m, &id, op, &a)? {
                return Ok(done);
            }
        }
        if matches!(kind.as_str(), "msg" | "btn" | "sbtn")
            && self.message_method(h, m, &id, ins, &a)?
        {
            return Ok(true);
        }
        if kind == "bar" {
            return self.bar_method(h, m, &id, op, &a);
        }
        if kind == "eff" {
            return self.effect(h, m, op, &a);
        }
        if kind == "inp" {
            if self.input_device_method(h, m, &id, op, &a)? {
                return Ok(true);
            }
            match op {
                // 6F98D0 ?mw: this frame's accumulated WheelDelta (+100),
                // +120 per notch away from the user. The host input frame
                // carries notch edges, at most one per direction per frame.
                "?mw" => {
                    use crysirror_core::service::buttons as B;
                    let up = i32::from(self.input.buttons_pressed & B::WHEEL_UP != 0);
                    let down = i32::from(self.input.buttons_pressed & B::WHEEL_DOWN != 0);
                    m.result(0, Value::Int(120 * (up - down)));
                }
                // swp,1 -> 5C7DA0 clears the swipe latch and velocity.
                "swp" => {
                    ensure!(a.len() == 1, "QLIE swipe reset ABI");
                    if n(0)? == 1 {
                        self.swipe.reset_latch();
                    }
                }
                // ?swp,masked -> 5C845C (shared pointer_swipe tracker).
                "?swp" => {
                    ensure!(a.len() == 1, "QLIE swipe query ABI");
                    let r = self.swipe.report(n(0)? == 1);
                    let values = [
                        r.start[0], r.start[1], r.pointer[0], r.pointer[1], r.state,
                        r.direction, r.delta[0], r.delta[1],
                    ];
                    for (i, v) in values.into_iter().enumerate() {
                        m.result(i, Value::Int(v));
                    }
                }
                // ?mgp,flag (5C7F50): press xy, pointer xy, drag buttons
                // (drag-masked when flag is 1) and the eight-way gesture
                // direction (-1 within the drag threshold).
                "?mgp" => {
                    ensure!(a.len() == 1, "QLIE gesture query ABI");
                    let g = self.swipe.gesture(n(0)? == 1, true);
                    let values = [
                        g.press[0], g.press[1], g.pointer[0], g.pointer[1], g.buttons, g.direction,
                    ];
                    for (i, v) in values.into_iter().enumerate() {
                        m.result(i, Value::Int(v));
                    }
                }
                "?mge" => m.result(
                    0,
                    self.objects[&id]
                        .properties
                        .get("mge")
                        .and_then(|v| v.first())
                        .cloned()
                        .unwrap_or(Value::Int(0)),
                ),
                "?mp" => {
                    m.result(0, Value::Int(self.input.pointer_x));
                    m.result(1, Value::Int(self.input.pointer_y));
                }
                "gbc" | "gsc" => {
                    ensure!(a.is_empty(), "QLIE native input receipt ABI");
                    m.thread_globals_mut()
                        .insert("resultobj[0]".into(), Value::Int(0));
                    let selected = if op == "gbc" {
                        // Read only button subtrees: a button poll must not
                        // consume a pending slider change or dialog result.
                        let buttons = self
                            .objects
                            .iter()
                            .filter(|(_, o)| {
                                matches!(o.kind.as_str(), "btn" | "sbtn")
                                    && o.ui_registered
                                    && o.thread == self.objects[&id].thread
                            })
                            .map(|(id, o)| (id.clone(), o.layer))
                            .collect::<Vec<_>>();
                        for (button, layer) in buttons {
                            let result =
                                h.query_engine(self.client, EngineQuery::UiSubtreeEvents(layer))?;
                            let events: Vec<serde_json::Value> = serde_json::from_slice(&result)?;
                            if events.iter().any(|e| e["kind"] == "activate") {
                                self.objects
                                    .get_mut(&button)
                                    .unwrap()
                                    .properties
                                    .insert("input/pending_click".into(), vec![Value::Int(1)]);
                            }
                        }
                        self.objects
                            .iter()
                            .find(|(_, o)| {
                                matches!(o.kind.as_str(), "btn" | "sbtn")
                                    && o.thread == self.objects[&id].thread
                                    && o.properties.contains_key("input/pending_click")
                            })
                            .map(|(id, _)| id.clone())
                    } else {
                        self.objects
                            .iter()
                            .find(|(_, o)| {
                                o.kind == "bar"
                                    && o.thread == self.objects[&id].thread
                                    && o.properties.contains_key("bar/pending_change")
                            })
                            .map(|(id, _)| id.clone())
                    };
                    if let Some(selected) = selected {
                        let o = self.objects.get_mut(&selected).unwrap();
                        o.properties.remove(if op == "gbc" {
                            "input/pending_click"
                        } else {
                            "bar/pending_change"
                        });
                        m.thread_globals_mut()
                            .insert("resultobj[0]".into(), Value::Text(selected));
                        for index in 0..4 {
                            m.result_text(
                                index,
                                o.properties
                                    .get("us")
                                    .and_then(|v| v.get(index))
                                    .map(Value::text)
                                    .unwrap_or_default(),
                            );
                            m.result(
                                index,
                                o.properties
                                    .get("ui")
                                    .and_then(|v| v.get(index))
                                    .cloned()
                                    .unwrap_or(Value::Int(0)),
                            );
                        }
                    }
                }
                // swe,mask[,inertia]: 6F98D0 stores +428/+432 (inertia
                // defaults to 1).
                "swe" => {
                    let inertia = if a.len() >= 2 { n(1)? } else { 1 };
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(op.into(), vec![Value::Int(n(0)?), Value::Int(inertia)]);
                }
                // mge,v only stores +424 for ?mge; scripts poll ?mgp.
                "mge" => {
                    ensure!(a.len() == 1, "QLIE gesture enable ABI");
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(op.into(), vec![Value::Int(n(0)?)]);
                }
                "sc" | "?sc" => {
                    use crysirror_core::service::buttons as B;
                    let mut clicked =
                        self.input.buttons_pressed & (B::MOUSE_LEFT | B::ENTER | B::SPACE) != 0;
                    if let Some(keys) = self
                        .objects
                        .get(&Self::builtin("skey", self.objects[&id].thread))
                        .filter(|o| !o.locked)
                    {
                        for (key, binding) in &keys.properties {
                            let Ok(code) = key.parse::<i32>() else {
                                continue;
                            };
                            if !binding.first().is_some_and(|v| {
                                matches!(
                                    v.text().to_ascii_lowercase().as_str(),
                                    "click" | "keyclick"
                                )
                            }) {
                                continue;
                            }
                            clicked |= self.native_key_active(code)?;
                        }
                    }
                    m.result(0, Value::Int(i32::from(clicked)));
                }
                // 6F95A0 k2,code...: every active code (at most 16) in
                // argument order; 6F9734 kl2,first,last scans a code range,
                // where codes the host cannot produce are never active.
                "k2" | "kl2" => {
                    let codes: Vec<i32> = if op == "k2" {
                        a.iter().map(Value::integer).collect::<Result<_>>()?
                    } else {
                        ensure!(a.len() == 2, "QLIE key range query ABI");
                        (n(0)?..=n(1)?).take(0x100000).collect()
                    };
                    let mut active = Vec::new();
                    for code in codes {
                        let on = if op == "kl2" {
                            self.native_key_active(code).unwrap_or(false)
                        } else {
                            self.native_key_active(code)?
                        };
                        if on && active.len() < 16 {
                            active.push(code);
                        }
                    }
                    if active.is_empty() {
                        m.result(0, Value::Int(0));
                    }
                    for (i, code) in active.into_iter().enumerate() {
                        m.result(i, Value::Int(code));
                    }
                }
                _ => bail!("unsupported QLIE input method \\{op}"),
            }
            return Ok(true);
        }
        if matches!(kind.as_str(), "img" | "btn" | "msg" | "simg" | "sbtn")
            && (self.abmp_native_method(h, m, &id, op, &a)?
                || self.image_native_method(h, m, &id, op, &a)?)
        {
            return Ok(true);
        }
        if kind == "theora" && self.theora_native_method(h, m, &id, op, &a)? {
            return Ok(true);
        }
        if kind == "snd" && self.snd_native_method(h, m, &id, op, &a)? {
            return Ok(true);
        }
        if matches!(
            kind.as_str(),
            "img" | "btn" | "msg" | "simg" | "sbtn" | "theora" | "snd"
        ) {
            match op {
                "ck" if matches!(kind.as_str(), "btn" | "sbtn") => {
                    ensure!(a.len() == 1, "QLIE button checked-state ABI");
                    let checked = n(0)? != 0;
                    let o = self.objects.get_mut(&id).unwrap();
                    o.properties
                        .insert("ck".into(), vec![Value::Int(i32::from(checked))]);
                    //6A1D70/6A1D08 use dedicated enable/disable motions; the
                    //normal motion remains the fallback when none is defined.
                    let names = if checked {
                        ["*OnCheckEnable", "*OnCheck"]
                    } else {
                        ["*OnCheckDisable", "*OffCheck"]
                    };
                    if let Some(motion) = o.bundle.as_ref().and_then(|b| {
                        names.iter().find_map(|name| {
                            b.animation
                                .motions
                                .iter()
                                .position(|v| v.name.eq_ignore_ascii_case(name))
                        })
                    }) {
                        o.motion = motion;
                        o.offset = 0;
                        o.started = h.now_ms();
                        o.events.clear();
                        o.animation_clocks.clear();
                    }
                }
                "?ck" if matches!(kind.as_str(), "btn" | "sbtn") => {
                    ensure!(a.is_empty(), "QLIE button checked-state query ABI");
                    m.result(
                        0,
                        self.objects[&id]
                            .properties
                            .get("ck")
                            .and_then(|v| v.first())
                            .cloned()
                            .unwrap_or(Value::Int(0)),
                    );
                }
                "?lk" => {
                    ensure!(a.is_empty(), "QLIE image lock query ABI");
                    m.result(0, Value::Int(i32::from(self.objects[&id].locked)));
                }
                "sc" if kind != "snd" => {
                    //The image dispatcher and5D03D0 bind the original snd[] slot, not the
                    //currently playing engine voice. A later snd reload must
                    //remain visible to the animation's wave observation.
                    ensure!(
                        a.len() == 1 && (-1..64).contains(&n(0)?),
                        "QLIE animation wave channel"
                    );
                    self.objects.get_mut(&id).unwrap().wave_channel = u32::try_from(n(0)?).ok();
                }
                "?sc" if kind != "snd" => {
                    ensure!(a.is_empty(), "QLIE animation wave channel query ABI");
                    m.result(
                        0,
                        Value::Int(
                            self.objects[&id]
                                .wave_channel
                                .map(|v| v as i32)
                                .unwrap_or(-1),
                        ),
                    );
                }
                "wim" if kind != "snd" => {
                    ensure!(a.len() == 1, "QLIE animation wave multiplier ABI");
                    let percent = n(0)?.max(0);
                    let o = self.objects.get_mut(&id).unwrap();
                    o.wave_multiplier = Some((f64::from(percent) / 100.) as f32);
                    o.properties.insert("wim".into(), vec![Value::Int(percent)]);
                }
                "?wim" if kind != "snd" => {
                    ensure!(a.is_empty(), "QLIE animation wave multiplier query ABI");
                    m.result(
                        0,
                        self.objects[&id]
                            .properties
                            .get("wim")
                            .and_then(|v| v.first())
                            .cloned()
                            .unwrap_or(Value::Int(100)),
                    );
                }
                "i" => {
                    let old = self.objects.remove(&id).unwrap();
                    let layer = old.layer;
                    let kind = old.kind.clone();
                    let thread = old.thread;
                    self.free_object(h, old)?;
                    self.objects.insert(
                        id,
                        Object {
                            kind,
                            layer,
                            thread,
                            gain: 1.,
                            ..Default::default()
                        },
                    );
                }
                "l" => {
                    if s(0).is_empty() {
                        let mut old = self.objects.remove(&id).unwrap();
                        if kind == "snd" {
                            if old.channel != 0 {
                                h.submit_audio(
                                    self.client,
                                    AudioRequest::Stop {
                                        channel: old.channel,
                                        fade_ms: 0.,
                                    },
                                )?;
                            }
                            old.properties.remove("sk");
                        }
                        let mut message = old.message.take();
                        if let Some(message) = &mut message {
                            message.created = false;
                            message.bound_parent = None;
                        }
                        self.free_visual(h, &old)?;
                        self.objects.insert(
                            id,
                            Object {
                                kind: old.kind,
                                thread: old.thread,
                                layer: old.layer,
                                position: old.position,
                                properties: old.properties,
                                gain: old.gain,
                                message,
                                ..Default::default()
                            },
                        );
                        return Ok(true);
                    }
                    if s(0).to_ascii_lowercase().starts_with("$$$gamethreadimage") {
                        return self.load_game_thread_image(h, &id, &s(0));
                    }
                    let path = if kind == "theora" {
                        self.resolve_movie(m, &s(0))?
                    } else {
                        self.resolve(m, &s(0))?
                    };
                    if kind == "theora" || kind == "snd" {
                        let o = self.objects.get_mut(&id).unwrap();
                        if kind == "snd" && o.asset.as_deref() != Some(&path) {
                            // Native 5C3F8C -> 582A88 -> 582858 closes the old
                            // decoder and starts the new source at zero. The
                            // previous source's seek is not a new-file offset.
                            if o.channel != 0 {
                                h.submit_audio(
                                    self.client,
                                    AudioRequest::Stop {
                                        channel: o.channel,
                                        fade_ms: 0.,
                                    },
                                )?;
                            }
                            o.channel = 0;
                            o.properties.remove("sk");
                        }
                        o.asset = Some(path);
                        return Ok(true);
                    }
                    return self.load_image(h, &id, &path);
                }
                "li" => {
                    let source = s(1);
                    let path = if source.is_empty() || source.starts_with("$$$") {
                        source
                    } else {
                        self.resolve(m, &source)?
                    };
                    // 5D2054 records the slot name for ?li (+144).
                    if let Ok(slot @ 0..32) = usize::try_from(n(0)?) {
                        self.objects.get_mut(&id).unwrap().properties.insert(format!("li/{slot}"), vec![Value::Text(s(1))]);
                    }
                    return self.replace_image(h, &id, usize::try_from(n(0)?)?, &path);
                }
                "ls" => {
                    let index = usize::try_from(n(0)?)?;
                    ensure!(index < 256, "QLIE animation sound slot limit");
                    // 5D220C records the slot name for ?ls (+536).
                    if index < 32 {
                        self.objects.get_mut(&id).unwrap().properties.insert(format!("ls/{index}"), vec![Value::Text(s(1))]);
                    }
                    let path = self.resolve(m, &s(1))?;
                    if self.load.is_none() {
                        let storage = h.storage(self.client)?;
                        let source = path.clone();
                        let client = self.client;
                        self.load = Some((
                            path.clone(),
                            self.pool.spawn(move |cancel| {
                                resources::load(storage, client, &source, &cancel)
                            })?,
                        ));
                    }
                    let (expected, job) = self.load.as_mut().unwrap();
                    ensure!(
                        expected == &path,
                        "QLIE replacement sound completion identity"
                    );
                    let Some(loaded) = job.try_take()? else {
                        return Ok(false);
                    };
                    self.load = None;
                    let resources::Loaded::Blob(bytes) = loaded else {
                        bail!("QLIE animation sound replacement is not encoded audio")
                    };
                    let Some(bundle) = self
                        .objects
                        .get_mut(&id)
                        .unwrap()
                        .bundle
                        .as_mut()
                    else {
                        // Some QLIE scripts replace a window sound before the
                        // object has an ABMP animation bundle. The command has
                        // no slot to attach to; keep the VM running and leave
                        // the requested path recorded in `ls/{index}`.
                        return Ok(true);
                    };
                    while bundle.sounds.len() <= index {
                        bundle.sounds.push(resources::Member {
                            name: String::new(),
                            bytes: Arc::from([]),
                            metadata: json!({}),
                            slot: usize::MAX,
                            nested: None,
                            model: None,
                            scene_model: None,
                        });
                    }
                    let sound = &mut bundle.sounds[index];
                    sound.bytes = bytes;
                    sound.name = path;
                }
                "s" if kind == "snd" => {
                    ensure!(a.len() <= 1, "QLIE sound stop ABI");
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .remove("save/sound_loops");
                    let channel = self.objects[&id].channel;
                    if channel != 0 {
                        h.submit_audio(
                            self.client,
                            AudioRequest::Stop {
                                channel,
                                fade_ms: n(0)?.max(0) as f32,
                            },
                        )?;
                    }
                }
                "p" | "p2" if kind == "snd" => {
                    if self.objects[&id].asset.is_none() {
                        return Ok(true);
                    }
                    let loops = if op == "p" && n(0)? != 0 { -1 } else { n(0)? };
                    if loops == -2 && self.objects[&id].channel != 0 {
                        h.submit_audio(
                            self.client,
                            AudioRequest::Resume {
                                channel: self.objects[&id].channel,
                                delay_ms: 0.,
                                fade_ms: 0.,
                            },
                        )?;
                        return Ok(true);
                    }
                    return self.play_sound(h, &id, loops);
                }
                // 6DC194 fo: per-channel filter stages (584CF4), applied by
                // the next playback through a prepared shared PCM chain.
                "fo" if kind == "snd" => {
                    ensure!(a.len() == 1, "QLIE sound filter ABI");
                    super::sound_filters::parse(&s(0))?;
                    let o = self.objects.get_mut(&id).unwrap();
                    if s(0).is_empty() {
                        o.properties.remove("fo");
                    } else {
                        o.properties.insert("fo".into(), vec![Value::Text(s(0))]);
                    }
                }
                "sk" if kind == "snd" => {
                    ensure!(a.len() == 1 && n(0)? >= 0, "QLIE sound seek ABI");
                    if let Some((owner, ticket)) = &self.audio_seek {
                        ensure!(owner == &id, "QLIE sound seek continuation owner");
                        let status = h.audio_seek_preparation(self.client, *ticket)?;
                        match status.phase {
                            AudioPreparePhase::Pending => return Ok(false),
                            AudioPreparePhase::Ready => {
                                h.release_audio_seek(self.client, *ticket)?;
                                self.audio_seek = None;
                            }
                            AudioPreparePhase::Failed => {
                                let error = status.error;
                                h.release_audio_seek(self.client, *ticket)?;
                                self.audio_seek = None;
                                bail!("QLIE sound seek failed: {error:?}");
                            }
                        }
                    } else {
                        let channel = self.objects[&id].channel;
                        if channel != 0 {
                            if let Some(info) = h.audio_info(self.client, channel)? {
                                let frame = (n(0)? as u64)
                                    .checked_mul(u64::from(info.source_rate))
                                    .context("QLIE sound seek frame overflow")?
                                    / 1000;
                                let ticket = h.begin_audio_seek(self.client, channel, frame)?;
                                self.audio_seek = Some((id.clone(), ticket));
                                return Ok(false);
                            }
                        }
                    }
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(op.into(), a);
                }
                "pan" if kind == "snd" => {
                    let o = self.objects.get_mut(&id).unwrap();
                    let channel = o.channel;
                    o.properties.insert(op.into(), a.clone());
                    if channel != 0 {
                        h.submit_audio(
                            self.client,
                            AudioRequest::SetPan {
                                channel,
                                pan: (n(0)? as f32 / 512. + 0.5).clamp(0., 1.),
                                fade_ms: n(1)?.max(0) as f32,
                            },
                        )?;
                    }
                }
                // 6DC194 ?sk -> 5C4B2C: decoder stream position, rounded ms.
                "?sk" if kind == "snd" => {
                    let time = h
                        .audio_info(self.client, self.objects[&id].channel)?
                        .map(|i| {
                            (i.position_frames as f64 * 1000. / f64::from(i.source_rate.max(1)))
                                .round() as i64
                        })
                        .unwrap_or(0);
                    m.result(0, Value::Int(i32::try_from(time)?));
                }
                "?nt" | "?tt" if kind == "snd" => {
                    let info = h.audio_info(self.client, self.objects[&id].channel)?;
                    let time = info
                        .map(|i| {
                            if op == "?nt" {
                                i.position_frames.saturating_mul(1000)
                                    / u64::from(i.source_rate.max(1))
                            } else {
                                i.source_frames.unwrap_or(0).saturating_mul(1000)
                                    / u64::from(i.source_rate.max(1))
                            }
                        })
                        .unwrap_or(0);
                    m.result(0, Value::Int(i32::try_from(time)?));
                }
                "p" if kind == "theora" => {
                    if self.objects[&id].media.is_none() {
                        let value = (0..1024)
                            .find(|n| !self.media_ids.contains(n))
                            .context("QLIE media IDs exhausted")?;
                        self.media_ids.insert(value);
                        self.objects.get_mut(&id).unwrap().media = Some(value);
                    }
                    let o = &self.objects[&id];
                    let asset = o.asset.clone().context("QLIE movie not loaded")?;
                    let layer = o.media.unwrap();
                    let image = h.media_request(
                        self.client,
                        MediaRequest::Play {
                            id: layer,
                            asset,
                            looping: false,
                            audio: true,
                            gain: o.gain,
                        },
                    )?;
                    self.objects.get_mut(&id).unwrap().properties.insert(
                        "media_image".into(),
                        vec![Value::Text(image.relative().into())],
                    );
                    self.put_parent(h, &id)?;
                }
                "?tt" | "tt" if kind == "theora" => {
                    let info = h.media_info(
                        self.client,
                        self.objects[&id].media.context("QLIE movie not playing")?,
                    )?;
                    if matches!(info.phase, MediaPhase::Loading) {
                        return Ok(false);
                    }
                    let duration = info
                        .duration_ms
                        .context("QLIE movie decoder duration unavailable")?;
                    m.result(0, Value::Int(i32::try_from(duration)?));
                }
                "p4" => {
                    ensure!((11..=12).contains(&a.len()), "QLIE full image pose ABI");
                    let repeat = animation::repeat(&self.objects[&id], "pose_repeat", n(11)?);
                    self.objects.get_mut(&id).unwrap().position = [n(0)?, n(1)?, n(9)?];
                    self.position_repeated(h, &id, n(10)?.max(0) as u64, repeat)?;
                    let o = &self.objects[&id];
                    let layer = o.layer;
                    // Native mm=6 applies jitter to XY only. The remaining
                    // pose components use its base ease-out calculator.
                    let mode = o
                        .properties
                        .get("mm")
                        .and_then(|v| v.first())
                        .and_then(|v| v.integer().ok())
                        .unwrap_or(3);
                    let easing = animation::easing_with_strength(
                        if mode == 6 { 3 } else { mode },
                        o.properties
                            .get("mm")
                            .and_then(|v| v.get(1))
                            .and_then(|v| v.integer().ok())
                            .unwrap_or(100),
                    )?;
                    let mut commands = Vec::new();
                    for (i, property) in [
                        "z",
                        "scale_x",
                        "scale_y",
                        "scale_z",
                        "rotation_x",
                        "rotation_y",
                        "rotation_z",
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let index = i + 2;
                        let value = if index == 2 {
                            n(index)? as f32
                        } else if index < 6 {
                            n(index)? as f32 / 100.
                        } else {
                            (n(index)? as f32).to_radians()
                        };
                        commands.push(json!({"op":"begin_motion","id":layer,"track":{"property":property,"to":value,"duration":n(10)?.max(0),"repeat":repeat,"easing":easing}}));
                    }
                    self.submit(h, json!({"layers":commands}))?;
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert("pose_repeat".into(), vec![Value::Int(repeat)]);
                }
                "alg" | "alg2" => {
                    let factor = if op == "alg2" { 1 } else { 50 };
                    self.objects.get_mut(&id).unwrap().properties.insert(
                        "alg".into(),
                        vec![Value::Int(n(0)? * factor), Value::Int(n(1)? * factor)],
                    );
                    self.put_parent(h, &id)?;
                    self.apply_alignment(h, &id)?;
                }
                "g2" => {
                    self.put_parent(h, &id)?;
                    self.generator_method(h, &id, op, &a)?;
                }
                // 6F1370 ost,on,start,length (5D1D64): sound-synchronised
                // timeline; ?ost returns them (6DF950 pushes the flag only when set).
                "ost" if kind != "snd" => {
                    ensure!(a.len() == 3, "QLIE sound-timeline ABI");
                    let values = vec![Value::Int(i32::from(n(0)? == 1)), Value::Int(n(1)?), Value::Int(n(2)?)];
                    self.objects.get_mut(&id).unwrap().properties.insert("ost".into(), values);
                }
                // 6F1370 pri/?pri: wrapper +20 update group (see render).
                "pri" if kind != "snd" => {
                    ensure!(a.len() == 1, "QLIE update priority ABI");
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert("pri".into(), vec![Value::Int(n(0)?)]);
                }
                "?pri" if kind != "snd" => {
                    let pri = self.objects[&id]
                        .properties
                        .get("pri")
                        .and_then(|v| v.first())
                        .cloned()
                        .unwrap_or(Value::Int(0));
                    m.result(0, pri);
                }
                // 6F1370 anc,obj: +132 anchor object reference (type 2);
                // any other argument type is the native type error.
                "anc" => {
                    ensure!(a.len() == 1, "QLIE anchor ABI");
                    let anchor = s(0);
                    ensure!(
                        anchor.is_empty() || m.object_types.contains_key(&anchor),
                        "QLIE anchor argument is not an object"
                    );
                    let o = self.objects.get_mut(&id).unwrap();
                    if anchor.is_empty() {
                        o.properties.remove("anc");
                    } else {
                        o.properties.insert("anc".into(), vec![Value::Text(anchor.clone())]);
                    }
                    if !anchor.is_empty() && !self.objects.contains_key(&anchor) {
                        let layer = self.allocate_layer()?;
                        let kind = m.object_types[&anchor].clone();
                        self.objects.insert(
                            anchor.clone(),
                            Object { thread: m.thread_kind(), kind, layer, gain: 1., ..Default::default() },
                        );
                    }
                    if self.objects[&id].created {
                        self.put_parent(h, &id)?;
                    }
                }
                // ?wh -> 5D12C0/5D1278: the image's canvas width and height.
                "?wh" if kind != "snd" => {
                    let [w, height] = super::render::image_extent(&self.objects[&id]);
                    m.result(0, Value::Int(w as i32));
                    m.result(1, Value::Int(height as i32));
                }
                // ?p4: the ten p4 pose values (x,y,z, scale x/y/z in percent,
                // rotation x/y/z in degrees, priority), sampled from the layer.
                "?p4" if kind != "snd" => {
                    let o = &self.objects[&id];
                    let mut pose = [o.position[0], o.position[1], 0, 100, 100, 100, 0, 0, 0, o.position[2]];
                    if o.created {
                        let layer: serde_json::Value = serde_json::from_slice(
                            &h.query_engine(self.client, EngineQuery::Layer(o.layer))?,
                        )?;
                        let f = |v: &serde_json::Value, d: f64| v.as_f64().unwrap_or(d);
                        let t = &layer["transform"];
                        pose[0] = f(&layer["rect"][0], pose[0] as f64).round() as i32;
                        pose[1] = f(&layer["rect"][1], pose[1] as f64).round() as i32;
                        pose[2] = f(&t["z"], 0.).round() as i32;
                        pose[3] = (f(&t["scale_x"], 1.) * 100.).round() as i32;
                        pose[4] = (f(&t["scale_y"], 1.) * 100.).round() as i32;
                        pose[5] = (f(&t["scale_z"], 1.) * 100.).round() as i32;
                        pose[6] = f(&t["rotate_x"], 0.).to_degrees().round() as i32;
                        pose[7] = f(&t["rotate_y"], 0.).to_degrees().round() as i32;
                        pose[8] = f(&t["rotate"], 0.).to_degrees().round() as i32;
                    }
                    for (i, v) in pose.into_iter().enumerate() {
                        m.result(i, Value::Int(v));
                    }
                }
                // 6F1370 mi/?mi: +3044 input-blocking byte checked by 6B04B4
                // (vmt+40): a hit on this object stops lower objects' hit
                // tests. The shared UI dispatches to the topmost widget.
                // A bare `\mi` (BIMAN1 uppermenu/layout.s) is valid: the
                // native argument list returns "" for a missing field and
                // `mi` has no omitted-field branch, so the calculator's empty
                // result 0 is stored as `0 == 1` (not blocking).
                "mi" => {
                    ensure!(a.len() <= 1, "QLIE input block ABI");
                    let block = i32::from(n(0)? == 1);
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert("mi".into(), vec![Value::Int(block)]);
                }
                "?mi" => {
                    let block = self.objects[&id]
                        .properties
                        .get("mi")
                        .and_then(|v| v.first())
                        .cloned()
                        .unwrap_or(Value::Int(0));
                    m.result(0, block);
                }
                // 6F1370 in: button input group at +3053 (omitted -> 255).
                "in" => {
                    let mask = if a.is_empty() { 255 } else { n(0)? };
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert("in".into(), vec![Value::Int(mask)]);
                }
                "dc" | "sc" | "out" => {
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(op.into(), a);
                }
                "ps" => {
                    ensure!((0..=3000).contains(&n(0)?), "QLIE animation play speed");
                    let o = self.objects.get_mut(&id).unwrap();
                    let old = o
                        .properties
                        .get("ps")
                        .and_then(|v| v.first())
                        .and_then(|v| v.integer().ok())
                        .unwrap_or(100)
                        .max(0) as u64;
                    if o.visible {
                        o.offset = o.offset.saturating_add(
                            h.now_ms().saturating_sub(o.started).saturating_mul(old) / 100,
                        );
                    }
                    o.started = h.now_ms();
                    o.properties.insert(op.into(), a);
                }
                "?us" | "?ui" => {
                    let values = self.objects[&id]
                        .properties
                        .get(op.trim_start_matches('?'))
                        .cloned()
                        .unwrap_or_default();
                    for i in 0..4 {
                        if op == "?us" {
                            m.result_text(i, values.get(i).map(Value::text).unwrap_or_default());
                        } else {
                            m.result(i, values.get(i).cloned().unwrap_or(Value::Int(0)));
                        }
                    }
                }
                "?m" => {
                    let o = &self.objects[&id];
                    let name = o
                        .bundle
                        .as_ref()
                        .and_then(|b| b.animation.motions.get(o.motion))
                        .map(|v| v.name.clone())
                        .unwrap_or_default();
                    m.result_text(0, name);
                    m.result(0, Value::Int(o.motion as i32));
                }
                "midx" => {
                    let index = self.objects[&id]
                        .bundle
                        .as_ref()
                        .and_then(|b| b.animation.motions.iter().position(|v| v.name == s(0)))
                        .map(|v| v as i32)
                        .unwrap_or(-1);
                    m.result(0, Value::Int(index));
                }
                "?e" => {
                    m.result(0, Value::Int(i32::from(self.objects[&id].visible)));
                }
                "bm" => {
                    ensure!(a.len() == 1, "QLIE image blend ABI");
                    let native = n(0)? as u8;
                    self.put_parent(h, &id)?;
                    //55E8EC dispatches all native byte modes, including the
                    //defined fallback for selectors outside its table.
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert("native_bm".into(), vec![Value::Int(i32::from(native))]);
                }
                "p" | "pp" => {
                    ensure!((3..=4).contains(&a.len()), "QLIE image position ABI");
                    let o = self.objects.get_mut(&id).unwrap();
                    o.position = [n(0)?, n(1)?, n(2)?];
                    self.position(
                        h,
                        &id,
                        a.get(3)
                            .map(Value::integer)
                            .transpose()?
                            .unwrap_or(0)
                            .max(0) as u64,
                    )?;
                }
                "?p" if kind == "snd" => {
                    let playing = h
                        .audio_info(self.client, self.objects[&id].channel)?
                        .is_some_and(|s| s.source_playing);
                    m.result(0, Value::Int(i32::from(playing)));
                }
                "?p" => {
                    let o = &self.objects[&id];
                    let mut position = o.position;
                    if o.created {
                        let layer: serde_json::Value = serde_json::from_slice(
                            &h.query_engine(self.client, EngineQuery::Layer(o.layer))?,
                        )?;
                        position[0] =
                            layer["rect"][0].as_f64().context("QLIE position query X")? as i32;
                        position[1] =
                            layer["rect"][1].as_f64().context("QLIE position query Y")? as i32;
                    }
                    for (i, value) in position.into_iter().enumerate() {
                        m.result(i, Value::Int(value));
                    }
                }
                "e" | "d" => {
                    let o = self.objects.get_mut(&id).unwrap();
                    let visible = op == "e";
                    if visible != o.visible {
                        if o.visible {
                            let speed = o
                                .properties
                                .get("ps")
                                .and_then(|v| v.first())
                                .and_then(|v| v.integer().ok())
                                .unwrap_or(100)
                                .max(0) as u64;
                            o.offset = o.offset.saturating_add(
                                h.now_ms().saturating_sub(o.started).saturating_mul(speed) / 100,
                            );
                        }
                        o.started = h.now_ms();
                        o.visible = visible;
                    }
                    self.put_parent(h, &id)?;
                }
                "m" => {
                    let motion = if let Some(Value::Text(name)) =
                        a.first().filter(|v| v.integer().is_err())
                    {
                        match self.objects[&id].bundle.as_ref().and_then(|b| {
                            b.animation
                                .motions
                                .iter()
                                .position(|v| v.name.eq_ignore_ascii_case(name))
                        }) {
                            Some(index) => index,
                            None => return Ok(true),
                        }
                    } else {
                        if n(0)? == -1 {
                            return Ok(true);
                        }
                        usize::try_from(n(0)?)?
                    };
                    let o = self.objects.get_mut(&id).unwrap();
                    if o.motion == motion && n(1)? == 0 {
                        return Ok(true);
                    }
                    o.motion = motion;
                    o.offset = 0;
                    o.started = h.now_ms();
                    o.events.clear();
                    o.animation_clocks.clear();
                }
                "skp" => {
                    let o = self.objects.get_mut(&id).unwrap();
                    o.offset = n(0)?.max(0) as u64;
                    o.started = h.now_ms();
                    for clock in o.animation_clocks.values_mut() {
                        clock.seek(o.offset);
                    }
                }
                "lk" => {
                    self.objects.get_mut(&id).unwrap().locked = n(0)? != 0;
                    self.put_parent(h, &id)?;
                }
                "v" if kind == "snd" => {
                    let o = self.objects.get_mut(&id).unwrap();
                    o.gain = n(0)? as f32 / 256.;
                    o.properties.insert("v".into(), a.clone());
                    if o.channel != 0 {
                        h.submit_audio(
                            self.client,
                            AudioRequest::SetGain {
                                channel: o.channel,
                                gain: o.gain,
                                fade_ms: n(1)?.max(0) as f32,
                            },
                        )?;
                    }
                }
                "?v" if kind == "snd" => {
                    let o = &self.objects[&id];
                    let gain = if o.channel == 0 {
                        o.gain
                    } else {
                        h.audio_info(self.client, o.channel)?
                            .map(|voice| voice.gain)
                            .unwrap_or(o.gain)
                    };
                    m.result(0, Value::Int((gain * 256.).round() as i32));
                }
                "exr" => {
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .retain(|k, _| !k.starts_with("exv/"));
                }
                "exv" | "?exv" => {
                    ensure!(
                        (0..32).contains(&n(0)?),
                        "QLIE animation external variable index"
                    );
                    let key = format!("exv/{}", n(0)?);
                    if op == "exv" {
                        self.objects
                            .get_mut(&id)
                            .unwrap()
                            .properties
                            .insert(key, vec![a.get(1).cloned().unwrap_or(Value::Int(0))]);
                    } else {
                        m.result(
                            0,
                            self.objects[&id]
                                .properties
                                .get(&key)
                                .and_then(|v| v.first())
                                .cloned()
                                .unwrap_or(Value::Int(0)),
                        );
                    }
                }
                "fo" => {
                    ensure!((0..32).contains(&n(0)?), "QLIE filter slot");
                    return self.image_filter(h, &id, n(0)? as usize, &s(1));
                }
                "g" => {
                    self.put_parent(h, &id)?;
                    self.generator_method(h, &id, op, &a)?;
                }
                "col" => {
                    ensure!((4..=5).contains(&a.len()), "QLIE image colour ABI");
                    let repeat = animation::repeat(&self.objects[&id], "colour_repeat", n(4)?);
                    self.put_parent(h, &id)?;
                    let layer = self.objects[&id].layer;
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(op.into(), a.clone());
                    let duration = n(3)?.max(0);
                    // Native image colour animation (5D14A0) fixes the
                    // calculator to linear, independently of image mm/cm.
                    let easing = crysirror_scene::layer::Easing::Linear;
                    let mut commands = vec![
                        json!({"op":"begin_motion","id":layer,"track":{"property":"opacity","to":n(0)?.clamp(0,255),"duration":duration,"repeat":repeat,"easing":easing}}),
                    ];
                    if n(1)? != -1 {
                        let color = n(1)? as u32;
                        for (property, shift) in [
                            ("multiply_red", 16),
                            ("multiply_green", 8),
                            ("multiply_blue", 0),
                        ] {
                            commands.push(json!({"op":"begin_motion","id":layer,"track":{"property":property,"to":((color>>shift)&255) as f32/255.,"duration":duration,"repeat":repeat,"easing":easing}}));
                        }
                    }
                    self.submit(h, json!({"layers":commands}))?;
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert("colour_repeat".into(), vec![Value::Int(repeat)]);
                }
                "v" => {
                    self.objects.get_mut(&id).unwrap().gain = n(0)? as f32 / 256.;
                }
                "?dc" => {
                    m.result(
                        0,
                        self.objects[&id]
                            .properties
                            .get("dc")
                            .and_then(|v| v.first())
                            .cloned()
                            .unwrap_or(Value::Int(0)),
                    );
                }
                "ds" | "hs" => {
                    ensure!(
                        a.len() == 2 || a.len() == 4,
                        "QLIE drawing/hit rectangle ABI"
                    );
                    let rect = if a.len() == 2 {
                        [0, 0, n(0)?, n(1)?]
                    } else {
                        [
                            n(0)?,
                            n(1)?,
                            n(2)?.saturating_sub(n(0)?),
                            n(3)?.saturating_sub(n(1)?),
                        ]
                    };
                    ensure!(
                        rect[2] >= 0 && rect[3] >= 0,
                        "QLIE drawing/hit rectangle order"
                    );
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(op.into(), rect.map(Value::Int).to_vec());
                    if op == "ds" {
                        self.put_parent(h, &id)?;
                        let layer = self.objects[&id].layer;
                        if rect[2] == 0 || rect[3] == 0 {
                            self.submit(h,json!({"layers":[{"op":"set_viewport_clip","id":layer,"clip":[0,0,0,0]}]}))?;
                        } else {
                            self.submit(h,json!({"layers":[{"op":"set_clip","id":layer,"clip":rect},{"op":"set_viewport_clip","id":layer,"clip":null}]}))?;
                        }
                    }
                }
                "ui" | "us" => {
                    let o = self.objects.get_mut(&id).unwrap();
                    if op == "us" {
                        o.properties.insert(
                            "callback_file".into(),
                            vec![Value::Text(m.reader_path().to_owned())],
                        );
                    }
                    o.properties.insert(op.into(), a);
                }
                "mm" => {
                    ensure!((1..=2).contains(&a.len()), "QLIE image movement-mode ABI");
                    // 5CFE78 stores the mode as a byte and clamps strength;
                    // 6F1370 supplies 100 when the second argument is omitted.
                    let mode = i32::from(n(0)? as u8);
                    let strength = if a.len() > 1 { n(1)? } else { 100 }.clamp(0, 200);
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(op.into(), vec![Value::Int(mode), Value::Int(strength)]);
                }
                "sp" | "tf" | "tab" | "cm" | "vtn" => {
                    self.objects
                        .get_mut(&id)
                        .unwrap()
                        .properties
                        .insert(op.into(), a);
                }
                _ => bail!("unsupported QLIE {kind} method \\{op}"),
            }
            return Ok(true);
        }
        bail!("unsupported QLIE object {kind} method \\{op}")
    }
    pub(super) fn play_sound(&mut self, h: &mut dyn HostServices, id: &str, loops: i32) -> Result<bool> {
        if let Some((ticket, owner)) = self.audio_resource.clone() {
            ensure!(owner == id, "QLIE filtered sound continuation owner");
            let status = h.audio_resource_preparation(self.client, ticket)?;
            match status.phase {
                AudioPreparePhase::Pending => return Ok(false),
                AudioPreparePhase::Failed => {
                    bail!("QLIE filtered audio preparation: {:?}", status.error)
                }
                AudioPreparePhase::Ready => {
                    let resource = status.resource.context("QLIE filtered audio handle")?;
                    h.release_audio_resource_preparation(self.client, ticket)?;
                    self.audio_resource = None;
                    let (channel, parameters) = self.sound_parameters(id, loops)?;
                    let ticket = h.begin_audio(
                        self.client,
                        AudioRequest::PlayResource { channel, resource, parameters },
                    )?;
                    // Admitted work keeps its own decoder; the handle may go.
                    h.release_audio_resource(self.client, resource)?;
                    self.audio_ticket = Some(ticket);
                    return Ok(false);
                }
            }
        }
        if let Some(ticket) = self.audio_ticket {
            let status = h.audio_preparation(self.client, ticket).with_context(|| {
                let object = &self.objects[id];
                format!(
                    "QLIE sound {id} source {:?}, seek {:?}",
                    object.asset,
                    object.properties.get("sk")
                )
            })?;
            match status.phase {
                AudioPreparePhase::Pending => return Ok(false),
                AudioPreparePhase::Failed => bail!("QLIE audio preparation: {:?}", status.error),
                AudioPreparePhase::Ready => {
                    h.release_audio(self.client, ticket)?;
                    self.audio_ticket = None;
                    return Ok(true);
                }
            }
        }
        let path = self.objects[id]
            .asset
            .clone()
            .context("QLIE audio not loaded")?;
        let o = self.objects.get_mut(id).unwrap();
        o.channel = o.layer;
        // Restoration evidence for a reader save: which loop request owns
        // this channel. One-shot voices/effects are not re-armed on load.
        o.properties
            .insert("save/sound_loops".into(), vec![Value::Int(loops)]);
        let filters = o
            .properties
            .get("fo")
            .and_then(|v| v.first())
            .map(Value::text)
            .map(|s| super::sound_filters::parse(&s))
            .transpose()?;
        let source = AudioAsset {
            path: VfsPath::new(Mount::Asset, &path)?,
            frame_limit: None,
        };
        if let Some(chain) = filters.filter(|c| !c.filters.is_empty()) {
            let ticket = h.begin_transformed_audio_resource(
                self.client,
                source,
                crysirror_core::service::AudioResourceTransform::PcmFilters(chain.filters.into()),
            )?;
            self.audio_resource = Some((ticket, id.to_owned()));
            return self.play_sound(h, id, loops);
        }
        let (channel, parameters) = self.sound_parameters(id, loops)?;
        let ticket = h.begin_audio(
            self.client,
            AudioRequest::PlayAsset {
                channel,
                source,
                intro: None,
                parameters,
            },
        )?;
        self.audio_ticket = Some(ticket);
        Ok(false)
    }
    /// Shared voice parameters for a snd object (timescale is a rate).
    fn sound_parameters(&self, id: &str, loops: i32) -> Result<(u32, AudioParameters)> {
        let o = &self.objects[id];
        let group = o
            .properties
            .get("vtn")
            .and_then(|v| v.first())
            .map(Value::integer)
            .transpose()?
            .unwrap_or(0);
        let rate = o
            .properties
            .get("fo")
            .and_then(|v| v.first())
            .map(|s| super::sound_filters::parse(&s.text()))
            .transpose()?
            .map_or(1., |c| c.rate);
        let mut parameters = AudioParameters {
                    looping: loops != 0,
                    loop_passes: if loops > 0 {
                        Some(loops as u32 + 1)
                    } else {
                        None
                    },
                    pan: o
                        .properties
                        .get("pan")
                        .and_then(|v| v.first())
                        .and_then(|v| v.integer().ok())
                        .map(|v| (v as f32 / 512. + 0.5).clamp(0., 1.))
                        .unwrap_or(0.5),
                    start_ms: o
                        .properties
                        .get("sk")
                        .and_then(|v| v.first())
                        .and_then(|v| v.integer().ok())
                        .unwrap_or(0)
                        .max(0) as u64,
                    gain: o.gain,
                    fade_ms: o
                        .properties
                        .get("v")
                        .and_then(|v| v.get(1))
                        .and_then(|v| v.integer().ok())
                        .unwrap_or(0)
                        .max(0) as f32,
                    mix_group: Some(format!("qlie/{group}")),
                    bus: if group == 0 {
                        PcmBus::Music
                    } else if group == 1 {
                        PcmBus::Voice
                    } else {
                        PcmBus::Sfx
                    },
                    ..Default::default()
                };
        parameters.rate = rate;
        Ok((o.channel, parameters))
    }
}
