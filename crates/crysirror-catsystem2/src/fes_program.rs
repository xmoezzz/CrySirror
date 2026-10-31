// SPDX-License-Identifier: MPL-2.0
//! Bounded original FES orchestration VM. Raster/text/UI/audio operations are
//! native pending operations; no renderer, image algorithm or mixer lives here.
use crate::native_interpreter::Interpreter;
use anyhow::{ensure, Context, Result};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct Alias {
    pub mode: Vec<u8>,
    pub name: Vec<u8>,
    pub resource: Vec<u8>,
    pub defaults: Vec<(Vec<u8>, Vec<u8>)>,
    pub arguments: Vec<Vec<u8>>,
}
#[derive(Clone, Debug)]
pub(crate) struct Line {
    pub offset: usize,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub(crate) struct Program {
    pub lines: Vec<Line>,
    pub sections: BTreeMap<Vec<u8>, Vec<usize>>,
    pub aliases: BTreeMap<Vec<u8>, Alias>,
}
fn words(bytes: &[u8], check: &dyn Fn() -> Result<()>) -> Result<Vec<Vec<u8>>> {
    //767050's grouped-token profile needs a broader shared lexer for quotes.
    //This explicit ASCII/no-quotes profile is equivalent to its space splitter;
    //the general byte splitting algorithm is the reusable Text service.
    ensure!(
        bytes.is_ascii() && !bytes.contains(&b'"') && !bytes.contains(&b'\t'),
        "CatSystem2 unsupported grouped/non-ASCII FES token profile"
    );
    let out = crysirror_text::byte_split::split_delimited_bytes(
        bytes,
        b" ",
        crysirror_text::byte_split::ByteSplitBoundary::SingleByte,
        Default::default(),
        check,
    )?;
    Ok(out.spans.into_iter().map(|r| bytes[r].to_vec()).collect())
}
impl Program {
    pub fn parse(bytes: &[u8], check: &dyn Fn() -> Result<()>) -> Result<Self> {
        check()?;
        ensure!(
            bytes.len() <= 16 * 1024 * 1024 && !bytes.contains(&0),
            "CatSystem2 FES text extent/NUL"
        );
        //73FF40 dollar-str substitution and766720 macro expansion are not
        //silently ignored. Plain-text include/preprocess is separately gated.
        ensure!(
            !bytes.windows(4).any(|w| w == b"$str"),
            "CatSystem2 unsupported FES string-variable expansion"
        );
        let mut lines = Vec::new();
        let mut sections: BTreeMap<Vec<u8>, Vec<usize>> = BTreeMap::new();
        let mut active: Option<Vec<u8>> = None;
        let mut at = 0;
        while at < bytes.len() {
            check()?;
            ensure!(lines.len() < 65536, "CatSystem2 FES line budget");
            let start = at;
            while at < bytes.len() && !matches!(bytes[at], b'\r' | b'\n') {
                at += 1;
            }
            let line = &bytes[start..at];
            ensure!(
                line.len() < 1024,
                "CatSystem2 FES native line buffer extent"
            );
            if at < bytes.len() {
                let cr = bytes[at] == b'\r';
                at += 1;
                if cr && bytes.get(at) == Some(&b'\n') {
                    at += 1;
                }
            }
            let w = words(line, check)?;
            ensure!(
                !line.windows(2).any(|w| matches!(w, b"//" | b"/*" | b"*/")),
                "CatSystem2 unsupported non-preprocessed FES comments"
            );
            let index = lines.len();
            if line.first() == Some(&b'#') {
                ensure!(
                    w.len() == 1 && w[0].len() > 1 && w[0].len() < 32,
                    "CatSystem2 unsupported FES section header"
                );
                let key = w[0][1..].to_vec();
                ensure!(
                    ![b"MACRO".as_slice(), b"DEFINE", b"KEYBLOCK", b"OBJECT"]
                        .contains(&key.as_slice()),
                    "CatSystem2 unsupported FES declaration section {}",
                    String::from_utf8_lossy(&key)
                );
                active = Some(key);
            }
            if let Some(key) = &active {
                sections.entry(key.clone()).or_default().push(index);
            } else {
                ensure!(line.is_empty(), "CatSystem2 FES text before first section");
            }
            lines.push(Line {
                offset: start,
                bytes: line.to_vec(),
            });
        }
        ensure!(
            sections.contains_key(b"START".as_slice()),
            "CatSystem2 FES START section missing"
        );
        let mut aliases = BTreeMap::new();
        if let Some(rows) = sections.get(b"SUBCOMMAND".as_slice()) {
            for &i in rows {
                let b = &lines[i].bytes;
                if b.first() == Some(&b'#') || b.is_empty() {
                    continue;
                }
                let w = words(b, check)?;
                let colon = w
                    .iter()
                    .position(|w| w == b":")
                    .context("CatSystem2 FES alias separator")?;
                ensure!(
                    colon >= 3
                        && colon <= 19
                        && matches!(w[0].as_slice(), b"oneshot" | b"continue")
                        && w[1].len() < 32,
                    "CatSystem2 unsupported FES alias layout"
                );
                let mut defaults = Vec::new();
                for p in &w[3..colon] {
                    let split = p
                        .iter()
                        .position(|b| *b == b'[')
                        .context("CatSystem2 FES alias parameter")?;
                    ensure!(
                        split > 0
                            && p.last() == Some(&b']')
                            && !p[split + 1..p.len() - 1].contains(&b'['),
                        "CatSystem2 FES alias parameter layout"
                    );
                    defaults.push((p[..split].to_vec(), p[split + 1..p.len() - 1].to_vec()));
                }
                ensure!(
                    w.len() - colon - 1 <= 16,
                    "CatSystem2 FES alias argument budget"
                );
                let alias = Alias {
                    mode: w[0].clone(),
                    name: w[1].clone(),
                    resource: w[2].clone(),
                    defaults,
                    arguments: w[colon + 1..].to_vec(),
                };
                aliases.insert(alias.name.clone(), alias);
            }
        }
        check()?;
        Ok(Self {
            lines,
            sections,
            aliases,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingOperation {
    pub section: Vec<u8>,
    pub ordinal: u32,
    pub line: usize,
    pub byte_offset: usize,
    pub name: Vec<u8>,
    pub arguments: Vec<Vec<u8>>,
}
#[derive(Clone, Debug)]
pub(crate) struct Machine {
    pub program: Arc<Program>,
    pub owner: u32,
    pub section: Vec<u8>,
    pub ordinal: u32,
    pub executed: u64,
    pub pending: Option<PendingOperation>,
}
impl Machine {
    pub fn new(program: Arc<Program>, owner: u32, interpreter: &mut Interpreter) -> Result<Self> {
        ensure!(
            interpreter.create_player_bank(owner)?,
            "CatSystem2 unsupported FES player reinitialization"
        );
        for name in [
            b"_EXITCODE_".as_slice(),
            b"_CLICK_L_",
            b"_CLICK_R_",
            b"_SCRL_U_",
            b"_SCRL_D_",
            b"_PUSH_UP_",
            b"_PUSH_DOWN_",
            b"_PUSH_RIGHT_",
            b"_PUSH_LEFT_",
        ] {
            interpreter.set_named_value(
                name,
                0,
                if name == b"_EXITCODE_" { u32::MAX } else { 0 },
            )?;
        }
        Ok(Self {
            program,
            owner,
            section: b"START".to_vec(),
            ordinal: 1,
            executed: 0,
            pending: None,
        })
    }
    ///Runs original integer-expression commands until a native operation. The
    ///operation is retained at its original section/row/byte offset on retries.
    pub fn step(
        &mut self,
        interpreter: &mut Interpreter,
        budget: u32,
    ) -> Result<Option<PendingOperation>> {
        if let Some(p) = &self.pending {
            return Ok(Some(p.clone()));
        }
        ensure!(
            interpreter.select_player_bank(self.owner),
            "CatSystem2 FES local bank missing"
        );
        //7563A0's first tick uses base=a2-1 and therefore_FRAME_=1, including
        //wrapping at0. Further native waits/clocks are not mapped by this VM.
        interpreter.set_named_value(b"_FRAME_", 0, 1)?;
        for _ in 0..budget {
            let rows = self
                .program
                .sections
                .get(&self.section)
                .context("CatSystem2 FES section missing")?;
            let Some(&i) = rows.get(self.ordinal as usize) else {
                return Ok(None);
            };
            let line = &self.program.lines[i];
            let w = words(&line.bytes, &|| Ok(()))?;
            if w.is_empty() {
                self.ordinal += 1;
                continue;
            }
            if matches!(line.bytes.first(), Some(b'$' | b'\\')) {
                interpreter.evaluate(&line.bytes)?;
                self.executed += 1;
                self.ordinal += 1;
                continue;
            }
            let p = PendingOperation {
                section: self.section.clone(),
                ordinal: self.ordinal,
                line: i,
                byte_offset: line.offset,
                name: w[0].clone(),
                arguments: w[1..].to_vec(),
            };
            self.pending = Some(p.clone());
            return Ok(Some(p));
        }
        Ok(None)
    }
    pub fn diagnostics(&self) -> serde_json::Value {
        let aliases=self.program.aliases.values().map(|a|serde_json::json!({"mode":String::from_utf8_lossy(&a.mode),"name":String::from_utf8_lossy(&a.name),"resource":String::from_utf8_lossy(&a.resource),"parameters":a.defaults.len(),"arguments":a.arguments.len()})).collect::<Vec<_>>();
        serde_json::json!({"owner":self.owner,"section":String::from_utf8_lossy(&self.section),"ordinal":self.ordinal,"executed":self.executed,"lines":self.program.lines.len(),"sections":self.program.sections.len(),"aliases":aliases,"pending":self.pending.as_ref().map(|p|serde_json::json!({"section":String::from_utf8_lossy(&p.section),"ordinal":p.ordinal,"line":p.line,"byte_offset":p.byte_offset,"name":String::from_utf8_lossy(&p.name),"arguments":p.arguments.iter().map(|b|String::from_utf8_lossy(b)).collect::<Vec<_>>()}))})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_local_bank_and_expression_execution_stop_at_original_pending_operation() {
        let p=Arc::new(Program::parse(b"#SUBCOMMAND\r\noneshot move motMove px[0] : [0]\r\n#START\r\n$3 = 9\r\n\\2 = 17\r\nplatform_query 4\r\n$3 = 10\r\n",&||Ok(())).unwrap());
        let mut interpreter = Interpreter::new(32).unwrap();
        let mut vm = Machine::new(p, 8, &mut interpreter).unwrap();
        assert_eq!(interpreter.evaluate(b"_EXITCODE_").unwrap(), u32::MAX);
        let pending = vm.step(&mut interpreter, 64).unwrap().unwrap();
        assert_eq!(interpreter.flag(3), 9);
        assert_eq!(interpreter.evaluate(b"\\2").unwrap(), 17);
        assert_eq!(pending.name, b"platform_query");
        assert_eq!(pending.ordinal, 3);
        assert_eq!(vm.executed, 2);
        assert_eq!(vm.step(&mut interpreter, 64).unwrap(), Some(pending));
        assert_eq!(interpreter.flag(3), 9);
        assert!(!interpreter.select_player_bank(999));
        assert_eq!(interpreter.evaluate(b"\\2").unwrap(), 17);
    }
    #[test]
    fn unsupported_declarations_templates_and_cancellation_never_make_a_fake_player() {
        for b in [
            b"#START\n#OBJECT\nimage foo".as_slice(),
            b"#START\n$str1",
            b"#START\nmsg \"x y\"",
            b"#START\n//not-preprocessed",
        ] {
            assert!(Program::parse(b, &|| Ok(())).is_err());
        }
        assert!(Program::parse(b"#START\n$0=1", &|| anyhow::bail!("cancelled")).is_err());
    }
}
