// SPDX-License-Identifier: MPL-2.0
//! RLD v1 local register overrides and native register arithmetic.
use crate::branch::{Condition, number};
use crate::midori_state::{DbDefinition, DbLookup, SceneDone};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Deserialize)]
pub(super) struct Limits {
    pub regs: usize,
    pub sys: usize,
    pub cg: usize,
    pub scene: usize,
    pub sound: usize,
    pub strings: usize,
    #[serde(default)]
    pub characters: usize,
    #[serde(default = "default_messages")]
    pub messages: usize,
    #[serde(default = "default_local_count")]
    pub locals: usize,
}

fn default_messages() -> usize {
    48000
}
fn default_local_count() -> usize {
    20
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Bank {
    Cg,
    Sound,
    Scene,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Value {
    Immediate(i32),
    Local(i32),
    System(i32),
    RelativeSystem(i32),
    Flag(i32),
    Seen(Bank, i32),
    TextNumber(i32),
}
impl Value {
    pub fn expression(raw: &[u8]) -> Result<Self> {
        let raw = raw.trim_ascii();
        let (&head, tail) = raw.split_first().context("empty register expression")?;
        Ok(match head.to_ascii_uppercase() {
            b'R' => Self::Local(number(tail)?),
            b'S' => Self::RelativeSystem(number(tail)?),
            b'L' => Self::RelativeSystem(number(tail)?.wrapping_add(1_000_000)),
            b'F' => Self::Flag(number(tail)?),
            b'M' => {
                let (&which, id) = tail.split_first().context("missing experience bank")?;
                let bank = match which.to_ascii_uppercase() {
                    b'G' => Bank::Cg,
                    b'S' => Bank::Scene,
                    b'M' => Bank::Sound,
                    _ => bail!("unsupported native experience bank"),
                };
                Self::Seen(bank, number(id)?)
            }
            _ => Self::Immediate(number(raw)?),
        })
    }
    pub fn from_pair(kind: i32, n: i32) -> Self {
        match kind {
            1 => Self::Local(n),
            2 => Self::System(n),
            _ => Self::Immediate(n),
        }
    }
}
#[derive(Debug, PartialEq)]
pub(super) enum TextValue {
    Literal(Vec<u8>),
    Global(i32),
    System(i32),
    Text(i32),
}

#[derive(Debug, PartialEq)]
pub(super) enum Change {
    Arithmetic {
        to: Value,
        op: i32,
        left: Value,
        right: Value,
    },
    Assign(Value, i32, Value, Condition),
    Action2011 {
        operation: Action2011,
        result: i32,
    },
    TextAssign {
        dest: i32,
        mode: i32,
        source: TextValue,
        replacement: Vec<u8>,
        when: Condition,
    },
    Experience(Bank, Vec<i32>),
    MidoriDefineDb(DbDefinition),
    MidoriGetDb(DbLookup),
    MidoriSceneDone(SceneDone),
    LocalFlag(i32, bool),
}
#[derive(Debug, PartialEq)]
pub(super) enum Action2011 {
    SetLocalFlag7,
    Text208And209(i32, Vec<u8>),
    Text206(Vec<u8>),
    ClearRange { bank: i32, first: i32, last: i32 },
    SetBits23(i32),
}
pub(super) fn action2011(raw: &[u8]) -> Result<Change> {
    // cmdAction takes six numeric fields and an unsplit final CP932 string.
    let mut fields = raw.splitn(7, |b| *b == b',');
    let mut numeric = || number(fields.next().context("missing action field")?);
    let (code, result) = (numeric()?, numeric()?);
    let args = [numeric()?, numeric()?, numeric()?, numeric()?];
    let text = fields.next().context("missing action string")?;
    let action_text = || if text == b"*" { vec![] } else { text.to_vec() };
    let operation = match code {
        190 => Action2011::SetLocalFlag7,
        360 => Action2011::Text208And209(args[0], action_text()),
        262 => Action2011::Text206(action_text()),
        346 => Action2011::ClearRange {
            bank: args[0],
            first: args[1].min(args[2]),
            last: args[1].max(args[2]),
        },
        348 => Action2011::SetBits23(args[0]),
        _ => bail!("unsupported ExHibit 2011 action {code}"),
    };
    Ok(Change::Action2011 { operation, result })
}

/// Detached native system-register image. Every absent cell in the
/// contiguous span is an explicit zero when this is eventually applied.
pub(super) struct ExperienceCapture {
    pub base: i32,
    pub counts: [usize; 4],
    pub words: usize,
    nonzero: BTreeMap<i32, i32>,
}
impl ExperienceCapture {
    pub fn nonzero_words(&self) -> usize {
        self.nonzero.len()
    }

    #[cfg(test)]
    pub fn cell(&self, offset: usize) -> Option<i32> {
        (offset < self.words).then(|| {
            self.nonzero
                .get(&(self.base + offset as i32))
                .copied()
                .unwrap_or(0)
        })
    }

    fn apply(self, system: &mut BTreeMap<i32, i32>) {
        system.retain(|&index, _| index < self.base || index >= self.base + self.words as i32);
        system.extend(self.nonzero);
    }
}

#[derive(Default)]
pub(super) struct Registers {
    limits: Option<Limits>,
    normal: BTreeMap<i32, i32>,
    local: BTreeMap<i32, i32>,
    sys: BTreeMap<i32, i32>,
    seen: [BTreeMap<i32, i32>; 3],
    strings: BTreeMap<i32, Vec<u8>>,
    local_flag: bool,
    messages: BTreeMap<usize, u32>,
    integer_tables: [BTreeMap<i32, [i32; 8]>; 8],
}
impl Registers {
    pub fn new(limits: Option<Limits>) -> Result<Self> {
        if let Some(l) = limits {
            ensure!(
                l.regs > 0
                    && l.sys > 0
                    && [l.regs, l.sys, l.cg, l.sound, l.scene, l.strings, l.messages]
                        .into_iter()
                        .all(|n| n <= 1_000_000),
                "invalid ExHibit native register dimensions"
            )
        }
        Ok(Self {
            limits,
            ..Self::default()
        })
    }
    pub fn configured(&self) -> bool {
        self.limits.is_some()
    }
    pub fn for_scenario(limits: Option<Limits>, abi: u32, canvas: [u32; 2]) -> Result<Self> {
        let mut regs = Self::new(limits)?;
        let Some(l) = limits else { return Ok(regs) };
        // RetouchParam::create reserves three additional character slots.
        let voices = l
            .characters
            .checked_add(3)
            .context("native character count overflow")?;
        let base = voices
            .checked_mul(2)
            .and_then(|v| v.checked_add(43))
            .context("native system register base overflow")?;
        let local_count = if abi == 1 { 20 } else { l.locals };
        ensure!(
            base.checked_add(local_count).is_some_and(|v| v < l.sys) && base + 4 < l.sys,
            "native system register areas exceed configuration"
        );
        let base = base as i32;
        let relative_default = if abi == 1 { 2 } else { 1 };
        for (k, v) in [
            (1, 1),
            (10, 0x2200),
            (12, 34),
            (13, 1500),
            (15, base),
            (16, base + local_count as i32),
            (18, 200),
            (19, 200),
            (20, 200),
            (22, 200),
            (23, 1),
            (base, relative_default),
            (base + 1, relative_default),
            (base + 2, 1),
            (base + 4, 6),
        ] {
            regs.sys.insert(k, v);
        }
        for i in 0..voices as i32 {
            regs.sys.insert(44 + 2 * i, 128);
        }
        if abi == 1 {
            regs.sys.extend([
                (17, 250),
                (21, 150),
                (24, 4316),
                (30, 1),
                (32, 96),
                (33, 96),
                (35, 192),
                (36, 192),
                (38, 192),
                (39, 192),
                (41, 128),
                (42, 128),
            ]);
        } else {
            let [w, h] = canvas.map(|n| n as i32);
            regs.sys.extend([
                (8, ((300 + (w - 800) / 2) << 16) + (360 + (h - 600) / 2)),
                (17, 200),
                (21, 200),
                (24, 220),
                (31, 0x600060),
                (34, 0x800080),
                (32, 0xc000c0),
                (33, 0xc000c0),
                (35, 0x800080),
                (36, 0x400040),
            ]);
        }
        Ok(regs)
    }
    pub fn capture_experience(&self) -> Result<ExperienceCapture> {
        let l = self
            .limits
            .context("missing ExHibit checkpoint dimensions")?;
        // The native history bitmap (2011 0x16ea50) allocates a minimum
        // of one bit and rounds storage up to a whole DWORD.
        let counts = [l.cg, l.scene, l.sound, l.messages.max(1).div_ceil(32)];
        let base = self.read(Value::System(16))?;
        let words = 4 + counts.into_iter().sum::<usize>();
        ensure!(
            base >= 0
                && (base as usize)
                    .checked_add(words)
                    .is_some_and(|end| end <= l.sys),
            "invalid ExHibit scene checkpoint layout"
        );
        let mut nonzero = BTreeMap::new();
        for (offset, count) in counts.into_iter().enumerate().filter(|(_, n)| *n != 0) {
            nonzero.insert(base + offset as i32, count as i32);
        }
        let mut at = base + 4;
        for (bank, count) in [Bank::Cg, Bank::Scene, Bank::Sound].into_iter().zip(counts) {
            nonzero.extend(
                self.seen[bank as usize]
                    .iter()
                    .filter(|&(index, value)| (*index as usize) < count && *value != 0)
                    .map(|(&index, &value)| (at + index, value)),
            );
            at += count as i32;
        }
        nonzero.extend(
            self.messages
                .iter()
                .filter(|&(index, bits)| *index < counts[3] && *bits != 0)
                .map(|(&index, &bits)| (at + index as i32, bits as i32)),
        );
        Ok(ExperienceCapture {
            base,
            counts,
            words,
            nonzero,
        })
    }

    pub fn checkpoint_v1(
        &mut self,
        block: i32,
        title: &[u8],
        draw: &[i32; 19],
        audio: &[i32; 12],
        views: [bool; 2],
    ) -> Result<()> {
        let l = self
            .limits
            .context("missing ExHibit checkpoint dimensions")?;
        ensure!(
            l.strings > 1 && l.regs >= 73,
            "invalid ExHibit scene checkpoint layout"
        );
        let experience = self.capture_experience()?;
        self.local.insert(2, block);
        self.strings.insert(1, title.to_vec());
        // blockEntry (v1 RVA 0x54b70) captures message open and local
        // control depth <= 0 before flushing the LocalHolder.
        let [message_open, local_controls_open] = views;
        let flags = self.normal.get(&4).copied().unwrap_or(0);
        self.normal.insert(
            4,
            (flags & !3) | i32::from(message_open) | (i32::from(local_controls_open) << 1),
        );
        for (i, v) in std::mem::take(&mut self.local) {
            self.normal.insert(Self::index(i, l.regs), v);
        }
        self.normal.extend((10..29).zip(draw.iter().copied()));
        // saveCharactersInfo clears all eight fields of each inactive display slot.
        self.normal.extend((29..61).map(|i| (i, -1)));
        self.normal.extend((61..73).zip(audio.iter().copied()));
        experience.apply(&mut self.sys);
        Ok(())
    }
    pub fn mark_message(&mut self, id: i32) -> Result<()> {
        let count = self
            .limits
            .context("missing ExHibit message capacity")?
            .messages
            .max(1);
        self.mark_message_in(id, count);
        Ok(())
    }
    fn mark_message_in(&mut self, id: i32, count: usize) {
        if (id as usize) < count {
            *self.messages.entry(id as usize / 32).or_default() |= 1u32 << ((id as usize % 32) ^ 7);
        }
    }
    fn increment_experience(&mut self, bank: Bank, id: i32, capacity: usize) {
        if (id as usize) < capacity {
            let n = self.seen[bank as usize].entry(id).or_default();
            if *n < 0x7fff_ff00 {
                *n += 1
            }
        }
    }
    pub fn set_normal(&mut self, index: i32, n: i32) -> Result<()> {
        let l = self
            .limits
            .context("missing ExHibit register configuration")?;
        self.normal.insert(Self::index(index, l.regs), n);
        Ok(())
    }
    pub fn set_local(&mut self, index: i32, n: i32) -> Result<()> {
        let limits = self
            .limits
            .context("missing ExHibit register configuration")?;
        self.store(Value::Local(index), n, limits);
        Ok(())
    }
    pub fn set_system(&mut self, index: i32, n: i32) -> Result<()> {
        let l = self
            .limits
            .context("missing ExHibit register configuration")?;
        self.sys.insert(Self::index(index, l.sys), n);
        Ok(())
    }
    // FCRegItem returns element zero for an invalid ordinary/system index.
    fn index(i: i32, limit: usize) -> i32 {
        if (i as usize) < limit { i } else { 0 }
    }
    fn relative(&self, index: i32, l: Limits) -> i32 {
        if index >= 1_000_000 {
            self.sys
                .get(&Self::index(15, l.sys))
                .copied()
                .unwrap_or(0)
                .wrapping_add(index - 1_000_000)
        } else {
            index
        }
    }
    pub fn text(&self, i: i32) -> Result<&[u8]> {
        let l = self.limits.context("missing register configuration")?;
        ensure!((i as usize) < l.strings, "text register out of bounds");
        Ok(self.strings.get(&i).map(Vec::as_slice).unwrap_or_default())
    }
    fn store(&mut self, to: Value, n: i32, l: Limits) {
        match to {
            Value::Local(i) if i >= 0 => {
                self.local.insert(i, n);
            }
            Value::System(i) => {
                self.sys.insert(Self::index(i, l.sys), n);
            }
            Value::RelativeSystem(i) => {
                let index = Self::index(self.relative(i, l), l.sys);
                self.sys.insert(index, n);
            }
            _ => {}
        }
    }
    /// Midori liteLoad (0xc5d50) publishes the caller's successful logical
    /// RLD path, not the joined host/prefix path and not the script stem.
    pub fn midori_scenario_loaded(&mut self, path: &crysirror_core::path::VfsPath) -> Result<()> {
        let Some(_) = self.limits else { return Ok(()) };
        self.text(2)?;
        let name = path.relative();
        ensure!(
            path.mount() == crysirror_core::path::Mount::Asset
                && name.starts_with("rld/") && name.ends_with(".rld"),
            "invalid Midori scenario register path"
        );
        let native = name.bytes().map(|b| if b == b'/' { b'\\' } else { b }).collect();
        self.strings.insert(2, native);
        Ok(())
    }
    pub fn enter_transferred_script(&mut self) {
        if let Some(l) = self.limits {
            for (i, n) in std::mem::take(&mut self.local) {
                self.normal.insert(Self::index(i, l.regs), n);
            }
            self.normal.insert(Self::index(2, l.regs), 0);
            self.strings.remove(&1);
        }
    }
    #[allow(dead_code)] // gated until action250 can execute its complete program
    pub fn apply_midori_rnf_literals(
        &mut self,
        writes: &[crate::midori_state::RnfWrite],
    ) -> Result<()> {
        use crate::midori_state::RnfWrite;
        let l = self
            .limits
            .context("missing Midori settings register dimensions")?;
        let base = self.read(Value::System(15))?;
        let flag_slot = base.checked_add(4).context("Midori flag base overflow")?;
        let relative = |index: i32| {
            base.checked_add(index)
                .context("Midori local settings index overflow")
        };
        let system = |index: i32| {
            ensure!(
                (index as usize) < l.sys && !matches!(index, 15 | 16),
                "Midori settings register out of scoped range"
            );
            Ok(())
        };
        // Validate the entire literal program before mutating any register.
        // The normal/relative banks keep native aliases; structural bases
        // cannot be altered by this subset.
        for &write in writes {
            match write {
                RnfWrite::LocalFlag(index, _) => {
                    ensure!(index < 32 && flag_slot >= 4, "unsupported Midori flag slot");
                    system(flag_slot)?;
                }
                RnfWrite::NormalAndSystem(index, _) => {
                    ensure!(
                        (index as usize) < l.regs,
                        "Midori normal settings register out of range"
                    );
                    system(index)?;
                }
                RnfWrite::System(index, _) => system(index)?,
                RnfWrite::Relative(index, _) => system(relative(index)?)?,
            }
        }
        for &write in writes {
            match write {
                RnfWrite::LocalFlag(index, on) => {
                    let value = self.sys.entry(flag_slot).or_default();
                    let mask = 1_i32 << index;
                    if on {
                        *value |= mask;
                    } else {
                        *value &= !mask;
                    }
                }
                RnfWrite::NormalAndSystem(index, value) => {
                    // Native action250's R case falls through to S, reading
                    // the same literal twice. LocalHolder overrides survive.
                    self.normal.insert(index, value);
                    self.sys.insert(index, value);
                }
                RnfWrite::System(index, value) => {
                    self.sys.insert(index, value);
                }
                RnfWrite::Relative(index, value) => {
                    self.sys.insert(base + index, value);
                }
            }
        }
        Ok(())
    }

    pub fn read(&self, v: Value) -> Result<i32> {
        let l = self
            .limits
            .context("missing ExHibit register configuration")?;
        Ok(match v {
            Value::Immediate(n) => n,
            Value::Local(i) => self
                .local
                .get(&i)
                .or_else(|| self.normal.get(&Self::index(i, l.regs)))
                .copied()
                .unwrap_or(0),
            Value::System(i) => self.sys.get(&Self::index(i, l.sys)).copied().unwrap_or(0),
            Value::RelativeSystem(i) => self.read(Value::System(self.relative(i, l)))?,
            Value::Flag(i) => {
                i32::from(self.read(Value::RelativeSystem(1_000_004))? & (1i32 << (i & 31)) != 0)
            }
            Value::Seen(bank, i) => {
                i32::from(self.seen[bank as usize].get(&i).copied().unwrap_or(0) > 0)
            }
            Value::TextNumber(i) => {
                let raw = self.text(i)?.trim_ascii_start();
                let sign = usize::from(matches!(raw.first(), Some(b'-' | b'+')));
                let n = raw[sign..]
                    .iter()
                    .take_while(|c| c.is_ascii_digit())
                    .count();
                if n == 0 { 0 } else { number(&raw[..sign + n])? }
            }
        })
    }
    pub fn change(&mut self, c: Change) -> Result<()> {
        let l = self
            .limits
            .context("missing ExHibit register configuration")?;
        match c {
            Change::Action2011 { operation, result } => {
                match operation {
                    Action2011::SetLocalFlag7 => {
                        let slot = self.read(Value::System(15))?.wrapping_add(4);
                        let flags = self.read(Value::System(slot))? | 0x80;
                        self.store(Value::System(slot), flags, l);
                        if slot == 3 {
                            self.local_flag = true;
                        }
                    }
                    Action2011::Text208And209(mode, bytes) => {
                        ensure!(
                            208 < l.strings && (!matches!(mode, 0 | 1) || 209 < l.strings),
                            "missing native action text registers"
                        );
                        if matches!(mode, 0 | 1) {
                            let old = self.read(Value::System(23))?;
                            self.store(
                                Value::System(23),
                                if mode == 0 { old & !32 } else { old | 32 },
                                l,
                            );
                            if mode == 0 {
                                self.strings.remove(&209);
                            } else {
                                self.strings.insert(209, bytes);
                            }
                        }
                        self.strings.remove(&208);
                    }
                    Action2011::Text206(bytes) => {
                        ensure!(206 < l.strings, "missing native text register 206");
                        self.strings.insert(206, bytes);
                    }
                    Action2011::SetBits23(value) => {
                        let old = self.read(Value::System(23))?;
                        self.store(Value::System(23), (old & !0x700) | ((value & 7) << 8), l);
                    }
                    Action2011::ClearRange { bank, first, last } => {
                        ensure!(
                            i64::from(last) - i64::from(first) < 1_000_000,
                            "native register range exceeds budget"
                        );
                        if bank == 1 {
                            ensure!(
                                first >= 0 && (last as usize) < l.strings,
                                "invalid native text register range"
                            );
                        }
                        for i in first..=last {
                            match bank {
                                0 => {
                                    self.normal.remove(&Self::index(i, l.regs));
                                }
                                1 => {
                                    self.strings.remove(&i);
                                }
                                2 => {
                                    let base = self.read(Value::System(15))?;
                                    self.store(Value::System(base.wrapping_add(i)), 0, l);
                                }
                                _ => break,
                            }
                        }
                    }
                }
                if result >= 0 {
                    self.store(Value::Local(result), 0, l);
                }
            }
            Change::Arithmetic {
                to,
                op,
                left,
                right,
            } => {
                let a = self.read(left)?;
                let b = self.read(right)?;
                let n = match op {
                    0 => a.wrapping_add(b),
                    1 => a.wrapping_sub(b),
                    2 => a.wrapping_mul(b),
                    3 | 4 if b == 0 => 0,
                    3 => a
                        .checked_div(b)
                        .context("ExHibit signed division overflow")?,
                    4 => a
                        .checked_rem(b)
                        .context("ExHibit signed remainder overflow")?,
                    _ => 0,
                };
                self.store(to, n, l);
            }
            Change::Assign(to, op, rhs, when) => {
                if when.matches(self)? {
                    let a = self.read(to)?;
                    let b = self.read(rhs)?;
                    let n = match op {
                        0 => b,
                        1 => a.wrapping_add(b),
                        2 => a.wrapping_mul(b),
                        3 => a
                            .checked_div(b)
                            .context("invalid native register division")?,
                        4 if b > 0 => a % b,
                        5 => a & b,
                        6 => a | b,
                        7 => a ^ b,
                        10 => 0,
                        _ => bail!("invalid native assignment operation {op}"),
                    };
                    self.store(to, n, l);
                }
            }
            Change::TextAssign {
                dest,
                mode,
                source,
                replacement,
                when,
            } => {
                if !when.matches(self)? {
                    return Ok(());
                }
                let old = self.text(dest)?;
                let rhs = match source {
                    TextValue::Literal(bytes) => bytes,
                    TextValue::Global(i) => self
                        .normal
                        .get(&Self::index(i, l.regs))
                        .copied()
                        .unwrap_or(0)
                        .to_string()
                        .into_bytes(),
                    TextValue::System(i) => self.read(Value::System(i))?.to_string().into_bytes(),
                    TextValue::Text(i) => self.text(i)?.to_vec(),
                };
                let mut bytes = Vec::new();
                let mut append = |s: &[u8]| -> Result<()> {
                    ensure!(
                        s.len() <= (8usize << 20) - bytes.len(),
                        "ExHibit text register exceeds budget"
                    );
                    bytes.try_reserve(s.len())?;
                    bytes.extend_from_slice(s);
                    Ok(())
                };
                match mode {
                    0 => append(&rhs)?,
                    1 => {
                        append(old)?;
                        append(&rhs)?;
                    }
                    2 => {
                        append(&rhs)?;
                        append(old)?;
                    }
                    3 => {
                        ensure!(
                            !rhs.is_empty(),
                            "unsupported empty native replacement pattern"
                        );
                        let mut start = 0;
                        let mut i = 0;
                        while i < old.len() {
                            if old[i..].starts_with(&rhs) {
                                append(&old[start..i])?;
                                append(&replacement)?;
                                i += rhs.len();
                                start = i;
                            } else {
                                // CharNextA advances past the trail byte in the original CP932 string.
                                i += if matches!(old[i],0x81..=0x9f|0xe0..=0xfc)
                                    && i + 1 < old.len()
                                {
                                    2
                                } else {
                                    1
                                };
                            }
                        }
                        append(&old[start..])?;
                    }
                    10 => {}
                    _ => bail!("unsupported native text operation {mode}"),
                }
                self.strings.insert(dest, bytes);
            }
            Change::Experience(bank, ids) => {
                let capacity = match bank {
                    Bank::Cg => l.cg,
                    Bank::Sound => l.sound,
                    Bank::Scene => l.scene,
                };
                for id in ids {
                    self.increment_experience(bank, id, capacity);
                }
            }
            Change::MidoriDefineDb(definition) => {
                // This map belongs to RetouchSystem, not SaveDataControl.
                self.integer_tables[definition.table] = definition.rows;
            }
            Change::MidoriGetDb(lookup) => {
                let key = self.read(lookup.key)?;
                if let Some(row) = self.integer_tables[lookup.table].get(&key).copied() {
                    for (col, register) in lookup.outputs {
                        let value = row.get(col as usize).copied().unwrap_or(0);
                        self.store(Value::Local(register), value, l);
                    }
                }
            }
            Change::MidoriSceneDone(SceneDone(id)) => {
                self.increment_experience(Bank::Scene, id, l.scene);
                self.mark_message_in(id, l.messages);
            }
            Change::LocalFlag(i, set) => {
                if (0..32).contains(&i) {
                    let base = self.read(Value::System(15))?.wrapping_add(4);
                    let reg = self.sys.entry(Self::index(base, l.sys)).or_default();
                    if set {
                        *reg |= 1 << i
                    } else {
                        *reg &= !(1 << i)
                    }
                }
                self.local_flag = set;
            }
        }
        Ok(())
    }
}

pub(super) fn syscall(raw: &[u8]) -> Result<crate::Action> {
    // cmdExHIBIT splits the first three numeric fields, then gives the rest
    // of the comma-delimited line to the selected native operation.
    let mut fields = raw.splitn(4, |b| *b == b',');
    let mut number = || -> Result<i32> {
        let bytes = fields.next().context("missing ExHibit syscall argument")?;
        Ok(std::str::from_utf8(bytes)?.trim().parse()?)
    };
    let code = number()?;
    let _result_register = number()?;
    let count = number()?;
    let tail = fields.next().unwrap_or_default();
    if (code, count) == (26, 4) {
        return Ok(crate::Action::Billboard(
            crate::billboard::Billboard::parse(tail)?,
        ));
    }
    let mut fields = tail.split(|b| *b == b',');
    let mut number =
        || super::branch::number(fields.next().context("missing ExHibit syscall argument")?);
    let change = match (code, count) {
        (27, 0) => return Ok(crate::Action::CancelSkip),
        (28, 0) => return Ok(crate::Action::CancelAuto),
        (29, 2) => return Ok(crate::Action::DisableButton(number()?, number()?)),
        (37, 1) => return Ok(crate::Action::QuerySlots(number()?)),
        (33, 1) => Change::Experience(Bank::Cg, vec![number()?]),
        (38, 2) => Change::LocalFlag(number()?, number()? != 0),
        (39, 3) => {
            let kind = number()?;
            let _declared = number()?;
            let list = fields.next().context("missing experience IDs")?;
            let bank = match kind {
                0 => Bank::Cg,
                1 => Bank::Sound,
                2 => Bank::Scene,
                _ => bail!("unknown experience kind {kind}"),
            };
            let ids = list
                .split(|b| *b == b'.')
                .filter(|s| !s.is_empty())
                .map(crate::branch::number)
                .collect::<Result<Vec<_>>>()?;
            Change::Experience(bank, ids)
        }
        _ => bail!("unsupported ExHibit native call {code} ({count} arguments)"),
    };
    Ok(crate::Action::Change(change))
}

pub(super) fn assignment(op: u16, raw: &[u8]) -> Result<Change> {
    assignment_for_abi(op, raw, 1)
}
pub(super) fn assignment_for_abi(op: u16, raw: &[u8], abi: u32) -> Result<Change> {
    let f: Vec<_> = raw.splitn(6, |b| *b == b',').collect();
    ensure!(f.len() == 6, "invalid ExHibit register assignment");
    let index = number(f[1])?;
    let mode = number(f[2])?;
    let text = f[3];
    let condition = Condition::for_abi(if f[4].is_empty() { b"*" } else { f[4] }, abi)?;
    if op == 10 {
        ensure!(
            (0..=3).contains(&mode) || (abi == 3 && mode == 10),
            "unsupported native text mapping {mode}"
        );
        let source = match text.split_first() {
            Some((h @ (b'R' | b'S' | b'T'), tail)) if number(tail).is_ok_and(|i| i > 0) => {
                let i = number(tail)?;
                match h {
                    b'R' => TextValue::Global(i),
                    b'S' => TextValue::System(i),
                    _ => TextValue::Text(i),
                }
            }
            Some((b'"', tail)) if abi == 3 => {
                let end = tail
                    .iter()
                    .position(|&b| b == b'"')
                    .context("unclosed quoted text register literal")?;
                TextValue::Literal(tail[..end].to_vec())
            }
            _ => TextValue::Literal(text.to_vec()),
        };
        return Ok(Change::TextAssign {
            dest: index,
            mode,
            source,
            replacement: f[5].to_vec(),
            when: condition,
        });
    }
    let to = if op == 6 {
        Value::Local(index)
    } else {
        Value::RelativeSystem(index)
    };
    ensure!(
        (0..=4).contains(&mode) || (abi == 3 && matches!(mode, 5..=7 | 10)),
        "unsupported native assignment flags {mode:#x}"
    );
    if abi == 3 {
        return Ok(Change::Assign(
            to,
            mode,
            Value::expression(text)?,
            condition,
        ));
    }
    let rhs = match text.split_first() {
        Some((b'R', id)) => Value::Local(number(id)?),
        Some((b'S', id)) => Value::System(number(id)?),
        Some((b'T', id)) => Value::TextNumber(number(id)?),
        _ => Value::Immediate(number(text)?),
    };
    Ok(Change::Assign(to, mode, rhs, condition))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn action_calls_preserve_raw_text_result_and_unrelated_flag_bits() -> Result<()> {
        let mut regs = Registers::new(Some(Limits {
            regs: 2048,
            sys: 128,
            cg: 1,
            sound: 1,
            scene: 1,
            strings: 256,
            characters: 0,
            locals: 20,
            messages: 64,
        }))?;
        regs.change(action2011(b"262,4,0,0,0,0,\x83\x65,second")?)?;
        assert_eq!(regs.text(206)?, b"\x83\x65,second");
        assert_eq!(regs.read(Value::Local(4))?, 0);
        regs.change(assignment(8, b"1,23,0,61712,*,")?)?;
        regs.change(action2011(b"348,-1,3,0,0,0,*")?)?;
        assert_eq!(regs.read(Value::System(23))?, (61712 & !0x700) | 0x300);
        regs.change(action2011(b"190,-1,0,0,0,0,*")?)?;
        assert_eq!(regs.read(Value::Flag(7))?, 1);
        regs.change(action2011(b"360,-1,1,0,0,0,voice")?)?;
        assert_eq!(regs.text(209)?, b"voice");
        assert_ne!(regs.read(Value::System(23))? & 32, 0);
        regs.change(action2011(b"360,-1,0,0,0,0,*")?)?;
        assert!(regs.text(209)?.is_empty());
        assert_eq!(regs.read(Value::System(23))? & 32, 0);
        regs.change(action2011(b"346,-1,1,207,205,0,*")?)?;
        assert!(regs.text(206)?.is_empty());
        Ok(())
    }

    #[test]
    fn scoped_assignments_and_native_experience_lists_drive_branches() -> Result<()> {
        let mut regs = Registers::new(Some(Limits {
            regs: 32,
            sys: 64,
            cg: 8,
            sound: 16,
            scene: 6,
            strings: 4,
            characters: 0,
            locals: 20,
            messages: 64,
        }))?;
        regs.change(assignment(6, b"0,3,0,7,*,")?)?;
        regs.change(assignment(6, b"0,3,2,3,*,")?)?;
        regs.change(assignment(8, b"1,15,0,20,*,")?)?;
        regs.change(assignment(8, b"1,1000008,0,R3,*,")?)?;
        assert_eq!(regs.read(Value::System(28))?, 21);
        let crate::Action::Change(flag) = syscall(b"38,-1,2,5,1")? else {
            panic!("flag handler")
        };
        regs.change(flag)?;
        assert_eq!(regs.read(Value::System(24))?, 32);
        let crate::Action::Change(history) = syscall(b"39,-1,3,2,3,1.1.9")? else {
            panic!("experience handler")
        };
        regs.change(history)?;
        assert_eq!(regs.seen[Bank::Scene as usize].get(&1), Some(&2));
        assert_eq!(regs.read(Value::Seen(Bank::Scene, 9))?, 0);
        assert!(Condition::text(b"F5\x81\x811&MS1\x81\x811;R3\x81\x810")?.matches(&regs)?);
        assert!(!Condition::text(b"R3\x81\x831;MS9\x81\x811")?.matches(&regs)?);
        Ok(())
    }
}
#[test]
fn midori_literal_scene_done_updates_two_banks_independently() -> Result<()> {
    let mut bytes = b"\0DLR".to_vec();
    for n in [3u32, 12, 5] {
        bytes.extend(n.to_le_bytes());
    }
    for text in [
        b"2,1,3\0".as_slice(),
        b"2,1,3\0",
        b"2,1,9\0",
        b"2,1,65\0",
        b"2,1,-1\0",
    ] {
        bytes.extend([0xbd, 0, 0, 1]);
        bytes.extend(text);
    }
    let mut script = crate::Script::new(bytes, 3)?;
    let mut regs = crate::native_state_tests::fresh(3)?;
    for _ in 0..5 {
        let crate::Action::Change(c) = script.next()? else {
            bail!("expected Midori change");
        };
        regs.change(c)?;
    }
    assert_eq!(regs.seen[Bank::Scene as usize], BTreeMap::from([(3, 2)]));
    assert_eq!(regs.read(Value::Seen(Bank::Scene, 3))?, 1);
    // Out of range for the scene bank is still in range for messages.
    assert_eq!(regs.messages, BTreeMap::from([(0, 0x4010)]));
    assert!(regs.seen[Bank::Cg as usize].is_empty());
    assert!(regs.seen[Bank::Sound as usize].is_empty());
    regs.seen[Bank::Scene as usize].insert(3, 0x7fff_ff00);
    regs.change(Change::MidoriSceneDone(SceneDone(3)))?;
    assert_eq!(regs.seen[Bank::Scene as usize][&3], 0x7fff_ff00);
    assert_eq!(regs.messages[&0], 0x4010);
    Ok(())
}
