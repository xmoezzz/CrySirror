// Copyright (C) 2026 CrySirror contributors. GPL-2.0-or-later.
//! Typed native HLL arguments and completion. No host pointers enter VM state.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Argument {
    Scalar { kind: i32, bits: i32 },
    String { slot: i32, bytes: Arc<Vec<u8>> },
    Page { kind: i32, slot: i32 },
    Reference { kind: i32, page: i32, index: i32 },
    HeapReference { kind: i32, slot: i32 },
}
#[derive(Clone, Debug)]
pub enum HllReturn {
    Void,
    Scalar(i32),
    String(Vec<u8>),
}
impl Machine {
    pub fn hll_arguments(&self, call: &ExternalCall) -> Result<Vec<Argument>> {
        ensure!(
            self.pending.as_ref() == Some(call),
            "System4 stale external receipt"
        );
        let ExternalTarget::Hll { library, function } = call.target else {
            bail!("System4 not a HLL receipt")
        };
        let f = &self.program.libraries[library].functions[function];
        let mut n = 0;
        let mut out = Vec::new();
        for (_, k) in f.arguments.iter().rev() {
            let arg = match *k {
                18 | 19 | 51 | 59 => {
                    let (p, i) = self.reference(n)?;
                    n += 2;
                    Argument::Reference {
                        kind: *k,
                        page: p,
                        index: i,
                    }
                }
                12 => {
                    let slot = self.peek(n)?;
                    n += 1;
                    Argument::String {
                        slot,
                        bytes: self.heap.string(slot)?,
                    }
                }
                13 | 14..=17 | 30 | 50 | 58 | 66 => {
                    let slot = self.peek(n)?;
                    self.heap.page_opt(slot)?;
                    n += 1;
                    Argument::Page { kind: *k, slot }
                }
                20..=25 | 32 | 52 | 60 | 67 | 69 | 80 => {
                    let slot = self.peek(n)?;
                    if slot != -1 {
                        self.heap.slot(slot)?;
                    }
                    n += 1;
                    Argument::HeapReference { kind: *k, slot }
                }
                0 | 10 | 11 | 26 | 27 | 31 | 47 | 55 => {
                    let bits = self.peek(n)?;
                    n += 1;
                    Argument::Scalar { kind: *k, bits }
                }
                _ => bail!("System4 unsupported HLL argument type {k}"),
            };
            out.push(arg);
        }
        out.reverse();
        Ok(out)
    }
    pub fn complete_hll(&mut self, call: &ExternalCall, value: HllReturn) -> Result<()> {
        let args = self.hll_arguments(call)?;
        let ExternalTarget::Hll { library, function } = call.target else {
            unreachable!()
        };
        let kind = self.program.libraries[library].functions[function].return_kind;
        let result = match (kind, value) {
            (0, HllReturn::Void) => None,
            (10 | 11 | 26 | 27 | 47 | 55, HllReturn::Scalar(v)) => {
                Some(if kind == 47 { (v != 0) as i32 } else { v })
            }
            (12, HllReturn::String(s)) => Some(self.heap.alloc(Object::String(Arc::new(s)))?),
            _ => bail!("System4 HLL return ABI mismatch"),
        };
        let mut words = 0;
        let mut releases = Vec::new();
        for arg in args {
            words += if matches!(arg, Argument::Reference { .. }) {
                2
            } else {
                1
            };
            match arg {
                Argument::String { slot, .. } | Argument::Page { slot, .. } => {
                    releases.push(Action::ExitRelease(slot))
                }
                _ => {}
            }
        }
        self.consume(words)?;
        if let Some(v) = result {
            self.push(v)?;
        }
        self.pending = None;
        self.instructions += 1;
        self.schedule(releases, call.next);
        Ok(())
    }
    pub(crate) fn validate_system(&self,call:&ExternalCall)->Result<()> {
        ensure!(self.pending.as_ref()==Some(call) && matches!(call.target,ExternalTarget::System(_)),"System4 stale syscall receipt");
        Ok(())
    }
    /// Shared-service adapters select exact, already-validated syscall ABIs.
    pub(crate) fn complete_system(
        &mut self,
        call: &ExternalCall,
        words: usize,
        value: Option<i32>,
    ) -> Result<()> {
        ensure!(
            self.pending.as_ref() == Some(call) && matches!(call.target, ExternalTarget::System(_)),
            "System4 stale syscall receipt"
        );
        self.peek(words.saturating_sub(1))
            .or_else(|e| if words == 0 { Ok(0) } else { Err(e) })?;
        self.consume(words)?;
        if let Some(v) = value {
            self.push(v)?;
        }
        self.pending = None;
        self.pc = call.next;
        self.instructions += 1;
        Ok(())
    }
    pub fn stack_value(&self, from_top: usize) -> Result<i32> {
        self.peek(from_top)
    }
}
