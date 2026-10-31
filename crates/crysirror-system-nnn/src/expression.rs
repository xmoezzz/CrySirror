// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Independently expressed SPT value and postfix-expression format facts.
//! Original variable references remain references through assignment/unary !.
use crate::Program;
use anyhow::{bail, ensure, Context, Result};
use crysirror_image::temporal_pattern::PatternRng;
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariableProfile {
    Legacy1000,
    Expanded2200,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeVariableWrite {
    pub variable: u16,
    pub layer: u8,
    pub value: i32,
}
#[derive(Clone, Debug)]
pub struct Variables {
    values: Vec<i32>,
    calculated: BTreeMap<u16, Vec<u8>>,
    write_effects: BTreeMap<u16, Vec<u8>>,
    calculated_values: BTreeMap<u16, i32>,
    random: RefCell<Option<PatternRng>>,
    capture_writes: bool,
    native_writes: Vec<NativeVariableWrite>,
}
impl Variables {
    /// Original fresh-game storage only. The adapter must restore the selected
    /// profile's persistent range and install configured calculated bindings.
    pub fn fresh(profile: VariableProfile) -> Self {
        Self {
            values: vec![
                0;
                match profile {
                    VariableProfile::Legacy1000 => 1000,
                    VariableProfile::Expanded2200 => 2200,
                }
            ],
            calculated: BTreeMap::new(),
            write_effects: BTreeMap::new(),
            calculated_values: BTreeMap::new(),
            random: RefCell::new(None),
            capture_writes: false,
            native_writes: Vec::new(),
        }
    }
    pub fn mark_calculated(&mut self, id: u16, name: Vec<u8>) {
        self.calculated.insert(id, name);
    }
    pub fn mark_write_effect(&mut self, id: u16, name: Vec<u8>) {
        self.write_effects.insert(id, name);
    }
    pub fn native_writes(&self) -> &[NativeVariableWrite] {
        &self.native_writes
    }
    pub(crate) fn begin_native_instruction(&mut self) {
        self.capture_writes = true;
        self.native_writes.clear();
    }
    pub(crate) fn finish_native_instruction(&mut self) {
        self.capture_writes = false;
        self.native_writes.clear();
    }
    /// Explicit owner-sampled native service state. Random draws deliberately
    /// cannot use a constant snapshot: their per-read sequence is separate.
    pub fn bind_calculated_value(&mut self, name: &[u8], value: i32) -> Result<bool> {
        ensure!(
            name != b"rnd",
            "System-NNN rnd requires per-read shared sequence draws"
        );
        let Some(id) = self
            .calculated
            .iter()
            .find(|(_, n)| n.as_slice() == name)
            .map(|(&id, _)| id)
        else {
            return Ok(false);
        };
        self.calculated_values.insert(id, value);
        Ok(true)
    }
    pub fn clear_calculated_values(&mut self) {
        self.calculated_values.clear();
    }
    /// The same engine RNG state is handed to shared temporal-pattern workers
    /// and returned on completion. Script-frame copies are transaction data,
    /// never separate effect seeds or private RNG implementations.
    pub fn random_state(&self) -> Option<PatternRng> {
        self.random.borrow().clone()
    }
    pub fn replace_random_state(&mut self, state: PatternRng) -> Result<()> {
        state.validate()?;
        ensure!(
            (
                state.multiplier,
                state.increment,
                state.output_shift,
                state.output_mask
            ) == (214013, 2531011, 16, 32767),
            "System-NNN native MSVC random profile"
        );
        *self.random.get_mut() = Some(state);
        Ok(())
    }
    pub fn write_native(&mut self, id: u16, value: i32) -> Result<()> {
        if let Some(name) = self.write_effects.get(&id) {
            ensure!(
                self.capture_writes,
                "System-NNN variable {id} needs its native Layer/service write binding"
            );
            let layer = std::str::from_utf8(name)?
                .strip_prefix("layerOffVar")
                .context("System-NNN unknown variable write service")?
                .parse::<u8>()?;
            ensure!(layer < 32, "System-NNN variable write layer bounds");
            ensure!(
                self.native_writes.len() < 4096,
                "System-NNN variable write receipt budget"
            );
            self.native_writes.push(NativeVariableWrite {
                variable: id,
                layer,
                value,
            });
        }
        self.write_stored(id, value);
        Ok(())
    }
    pub fn read_stored(&self, id: u16) -> i32 {
        self.values.get(id as usize).copied().unwrap_or_else(|| {
            if id & 0x8000 != 0 {
                i32::from(id & 0x7fff)
            } else {
                0
            }
        })
    }
    pub fn read_calculated(&self, id: u16) -> Result<i32> {
        if let Some(name) = self.calculated.get(&id) {
            if name == b"rnd" {
                let mut candidate = self
                    .random
                    .borrow()
                    .clone()
                    .context("System-NNN rnd requires original shared sequence state")?;
                for attempt in 0..=50 {
                    let draw = candidate.next_draw()?;
                    if draw < 30000 || attempt == 50 {
                        *self.random.borrow_mut() = Some(candidate);
                        return Ok((draw % 10000) as i32);
                    }
                }
                unreachable!("bounded native random rejection loop")
            }
            if let Some(value) = self.calculated_values.get(&id) {
                return Ok(*value);
            }
            bail!(
                "System-NNN calculated variable {id} ({}) needs a native service binding",
                String::from_utf8_lossy(name)
            );
        }
        Ok(self.read_stored(id))
    }
    /// Native callback ignores out-of-range writes. Resource-affecting bindings
    /// must be rejected/mapped by the adapter before ordinary bank execution.
    pub fn write_stored(&mut self, id: u16, value: i32) {
        if let Some(v) = self.values.get_mut(id as usize) {
            *v = value;
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum Cell {
    Reference(u16),
    Value(i32),
}
fn value(c: Cell, vars: &Variables) -> Result<i32> {
    match c {
        Cell::Value(v) => Ok(v),
        Cell::Reference(id) if id & 0x8000 != 0 => Ok(i32::from(id & 0x7fff)),
        Cell::Reference(id) if id < 100 => vars.read_calculated(id),
        Cell::Reference(id) => Ok(vars.read_stored(id)),
    }
}
fn literal(word: u32, p: &Program) -> Result<Cell> {
    Ok(match word & 0xffff0000 {
        0x80000000 => Cell::Reference(word as u16),
        0x90000000 => Cell::Value(word as i16 as i32),
        0x91000000 => Cell::Value(
            *p.words
                .get(
                    (p.integer_pool as usize)
                        .checked_add(word as u16 as usize)
                        .context("SPT integer address overflow")?,
                )
                .context("SPT integer constant bounds")? as i32,
        ),
        _ => bail!("unsupported System-NNN expression literal {word:#x}"),
    })
}
fn arithmetic(op: u16, a: i32, mut b: i32) -> Result<i32> {
    Ok(match op as u8 {
        b'+' => a.wrapping_add(b),
        b'-' => a.wrapping_sub(b),
        b'*' => a.wrapping_mul(b),
        b'/' => {
            if b == 0 {
                b = 1;
            }
            a.checked_div(b)
                .context("System-NNN signed division overflow")?
        }
        b'%' => {
            if b == 0 {
                b = 1;
            }
            a.checked_rem(b)
                .context("System-NNN signed remainder overflow")?
        }
        b'&' => a & b,
        b'^' => a ^ b,
        b'|' => a | b,
        _ => bail!("unsupported System-NNN arithmetic operator {op:#x}"),
    })
}
/// Returns the evaluated native i32 and exact operand-word consumption.
/// Caller uses an instruction-local Variables candidate for atomic commit.
pub fn evaluate(words: &[u32], p: &Program, vars: &mut Variables) -> Result<(i32, usize)> {
    let first = *words.first().context("SPT expression absent")?;
    if first != 0xa0000000 {
        // Enzan's original single-token dispatch uses the high nibble. In
        // particular direct 0x91 constants take this signed-low16 path; the
        // full-width integer-pool interpretation belongs to postfix literals.
        return Ok((
            match first >> 28 {
                8 => vars.read_calculated(first as u16)?,
                9 => first as i16 as i32,
                12 | 13 => first as i32,
                _ => bail!("unsupported System-NNN direct expression {first:#x}"),
            },
            1,
        ));
    }
    let count = *words.get(1).context("SPT postfix size absent")? as usize;
    ensure!((1..=4096).contains(&count), "SPT postfix token budget");
    let tokens = words
        .get(2..2 + count)
        .context("SPT postfix token bounds")?;
    let mut stack = Vec::new();
    for &token in tokens {
        if matches!(token >> 28, 8 | 9) {
            ensure!(stack.len() < 255, "SPT expression stack limit");
            stack.push(literal(token, p)?);
            continue;
        }
        ensure!(
            token >> 28 == 10,
            "unsupported System-NNN postfix token {token:#x}"
        );
        let kind = token & 0xffff0000;
        let op = token as u16;
        ensure!(op <= 255, "unsupported System-NNN operator code {op:#x}");
        if kind == 0xa0000000 && op == u16::from(b'!') {
            let slot = stack.last_mut().context("SPT unary operand absent")?;
            let result = i32::from(value(*slot, vars)? == 0);
            match *slot {
                Cell::Reference(id) => vars.write_native(id, result)?,
                Cell::Value(_) => *slot = Cell::Value(result),
            }
            continue;
        }
        let rhs = stack.pop().context("SPT binary right operand absent")?;
        let lhs = stack.pop().context("SPT binary left operand absent")?;
        let assignment = kind == 0xa0000000 && op == u16::from(b'=');
        if assignment {
            ensure!(
                matches!(lhs, Cell::Reference(_)),
                "SPT assignment target is not a variable"
            );
        }
        // Original callback evaluation reads left before right, except plain
        // assignment which only resolves the destination identity.
        let a = if assignment {
            None
        } else {
            Some(value(lhs, vars)?)
        };
        let b = value(rhs, vars)?;
        let result = match (kind, op as u8) {
            (0xa0000000, b'=') => {
                let Cell::Reference(id) = lhs else {
                    bail!("SPT assignment target is not a variable")
                };
                vars.write_native(id, b)?;
                lhs
            }
            (0xa2000000, op) if op != b'=' => {
                let Cell::Reference(id) = lhs else {
                    bail!("SPT compound target is not a variable")
                };
                let a = a.context("SPT compound left value absent")?;
                vars.write_native(
                    id,
                    if op == b'!' {
                        i32::from(b == 0)
                    } else {
                        arithmetic(u16::from(op), a, b)?
                    },
                )?;
                lhs
            }
            (0xa0000000, _) => Cell::Value(arithmetic(
                op,
                a.context("SPT arithmetic left value absent")?,
                b,
            )?),
            (0xa2000000, b'=') | (0xa3000000, b'=') | (0xa5000000, b'=') => Cell::Value(i32::from(
                a.context("SPT comparison left value absent")? == b,
            )),
            (0xa3000000, b'&') => Cell::Value(i32::from(
                a.context("SPT logical left value absent")? != 0 && b != 0,
            )),
            (0xa3000000, b'|') => Cell::Value(i32::from(
                a.context("SPT logical left value absent")? != 0 || b != 0,
            )),
            (0xa3000000, b'^') => Cell::Value(i32::from(
                (a.context("SPT logical left value absent")? != 0) ^ (b != 0),
            )),
            (0xa4000000, b'<') => Cell::Value(i32::from(
                a.context("SPT comparison left value absent")? < b,
            )),
            (0xa4000000, b'>') => Cell::Value(i32::from(
                a.context("SPT comparison left value absent")? > b,
            )),
            (0xa5000000, b'<') => Cell::Value(i32::from(
                a.context("SPT comparison left value absent")? <= b,
            )),
            (0xa5000000, b'>') => Cell::Value(i32::from(
                a.context("SPT comparison left value absent")? >= b,
            )),
            (0xa5000000, b'!') => Cell::Value(i32::from(
                a.context("SPT comparison left value absent")? != b,
            )),
            _ => bail!("unsupported System-NNN postfix operator {token:#x}"),
        };
        stack.push(result);
    }
    ensure!(stack.len() == 1, "SPT postfix final stack size");
    Ok((value(stack[0], vars)?, count + 2))
}
