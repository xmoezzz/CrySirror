// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! KAGParser native semantics only. The original parser owns scenario control;
//! script handlers draw through CrySirror's existing Layer/Canvas/Text services.
use crate::natives::*;
use krkr_kag::{
    Attribute, AttributeValue, CallFrame, DebugLevel, KagError, KagHost, KagParser, LabelEvent,
    ScenarioLoadEvent, ScriptEvent, Tag,
};
use krkr_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, TjsHost, Variant},
};

#[derive(Clone)]
pub struct ParserState {
    pub boundary: bool,
    pub parser: KagParser,
    pub revision: u64,
}
impl Default for ParserState {
    fn default() -> Self {
        Self {
            parser: KagParser::new(),
            revision: 0,
            boundary: true,
        }
    }
}
fn error(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(e.to_string())
}
fn state(rt: &Rt, h: ObjectHandle) -> Result<&ParserState> {
    rt.host()
        .parsers
        .get(&h.0)
        .ok_or_else(|| error("KAGParser is not constructed"))
}
fn dict(rt: &mut Rt, attrs: &[Attribute]) -> Result<ObjectHandle> {
    let h = rt.alloc_dictionary_object();
    for a in attrs {
        match a {
            Attribute::Named { name, value } => {
                let v = match value {
                    AttributeValue::Literal(s) => Variant::String(s.clone()),
                    AttributeValue::Void => Variant::Void,
                    _ => return Err(error("unresolved KAG attribute")),
                };
                rt.set_object_member(h, name, v);
            }
            Attribute::Spread => return Err(error("unresolved KAG macro spread")),
        }
    }
    Ok(h)
}
fn tag(rt: &mut Rt, t: &Tag) -> Result<Variant> {
    let h = dict(rt, &t.attributes)?;
    rt.set_object_member(h, "tagname", Variant::String(t.tagname.clone()));
    Ok(Variant::Object(h))
}
fn macro_table(rt: &Rt, h: ObjectHandle) -> Result<ObjectHandle> {
    rt.object_member(h, "__kag_macros")
        .object_handle()
        .ok_or_else(|| error("native macro dictionary absent"))
}
fn synced_parser(rt: &Rt, h: ObjectHandle) -> Result<KagParser> {
    let mut p = state(rt, h)?.parser.clone();
    let m = macro_table(rt, h)?;
    let definitions = rt
        .object_members(m)
        .into_iter()
        .map(|(n, v)| match v {
            Variant::String(s) => Ok((n, s)),
            _ => Err(error("native macro dictionary contains a non-string body")),
        })
        .collect::<Result<Vec<_>>>()?;
    p.set_macro_definitions(definitions);
    Ok(p)
}
fn publish_macros(rt: &mut Rt, h: ObjectHandle, p: &KagParser) -> Result<()> {
    let m = macro_table(rt, h)?;
    for (n, _) in rt.object_members(m) {
        rt.delete_object_member(m, &n);
    }
    for (n, s) in p.macro_definitions() {
        rt.set_object_member(m, n, Variant::String(s.into()));
    }
    Ok(())
}
fn operate(
    rt: &mut Rt,
    this: Option<ObjectHandle>,
    f: impl FnOnce(&mut KagParser, &mut Bridge<'_>) -> krkr_kag::Result<Variant>,
) -> Result<Variant> {
    let h = this_of(rt, this, "KAGParser")?;
    let mut p = synced_parser(rt, h)?;
    let rev = state(rt, h)?.revision;
    let mut host = Bridge {
        rt,
        owner: h,
        exception: None,
    };
    let result = f(&mut p, &mut host);
    // Callbacks see the published parser state. A nested mutation takes precedence
    // over the outer copy; native getter/store calls do not mark a mutation.
    let nested = state(host.rt, h)?.revision != rev;
    if !nested {
        let boundary = state(host.rt, h)?.boundary;
        publish_macros(host.rt, h, &p)?;
        host.rt.host_mut().parsers.insert(
            h.0,
            ParserState {
                boundary,
                parser: p,
                revision: rev
                    .checked_add(1)
                    .ok_or_else(|| error("KAG parser revision exhausted"))?,
            },
        );
    }
    match result {
        Ok(v) => Ok(v),
        Err(e) => Err(host.exception.take().unwrap_or_else(|| error(e))),
    }
}
fn get(rt: &mut Rt, t: Option<ObjectHandle>, field: &str) -> Result<Variant> {
    let h = this_of(rt, t, "KAGParser")?;
    let p = &state(rt, h)?.parser;
    Ok(match field {
        "curStorage" => Variant::String(p.cur_storage().unwrap_or("").into()),
        "curLabel" => Variant::String(p.cur_label().unwrap_or("").into()),
        "curLine" => Variant::Integer(p.cur_line().unwrap_or(1).saturating_sub(1) as i64),
        "curPos" => Variant::Integer(p.cur_pos().unwrap_or(0) as i64),
        "curLineStr" => Variant::String(p.cur_line_str().unwrap_or("").into()),
        "callStackDepth" => Variant::Integer(p.call_stack_depth() as i64),
        "ignoreCR" => Variant::Integer(p.ignore_cr() as i64),
        "processSpecialTags" => Variant::Integer(p.process_special_tags() as i64),
        "debugLevel" => Variant::Integer(match p.debug_level() {
            DebugLevel::None => 0,
            DebugLevel::Simple => 1,
            DebugLevel::Verbose => 2,
        }),
        "macros" => Variant::Object(macro_table(rt, h)?),
        "macroParams" | "mp" => {
            let attrs = p.macro_params().to_vec();
            Variant::Object(dict(rt, &attrs)?)
        }
        _ => return Err(error("unknown KAG parser property")),
    })
}
pub fn install(rt: &mut Rt) {
    let c = native_class(rt, "KAGParser", |rt, t, _| {
        let h = this_of(rt, t, "KAGParser")?;
        if rt.host().parsers.len() >= 128 && !rt.host().parsers.contains_key(&h.0) {
            return Err(error("KAG parser limit"));
        }
        if !rt.host().parsers.contains_key(&h.0) {
            let m = rt.alloc_dictionary_object();
            rt.set_object_member(h, "__kag_macros", Variant::Object(m));
            rt.host_mut().parsers.insert(h.0, ParserState::default());
        }
        Ok(Variant::Void)
    });
    methods(
        rt,
        c,
        &[
            ("finalize", |rt, t, _| {
                let h = this_of(rt, t, "KAGParser.finalize")?;
                rt.host_mut().parsers.remove(&h.0);
                Ok(Variant::Void)
            }),
            ("loadScenario", |rt, t, a| {
                let s = string(&a, 0, "KAGParser.loadScenario")?;
                operate(rt, t, |p, h| {
                    p.load_scenario_with(s, h)?;
                    Ok(Variant::Void)
                })
            }),
            ("getNextTag", |rt, t, _| {
                operate(rt, t, |p, h| match p.next_tag_with(h)? {
                    Some(t) => {
                        h.rt.host_mut()
                            .parsers
                            .get_mut(&h.owner.0)
                            .unwrap()
                            .boundary = false;
                        tag(h.rt, &t).map_err(|e| h.wrap(e))
                    }
                    None => Ok(Variant::Void),
                })
            }),
            ("goToLabel", |rt, t, a| {
                let label = string(&a, 0, "KAGParser.goToLabel")?;
                operate(rt, t, |p, h| {
                    p.go_to_label(&label)?;
                    h.rt.host_mut()
                        .parsers
                        .get_mut(&h.owner.0)
                        .unwrap()
                        .boundary = true;
                    Ok(Variant::Void)
                })
            }),
            ("callLabel", |rt, t, a| {
                let label = string(&a, 0, "KAGParser.callLabel")?;
                operate(rt, t, |p, _| {
                    p.call_label(&label)?;
                    Ok(Variant::Void)
                })
            }),
            ("clear", |rt, t, _| {
                operate(rt, t, |p, h| {
                    p.clear();
                    h.rt.host_mut()
                        .parsers
                        .get_mut(&h.owner.0)
                        .unwrap()
                        .boundary = true;
                    Ok(Variant::Void)
                })
            }),
            ("clearCallStack", |rt, t, _| {
                operate(rt, t, |p, _| {
                    p.clear_call_stack();
                    Ok(Variant::Void)
                })
            }),
            ("interrupt", |rt, t, _| {
                operate(rt, t, |p, _| {
                    p.interrupt();
                    Ok(Variant::Void)
                })
            }),
            ("resetInterrupt", |rt, t, _| {
                operate(rt, t, |p, _| {
                    p.reset_interrupt();
                    Ok(Variant::Void)
                })
            }),
            ("popMacroArgs", |rt, t, _| {
                operate(rt, t, |p, _| {
                    p.pop_macro_args()?;
                    Ok(Variant::Void)
                })
            }),
            ("assign", |rt, t, a| {
                let h = a
                    .first()
                    .and_then(Variant::object_handle)
                    .ok_or_else(|| error("KAGParser.assign needs a parser"))?;
                let h = rt.bound_this(h).unwrap_or(h);
                let boundary = state(rt, h)?.boundary;
                let src = synced_parser(rt, h)?;
                operate(rt, t, |p, h| {
                    p.assign(&src);
                    h.rt.host_mut()
                        .parsers
                        .get_mut(&h.owner.0)
                        .unwrap()
                        .boundary = boundary;
                    Ok(Variant::Void)
                })
            }),
            ("store", |rt, t, _| {
                let h = this_of(rt, t, "KAGParser.store")?;
                let s = state(rt, h)?;
                if !s.boundary {
                    return Err(unsupported(
                        "KAGParser.store",
                        "plain scenario/label save boundary",
                    ));
                }
                let p = synced_parser(rt, h)?;
                crate::kag_save::capture(rt, &p)
            }),
            ("restore", |rt, t, a| {
                let data = a
                    .first()
                    .and_then(Variant::object_handle)
                    .ok_or_else(|| error("native parser save dictionary required"))?;
                crate::kag_save::validate(rt, data)?;
                let storage = rt.object_member(data, "storageName").to_tjs_string()?;
                operate(rt, t, |p, h| {
                    p.load_scenario_with(storage, h)?;
                    let snapshot = crate::kag_save::decode(h.rt, p, data).map_err(|e| h.wrap(e))?;
                    p.restore(snapshot)?;
                    h.rt.host_mut()
                        .parsers
                        .get_mut(&h.owner.0)
                        .unwrap()
                        .boundary = true;
                    Ok(Variant::Void)
                })
            }),
        ],
    );
    for name in [
        "curStorage",
        "curLabel",
        "curLine",
        "curPos",
        "curLineStr",
        "callStackDepth",
        "ignoreCR",
        "processSpecialTags",
        "debugLevel",
        "macros",
        "macroParams",
        "mp",
    ] {
        rt.register_object_native_property(
            c,
            name,
            move |rt, t| get(rt, t, name),
            move |rt, t, v| match name {
                "ignoreCR" | "processSpecialTags" | "debugLevel" => operate(rt, t, |p, h| {
                    match name {
                        "ignoreCR" => p.set_ignore_cr(v.is_truthy()),
                        "processSpecialTags" => p.set_process_special_tags(v.is_truthy()),
                        _ => p.set_debug_level(match v.to_integer().map_err(|e| h.wrap(e))? {
                            0 => DebugLevel::None,
                            1 => DebugLevel::Simple,
                            2 => DebugLevel::Verbose,
                            _ => return Err(KagError::host("invalid parser debug level")),
                        }),
                    };
                    Ok(Variant::Void)
                })
                .map(|_| ()),
                "curStorage" => {
                    let s = v.to_tjs_string()?;
                    operate(rt, t, |p, h| {
                        p.load_scenario_with(s, h)?;
                        Ok(Variant::Void)
                    })
                    .map(|_| ())
                }
                _ => Err(error("read-only KAG parser property")),
            },
        );
    }
}
struct Bridge<'a> {
    rt: &'a mut Rt,
    owner: ObjectHandle,
    exception: Option<TjsError>,
}
impl Bridge<'_> {
    fn wrap(&mut self, e: TjsError) -> KagError {
        let msg = e.to_string();
        self.exception = Some(e);
        KagError::host(msg)
    }
    fn call(&mut self, name: &str, args: Vec<Variant>) -> krkr_kag::Result<Option<Variant>> {
        if !has_member(self.rt, self.owner, name) {
            return Ok(None);
        }
        let old = self.rt.host().can_suspend;
        self.rt.host_mut().can_suspend = false;
        let result = self.rt.call_object_method(self.owner, name, args);
        self.rt.host_mut().can_suspend = old;
        result.map(Some).map_err(|e| self.wrap(e))
    }
    fn eval(&mut self, s: &str) -> krkr_kag::Result<Variant> {
        let old = self.rt.host().can_suspend;
        self.rt.host_mut().can_suspend = false;
        let result = run_source(
            self.rt,
            "KAG expression",
            &format!("return ({s});"),
            Some(self.owner),
        );
        self.rt.host_mut().can_suspend = old;
        result.map_err(|e| self.wrap(e))
    }
    fn process(&mut self, name: &str, t: &Tag) -> krkr_kag::Result<bool> {
        let value = tag(self.rt, t).map_err(|e| self.wrap(e))?;
        Ok(self.call(name, vec![value])?.is_none_or(|v| v.is_truthy()))
    }
}
impl KagHost for Bridge<'_> {
    fn sync_parser_state(&mut self, p: &KagParser) -> krkr_kag::Result<()> {
        if let Some(s) = self.rt.host_mut().parsers.get_mut(&self.owner.0) {
            s.parser = p.clone();
        }
        publish_macros(self.rt, self.owner, p).map_err(|e| self.wrap(e))?;
        Ok(())
    }
    fn load_scenario(&mut self, s: &str) -> krkr_kag::Result<String> {
        self.rt
            .host_mut()
            .read_text(s, "")
            .map_err(|e| self.wrap(e))
    }
    fn on_scenario_load(&mut self, e: ScenarioLoadEvent<'_>) -> krkr_kag::Result<Option<String>> {
        Ok(
            match self.call("onScenarioLoad", vec![Variant::String(e.storage.into())])? {
                Some(Variant::String(s)) => Some(s),
                _ => None,
            },
        )
    }
    fn on_scenario_loaded(&mut self, e: ScenarioLoadEvent<'_>) -> krkr_kag::Result<()> {
        self.rt
            .host_mut()
            .parsers
            .get_mut(&self.owner.0)
            .unwrap()
            .boundary = true;
        self.call("onScenarioLoaded", vec![Variant::String(e.storage.into())])?;
        Ok(())
    }
    fn eval_bool(&mut self, s: &str) -> krkr_kag::Result<bool> {
        Ok(self.eval(s)?.is_truthy())
    }
    fn eval_string(&mut self, s: &str) -> krkr_kag::Result<String> {
        self.eval(s)?.to_tjs_string().map_err(|e| self.wrap(e))
    }
    fn eval_attribute(&mut self, s: &str) -> krkr_kag::Result<Option<String>> {
        match self.eval(s)? {
            Variant::Void => Ok(None),
            v => v.to_tjs_string().map(Some).map_err(|e| self.wrap(e)),
        }
    }
    fn on_label(&mut self, e: LabelEvent<'_>) -> krkr_kag::Result<()> {
        self.rt
            .host_mut()
            .parsers
            .get_mut(&self.owner.0)
            .unwrap()
            .boundary = true;
        self.call(
            "onLabel",
            vec![
                Variant::String(e.label.name.clone()),
                e.label
                    .page_name
                    .as_ref()
                    .map_or(Variant::Void, |s| Variant::String(s.clone())),
            ],
        )?;
        Ok(())
    }
    fn on_script(&mut self, e: ScriptEvent<'_>) -> krkr_kag::Result<()> {
        let p = &state(self.rt, self.owner)
            .map_err(|e| KagError::host(e.to_string()))?
            .parser;
        let line = p
            .line_pos_for_offset(e.storage, e.span.start)
            .map_or(0, |(l, _)| l);
        if self
            .call(
                "onScript",
                vec![
                    Variant::String(e.script.into()),
                    Variant::String(e.storage.into()),
                    Variant::Integer(line as i64),
                ],
            )?
            .is_none()
        {
            let old = self.rt.host().can_suspend;
            self.rt.host_mut().can_suspend = false;
            let result = run_source(self.rt, e.storage, e.script, Some(self.owner));
            self.rt.host_mut().can_suspend = old;
            result.map_err(|e| self.wrap(e))?;
        }
        Ok(())
    }
    fn on_jump(&mut self, t: &Tag, _: Option<&str>, _: Option<&str>) -> krkr_kag::Result<bool> {
        self.process("onJump", t)
    }
    fn on_call(&mut self, t: &Tag, _: Option<&str>, _: Option<&str>) -> krkr_kag::Result<bool> {
        self.process("onCall", t)
    }
    fn on_return(&mut self, t: &Tag, _: Option<&str>, _: Option<&str>) -> krkr_kag::Result<bool> {
        self.process("onReturn", t)
    }
    fn on_after_return(&mut self, _: &CallFrame) -> krkr_kag::Result<()> {
        self.call("onAfterReturn", vec![])?;
        Ok(())
    }
}
