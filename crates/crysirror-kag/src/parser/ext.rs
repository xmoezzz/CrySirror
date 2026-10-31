//! ExtKAGParser 0.1.4.3 control semantics, reimplemented for the first-class KAG
//! VM. No DLL, Windows SDK or legacy host is embedded. Provenance and upstream
//! notices: docs/third-party/ext-kag-parser.txt.
use super::*;

const MAX_LOOP_DEPTH: usize = 256;
const MAX_LOCAL_DEPTH: usize = 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExtState {
    pub multi_line: bool,
    pub numeric_arguments: bool,
    pub ignore_cr: bool,
    pub while_stack: Vec<WhileFrame>,
    pub local_depth: usize,
    pub parameter_macros: BTreeMap<String, Vec<Attribute>>,
}
impl Default for ExtState {
    fn default() -> Self {
        Self {
            multi_line: false,
            numeric_arguments: false,
            ignore_cr: false,
            while_stack: Vec::new(),
            local_depth: 1,
            parameter_macros: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WhileFrame {
    offset: usize,
    macro_stack: Vec<ExpansionFrame>,
    macro_params: Vec<Attribute>,
    conditions: Vec<ConditionFrame>,
    expression: String,
    each: String,
}

impl KagParser {
    /// Select the original ExtKAGParser control dialect for this VM instance.
    /// Existing classic KAG instances are unaffected by replacing its TJS class.
    pub fn enable_ext_kag(&mut self) {
        if self.ext.is_none() {
            self.ext = Some(ExtState::default());
        }
    }
    pub fn is_ext_kag(&self) -> bool {
        self.ext.is_some()
    }
    pub fn multi_line_tag_enabled(&self) -> bool {
        self.ext.as_ref().is_some_and(|s| s.multi_line)
    }
    pub fn set_multi_line_tag_enabled(&mut self, value: bool) -> Result<()> {
        self.ext
            .as_mut()
            .ok_or_else(|| KagError::host("ExtKAGParser dialect required"))?
            .multi_line = value;
        Ok(())
    }
    pub fn numeric_macro_arguments_enabled(&self) -> bool {
        self.ext.as_ref().is_some_and(|s| s.numeric_arguments)
    }
    pub fn set_numeric_macro_arguments_enabled(&mut self, value: bool) -> Result<()> {
        self.ext
            .as_mut()
            .ok_or_else(|| KagError::host("ExtKAGParser dialect required"))?
            .numeric_arguments = value;
        Ok(())
    }
    pub fn if_level(&self) -> usize {
        self.condition_stack.len()
    }
    pub fn while_stack_depth(&self) -> usize {
        self.ext.as_ref().map_or(0, |s| s.while_stack.len())
            + self
                .call_stack
                .iter()
                .map(|f| f.resume.while_stack.len() + usize::from(self.is_ext_kag()))
                .sum::<usize>()
    }
    pub fn local_variables_depth(&self) -> usize {
        self.ext.as_ref().map_or(1, |s| s.local_depth)
    }
    pub fn parameter_macros(&self) -> impl Iterator<Item = (&str, &[Attribute])> {
        self.ext.iter().flat_map(|s| {
            s.parameter_macros
                .iter()
                .map(|(n, a)| (n.as_str(), a.as_slice()))
        })
    }
    pub fn set_parameter_macros(&mut self, entries: Vec<(String, Vec<Attribute>)>) -> Result<()> {
        if entries.len() > 4096 {
            return Err(KagError::host("ExtKAGParser parameter macro limit"));
        }
        self.ext
            .as_mut()
            .ok_or_else(|| KagError::host("ExtKAGParser dialect required"))?
            .parameter_macros = entries.into_iter().collect();
        Ok(())
    }
    pub(super) fn validate_ext_restore(&self, snapshot: &ParserSnapshot) -> Result<()> {
        let Some(ext) = snapshot.ext.as_ref() else {
            return Ok(());
        };
        let check = |storage: Option<&str>,
                     offset: usize,
                     macros: &[ExpansionFrame],
                     loops: &[WhileFrame]|
         -> Result<()> {
            let source = storage.and_then(|name| self.scenarios.get(name));
            if source.is_none_or(|s| offset > s.source.len() || !s.source.is_char_boundary(offset))
                && storage.is_some()
            {
                return Err(KagError::host(
                    "ExtKAGParser saved cursor is outside scenario",
                ));
            }
            for frame in macros
                .iter()
                .chain(loops.iter().flat_map(|f| f.macro_stack.iter()))
            {
                if frame.offset > frame.source.len() || !frame.source.is_char_boundary(frame.offset)
                {
                    return Err(KagError::host(
                        "ExtKAGParser saved expansion cursor is invalid",
                    ));
                }
            }
            for frame in loops {
                if source.is_none_or(|s| {
                    frame.offset > s.source.len() || !s.source.is_char_boundary(frame.offset)
                }) {
                    return Err(KagError::host("ExtKAGParser saved loop cursor is invalid"));
                }
            }
            Ok(())
        };
        check(
            snapshot.current_storage.as_deref(),
            snapshot.cursor.offset,
            &snapshot.macro_stack,
            &ext.while_stack,
        )?;
        for frame in &snapshot.call_stack {
            check(
                Some(&frame.storage),
                frame.offset,
                &frame.resume.macro_stack,
                &frame.resume.while_stack,
            )?;
        }
        Ok(())
    }
    pub(super) fn local_scope_base(&self) -> usize {
        self.call_stack
            .last()
            .map_or(1, |f| f.resume.local_depth + 1)
    }
    pub(super) fn reset_ext_flow(&mut self) {
        if let Some(ext) = &mut self.ext {
            ext.while_stack.clear();
            ext.local_depth = 1;
        }
    }
    pub(super) fn break_ext_flow(&mut self) {
        let base = self.local_scope_base();
        if let Some(ext) = &mut self.ext {
            ext.while_stack.clear();
            ext.local_depth = base;
        }
    }
    pub(super) fn push_ext_locals<H: KagHost>(&mut self, tag: &Tag, host: &mut H) -> Result<()> {
        if self.ext.is_none() {
            return Ok(());
        }
        if self.local_variables_depth() >= MAX_LOCAL_DEPTH {
            return Err(KagError::host("ExtKAGParser local variable depth limit"));
        }
        host.sync_parser_state(self)?;
        let copy = match tag.literal_attr("copyvar") {
            Some(exp) if !exp.is_empty() => host.eval_bool(exp)?,
            _ => false,
        };
        host.local_variables(tag, true, copy)
    }
    pub(super) fn resolve_local_arguments<H: KagHost>(
        &self,
        tag: &mut Tag,
        host: &mut H,
    ) -> Result<()> {
        if self.ext.is_none() {
            return Ok(());
        }
        host.sync_parser_state(self)?;
        for a in &mut tag.attributes {
            if let Attribute::Named {
                value: AttributeValue::Literal(s),
                ..
            } = a
            {
                if let Some(spec) = s.strip_prefix('$') {
                    let (name, fallback) = spec
                        .split_once('|')
                        .map_or((spec, None), |(n, v)| (n, Some(v)));
                    let v = host
                        .local_variable(name)?
                        .or_else(|| fallback.map(str::to_owned));
                    if let Attribute::Named { value, .. } = a {
                        *value = v
                            .map(AttributeValue::Literal)
                            .unwrap_or(AttributeValue::Void);
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn expand_parameter_macros(&self, tag: &mut Tag) -> Result<()> {
        let Some(ext) = &self.ext else {
            return Ok(());
        };
        let mut attrs: Vec<Attribute> = Vec::new();
        for a in std::mem::take(&mut tag.attributes) {
            // A bare flag is parsed by the base grammar as the literal true.
            // ExtKAGParser expands only arguments without '='. The scanner sets
            // bare flag markers before this function (see extended_tag_content).
            if let Attribute::Named {
                name,
                value: AttributeValue::Literal(ref v),
            } = a
            {
                if let Some(n) = name.strip_prefix('\0') {
                    if let Some(values) = ext.parameter_macros.get(n) {
                        for value in values {
                            replace_attr(&mut attrs, value.clone());
                        }
                    } else {
                        replace_attr(&mut attrs, Attribute::named(n, AttributeValue::literal(v)));
                    }
                } else {
                    replace_attr(
                        &mut attrs,
                        Attribute::named(name, AttributeValue::literal(v)),
                    );
                }
            } else {
                replace_attr(&mut attrs, a);
            }
        }
        tag.attributes = attrs;
        Ok(())
    }
    pub fn macro_arguments_active(&self) -> bool {
        !self.macro_params.is_empty()
            || self
                .macro_stack
                .iter()
                .any(|frame| frame.previous_params.is_some())
    }
    /// Native argument dictionaries remain live across call-suspended macro
    /// expansions. Embedded-text expansions do not allocate argument frames.
    pub fn macro_argument_depth(&self) -> usize {
        self.macro_stack
            .iter()
            .filter(|frame| frame.previous_params.is_some())
            .count()
            + self
                .call_stack
                .iter()
                .map(|frame| {
                    frame
                        .resume
                        .macro_stack
                        .iter()
                        .filter(|frame| frame.previous_params.is_some())
                        .count()
                })
                .sum::<usize>()
    }
    pub(super) fn validate_ext_macro_arguments(&self, tag: &Tag) -> Result<()> {
        if self.is_ext_kag()
            && !self.macro_arguments_active()
            && tag.attributes.iter().any(|a| {
                matches!(
                    a,
                    Attribute::Spread
                        | Attribute::Named {
                            value: AttributeValue::MacroArgument(_),
                            ..
                        }
                )
            })
        {
            return Err(self.parse_error(tag.span, "ExtKAGParser macro argument outside macro"));
        }
        Ok(())
    }
    pub(super) fn validate_ext_condition(&self, tag: &Tag) -> Result<()> {
        if self.ext.is_some()
            && matches!(
                tag.tagname.as_str(),
                "elsif" | "else" | "endif" | "endignore"
            )
        {
            let minimum = self
                .ext
                .as_ref()
                .and_then(|s| s.while_stack.last())
                .map_or(0, |f| f.conditions.len() + 1);
            if self.condition_stack.len() <= minimum {
                return Err(self.parse_error(tag.span, "ExtKAGParser conditional stack underflow"));
            }
        }
        Ok(())
    }
    pub(super) fn handle_ext_loop<H: KagHost>(&mut self, tag: &Tag, host: &mut H) -> Result<bool> {
        if self.ext.is_none() {
            return Ok(false);
        }
        match tag.tagname.as_str() {
            "while" => {
                if self.ext.as_ref().unwrap().while_stack.len() >= MAX_LOOP_DEPTH {
                    return Err(KagError::host("ExtKAGParser while stack limit"));
                }
                let active = self.is_executing();
                let mut resolved = tag.clone();
                if active {
                    self.validate_ext_macro_arguments(&resolved)?;
                    self.expand_parameter_macros(&mut resolved)?;
                    self.apply_macro_arguments(&mut resolved);
                    self.resolve_local_arguments(&mut resolved, host)?;
                    self.resolve_entities(&mut resolved, host)?;
                }
                let expression = resolved.literal_attr("exp").unwrap_or("").to_owned();
                let each = resolved.literal_attr("each").unwrap_or("").to_owned();
                host.sync_parser_state(self)?;
                let cond = if active {
                    if expression.is_empty() {
                        return Err(KagError::MissingAttribute {
                            tag: "while".into(),
                            attribute: "exp".into(),
                        });
                    }
                    if let Some(init) = resolved.literal_attr("init").filter(|s| !s.is_empty()) {
                        host.eval_void(init)?;
                    }
                    host.eval_bool(&expression)?
                } else {
                    false
                };
                let frame = WhileFrame {
                    offset: self.cursor.offset,
                    macro_stack: self.macro_stack.clone(),
                    macro_params: self.macro_params.clone(),
                    conditions: self.condition_stack.clone(),
                    expression,
                    each,
                };
                self.ext.as_mut().unwrap().while_stack.push(frame);
                self.condition_stack.push(ConditionFrame {
                    parent_active: active,
                    branch_taken: active && cond,
                    current_active: active && cond,
                });
                Ok(true)
            }
            "endwhile" | "continue" | "break" => {
                if tag.tagname != "endwhile" && !self.is_executing() {
                    return Ok(true);
                }
                let frame = self
                    .ext
                    .as_ref()
                    .unwrap()
                    .while_stack
                    .last()
                    .cloned()
                    .ok_or_else(|| {
                        self.parse_error(tag.span, "ExtKAGParser while stack underflow")
                    })?;
                if self.condition_stack.len() != frame.conditions.len() + 1 {
                    return Err(self.parse_error(
                        tag.span,
                        "ExtKAGParser loop control crosses conditional scope",
                    ));
                }
                // continue/break may carry the same ordinary cond= gate as the
                // native plugin; while/endwhile deliberately ignore that gate.
                if tag.tagname != "endwhile" {
                    let mut resolved = tag.clone();
                    self.expand_parameter_macros(&mut resolved)?;
                    self.apply_macro_arguments(&mut resolved);
                    self.resolve_local_arguments(&mut resolved, host)?;
                    if !self.process_cond_attr(&mut resolved, host)? {
                        return Ok(true);
                    }
                }
                host.sync_parser_state(self)?;
                let active = self.is_executing();
                let cond = if active && tag.tagname != "break" {
                    if !frame.each.is_empty() {
                        host.eval_void(&frame.each)?;
                    }
                    host.eval_bool(&frame.expression)?
                } else {
                    false
                };
                if tag.tagname == "endwhile" && !cond {
                    self.ext.as_mut().unwrap().while_stack.pop();
                    self.condition_stack = frame.conditions;
                } else {
                    // Native break/continue re-enter the body, possibly in the
                    // excluded state, so structural errors in the tail remain
                    // observable and no skipped expressions execute.
                    self.cursor.offset = frame.offset;
                    self.macro_stack = frame.macro_stack;
                    self.macro_params = frame.macro_params;
                    self.condition_stack = frame.conditions;
                    self.condition_stack.push(ConditionFrame {
                        parent_active: active,
                        branch_taken: active && cond,
                        current_active: active && cond,
                    });
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    pub(super) fn handle_ext_tag<H: KagHost>(&mut self, tag: &Tag, host: &mut H) -> Result<bool> {
        if self.ext.is_none() {
            return Ok(false);
        }
        match tag.tagname.as_str() {
            "localvar" => {
                host.sync_parser_state(self)?;
                host.local_variables(tag, false, false)?;
                Ok(true)
            }
            "pushlocalvar" => {
                self.push_ext_locals(tag, host)?;
                self.ext.as_mut().unwrap().local_depth += 1;
                Ok(true)
            }
            "poplocalvar" => {
                if self.local_variables_depth() <= self.local_scope_base() {
                    return Err(
                        self.parse_error(tag.span, "ExtKAGParser local variable stack underflow")
                    );
                }
                self.ext.as_mut().unwrap().local_depth -= 1;
                host.sync_parser_state(self)?;
                Ok(true)
            }
            "pmacro" => {
                let name = tag
                    .literal_attr("name")
                    .ok_or_else(|| KagError::MissingAttribute {
                        tag: "pmacro".into(),
                        attribute: "name".into(),
                    })?;
                let mut attrs = tag.attributes.clone();
                attrs.retain(|a| a.name() != Some("name") && a.name() != Some("tagname"));
                let ext = self.ext.as_mut().unwrap();
                if ext.parameter_macros.len() >= 4096 && !ext.parameter_macros.contains_key(name) {
                    return Err(KagError::host("ExtKAGParser parameter macro limit"));
                }
                ext.parameter_macros.insert(name.to_owned(), attrs);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
fn replace_attr(attrs: &mut Vec<Attribute>, value: Attribute) {
    if let Some(name) = value.name() {
        if let Some(existing) = attrs.iter_mut().find(|a| a.name() == Some(name)) {
            *existing = value;
            return;
        }
    }
    attrs.push(value);
}

// V2 is an additive appendix; classic v1 snapshot bytes remain unchanged.
impl SnapshotWriter {
    fn while_frames(&mut self, frames: &[WhileFrame]) {
        self.usize(frames.len());
        for f in frames {
            self.usize(f.offset);
            self.expansion_frames(&f.macro_stack);
            self.attributes(&f.macro_params);
            self.condition_frames(&f.conditions);
            self.string(&f.expression);
            self.string(&f.each);
        }
    }
    pub(super) fn ext_state(&mut self, ext: &ExtState, calls: &[CallFrame]) {
        self.bool(ext.multi_line);
        self.bool(ext.numeric_arguments);
        self.bool(ext.ignore_cr);
        self.usize(ext.local_depth);
        self.while_frames(&ext.while_stack);
        self.usize(ext.parameter_macros.len());
        for (name, attrs) in &ext.parameter_macros {
            self.string(name);
            self.attributes(attrs);
        }
        self.usize(calls.len());
        for call in calls {
            self.while_frames(&call.resume.while_stack);
            self.usize(call.resume.local_depth);
        }
    }
}
impl SnapshotReader<'_> {
    fn while_frames(&mut self) -> Result<Vec<WhileFrame>> {
        let count = self.usize()?;
        if count > MAX_LOOP_DEPTH {
            return Err(KagError::host("ExtKAGParser saved loop limit"));
        }
        let mut frames = Vec::with_capacity(count);
        for _ in 0..count {
            frames.push(WhileFrame {
                offset: self.usize()?,
                macro_stack: self.expansion_frames()?,
                macro_params: self.attributes()?,
                conditions: self.condition_frames()?,
                expression: self.string()?,
                each: self.string()?,
            });
        }
        Ok(frames)
    }
    pub(super) fn ext_state(&mut self, calls: &mut [CallFrame]) -> Result<ExtState> {
        let mut ext = ExtState {
            multi_line: self.bool()?,
            numeric_arguments: self.bool()?,
            ignore_cr: self.bool()?,
            local_depth: self.usize()?,
            while_stack: self.while_frames()?,
            parameter_macros: BTreeMap::new(),
        };
        if ext.local_depth == 0 || ext.local_depth > MAX_LOCAL_DEPTH {
            return Err(KagError::host("ExtKAGParser saved local depth limit"));
        }
        let count = self.usize()?;
        if count > 4096 {
            return Err(KagError::host("ExtKAGParser saved parameter macro limit"));
        }
        for _ in 0..count {
            ext.parameter_macros
                .insert(self.string()?, self.attributes()?);
        }
        if self.usize()? != calls.len() {
            return Err(KagError::host("ExtKAGParser saved call stack mismatch"));
        }
        for call in calls {
            call.resume.while_stack = self.while_frames()?;
            call.resume.local_depth = self.usize()?;
            if call.resume.local_depth == 0 || call.resume.local_depth > MAX_LOCAL_DEPTH {
                return Err(KagError::host("ExtKAGParser saved call scope limit"));
            }
        }
        Ok(ext)
    }
}

pub(super) fn tjs_quote(value: &str) -> String {
    let mut out = String::from("'");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}
pub(super) fn kag_quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        if c == '`' || c == '"' {
            out.push('`');
        }
        out.push(c);
    }
    out.push('"');
    out
}
impl KagParser {
    pub(super) fn read_extended_tag(&mut self, bracket: bool) -> Result<RawItem> {
        let start = self.active_offset();
        let (mut tag, next) = {
            let (source, line_starts) = self.active_source_parts()?;
            let mut content = String::new();
            let mut cursor = start + 1;
            let mut quote = None;
            let mut escaped = false;
            let mut boundary = false;
            let end;
            loop {
                let Some(ch) = char_at_in(source, cursor) else {
                    if bracket {
                        return Err(self.parse_error(
                            SourceSpan::new(start, cursor),
                            "unterminated ExtKAGParser tag",
                        ));
                    }
                    end = cursor;
                    break;
                };
                if ch == '\n' || ch == '\r' {
                    if bracket || quote.is_some() {
                        return Err(self.parse_error(
                            SourceSpan::new(start, cursor),
                            "line end inside ExtKAGParser tag",
                        ));
                    }
                    end = newline_end_in(source, cursor);
                    break;
                }
                if !escaped && quote.is_none() {
                    if bracket && ch == ']' {
                        end = cursor + 1;
                        break;
                    }
                    if ch == '\\' && boundary {
                        if !self.multi_line_tag_enabled() {
                            return Err(self.parse_error(
                                SourceSpan::new(start, cursor + 1),
                                "ExtKAGParser multiline tags are disabled",
                            ));
                        }
                        let newline = line_end_in(source, cursor);
                        if newline == source.len() {
                            return Err(self.parse_error(
                                SourceSpan::new(start, cursor + 1),
                                "ExtKAGParser continuation at end of storage",
                            ));
                        }
                        cursor = newline_end_in(source, newline);
                        if cursor >= source.len() {
                            return Err(self.parse_error(
                                SourceSpan::new(start, cursor),
                                "ExtKAGParser continuation at end of storage",
                            ));
                        }
                        continue;
                    }
                }
                content.push(ch);
                cursor += ch.len_utf8();
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '`' {
                    escaped = true;
                    continue;
                }
                if let Some(q) = quote {
                    if ch == q {
                        quote = None;
                    }
                } else if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                }
                boundary = quote.is_none() && is_kag_ws(ch);
            }
            let span = SourceSpan::new(start, if bracket { end } else { cursor });
            let tag = parse_tag_content(
                self.current_storage.as_deref(),
                source,
                line_starts,
                &content,
                start + 1,
                span,
                if bracket {
                    self.active_origin(TagOrigin::Bracket)
                } else {
                    TagOrigin::CommandLine
                },
                true,
            )?;
            (tag, end)
        };
        // Bare flags are internal markers only. They are resolved during normal
        // semantic processing, after excluded branches have been discarded.
        self.set_active_offset(next);
        if !self.options.process_special_tags {
            self.expand_parameter_macros(&mut tag)?;
        }
        Ok(RawItem::Tag(tag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Host {
        i: i32,
        j: i32,
        calls: Vec<String>,
    }
    impl KagHost for Host {
        fn eval_void(&mut self, s: &str) -> Result<()> {
            self.calls.push(s.into());
            match s {
                "i=0" => self.i = 0,
                "j=0" => self.j = 0,
                "i++" => self.i += 1,
                "j++" => self.j += 1,
                "forbidden" => panic!("excluded loop executed initializer"),
                _ => return Err(KagError::host(format!("unexpected expression {s}"))),
            };
            Ok(())
        }
        fn eval_bool(&mut self, s: &str) -> Result<bool> {
            Ok(match s {
                "i<3" => self.i < 3,
                "j<2" => self.j < 2,
                "i==1" => self.i == 1,
                "false" => false,
                _ => return Err(KagError::host(format!("unexpected condition {s}"))),
            })
        }
        fn eval_attribute(&mut self, s: &str) -> Result<Option<String>> {
            Ok(Some(match s {
                "i" => self.i.to_string(),
                "j" => self.j.to_string(),
                _ => return Err(KagError::host("unexpected entity")),
            }))
        }
    }
    fn parser(source: &str) -> KagParser {
        let mut p = KagParser::new();
        p.enable_ext_kag();
        p.set_ignore_cr(true);
        p.load_scenario_text("authored.ks", source).unwrap();
        p
    }
    #[test]
    fn nested_loops_resume_macros_without_replaying_init_or_excluded_effects() {
        let mut p = parser(
            "[macro name=m][while init=\"i=0\" exp=\"i<3\" each=\"i++\"][while init=\"j=0\" exp=\"j<2\" each=\"j++\"][item i=&i j=&j][endwhile][endwhile][endmacro][m][while exp=false][while init=forbidden exp=bad][item bad=true][endwhile][endwhile][done]",
        );
        let mut host = Host::default();
        let mut result = Vec::new();
        loop {
            let Some(tag) = p.next_tag_with(&mut host).unwrap() else {
                break;
            };
            if tag.tagname == "item" {
                result.push((
                    tag.literal_attr("i").unwrap().to_owned(),
                    tag.literal_attr("j").unwrap().to_owned(),
                ));
            }
        }
        assert_eq!(
            result,
            vec![
                ("0".into(), "0".into()),
                ("0".into(), "1".into()),
                ("1".into(), "0".into()),
                ("1".into(), "1".into()),
                ("2".into(), "0".into()),
                ("2".into(), "1".into())
            ]
        );
        assert_eq!(host.calls.iter().filter(|s| s.as_str() == "i=0").count(), 1);
        assert_eq!(p.while_stack_depth(), 0);
    }
    #[test]
    fn assignment_copies_loop_cursor_but_keeps_destination_owner_switches() {
        let mut source =
            parser("[while init=\"i=0\" exp=\"i<3\" each=\"i++\"][item i=&i][endwhile][done]");
        source.set_multi_line_tag_enabled(true).unwrap();
        source.set_numeric_macro_arguments_enabled(true).unwrap();
        let mut host = Host::default();
        assert_eq!(
            source
                .next_tag_with(&mut host)
                .unwrap()
                .unwrap()
                .literal_attr("i"),
            Some("0")
        );
        let mut destination = KagParser::new();
        destination.enable_ext_kag();
        destination.set_process_special_tags(false);
        destination.interrupt();
        destination.assign(&source);
        assert_eq!(destination.while_stack_depth(), 1);
        assert_eq!(destination.cursor, source.cursor);
        assert!(!destination.multi_line_tag_enabled());
        assert!(!destination.numeric_macro_arguments_enabled());
        assert!(!destination.process_special_tags());
        assert!(destination.interrupted);
        destination.reset_interrupt();
        destination.set_process_special_tags(true);
        assert_eq!(
            destination
                .next_tag_with(&mut host)
                .unwrap()
                .unwrap()
                .literal_attr("i"),
            Some("1")
        );
        destination.clear();
        assert_eq!(source.while_stack_depth(), 1);
        assert_eq!(
            source
                .next_tag_with(&mut host)
                .unwrap()
                .unwrap()
                .literal_attr("i"),
            Some("2")
        );
    }
    #[test]
    fn break_continue_snapshot_and_structural_errors_match_extension_scope() {
        let mut p = parser(
            "[while init=\"i=0\" exp=\"i<3\" each=\"i++\"][continue cond=\"i==1\"][item i=&i][endwhile][done]",
        );
        let mut host = Host::default();
        assert_eq!(
            p.next_tag_with(&mut host)
                .unwrap()
                .unwrap()
                .literal_attr("i"),
            Some("0")
        );
        let saved = p.store().to_persistent_string();
        assert!(saved.starts_with("kag-snapshot-v2:"));
        p.set_ignore_cr(false);
        p.restore(ParserSnapshot::from_persistent_string(&saved).unwrap())
            .unwrap();
        assert!(p.ignore_cr());
        assert_eq!(
            p.next_tag_with(&mut host)
                .unwrap()
                .unwrap()
                .literal_attr("i"),
            Some("2")
        );
        assert_eq!(p.next_tag_with(&mut host).unwrap().unwrap().tagname, "done");
        let mut p = parser(
            "[while init=\"i=0\" exp=\"i<3\" each=\"i++\"][break][item bad=true][endwhile][done]",
        );
        let mut h = Host::default();
        assert_eq!(p.next_tag_with(&mut h).unwrap().unwrap().tagname, "done");
        assert_eq!(h.i, 0);
        let mut p = parser("[while exp=\"i<3\"][if exp=\"i<3\"][break][endif][endwhile]");
        assert!(
            p.next_tag_with(&mut Host::default())
                .unwrap_err()
                .to_string()
                .contains("crosses conditional scope")
        );
    }
    #[test]
    fn multiline_flags_parameter_macros_emb_and_disabled_errors() {
        let mut p = parser(
            "[pmacro name=paint red=42][item paint explicit=true \\ignored trailer\r\n value=ok][&i]",
        );
        p.set_multi_line_tag_enabled(true).unwrap();
        let mut h = Host::default();
        let t = p.next_tag_with(&mut h).unwrap().unwrap();
        assert_eq!(t.literal_attr("red"), Some("42"));
        assert_eq!(t.literal_attr("explicit"), Some("true"));
        assert_eq!(t.literal_attr("value"), Some("ok"));
        assert!(
            t.attributes
                .iter()
                .all(|a| !a.name().is_some_and(|n| n.contains('\0')))
        );
        let mut p = parser("@item a=yes \\\n b=ok\n[done]");
        p.set_multi_line_tag_enabled(true).unwrap();
        assert_eq!(
            p.next_tag_with(&mut h).unwrap().unwrap().literal_attr("b"),
            Some("ok")
        );
        assert_eq!(p.next_tag_with(&mut h).unwrap().unwrap().tagname, "done");
        let mut p = parser("[item a=yes \\\n b=bad]");
        assert!(
            p.next_tag_with(&mut h)
                .unwrap_err()
                .to_string()
                .contains("disabled")
        );
    }
}

#[cfg(test)]
mod void_arguments {
    use super::*;
    #[test]
    fn missing_macro_argument_remains_void_and_an_explicit_fallback_is_literal() {
        let mut parser = KagParser::new();
        parser.enable_ext_kag();
        parser.set_ignore_cr(true);
        parser
            .load_scenario_text(
                "authored.ks",
                "[macro name=m][item absent=%missing fallback=%missing|fallback][endmacro][m]",
            )
            .unwrap();
        let tag = parser.next_tag().unwrap().unwrap();
        assert_eq!(tag.attr("absent"), Some(&AttributeValue::Void));
        assert_eq!(tag.literal_attr("fallback"), Some("fallback"));
    }
}
