//! Original ExtKAGParser call/macro control positions. Rendering and resources
//! never enter this representation. Reference: ExtKAGParser 0.1.4.3 Store,
//! Restore, PushCallStack and macro LineBuffer expansion.
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativePosition {
    pub storage: String,
    pub label: String,
    pub offset: usize,
    pub original_line: String,
    pub line_buffer: String,
    pub position: usize,
    pub using_buffer: bool,
    pub conditions: Krkr2ConditionState,
}

#[cfg(test)]
mod tests {
    use super::*;
    const MAIN: &str = "[macro name=outer][item name=before][inner][item name=outer-after][endmacro][macro name=inner][item name=あ😀][call storage=callee.ks target=*save][item name=inner-after][endmacro][outer][done]";
    const CALLEE: &str = "*save\n[item name=callee][return]";
    struct Host;
    impl KagHost for Host {
        fn local_variables(&mut self, tag: &Tag, push: bool, copy: bool) -> Result<()> {
            if tag.tagname == "call" && push && !copy {
                Ok(())
            } else {
                Err(KagError::host(
                    "unexpected local operation in control-only test",
                ))
            }
        }
        fn load_scenario(&mut self, name: &str) -> Result<String> {
            match name {
                "callee.ks" => Ok(CALLEE.into()),
                _ => Err(KagError::host("unexpected scenario")),
            }
        }
    }
    #[test]
    fn original_buffer_and_nested_macro_arguments_restore_through_native_call_return() {
        let mut p = KagParser::new();
        p.enable_ext_kag();
        p.set_ignore_cr(true);
        p.load_scenario_text("main.ks", MAIN).unwrap();
        let mut host = Host;
        for name in ["before", "あ😀", "callee"] {
            assert_eq!(
                p.next_tag_with(&mut host)
                    .unwrap()
                    .unwrap()
                    .literal_attr("name"),
                Some(name)
            );
        }
        let control = p.native_ext_control().unwrap();
        assert_eq!(control.macro_base, 2);
        assert_eq!(control.calls.len(), 1);
        assert_eq!(control.local_count, 2);
        let resume = &p.call_stack[0].resume;
        let args = vec![
            resume.macro_stack[1].previous_params.clone().unwrap(),
            resume.macro_params.clone(),
        ];
        let macros = p
            .macro_definitions()
            .map(|(n, s)| (n.to_owned(), s.to_owned()))
            .collect();
        let mut q = KagParser::new();
        q.enable_ext_kag();
        q.set_ignore_cr(true);
        q.cache_native_scenario("main.ks".into(), MAIN.into())
            .unwrap();
        q.cache_native_scenario("callee.ks".into(), CALLEE.into())
            .unwrap();
        q.restore_native_ext_control(control, args, macros).unwrap();
        assert_eq!(
            q.next_tag_with(&mut host)
                .unwrap()
                .unwrap()
                .literal_attr("name"),
            Some("callee"),
            "original Restore rewinds current label"
        );
        for name in ["inner-after", "outer-after"] {
            assert_eq!(
                q.next_tag_with(&mut host)
                    .unwrap()
                    .unwrap()
                    .literal_attr("name"),
                Some(name)
            );
            assert_eq!(
                p.next_tag_with(&mut host)
                    .unwrap()
                    .unwrap()
                    .literal_attr("name"),
                Some(name)
            );
        }
        assert_eq!(q.next_tag_with(&mut host).unwrap().unwrap().tagname, "done");
        assert_eq!(q.macro_argument_depth(), 0);
        assert_eq!(q.call_stack_depth(), 0);
        assert_eq!(q.local_variables_depth(), 1);
    }
    #[test]
    fn native_utf16_positions_refuse_surrogate_halves_and_changed_original_lines() {
        assert_eq!(utf16_offset("あ😀x", 3).unwrap(), 7);
        assert!(utf16_offset("あ😀x", 2).is_err());
        let mut p = KagParser::new();
        p.enable_ext_kag();
        p.load_scenario_text("a.ks", "[call storage=callee.ks target=*save]")
            .unwrap();
        let mut host = Host;
        p.next_tag_with(&mut host).unwrap();
        let mut control = p.native_ext_control().unwrap();
        control.calls[0].position.original_line = "changed".into();
        assert!(
            p.restore_native_ext_control(control, Vec::new(), Vec::new())
                .is_err()
        );
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeCall {
    pub position: NativePosition,
    pub macro_base: usize,
    pub macro_depth: usize,
    pub while_depth: usize,
    pub local_count: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeExtControl {
    pub current: NativePosition,
    pub calls: Vec<NativeCall>,
    pub macro_base: usize,
    pub macro_depth: usize,
    pub local_count: usize,
}

fn arg_frames(frames: &[ExpansionFrame]) -> usize {
    frames
        .iter()
        .filter(|frame| frame.previous_params.is_some())
        .count()
}
fn utf16_offset(text: &str, position: usize) -> Result<usize> {
    let mut units = 0;
    for (offset, ch) in text.char_indices() {
        if units == position {
            return Ok(offset);
        }
        units += ch.len_utf16();
        if units > position {
            return Err(KagError::host(
                "native save position splits UTF16 surrogate",
            ));
        }
    }
    if units == position {
        Ok(text.len())
    } else {
        Err(KagError::host("native save position outside buffer"))
    }
}

impl KagParser {
    fn native_position(
        &self,
        storage: &str,
        offset: usize,
        label: Option<&str>,
        frames: &[ExpansionFrame],
        conditions: &[ConditionFrame],
    ) -> Result<NativePosition> {
        let scenario = self
            .scenarios
            .get(storage)
            .ok_or_else(|| KagError::ScenarioNotLoaded {
                storage: storage.into(),
            })?;
        if offset > scenario.source.len() || !scenario.source.is_char_boundary(offset) {
            return Err(KagError::host("native save scenario cursor outside source"));
        }
        // Command-line macro parsing consumes its physical newline before
        // executing the expansion. Native LineBuffer still belongs to that
        // invocation line, so do not append the next physical command/label.
        let physical = if !frames.is_empty() && offset > 0 && scenario.line_starts.contains(&offset)
        {
            offset - 1
        } else {
            offset
        };
        let line = scenario.location(physical).line.saturating_sub(1);
        let start = scenario
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(scenario.source.len());
        let end = scenario.line_end(physical);
        let label = label.unwrap_or("");
        let base = if label.is_empty() {
            0
        } else {
            self.label_line(storage, label)
                .ok_or_else(|| KagError::host("native save label absent"))?
        };
        let (line_buffer, position) = if let Some(active) = frames.last() {
            let mut buffer = active.source.clone();
            for parent in frames[..frames.len() - 1].iter().rev() {
                buffer.push_str(parent.source.get(parent.offset..).ok_or_else(|| {
                    KagError::host("native save expansion cursor outside source")
                })?);
            }
            if offset <= end {
                buffer.push_str(&scenario.source[offset..end]);
            }
            if buffer.len() > 4 * 1024 * 1024 {
                return Err(KagError::host("native save line buffer budget"));
            }
            (
                buffer,
                active.source[..active.offset].encode_utf16().count(),
            )
        } else {
            (
                String::new(),
                scenario.source[start..physical.min(end)]
                    .encode_utf16()
                    .count(),
            )
        };
        Ok(NativePosition {
            storage: storage.into(),
            label: label.into(),
            offset: line.saturating_sub(base),
            original_line: scenario.line_text_at(physical).into(),
            line_buffer,
            position,
            using_buffer: !frames.is_empty(),
            conditions: Krkr2ConditionState::from_frames(conditions),
        })
    }
    /// Native call/macro save projection. Active while loops remain explicit
    /// unsupported positions until their original loop checkpoints are mapped.
    pub fn native_ext_control(&self) -> Result<NativeExtControl> {
        let ext = self
            .ext
            .as_ref()
            .ok_or_else(|| KagError::host("ExtKAGParser dialect required"))?;
        if !ext.while_stack.is_empty()
            || self
                .call_stack
                .iter()
                .any(|f| !f.resume.while_stack.is_empty())
        {
            return Err(KagError::host(
                "active original while-loop save mapping is unsupported",
            ));
        }
        let mut base = 0;
        let mut calls = Vec::new();
        for (index, frame) in self.call_stack.iter().enumerate() {
            let depth = base + arg_frames(&frame.resume.macro_stack);
            calls.push(NativeCall {
                position: self.native_position(
                    &frame.storage,
                    frame.offset,
                    frame.current_label.as_deref(),
                    &frame.resume.macro_stack,
                    &frame.resume.condition_stack,
                )?,
                macro_base: base,
                macro_depth: depth,
                while_depth: index + 1,
                local_count: frame.resume.local_depth + 1,
            });
            base = depth;
        }
        Ok(NativeExtControl {
            current: self.native_position(
                self.current_storage_name()?,
                self.cursor.offset,
                self.current_label.as_deref(),
                &self.macro_stack,
                &self.condition_stack,
            )?,
            calls,
            macro_base: base,
            macro_depth: self.macro_argument_depth(),
            local_count: ext.local_depth,
        })
    }
    /// Cache a file-backed native return scenario without moving this parser
    /// or firing another scenario's callbacks during restoration.
    pub fn cache_native_scenario(&mut self, name: String, text: String) -> Result<()> {
        Arc::make_mut(&mut self.scenarios).insert(name.clone(), Scenario::new(name, text)?);
        Ok(())
    }
    /// Native Restore reloads the current scenario after installing saved
    /// caller/argument state, so its callbacks must observe those live args.
    pub fn reload_native_scenario_with<H: KagHost>(
        &mut self,
        name: String,
        params: Vec<Attribute>,
        host: &mut H,
    ) -> Result<()> {
        let source = match host.on_scenario_load(ScenarioLoadEvent { storage: &name })? {
            Some(source) => source,
            None => host.load_scenario(&name)?,
        };
        self.install_scenario(name.clone(), source)?;
        self.macro_params = params;
        host.sync_parser_state(self)?;
        host.on_scenario_loaded(ScenarioLoadEvent { storage: &name })
    }
    fn import_native_position(
        &self,
        value: &NativePosition,
    ) -> Result<(usize, Vec<ExpansionFrame>)> {
        let source =
            self.scenarios
                .get(&value.storage)
                .ok_or_else(|| KagError::ScenarioNotLoaded {
                    storage: value.storage.clone(),
                })?;
        let base = if value.label.is_empty() {
            0
        } else {
            self.label_line(&value.storage, &value.label)
                .ok_or_else(|| KagError::host("native return label absent"))?
        };
        let line = base
            .checked_add(value.offset)
            .ok_or_else(|| KagError::host("native return line overflow"))?;
        let start = *source
            .line_starts
            .get(line)
            .ok_or_else(|| KagError::host("native return line outside scenario"))?;
        if source.line_text_at(start) != value.original_line {
            return Err(KagError::ReturnLostSync {
                storage: value.storage.clone(),
            });
        }
        if value.using_buffer {
            if value.line_buffer.len() > 4 * 1024 * 1024 {
                return Err(KagError::host("native line buffer budget"));
            }
            let offset = utf16_offset(&value.line_buffer, value.position)?;
            Ok((
                source.line_end_with_newline(start),
                vec![ExpansionFrame {
                    name: "<native-buffer>".into(),
                    source: value.line_buffer.clone(),
                    line_starts: line_starts_for(&value.line_buffer),
                    offset,
                    origin: TagOrigin::MacroExpansion {
                        name: "<native-buffer>".into(),
                    },
                    previous_params: None,
                }],
            ))
        } else {
            let line_text = source.line_text_at(start);
            Ok((start + utf16_offset(line_text, value.position)?, Vec::new()))
        }
    }
    /// Restore original label/call/macro data, with the original top-level
    /// label rewind. Current active macro buffers and true while loops fail.
    pub fn restore_native_ext_control(
        &mut self,
        value: NativeExtControl,
        args: Vec<Vec<Attribute>>,
        macros: Vec<(String, String)>,
    ) -> Result<()> {
        if value.calls.len() > 128
            || args.len() > 1024
            || value.macro_depth != args.len()
            || value.macro_base != value.macro_depth
            || !(1..=1024).contains(&value.local_count)
        {
            return Err(KagError::host(
                "native Ext save bounds or current active macro unsupported",
            ));
        }
        let mut calls = Vec::new();
        let mut base = 0;
        for (index, call) in value.calls.iter().enumerate() {
            if call.macro_base != base
                || call.macro_depth < base
                || call.macro_depth > args.len()
                || call.macro_depth - base > self.options.max_macro_depth
                || call.while_depth != index + 1
                || call.local_count < 2
                || call.local_count > value.local_count
            {
                return Err(KagError::host("native Ext call stack bounds"));
            }
            let (offset, mut frames) = self.import_native_position(&call.position)?;
            let count = call.macro_depth - base;
            if count > 0 {
                if frames.is_empty() {
                    return Err(KagError::host("native active macro return buffer missing"));
                }
                let mut expansion = frames.pop().unwrap();
                for n in 0..count {
                    let previous = if base + n == 0 {
                        Vec::new()
                    } else {
                        args[base + n - 1].clone()
                    };
                    if n + 1 == count {
                        expansion.previous_params = Some(previous.clone());
                    } else {
                        frames.push(ExpansionFrame {
                            name: "<native-args>".into(),
                            source: String::new(),
                            line_starts: vec![0],
                            offset: 0,
                            origin: TagOrigin::MacroExpansion {
                                name: "<native-args>".into(),
                            },
                            previous_params: Some(previous),
                        });
                    }
                }
                frames.push(expansion);
            }
            let source = &self.scenarios[&call.position.storage];
            calls.push(CallFrame {
                storage: call.position.storage.clone(),
                offset,
                line_text: source.line_text_at(offset).into(),
                current_label: (!call.position.label.is_empty())
                    .then(|| call.position.label.clone()),
                resume: ResumeState {
                    macro_stack: frames,
                    macro_params: call
                        .macro_depth
                        .checked_sub(1)
                        .map_or_else(Vec::new, |n| args[n].clone()),
                    condition_stack: call.position.conditions.to_frames(),
                    while_stack: Vec::new(),
                    local_depth: call.local_count - 1,
                },
            });
            base = call.macro_depth;
        }
        if base != value.macro_base {
            return Err(KagError::host("native macro base/call mismatch"));
        }
        let label = (!value.current.label.is_empty()).then(|| value.current.label.clone());
        let line = label.as_deref().map_or(Ok(0), |label| {
            self.label_line(&value.current.storage, label)
                .ok_or_else(|| KagError::host("native current label absent"))
        })?;
        let cursor = self.offset_for_line_pos(&value.current.storage, line, 0)?;
        let mut snapshot = self.store();
        snapshot.current_storage = Some(value.current.storage);
        snapshot.cursor.offset = cursor;
        snapshot.current_label = label;
        snapshot.call_stack = calls;
        snapshot.macro_stack.clear();
        snapshot.macro_params = args.last().cloned().unwrap_or_default();
        snapshot.condition_stack = value.current.conditions.to_frames();
        snapshot.interrupted = false;
        let ext = snapshot
            .ext
            .as_mut()
            .ok_or_else(|| KagError::host("Ext dialect required"))?;
        ext.local_depth = value.local_count;
        ext.while_stack.clear();
        snapshot.macros = macros
            .into_iter()
            .map(|(name, source)| (name, MacroDefinition { source }))
            .collect();
        self.restore(snapshot)
    }
}
