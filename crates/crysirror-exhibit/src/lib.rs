// SPDX-License-Identifier: MPL-2.0
//! ExHibit loose RLD scenarios. The host owns rendering, time and file access.
#![forbid(unsafe_code)]
mod billboard;
mod branch;
mod character;
mod character_body;
mod clip;
mod environment;
mod message_area;
mod local_controls;
mod later_skip;
mod later_dispatch;
mod later_flow;
mod later_string_db;
mod later_screen;
mod late_numeric;
mod midori_state;
mod midori_transfer;
mod midori_music;
mod midori_sound_capture;
mod midori_status_capture;
mod midori_checkpoint;
mod midori_ambient;
mod midori_actor;
mod midori_actor_image;
mod midori_color_filter;
mod midori_background;
mod midori_message;
mod midori_positions;
mod midori_prelude;
mod midori_control_parts;
mod midori_volume;
mod midori_control_sound;
mod midori_shortcuts;
mod movie;
mod opening_dlt;
mod panel_animation;
mod printer;
mod profile;
mod registers;
mod scene_fade;
mod select_items;
mod slot_index;
mod sound;
mod surfaces;
mod voice;
mod v1_dialog;
mod v1_precache;
mod v1_printer_page;
mod v1_image_billboard;
mod v1_save;
mod v1_scaled_draw;
mod v1_services;
mod v3_control;
mod native_numbers;
mod view_definitions;
#[cfg(test)]
mod v1_census;
#[cfg(test)]
mod v3_census;
#[cfg(test)]
mod scenario_load_tests;
#[cfg(test)]
mod v1_media_tests;
use anyhow::{Context, Result, bail, ensure};
use branch::{Condition, Position};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_tasks::{Task, TaskPool};
use registers::{Change, Registers, Value};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

pub fn script_path(name: &str) -> Result<VfsPath> {
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')),
        "invalid ExHibit script name"
    );
    Ok(VfsPath::new(Mount::Asset, &format!("rld/{name}.rld"))?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    entry: String,
    #[serde(default)]
    native_bootstrap: bool,
    abi: u32,
    #[serde(default)]
    sources: Option<profile::Sources>,
    #[serde(default)]
    limits: Option<registers::Limits>,
    #[serde(default = "slot_index::default_counts")]
    slots: [usize; 3],
    #[serde(default)]
    save_class: Option<String>,
    #[serde(default)]
    native_windows_version: Option<[u8; 3]>,
    canvas: [u32; 2],
}

struct Script {
    bytes: Vec<u8>,
    pc: usize,
    remaining: u32,
    abi: u32,
    resident: Option<profile::Resident>,
    start: Position,
    labels: Option<BTreeMap<i32, Position>>,
    returns: Vec<(u32, i32)>,
    record_offsets: Vec<u32>,
    block: i32,
}

fn word(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("truncated RLD word")?
            .try_into()?,
    ))
}
impl Script {
    fn new(bytes: Vec<u8>, expected: u32) -> Result<Self> {
        ensure!(bytes.starts_with(b"\0DLR"), "invalid RLD signature");
        let version = word(&bytes, 4)?;
        ensure!(
            version == expected && matches!(version, 1 | 3),
            "unsupported ExHibit RLD version {version}"
        );
        let pc = (word(&bytes, 8)? as usize)
            .checked_add(if version == 3 { 4 } else { 0 })
            .context("RLD code offset overflow")?;
        let remaining = word(&bytes, 12)?;
        ensure!(
            pc >= 16 && pc <= bytes.len() && remaining as usize <= (bytes.len() - pc) / 4,
            "invalid RLD code bounds"
        );
        Ok(Self {
            bytes,
            pc,
            remaining,
            abi: expected,
            resident: (expected == 3).then_some(profile::Resident::Midori2011),
            start: Position { pc, remaining },
            labels: None,
            returns: vec![],
            record_offsets: vec![u32::try_from(pc)?],
            block: 0,
        })
    }
    fn seek_record(&mut self, index: u32) -> Result<()> {
        // liteExec keeps indices, even when cmdExHIBIT replaces the CMD_DATA array.
        // Native indices at or beyond the end leave the interpreter loop.
        let index = index.min(self.start.remaining);
        while self.record_offsets.len() <= index as usize {
            let last = *self.record_offsets.last().context("missing RLD start")? as usize;
            let (_, _, next) = branch::envelope(&self.bytes, last)?;
            self.record_offsets.push(u32::try_from(next)?);
        }
        self.pc = self.record_offsets[index as usize] as usize;
        self.remaining = self.start.remaining - index;
        Ok(())
    }

    fn branch(&mut self, op: u16, target: i32) -> Result<()> {
        if op == 0x17 {
            let (frame, block) = self.returns.pop().context("RLD return without call")?;
            self.seek_record(frame)?;
            self.block = block;
            return Ok(());
        }
        if self.labels.is_none() {
            self.labels = Some(if self.resident.is_some_and(profile::Resident::late) {
                later_flow::labels(&self.bytes, self.start)?
            } else {
                branch::labels(&self.bytes, self.start, self.abi)?
            });
        }
        let to = self
            .labels
            .as_ref()
            .and_then(|l| l.get(&target))
            .with_context(|| format!("unknown RLD block {target}"))?;
        if op == 0x16 {
            ensure!(self.returns.len() < 65536, "RLD call stack limit");
            self.returns.push((self.start.remaining - self.remaining, self.block));
        }
        (self.pc, self.remaining) = (to.pc, to.remaining);
        Ok(())
    }
    fn visit_block(
        &mut self,
        block: branch::Block,
        registers: &mut Registers,
        checkpoint_flag: &mut bool,
        draw: &[i32; 19],
        audio: &[i32; 12],
        views: [bool; 2],
    ) -> Result<()> {
        if block.condition.matches(registers)? {
            if !block.marker {
                ensure!(
                    self.abi == 1,
                    "unsupported ExHibit v{} block {} camera, layer and printer checkpoint (opcode 0x05)",
                    self.abi,
                    block.id
                );
                let id = if block.id == -1 {
                    self.block.wrapping_add(1)
                } else {
                    block.id
                };
                registers.checkpoint_v1(id, &block.title, draw, audio, views)?;
                self.block = id;
            }
        } else {
            let next_block = if block.id == -1 {
                self.block.wrapping_add(1)
            } else {
                block.id
            };
            let implicit = block.false_target == -1;
            let target = if implicit {
                (if block.marker { self.block } else { next_block }).wrapping_add(1)
            } else {
                block.false_target
            };
            self.branch(0x14, target)?;
            if implicit && !block.marker {
                self.block = next_block;
            }
        }
        if let Some(value) = block.checkpoint_flag {
            *checkpoint_flag = value;
        }
        Ok(())
    }
    fn next(&mut self) -> Result<Action> {
        ensure!(
            self.remaining > 0,
            "RLD reached end without a control transfer"
        );
        let mut at = self.pc;
        let header = word(&self.bytes, at)?;
        at += 4;
        let count = ((header >> 16) & 255) as usize;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(word(&self.bytes, at)? as i32);
            at += 4;
        }
        let op = header as u16;
        let flags = (header >> 24) as u8;
        if let Some(late) = self.resident.filter(|profile| profile.late()) {
            if let Some((action, end)) = later_dispatch::check(late, &self.bytes, self.pc, at, header)? {
                self.pc = end;
                self.remaining -= 1;
                return Ok(action);
            }
        } else if self.abi == 3 && later_dispatch::empty_native_method(profile::Resident::Midori2011, op) {
            let (_, _, end) = branch::envelope(&self.bytes, self.pc)?;
            self.pc = end;
            self.remaining -= 1;
            return Ok(Action::NativeMethodReturn);
        }
        ensure!(
            self.abi == 1
                || matches!(
                    op,
                    0x05..=0x08
                        | 0x0a
                        | 0x0b
                        | 0x0d
                        | 0x11
                        | 0x12
                        | 0x14
                        | 0x16
                        | 0x17
                        | 0x18
                        | 0x29..=0x2f
                        | 0x30..=0x35 | 0xf9..=0xfd
                        | 0x38..=0x3a
                        | 0x3d..=0x3f
                        | 0x40 | 0x42 | 0x46 | 0x47
                        | 0x48
                        | 0x49
                        | 0x4a
                        | 0x50
                        | 0x6c | 0x6d
                        | 0x8c
                        | 0xa7
                        | 0xbd | 0xbe | 0xd0..=0xd2 | 0xea
                        | 0xbf
                ),
            "unsupported ExHibit v{} execution opcode {op:#06x}",
            self.abi
        );
        let action = match (op, flags, values.as_slice()) {
            (0xf9 | 0xfa, _, _) if self.abi == 3 => {
                for _ in 0..flags & 15 {
                    at = cstring(&self.bytes, at)?.1;
                }
                Action::SkipBlock(op == 0xf9)
            }
            (0x0c, 1, []) if self.abi == 1 => {
                let (memo, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::V1AutoSave(memo.to_vec())
            }
            (0x05, 1, [id, marker]) => {
                let (title, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Block(branch::Block::binary(*id, *marker, title)?)
            }
            (0x05, 0x11 | 0x21, []) if flags == 0x11 || self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Block(branch::Block::csv(raw, self.abi, flags == 0x21)?)
            }
            (0x16, 0x11, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                let fields: Vec<_> = raw.splitn(3, |b| *b == b',').collect();
                ensure!(
                    fields.len() == 3 && fields[2] == b"*",
                    "unsupported ExHibit call with register assignments"
                );
                at = end;
                Action::Branch(
                    op,
                    branch::number(fields[0])?,
                    Condition::for_abi(fields[1], 3)?,
                )
            }
            (0x17, 0x11, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                let split = raw
                    .iter()
                    .position(|&b| b == b',')
                    .context("missing return condition")?;
                let when = if branch::number(&raw[..split])? & 1 == 0 {
                    Condition::text(b"*")?
                } else {
                    // cmdReturn calls the engine predicate directly in the conditional form.
                    Condition::for_abi(&raw[split + 1..], 3)?
                };
                at = end;
                Action::Branch(op, 0, when)
            }
            (0x46, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineMidoriAmbient(midori_ambient::Definition::parse(raw)?)
            }
            (0x32, 0x31, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::MidoriMusic(midori_music::Track::parse(raw)?)
            }
            (0x33, 0, [ms, wait]) if self.abi == 3 => {
                Action::StopMidoriMusic(midori_music::stop_duration(*ms, *wait)?)
            }
            (0x34 | 0x35, _, args) if self.abi == 3 => {
                Action::MidoriAmbient(midori_ambient::Command::parse(op, flags, args)?)
            }
            (0xa7, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::MidoriColorFilter(midori_color_filter::ColorFilter::deferred(raw)?)
            }
            (0xfc | 0xfd, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                if op == 0xfc {
                    Action::DrawMidoriActor(midori_actor::Draw::parse(raw)?)
                } else {
                    Action::ClearMidoriActor(midori_actor::clear_slot(raw)?)
                }
            }
            (0xfb, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                let resident = self.resident.unwrap_or(profile::Resident::Midori2011);
                Action::DefineMidoriActorSlot(midori_actor::Definition::parse_profile(raw, resident)?)
            }
            (0x46, 1, []) => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Environment(environment::Definition::parse(raw)?)
            }
            (0xbe | 0xea, _, _) if self.resident == Some(profile::Resident::Midori2011) => {
                let (_, first, end) = branch::envelope(&self.bytes, self.pc)?;
                at = end;
                let raw = first.unwrap_or_default();
                let command = if op == 0xbe {
                    midori_control_sound::Command::basic(raw)
                } else {
                    midori_control_sound::Command::extended(raw)
                };
                Action::BindMidoriControlSound(command)
            }
            (0xbf, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                if raw.starts_with(b"338,") {
                    Action::MidoriControlSounds(midori_control_parts::SoundUpdate::parse(raw)?)
                } else if self.resident == Some(profile::Resident::Midori2011)
                    && let Some(command) = midori_volume::Command::parse(raw)?
                {
                    Action::MidoriChannelVolume(command)
                } else {
                    Action::Change(registers::action2011(raw)?)
                }
            }
            (0x32..=0x3a, _, args) if self.abi == 1 => {
                Action::Sound(sound::decode(op, flags, args)?)
            }
            (0x38..=0x3a, _, args) if self.abi == 3 => {
                Action::MidoriEffect(sound::effect2011(op, flags, args)?)
            }
            (0x49, 0x91, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineMidoriArea(midori_message::AreaDefinition::parse(raw)?)
            }
            (0x47, 0x21, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineDebug(midori_prelude::DebugDefinition::parse(raw)?)
            }
            (0x48, 0x31, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                ensure!(raw == b"L9,0,-1", "unsupported ExHibit 2011 printer frame expression");
                at = end;
                Action::SelectMidoriFrame
            }
            (0x12, 0, [open, duration]) if self.abi == 3 => {
                Action::ChangeMidoriVisibility(midori_message::Visibility::parse(*open, *duration)?)
            }
            (0x06 | 0x08 | 0x0a, 0x11, []) => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Change(registers::assignment_for_abi(op, raw, self.abi)?)
            }
            (0x2e, format, []) if self.abi == 3 && format & 15 == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineSelect2011(format >> 4, raw.to_vec())
            }
            (0x29 | 0x2a | 0x2b | 0x2e, format, []) if format & 15 == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                let kind = match op {
                    0x29 => view_definitions::Definition::Save,
                    0x2a => view_definitions::Definition::Load,
                    0x2b => view_definitions::Definition::Log,
                    _ => view_definitions::Definition::Selection,
                };
                Action::DefineViews(
                    kind,
                    if self.abi == 1 {
                        format & 0x70 != 0
                    } else {
                        format >> 4 != 0
                    },
                    raw.to_vec(),
                )
            }
            (0x2f, 0x10, [_, _]) if self.abi == 3 => Action::Label,
            (0x2f, 0, [_]) => Action::Label,
            (0x14, 0x11, [id]) => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Branch(op, *id, Condition::for_abi(raw, self.abi)?)
            }
            (0x14 | 0x16 | 0x17, 5, values) if values.len() == 16 => {
                let mut strings = Vec::with_capacity(5);
                for _ in 0..5 {
                    let (raw, end) = cstring(&self.bytes, at)?;
                    at = end;
                    strings.push(raw);
                }
                Action::Branch(op, values[0], Condition::binary(values, &strings)?)
            }
            (0x07, 0, [dk, d, op, ak, a, bk, b]) => Action::Change(Change::Arithmetic {
                to: Value::from_pair(*dk, *d),
                op: *op,
                left: Value::from_pair(*ak, *a),
                right: Value::from_pair(*bk, *b),
            }),
            (0x0b, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                let mut fields = raw.splitn(4, |&b| b == b',');
                let code = native_numbers::integer2011(fields.next().unwrap_or_default());
                let _result = fields.next();
                let count = native_numbers::integer2011(fields.next().unwrap_or_default());
                let tail = fields.next().unwrap_or_default();
                if matches!(code, 23 | 44) {
                    Action::Change(registers::service_flag2011(i32::from(code == 44), native_numbers::integer2011(tail) != 0))
                } else if code == 30 {
                    Action::MidoriEntryName(midori_state::EntryAssignment::parse(count, tail)?)
                } else if code == 33 {
                    let args = v1_services::args(tail, count, false)?;
                    Action::Change(Change::Experience(registers::Bank::Cg, vec![native_numbers::integer2011(args[0])]))
                } else if code == 39 {
                    Action::Change(Change::MidoriDone(registers::DoneFlags::service(count, tail)?))
                } else if code == 47 {
                    let args = v1_services::args(tail, count, false)?;
                    let source = if native_numbers::integer2011(args[0]) == 0 {
                        registers::TextValue::Literal(args[1].to_vec())
                    } else {
                        registers::TextValue::MidoriText(native_numbers::integer2011(args[1]))
                    };
                    Action::Change(Change::TextAssign {
                        dest: 1, mode: 0, source, replacement: Vec::new(),
                        when: Condition::text(b"*")?,
                    })
                } else if code == 48 {
                    let value = native_numbers::integer2011(tail.split(|&b| b == b',').next().unwrap_or_default());
                    Action::Change(Change::Assign(Value::RelativeSystem(1_000_005), 0, Value::Immediate(value).into(), Condition::text(b"*")?))
                } else if code == 36 {
                    let mut args = tail.split(|&b| b == b',');
                    Action::PlaceLoadPage(std::array::from_fn(|_| {
                        native_numbers::integer2011(args.next().unwrap_or_default())
                    }))
                } else if raw.starts_with(b"60,") {
                    Action::DefineMidoriMarker(midori_prelude::MarkerDefinition::parse(raw)?)
                } else if raw.starts_with(b"57,") {
                    Action::DefineMidoriControlPart(midori_control_parts::Part::parse(raw)?)
                } else {
                    bail!("unsupported ExHibit 2011 service {code} ({count} arguments)")
                }
            }
            (0x40, flags, []) if self.abi == 3 && flags & 15 == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                let resident = self.resident.unwrap_or(profile::Resident::Midori2011);
                v3_control::Definition::parse(resident, flags, raw)?
                    .map_or(Action::NativeMethodReturn, |definition| Action::DefineV3Control(Box::new(definition)))
            }
            (0x42, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                let resident = self.resident.context("missing ExHibit v3 dialog profile")?;
                Action::DefineV3Dialog(v1_dialog::DialogV3::parse(raw, resident)?)
            }
            (0x42, _, _) if self.abi == 1 => {
                let (_, first, end) = branch::envelope(&self.bytes, self.pc)?;
                at = end;
                Action::DefineV1Dialog(v1_dialog::Dialog::parse(first.unwrap_or_default())?)
            }
            (0x2d, _, _) if self.abi == 1 || (flags == 1 && count == 1) => {
                let (_, first, end) = branch::envelope(&self.bytes, self.pc)?;
                at = end;
                Action::PreCache(v1_precache::path(first.unwrap_or_default()), values.first().copied().unwrap_or(0) as u32)
            }
            (0x0b, _, _) if self.abi == 1 => {
                let (_, first, end) = branch::envelope(&self.bytes, self.pc)?;
                at = end;
                registers::syscall(first.unwrap_or_default())?
            }
            (0x3d, 1, [table]) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Change(Change::MidoriDefineDb(midori_state::DbDefinition::parse(
                    *table, raw,
                )?))
            }
            (0xd0, 1, [table]) if self.resident == Some(profile::Resident::Midori2011) => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Change(Change::MidoriStringDb(registers::MidoriStringDb::create(*table, raw)?))
            }
            (0xd1 | 0xd2, 1, []) if self.resident == Some(profile::Resident::Midori2011) => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Change(Change::MidoriStringDb(registers::MidoriStringDb::access(raw, op == 0xd2)?))
            }
            (0x3e | 0x3f | 0xbd, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Change(match op {
                    0x3e => Change::MidoriGetDb(midori_state::DbLookup::parse(raw)?),
                    0x3f => Change::MidoriSetDb(midori_state::DbWrite::parse(raw)?),
                    _ => Change::MidoriDone(registers::DoneFlags::parse(raw)),
                })
            }
            (0x2c, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineMidoriControlParts(midori_control_parts::Parts::parse(raw)?)
            }
            (0x6d, flags, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineMidoriShortcut(midori_shortcuts::Define::parse(flags, raw)?)
            }
            (0x6c, flags, args) if self.abi == 3 => {
                Action::MidoriPosition(midori_positions::PositionCommand::read(flags,args)?)
            }
            (0x50, flags, args) if self.abi == 3 => {
                Action::PanMidoriBackground(midori_background::MoveCrop::read(flags,args)?)
            }
            (0x4a, flags, args) if self.abi == 3 => {
                Action::DrawMidoriBackground(midori_background::Draw::full_canvas(flags, args)?)
            }
            (
                0x4a,
                0,
                [id, mode @ (9 | 255), 0, ms, 0, 0, 0, x, y, w, h, -1, -1, flags, 0]
                | [id, mode @ (9 | 255), 0, ms, 0, 0, 0, x, y, w, h, -1, -1, flags, 0, 0],
            ) if (0..=600_000).contains(ms)
                && matches!(flags, 0 | 65536)
                && ((*mode == 9 && *id >= 0 && [*x, *y, *w, *h].into_iter().all(|v| v >= 0))
                    || (*id != -1 && [*x, *y, *w, *h] == [0; 4] && (*mode == 9 || *ms == 0))) =>
            {
                Action::Draw(*id, *ms as u64, [*x, *y, *w, *h], flags & 65536 != 0)
            }
            (0x8b, 0, [first, last, enable, complement])
                if self.abi == 1 && first == last && (0..127).contains(first) =>
            {
                Action::ControlInput(*first, *enable != 0, *complement != 0)
            }
            (0x2c, 1, []) if self.abi == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineLocalControls(local_controls::Definition::parse(raw)?)
            }
            (0x13, 0, [open @ (0 | 1)]) if self.abi == 1 => Action::LocalControlScope(*open != 0),
            (0x12, _, _) if self.abi == 1 => {
                let (_, _, end) = branch::envelope(&self.bytes, self.pc)?;
                at = end;
                if values.first().copied().unwrap_or(0) != 0 {
                    Action::OpenMessageBackdrop
                } else {
                    Action::CloseV1MessageBackdrop(if count == 2 { values[1] as u32 } else { 0 })
                }
            }
            (0x48, 0, [id]) if self.abi == 1 && (0..=6).contains(id) => {
                Action::MessageFrame(*id as u8)
            }
            (0x49, 1, []) if self.abi == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineMessageArea(message_area::Area::parse(raw)?)
            }
            (0x4e, 0, [1200, 0x30001, 400, 0, 83]) if self.abi == 1 => {
                Action::V1SceneFade(scene_fade::Kind::Title)
            }
            (0x4e, 0, [800, 0x801, 400, 0, 83]) if self.abi == 1 => {
                Action::V1SceneFade(scene_fade::Kind::ClosePrinter)
            }
            (0x52, 0, [1, 1, _, _, 0, _] | [1, 1, _, _, 0, _, _, _, _, _]) if self.abi == 1 => {
                Action::LeaveCompletedClip
            }
            (0x52, 0, [mode @ (0 | 1), 0, _, _, _, _]) if self.abi == 1 => {
                Action::RetireV1Clip(*mode == 0)
            }
            (0x52, 0, [0, 0, _, _, _, _, _, _, _, _] | [1, 0, _, _, _, _, _, _, _, 0])
                if self.abi == 1 =>
            {
                Action::RetireV1Clip(values[0] == 0)
            }
            (0x64 | 0x69, 1, []) if self.abi == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                let kind = if op == 0x64 {
                    panel_animation::Kind::Move
                } else {
                    panel_animation::Kind::Fade
                };
                Action::PanelAnimation(panel_animation::Spec::parse(kind, raw)?)
            }
            (0x65 | 0x6a, 0, [channel, wait]) if self.abi == 1 => {
                let kind = if op == 0x65 {
                    panel_animation::Kind::Move
                } else {
                    panel_animation::Kind::Fade
                };
                Action::LeavePanelAnimation(kind, *channel, *wait != 0)
            }
            (0x5b, 0x21, []) if self.abi == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::SelectItem(select_items::Button::parse(raw)?)
            }
            (0x5c, 0x20, [group, -1, dest, -1, -1, -1, -1, first, last, 23])
                if self.abi == 1
                    && (0..8).contains(group)
                    && *dest >= 0
                    && (0..127).contains(first)
                    && (*first..127).contains(last) =>
            {
                Action::WaitForSelection(*group as u8, *dest)
            }
            (0x5d, 0, [group]) if self.abi == 1 && (0..8).contains(group) => {
                Action::ExitSelection(*group as u8)
            }
            (0x5f, 0, [slot, duration]) if self.abi == 1 && (0..127).contains(slot) => {
                Action::RemoveSurface(*slot as u8, *duration as u32)
            }
            (0x5e, 0x11 | 0x21, []) if self.abi == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                if raw.split(|&b| b == b',').next() == Some(&b"0"[..]) {
                    Action::V1CgBillboard(v1_image_billboard::Draw::parse(raw, flags == 0x21)?)
                } else {
                    Action::Surface(surfaces::Surface::image_csv(raw, flags == 0x21)?)
                }
            }
            (0x51, 0, [x, y, w, h, ms, 0, mode, 0])
                if self.abi == 1 && *ms >= 0 && (0..=3).contains(mode) =>
            {
                Action::Clip([*x, *y, *w, *h], *ms as u32, *mode)
            }
            (0x0d, 0, [ms] | [ms, 0] | [ms, 1]) if *ms >= 0 => Action::Wait(*ms as u64),
            (0x71, 1, [id, sound, -1, 0, x, y, w, h]) if *w >= 0 && *h >= 0 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                let path = if *id < 0 {
                    let name = std::str::from_utf8(raw)?.replace('\\', "/");
                    VfsPath::new(Mount::Asset, &name)?.relative().to_owned()
                } else {
                    movie::Movie::path(*id)
                };
                Action::Movie(movie::Movie {
                    cg: (*id >= 0).then_some(*id),
                    path,
                    sound: *sound,
                    rect: [*x, *y, *w, *h],
                })
            }
            (0x72, 0, [wait, fade]) => Action::LeaveMovie(*wait != 0, *fade),
            (0x76, 0, [0, id, slot, x, y, -1, -1, -2, -1])
                if self.abi == 1 && *id >= 0 && (0..127).contains(slot) => {
                Action::PrepareOpeningDlt(opening_dlt::Placement { id: *id, slot: *slot as u8, origin: [*x, *y] })
            }
            (0x77, 0, [0, level, duration, -1])
                if self.abi == 1 && (0..=16).contains(level) && *duration >= 0 => {
                Action::StartOpeningDlt(*level as u8, *duration as u32)
            }
            (0x1e, _, _) if self.abi == 1 => {
                let (_, _, end) = branch::envelope(&self.bytes, self.pc)?;
                at = end;
                Action::V1ClearPage(values.first().copied().unwrap_or(0) != 0, values.get(1).copied().unwrap_or(0) as u32)
            }
            (0x1c, 2, [character, ordinal, wait @ (0 | 1), 1, 0, 0, 0]) if self.abi == 1 => {
                let (speaker, end) = cstring(&self.bytes, at)?;
                let (body, end) = cstring(&self.bytes, end)?;
                let say = printer::OrdinarySay::decode(speaker, body)?;
                at = end;
                Action::V1Say(printer::Line {
                    character: u8::try_from(*character)?,
                    ordinal: *ordinal,
                    wait: *wait != 0,
                    say,
                })
            }
            (0x30, 1, []) if matches!(self.abi, 1 | 3) => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineCharacter(if self.abi == 3 {
                    character::Definition::parse_2011(raw)?
                } else {
                    character::Definition::parse(raw)?
                })
            }
            (0x31, 1, []) if self.abi == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineBody(character_body::Definition::parse(raw)?)
            }
            (0x31, 0x11, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineBody2011(character_body::Definition2011::parse(raw)?)
            }
            (0x75 | 0x78, 0, [channel, duration, wait])
                if self.abi == 1 && (0..=2).contains(channel) && *duration >= -1 => {
                Action::StopOpeningDlt(*channel as u8, *duration, *wait != 0)
            }
            (0x18 | 0x8c, 0, []) => Action::End,
            (0x11, flags @ (0x01 | 0x12 | 0x21), []) if self.abi == 3 => {
                let (target, end) = cstring(&self.bytes, at)?;
                at = end;
                let condition = if flags == 0x12 {
                    let (when, end) = cstring(&self.bytes, at)?;
                    at = end;
                    Some(when)
                } else {
                    None
                };
                Action::Transfer2011(midori_transfer::Command::parse(target, condition, flags == 0x21)?)
            }
            (0x11, 0x12, []) => {
                let (name, end) = cstring(&self.bytes, at)?;
                let (condition, end) = cstring(&self.bytes, end)?;
                let when = if condition.is_empty() {
                    Condition::text(b"*")?
                } else {
                    Condition::text(condition)?
                };
                let name = std::str::from_utf8(name)?;
                script_path(name)?;
                at = end;
                Action::Transfer(name.to_owned(), when)
            }
            _ => bail!(
                "unsupported ExHibit opcode {op:#06x} (flags {flags:#04x}, operands {values:?})"
            ),
        };
        self.pc = at;
        self.remaining -= 1;
        Ok(action)
    }
}
fn cstring(bytes: &[u8], at: usize) -> Result<(&[u8], usize)> {
    let tail = bytes.get(at..).context("missing RLD string")?;
    let n = tail
        .iter()
        .position(|b| *b == 0)
        .context("unterminated RLD string")?;
    Ok((&tail[..n], at + n + 1))
}
#[derive(Debug, PartialEq)]
enum Action {
    LateFlow(later_flow::Flow),
    V1Say(printer::Line),
    V1ClearPage(bool, u32),
    V1AutoSave(Vec<u8>),
    LateAssign(late_numeric::Assignment),
    LateScreenUpdate(later_screen::Command),
    NativeMethodReturn,
    SkipBlock(bool),
    Draw(i32, u64, [i32; 4], bool),
    V1SceneFade(scene_fade::Kind),
    DefineLocalControls(local_controls::Definition),
    DefineCharacter(character::Definition),
    DefineBody(character_body::Definition),
    DefineBody2011(character_body::Definition2011),
    DefineMidoriActorSlot(midori_actor::Definition),
    MidoriColorFilter(midori_color_filter::ColorFilter),
    DrawMidoriBackground(midori_background::Draw),
    PanMidoriBackground(midori_background::MoveCrop),
    MidoriPosition(midori_positions::PositionCommand),
    DrawMidoriActor(midori_actor::Draw),
    ClearMidoriActor(usize),
    DefineMidoriAmbient(midori_ambient::Definition),
    MidoriAmbient(midori_ambient::Command),
    MidoriChannelVolume(midori_volume::Command),
    MidoriMusic(midori_music::Track),
    StopMidoriMusic(u32),
    MidoriEntryName(midori_state::EntryAssignment),
    DefineDebug(midori_prelude::DebugDefinition),
    DefineMidoriMarker(midori_prelude::MarkerDefinition),
    DefineMidoriControlPart(midori_control_parts::Part),
    DefineNotification(midori_control_parts::Notification),
    DefineNotifications(Vec<midori_control_parts::Notification>),
    DefineV3Control(Box<v3_control::Definition>),
    DefineMidoriControlParts(midori_control_parts::Parts),
    DefineMidoriShortcut(midori_shortcuts::Define),
    MidoriControlSounds(midori_control_parts::SoundUpdate),
    BindMidoriControlSound(midori_control_sound::Command),
    LocalControlScope(bool),
    MessageFrame(u8),
    OpenMessageBackdrop,
    CloseV1MessageBackdrop(u32),
    DefineMessageArea(message_area::Area),
    DefineMidoriArea(midori_message::AreaDefinition),
    SelectMidoriFrame,
    ChangeMidoriVisibility(midori_message::Visibility),
    Billboard(billboard::Billboard),
    Surface(surfaces::Surface),
    SelectItem(select_items::Button),
    WaitForSelection(u8, i32),
    ExitSelection(u8),
    RemoveItems(i32, i32, i32, i32),
    RemoveSurface(u8, u32),
    V1CgBillboard(v1_image_billboard::Draw),
    DisableButton(i32, i32),
    ExperienceGate(v1_services::ExperienceGate),
    IgnoredExperienceBank,
    LoadV1Entry(usize),
    DefineV1Dialog(v1_dialog::Dialog),
    DefineV3Dialog(v1_dialog::DialogV3),
    V1GalleryMovies(movie::Gallery),
    PrepareV1Effect(i32, i32),
    PreCache(Option<VfsPath>, u32),
    EntryNameV1(v1_services::Entry),
    DefineSelect2011(u8, Vec<u8>),
    DefineViews(view_definitions::Definition, bool, Vec<u8>),
    PlaceLoadPage([i32; 3]),
    ControlInput(i32, bool, bool),
    LeaveCompletedClip,
    RetireV1Clip(bool),
    PanelAnimation(panel_animation::Spec),
    LeavePanelAnimation(panel_animation::Kind, i32, bool),
    Clip([i32; 4], u32, i32),
    Change(Change),
    CancelSkip,
    CancelAuto,
    QuerySlots(i32),
    Sound(sound::Sound),
    MidoriEffect(sound::Effect2011),
    Environment(environment::Definition),
    Wait(u64),
    Transfer(String, Condition),
    Transfer2011(midori_transfer::Command),
    Block(branch::Block),
    Branch(u16, i32, Condition),
    Label,
    Movie(movie::Movie),
    LeaveMovie(bool, i32),
    PrepareOpeningDlt(opening_dlt::Placement),
    StartOpeningDlt(u8, u32),
    StopOpeningDlt(u8, i32, bool),
    End,
}

#[derive(Default)]
struct Controls {
    block: later_skip::SkipBlock,
    skip: bool,
    auto: bool,
}
impl Controls {
    fn delay(&self, ms: u64) -> u64 {
        if self.skipping() { 0 } else { ms }
    }
    fn skipping(&self) -> bool {
        self.skip || self.block.active()
    }
}
// Keep the capability path that actually produced the accepted script. A
// queued transfer name is not yet the resident's current scenario (T2).
struct ScenarioLoad {
    system: Option<Vec<i32>>,
    script: Script,
    seeds: Option<profile::Seeds>,
    path: VfsPath,
    midori_system_path: Option<VfsPath>,
}

enum Autosave {
    Clock(v1_save::Slot, Box<dyn crysirror_core::service::CivilTimeQuery>),
    Write(Task<i32>),
}

enum V1Dialogue {
    Definitions(printer::Line, Task<character::Catalog>),
    Voice(printer::Line, serde_json::Value, [i32; 2], Option<voice::Voice>),
    Wait(printer::Wait),
}

struct Vm {
    native_definition: bool,
    client: ClientId,
    name: String,
    canvas: [u32; 2],
    script: Option<Script>,
    worker: TaskPool,
    load: Option<Task<ScenarioLoad>>,
    abi: u32,
    sources: Option<profile::Sources>,
    seeds: Option<profile::Seeds>,
    late_screen: later_screen::Updates,
    late_screen_resume: Option<usize>,
    deadline: u64,
    movie: Option<movie::Playing>,
    opening_dlt: Option<opening_dlt::OpeningDlt>,
    leaving: Option<(bool, i32)>,
    halted: bool,
    checkpoint_flag: bool,
    slots: [usize; 3],
    save_class: Option<String>,
    system_opened: bool,
    midori_system_path: Option<VfsPath>,
    midori_transfer: Option<midori_transfer::Pending>,
    autosave: Option<(usize, Autosave)>,
    slot_query: Option<(i32, Task<bool>)>,
    controls: Controls,
    clip: Option<clip::Clip>,
    magnify_target: Option<[i32; 4]>,
    panels: select_items::Panels,
    selection: Option<select_items::Selection>,
    last_selected_item: Option<u8>,
    animations: panel_animation::Animations,
    billboard_shown: bool,
    billboard_fade: Option<billboard::Fade>,
    draw_scene: [i32; 19],
    v1_scaled_draw: Option<v1_scaled_draw::Pending>,
    v1_scaled_image: Option<String>,
    scene_fade: Option<scene_fade::SceneFade>,
    message_areas: message_area::Catalog,
    selected_message_area: Option<u8>,
    message_backdrop: message_area::Backdrop,
    local_controls: local_controls::Toolbar,
    characters: character::Catalog,
    v1_entries: v1_services::Entries,
    v1_resume: Option<v1_services::Resume>,
    v1_voice_defs: Option<character::Catalog>,
    v1_dialogue: Option<V1Dialogue>,
    v1_page_fade: Option<v1_printer_page::ClearFade>,
    v1_image_billboard: Option<v1_image_billboard::Pending>,
    v1_history: std::collections::VecDeque<printer::Line>,
    v1_gallery_movies: Option<movie::Gallery>,
    v1_precache: Option<Task<()>>,
    v1_dialogs: Vec<v1_dialog::Dialog>,
    v3_dialogs: Vec<v1_dialog::DialogV3>,
    view_definitions: view_definitions::Views,
    select_view_2011: view_definitions::SelectView2011,
    midori_entries: midori_state::EntryNames,
    midori_ambient: midori_ambient::Ambient,
    midori_music: midori_music::Music,
    midori_actor_slots: midori_actor::Slots,
    midori_background: midori_background::Background,
    midori_areas: midori_message::Catalog,
    midori_positions: Option<midori_positions::Work>,
    debug_definition: Option<midori_prelude::DebugDefinition>,
    midori_marker: midori_prelude::Marker,
    midori_control_defs: midori_control_parts::Definitions,
    midori_shortcuts: midori_shortcuts::Catalog,
    pending_message_area: Option<(u8, Task<message_area::Catalog>)>,
    registers: Registers,
    limits: Option<registers::Limits>,
    audio: sound::Audio,
}
impl Vm {
    // liteExec 0x72127: a live frame is closed, rebound, and reopened. Re-selecting
    // the active area returns before touching LocalHolder's R77 result.
    fn bind_v1_message_frame(&mut self, id: u8, h: &mut dyn HostServices) -> Result<()> {
        let opened = self.message_backdrop.opened;
        let same = self.registers.read(Value::System(14))? as u8 == id;
        if same {
            if !opened {
                self.registers.set_local(77, i32::from(id))?;
            }
            return Ok(());
        }
        if opened {
            let previous = self.selected_message_area.context("no selected ExHibit message area")?;
            self.message_areas.close(previous, &mut self.message_backdrop, h, self.client, 0)?;
        }
        self.message_backdrop.page = Some(self.message_areas.attach_inactive(id, h, self.client, &mut self.registers)?);
        self.selected_message_area = Some(id);
        if opened {
            self.message_areas.open(id, &mut self.message_backdrop, h, self.client, &self.registers, self.controls.skip)?;
        }
        Ok(())
    }

    fn prepare_say(&self, host: &mut dyn HostServices, line: printer::Line) -> Result<V1Dialogue> {
        let fonts = host.font_library(self.client)?;
        let area = self.selected_message_area.context("no selected ExHibit message area")?;
        let style = self.message_areas.plain_style(area)?;
        let (commands, cursor) = style.layout(&line.say, fonts.as_ref())?;
        if line.wait {
            ensure!(host.image_size(self.client, printer::MARKER)? == [140, 28],
                "unsupported ExHibit hit-wait atlas");
        }
        let voice = if line.wait {
            let characters = if line.character < 3 || self.characters.has_definition(line.character) {
                &self.characters
            } else {
                self.v1_voice_defs.as_ref().context("missing ExHibit voice definitions")?
            };
            voice::Voice::begin_character(
                host, &self.worker, self.client, characters, &self.registers, line.character, line.ordinal,
            )?
        } else {
            None
        };
        Ok(V1Dialogue::Voice(line, commands, cursor, voice))
    }

    fn tick_say(&mut self, host: &mut dyn HostServices, input: InputFrame) -> Result<Option<VmYield>> {
        let Some(dialogue) = self.v1_dialogue.take() else { return Ok(None) };
        let client = self.client;
        match dialogue {
            V1Dialogue::Definitions(line, mut task) => {
                if let Some(catalog) = task.try_take()? {
                    self.v1_voice_defs = Some(catalog);
                    self.v1_dialogue = Some(self.prepare_say(host, line)?);
                } else {
                    self.v1_dialogue = Some(V1Dialogue::Definitions(line, task));
                }
                Ok(Some(VmYield::BudgetExhausted))
            }
            V1Dialogue::Voice(line, mut commands, cursor, mut job) => {
                if let Some(voice) = &mut job {
                    if !voice.poll(host, client)? {
                        self.v1_dialogue = Some(V1Dialogue::Voice(line, commands, cursor, job));
                        return Ok(Some(VmYield::BudgetExhausted));
                    }
                }
                if line.wait {
                    commands["layers"].as_array_mut().context("missing ExHibit glyph batch")?
                        .push(printer::marker(cursor, 0));
                }
                host.submit_engine(client, 1, &serde_json::to_vec(&commands)?)?;
                self.registers.mark_message(line.ordinal)?;
                if line.wait {
                    self.v1_dialogue = Some(V1Dialogue::Wait(printer::Wait {
                        origin: cursor,
                        since: host.now_ms(),
                        frame: 0,
                    }));
                }
                if self.v1_history.len() == 256 {
                    self.v1_history.pop_front();
                }
                self.v1_history.push_back(line);
                Ok(Some(VmYield::BudgetExhausted))
            }
            V1Dialogue::Wait(mut wait) => {
                let owned = host.ui_input_owned(client).unwrap_or(false);
                let focus = host.input_observation(client).ok();
                let pointer = focus.as_ref().and_then(|v| {
                    (v.focused != Some(false) && v.pointer_inside != Some(false)).then_some(
                        v.pointer.unwrap_or([input.pointer_x, input.pointer_y])
                    )
                }).or_else(|| focus.is_none().then_some([input.pointer_x, input.pointer_y]));
                let toolbar_consumed = if owned { true } else {
                    let (consumed, action) = self.local_controls.pointer(host, client, input, pointer,
                        |slot| self.panels.input_blocked(slot))?;
                    if let Some(action) = action {
                        bail!("unsupported ExHibit dialogue control {}", action.native_id());
                    }
                    consumed
                };
                let mut advance = input;
                if let Some([x, y]) = pointer {
                    advance.pointer_x = x;
                    advance.pointer_y = y;
                } else {
                    advance.buttons_pressed &= !crysirror_core::service::buttons::MOUSE_LEFT;
                }
                if !toolbar_consumed
                    && focus.as_ref().is_none_or(|v| v.focused != Some(false))
                    && printer::advances(advance, self.canvas)
                {
                    host.submit_audio(client, crysirror_core::service::AudioRequest::Stop {
                        channel: voice::CHANNEL,
                        fade_ms: 0.,
                    })?;
                    host.submit_engine(client, 1, &serde_json::to_vec(&json!({
                        "layers":[{"op":"remove","id":printer::MARKER_LAYER}]
                    }))?)?;
                    return Ok(None);
                }
                wait.update(host, client)?;
                self.v1_dialogue = Some(V1Dialogue::Wait(wait));
                Ok(Some(VmYield::BudgetExhausted))
            }
        }
    }

    fn next_gallery_movie(&mut self, h: &mut dyn HostServices) -> Result<bool> {
        let Some(movie) = self.v1_gallery_movies.as_mut().and_then(movie::Gallery::next) else {
            self.v1_gallery_movies = None;
            return Ok(false);
        };
        self.movie = Some(movie::Playing::start(self.client, h, self.canvas, movie,
            &mut self.registers, &mut self.audio, &self.worker)?);
        self.leaving = Some((true, -1));
        Ok(true)
    }
    fn request_transferred_script(&mut self, name: String) {
        self.registers.enter_transferred_script();
        self.name = name;
        self.script = None;
    }

    fn begin_midori_transfer(
        &mut self,
        h: &mut dyn HostServices,
        command: midori_transfer::Command,
    ) -> Result<()> {
        let script = self.script.as_ref().context("missing ExHibit transfer source")?;
        let seeds = self.seeds
            .filter(|s| s.resident == profile::Resident::Midori2011 && s.checkpoint_layout)
            .context("unsupported ExHibit transfer checkpoint layout")?;
        let system = self.midori_system_path.clone().context("missing ExHibit system path")?;
        let limits = self.limits.context("missing ExHibit register dimensions")?;
        let position = midori_positions::Work::begin_script();
        let scene = || -> Result<midori_checkpoint::Scene<'_>> {
            Ok(midori_checkpoint::Scene {
                resident: seeds.resident,
                canvas: self.canvas,
                actors: &self.midori_actor_slots,
                background: &self.midori_background,
                printer: &self.midori_areas,
                marker: &self.midori_marker,
                positions: &position,
                entries: &self.midori_entries,
                sound: midori_sound_capture::SoundCapture::from_owners(
                    &self.midori_music, &self.midori_ambient,
                ).context("ExHibit sound scene is changing")?,
            })
        };
        let mut registers = self.registers.for_transfer();
        registers.clear_transfer_memo();
        let mut destination = command.destination(&self.registers, &registers)?;
        if command.reset_scene_flags {
            registers.set_normal(3, -1)?;
            registers.set_normal(6, 0)?;
        }
        let outgoing = scene()?.capture(&registers, self.checkpoint_flag, false)?;
        let outgoing_title = registers.text(1)?.to_vec();
        let outgoing_block = registers.read(Value::Local(2))?;
        registers.checkpoint_v3(outgoing_block, outgoing_title, None, outgoing)?;
        registers.set_system(0, 0)?;
        let bytes = v1_save::write_integer_bank(limits.sys, |i| registers.read(Value::System(i)))?;
        if let Some(destination) = &mut destination {
            registers.scenario_loaded_native(std::mem::take(&mut destination.native_path))?;
            let incoming = scene()?.capture(&registers, self.checkpoint_flag, false)?;
            registers.checkpoint_v3(0, Vec::new(), None, incoming)?;
        } else {
            ensure!(!self.native_definition, "unsupported ExHibit bootstrap completion");
        }
        let storage = h.storage(self.client)?;
        let task = self.worker.spawn(move |cancel| {
            cancel.check()?;
            let decoded = destination.map(|midori_transfer::Destination { path, name, .. }| -> Result<_> {
                let reader = storage.open_read(&path)?;
                let mut data = crysirror_resource::read_all(reader.as_ref(), 8 << 20)?;
                seeds.decode(&name, &mut data)?;
                let mut next = Script::new(data, 3)?;
                next.labels = Some(branch::labels(&next.bytes, next.start, 3)?);
                Ok((name, next))
            }).transpose()?;
            cancel.check()?;
            storage.write_atomic(&system, &bytes)?;
            Ok(decoded)
        })?;
        // The caller retains the original instruction until the worker has
        // validated the successor and committed the detached system bank.
        debug_assert!(script.resident == Some(profile::Resident::Midori2011));
        self.midori_transfer = Some(midori_transfer::Pending { task, registers });
        Ok(())
    }

    fn tick(
        &mut self,
        h: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> Result<VmYield> {
        if self.halted {
            return Ok(VmYield::Halted);
        }
        if let Some(mut pending) = self.midori_transfer.take() {
            let Some(next) = pending.task.try_take()? else {
                self.midori_transfer = Some(pending);
                return Ok(VmYield::BudgetExhausted);
            };
            self.registers.commit_transfer(pending.registers);
            if let Some((name, script)) = next {
                self.name = name;
                self.script = Some(script);
                self.midori_positions = Some(midori_positions::Work::begin_script());
                return Ok(VmYield::BudgetExhausted);
            }
            if let Some(script) = self.script.as_mut() {
                script.pc = script.bytes.len();
                script.remaining = 0;
                script.returns.clear();
            }
            self.halted = true;
            return Ok(VmYield::Halted);
        }
        if let Some(after) = self.late_screen_resume {
            if self.late_screen.finish(h, self.client, &mut self.registers)? {
                let script = self.script.as_mut().context("missing screen-update continuation")?;
                script.pc = after;
                script.remaining -= 1;
                self.late_screen_resume = None;
            }
            return Ok(VmYield::BudgetExhausted);
        }
        self.controls.block.poll(input, self.canvas);
        let clock = h.now_ms();
        self.animations
            .tick(&mut self.panels, h, self.client, clock)?;
        if let Some(fade) = &mut self.billboard_fade {
            if fade.present(h, self.client)? {
                self.billboard_fade = None;
            }
        }
        if let Some(clip) = &mut self.clip {
            if clip.present(h, self.client, self.canvas)? {
                self.draw_scene[2..6].copy_from_slice(&clip.crop());
                self.clip = None;
            }
        }
        let mut delta_ready = true;
        if let Some(delta) = &mut self.opening_dlt {
            let preparing = delta.preparing_cg();
            delta_ready = delta.present(h, self.client)?;
            if let Some(id) = preparing && delta.preparing_cg().is_none() && self.registers.configured() {
                self.registers.change(Change::Experience(registers::Bank::Cg, vec![id]))?;
            }
            if delta.retired() {
                self.opening_dlt = None;
            }
        }
        let audio_ready = self
            .audio
            .poll(h, self.client, &mut self.registers, input)?;
        let ambience_ready = self.midori_ambient.poll(h, self.client)?;
        let actor_ready = self.midori_actor_slots.present(h, self.client)?;
        let music_ready = self.midori_music.poll(h, self.client, &mut self.registers)?;
        let background_ready = self.midori_background.poll(h, self.client, &mut self.registers)?;
        if !delta_ready || !audio_ready || !ambience_ready || !actor_ready || !music_ready {
            return Ok(VmYield::BudgetExhausted);
        }
        if !background_ready {
            return Ok(self.midori_background.wait_until(clock).map_or(VmYield::BudgetExhausted, VmYield::WaitUntil));
        }
        let draw_pc = self.script.as_ref().map_or(0, |script| script.pc);
        if let Some(reason) = self.poll_v1_scaled_draw(h, clock)
            .with_context(|| format!("ExHibit {}.rld+{draw_pc:#x}", self.name))? {
            return Ok(reason);
        }
        if let Some(fade) = &mut self.scene_fade {
            if fade.closes_printer(clock) {
                if self.message_backdrop.opened {
                    let area = self.selected_message_area.context("no selected ExHibit message area")?;
                    self.message_areas.close(area, &mut self.message_backdrop, h, self.client, 0)?;
                } else if let Some(page) = self.message_backdrop.page {
                    page.clear(h, self.client, false)?;
                }
                if self.local_controls.open() {
                    let mut banks = [false; 8];
                    banks[5] = self.controls.auto;
                    banks[6] = self.registers.read(Value::System(10))? & 15 != 0;
                    self.local_controls.change_scope(false, self.controls.skip, banks, h, self.client, &self.worker)?;
                }
            }
            if !fade.present(h, self.client, self.canvas, &mut self.audio, &mut self.draw_scene)? {
                return Ok(VmYield::WaitUntil(clock.saturating_add(1)));
            }
            self.scene_fade = None;
            self.release_v1_scaled_image(h)?;
        }
        if let Some(fade) = &mut self.v1_page_fade {
            if !fade.present(h, self.client)? {
                return Ok(VmYield::WaitUntil(clock.saturating_add(1)));
            }
            self.v1_page_fade = None;
        }
        if !self.message_backdrop.present(h, self.client)? {
            return Ok(VmYield::WaitUntil(clock.saturating_add(1)));
        }
        if !self.local_controls.present(h, self.client)? {
            return Ok(VmYield::BudgetExhausted);
        }
        if !self.midori_marker.poll(self.client)? {
            return Ok(VmYield::BudgetExhausted);
        }
        if !self.midori_areas.poll()? {
            return Ok(VmYield::BudgetExhausted);
        }
        if !self.midori_areas.present(h, self.client, self.controls.skip)? {
            return Ok(VmYield::WaitUntil(clock.saturating_add(1)));
        }
        if let Some(task) = &mut self.v1_precache {
            if task.try_take()?.is_none() {
                return Ok(VmYield::BudgetExhausted);
            }
            self.v1_precache = None;
        }
        if let Some((id, task)) = &mut self.pending_message_area {
            let Some(catalog) = task.try_take()? else {
                return Ok(VmYield::BudgetExhausted);
            };
            let id = *id;
            self.message_areas.merge_missing(catalog);
            self.bind_v1_message_frame(id, h)?;
            self.pending_message_area = None;
            if self.message_backdrop.waiting() {
                return Ok(VmYield::WaitUntil(clock.saturating_add(1)));
            }
        }
        if let Some(reason) = self.poll_cg_billboard(h)? {
            return Ok(reason);
        }
        if self.animations.waiting() {
            return Ok(VmYield::WaitUntil(clock.saturating_add(1)));
        }
        if !self.panels.poll(h, self.client)? {
            return Ok(VmYield::BudgetExhausted);
        }
        if let Some(selection) = &mut self.selection {
            let pointer = if h.ui_input_owned(self.client).unwrap_or(false) {
                None
            } else {
                match h.input_observation(self.client) {
                    Ok(observed) if observed.focused == Some(false)
                        || observed.pointer_inside == Some(false) => None,
                    Ok(observed) => observed.pointer.or(Some([input.pointer_x, input.pointer_y])),
                    Err(_) => Some([input.pointer_x, input.pointer_y]),
                }
            };
            let (owned, action) = self.local_controls.pointer(h, self.client, input, pointer,
                |slot| self.panels.input_blocked(slot))?;
            if owned {
                selection.poll(&self.panels, input, None);
                if let Some(action) = action {
                    use local_controls::ToolbarAction;
                    match action {
                        ToolbarAction::Auto => {
                            let active = !self.controls.auto;
                            self.controls.auto = active;
                            self.registers.set_system(11, i32::from(active))?;
                            self.local_controls.set_bank(h, self.client, 5, active)?;
                            if active {
                                self.controls.skip = false;
                                let value = self.registers.read(Value::System(10))? & !15;
                                self.registers.set_system(10, value)?;
                                self.local_controls.set_bank(h, self.client, 6, false)?;
                            }
                        }
                        ToolbarAction::Skip => {
                            let value = self.registers.read(Value::System(10))?;
                            let active = value & 15 == 0;
                            self.registers.set_system(10, (value & !15) | i32::from(active))?;
                            self.local_controls.set_bank(h, self.client, 6, active)?;
                            if active {
                                self.controls.auto = false;
                                self.registers.set_system(11, 0)?;
                                self.local_controls.set_bank(h, self.client, 5, false)?;
                            } else {
                                self.controls.skip = false;
                            }
                        }
                        other => bail!("unsupported ExHibit toolbar action {} ({other:?})", other.native_id()),
                    }
                }
                return Ok(VmYield::WaitForInput);
            }
            let Some(id) = selection.poll(&self.panels, input, pointer) else {
                return Ok(VmYield::WaitForInput);
            };
            self.registers.set_normal(selection.destination, i32::from(id))?;
            self.last_selected_item = Some(id);
            self.selection = None;
        }
        if let Some((dest, task)) = &mut self.slot_query {
            let Some(occupied) = task.try_take()? else {
                return Ok(VmYield::BudgetExhausted);
            };
            self.registers.set_normal(*dest, i32::from(occupied))?;
            self.slot_query = None;
        }
        if let Some((after, pending)) = self.autosave.take() {
            use crysirror_core::service::CivilTimeState;
            match pending {
                Autosave::Clock(slot, mut clock) => {
                    self.autosave = Some((after, match clock.poll()? {
                        CivilTimeState::Pending => Autosave::Clock(slot, clock),
                        CivilTimeState::Failed(error) => bail!("ExHibit autosave clock: {error}"),
                        CivilTimeState::Ready(time) => {
                            ensure!(time.client == self.client, "foreign ExHibit autosave clock");
                            let class = self.save_class.clone().context("missing ExHibit save class")?;
                            let storage = h.storage(self.client)?;
                            Autosave::Write(self.worker.spawn(move |cancel| {
                                cancel.check()?;
                                let next = slot.next;
                                slot.write(storage.as_ref(), &class, time)?;
                                Ok(next)
                            })?)
                        }
                    }));
                    return Ok(VmYield::BudgetExhausted);
                }
                Autosave::Write(mut write) => {
                    let Some(next) = write.try_take()? else {
                        self.autosave = Some((after, Autosave::Write(write)));
                        return Ok(VmYield::BudgetExhausted);
                    };
                    self.registers.set_system(0, 0)?;
                    self.registers.set_system(7, next)?;
                    self.registers.set_normal(3, -1)?;
                    let script = self.script.as_mut().context("missing autosave continuation")?;
                    script.pc = after;
                    script.remaining -= 1;
                }
            }
        }
        if let Some((wait, fade)) = self.leaving {
            if let Some(movie) = self.movie.as_mut() {
                if let Some(until) = movie.leave(self.client, h, wait, fade)? {
                    return Ok(VmYield::WaitUntil(until));
                }
            }
            self.movie = None;
            self.leaving = None;
        }
        if self.v1_gallery_movies.is_some() && self.next_gallery_movie(h)? {
            return Ok(VmYield::BudgetExhausted);
        }
        if let Some(reason) = self.tick_say(h, input)? {
            return Ok(reason);
        }
        if self.script.is_none() {
            if self.load.is_none() {
                let storage = h.storage(self.client)?;
                let path = if let Some(resume) = &self.v1_resume {
                    resume.asset_path.clone()
                } else {
                    script_path(&self.name)?
                };
                let (abi, sources, mut seeds, name) = (
                    self.abi,
                    self.sources.clone(),
                    self.seeds,
                    self.name.clone(),
                );
                let open_system = !self.system_opened;
                let system_limits = self.limits;
                let initial_system = if !open_system {
                    None
                } else {
                    self.save_class.clone().zip(self.limits)
                };
                self.load = Some(self.worker.spawn(move |cancel| {
                    cancel.check()?;
                    let reader = storage.open_read(&path)?;
                    let mut bytes = crysirror_resource::read_all(reader.as_ref(), 8 << 20)?;
                    if abi == 3 {
                        if seeds.is_none() {
                            seeds = Some(profile::Seeds::read(
                                storage.as_ref(),
                                sources
                                    .as_ref()
                                    .context("missing v3 initialization sources")?,
                            )?)
                        }
                        seeds
                            .context("missing v3 initialization")?
                            .decode(&name, &mut bytes)?;
                    }
                    cancel.check()?;
                    let mut script = Script::new(bytes, abi)?;
                    script.resident = seeds.map(|s| s.resident);
                    let midori_system_path = if open_system
                        && seeds.is_some_and(|s| s.resident == profile::Resident::Midori2011)
                    {
                        sources.as_ref().map(|sources| profile::Seeds::midori_system_path(storage.as_ref(), sources)).transpose()?
                    } else {
                        None
                    };
                    let mut system = initial_system
                        .map(|(class, limits)| v1_save::read_system(storage.as_ref(), &class, limits))
                        .transpose()?.flatten();
                    if let Some((path, limits)) = midori_system_path.as_ref().zip(system_limits) {
                        system = v1_save::read_system_path(storage.as_ref(), path, limits)?;
                    }
                    Ok(ScenarioLoad { script, seeds, path, system, midori_system_path })
                })?);
            }
            if let Some(ScenarioLoad { mut script, seeds, path, system, midori_system_path }) = self
                .load
                .as_mut()
                .context("missing scenario task")?
                .try_take()?
            {
                if let Some(system) = system {
                    self.registers.load_system_bank(system)?;
                }
                if let Some(path) = midori_system_path {
                    self.midori_system_path = Some(path);
                }
                self.system_opened = true;
                if let Some(resume) = self.v1_resume.take() {
                    script.seek_record(resume.next)?;
                    script.returns = resume.returns;
                    script.block = resume.block;
                    self.registers.scenario_loaded_native(resume.native_path)?;
                } else {
                    self.registers.scenario_loaded(&path)?;
                }
                if script.resident.is_some() {
                    self.midori_positions = Some(midori_positions::Work::begin_script());
                }
                self.script = Some(script);
                self.seeds = seeds;
                self.load = None;
            }
            return Ok(VmYield::BudgetExhausted);
        }
        let now = h.now_ms();
        if now < self.deadline {
            return Ok(VmYield::WaitUntil(
                if self.clip.is_some() || self.billboard_fade.is_some() || self.animations.live() {
                    now.saturating_add(1).min(self.deadline)
                } else {
                    self.deadline
                },
            ));
        }
        for _ in 0..budget {
            let script = self.script.as_mut().context("missing ExHibit script")?;
            let pc = script.pc;
            if script.remaining == 0 {
                ensure!(!self.native_definition, "unsupported ExHibit original startup after def.rld (camera baseline, local controls and release handoff)");
                self.halted = true;
                return Ok(VmYield::Halted);
            }
            if script.abi == 3 && script.returns.is_empty() {
                self.registers
                    .set_normal(5, (script.start.remaining - script.remaining) as i32)?;
            }
            let action = script
                .next()
                .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
            let ms = match action {
                Action::V1ClearPage(with_name, duration) => {
                    if let Some(page) = self.message_backdrop.page {
                        if duration == 0 {
                            page.clear(h, self.client, with_name)?;
                        } else {
                            self.animations.leave(panel_animation::Kind::Fade, 5, false);
                            if self.controls.skip {
                                page.clear(h, self.client, true)?;
                            } else {
                                self.v1_page_fade = Some(v1_printer_page::ClearFade::new(page, now, duration));
                                return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                            }
                        }
                    }
                    0
                }
                Action::V1Say(line) => {
                    ensure!(self.message_backdrop.opened
                        && !self.controls.skipping() && !self.controls.auto,
                        "unsupported ExHibit dialogue area or advance mode");
                    self.v1_dialogue = Some(if line.wait && line.character >= 3
                        && !self.characters.has_definition(line.character) && self.v1_voice_defs.is_none()
                    {
                        let storage = h.storage(self.client)?;
                        let capacity = self.limits.context("missing ExHibit character limits")?.characters + 3;
                        V1Dialogue::Definitions(line, self.worker.spawn(move |cancel| {
                            cancel.check()?;
                            character::Catalog::read_v1_voice_definitions(storage.as_ref(), capacity)
                        })?)
                    } else {
                        self.prepare_say(h, line)?
                    });
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::V1AutoSave(memo) => {
                    // liteExec reads the global R3 even if LocalHolder has an
                    // override. Re-entering that block after a load consumes R3
                    // without overwriting the saved slot.
                    if self.registers.read_normal(3)? != script.block {
                        let after = script.pc;
                        script.pc = pc;
                        script.remaining += 1;
                        ensure!(self.save_class.is_some(), "missing ExHibit save class for opcode 0x0c");
                        let limits = self.limits.context("missing ExHibit save register dimensions")?;
                        let slot = v1_save::Slot::capture(&self.registers, limits, self.slots[1], memo)?;
                        let clock = h.begin_civil_time(self.client)?;
                        self.autosave = Some((after, Autosave::Clock(slot, clock)));
                        return Ok(VmYield::BudgetExhausted);
                    }
                    self.registers.set_normal(3, -1)?;
                    0
                }
                Action::SkipBlock(enter) => {
                    self.controls.block.set(enter);
                    0
                }
                Action::DefineMidoriAmbient(def) => {
                    self.midori_ambient.define(def);
                    0
                }
                Action::MidoriChannelVolume(command) => {
                    command.apply(h, self.client, &mut self.registers, &mut self.midori_music, &mut self.midori_ambient)?;
                    0
                }
                Action::MidoriMusic(track) => {
                    if self.midori_music.play(h, self.client, &self.registers, track, self.controls.skipping())
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))? {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::StopMidoriMusic(fade) => {
                    if self.midori_music.stop(h, self.client, fade, self.controls.skipping())? {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::MidoriAmbient(command) => {
                    if self.midori_ambient.issue(h, self.client, &self.registers, command, self.controls.skip)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))? {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::MidoriPosition(cmd) => {
                    self.midori_positions.as_mut().context("missing Midori local position work")?.apply(cmd)?;
                    0
                }
                Action::PanMidoriBackground(pan) => {
                    ensure!(!self.controls.skip && self.registers.read(Value::Flag(3))? == 0, "unsupported Midori pan HQ or skip state");
                    self.midori_background.pan(pan, now)?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::DrawMidoriBackground(draw) => {
                    ensure!(self.canvas == midori_background::VIEW && self.clip.is_none() && self.magnify_target.is_none()
                        && !self.controls.skip, "unsupported Midori background canvas, clip or skip state");
                    if self.midori_background.issue(draw, h, self.client, &self.worker, &mut self.registers)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))? {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::MidoriColorFilter(filter) => {
                    self.midori_actor_slots.set_filter(filter)?;
                    0
                }
                Action::DefineMidoriActorSlot(def) => {
                    if let Err(error) = self.midori_actor_slots.define(def) {
                        script.pc = pc;
                        script.remaining += 1;
                        return Err(error).with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name));
                    }
                    0
                }
                Action::DrawMidoriActor(draw) => {
                    self.midori_actor_slots.draw(draw, &self.characters, h, self.client, &self.worker)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::ClearMidoriActor(slot) => {
                    self.midori_actor_slots.clear(slot, h, self.client)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    0
                }
                Action::Environment(def) => {
                    self.audio.environment.define(def)?;
                    0
                }
                Action::Sound(sound) => {
                    self.audio
                        .issue(h, self.client, &self.registers, &self.worker, sound)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::MidoriEffect(effect) => {
                    if self.audio
                        .issue_2011_effect(h, self.client, &self.registers, effect, self.controls.skip)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))? {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::CancelSkip => {
                    self.controls.skip = false;
                    let value = self.registers.read(Value::System(10))? & !15;
                    self.registers.set_system(10, value)?;
                    self.local_controls.set_bank(h, self.client, 6, false)?;
                    0
                }
                Action::CancelAuto => {
                    self.controls.auto = false;
                    self.registers.set_system(11, 0)?;
                    self.local_controls.set_bank(h, self.client, 5, false)?;
                    0
                }
                Action::QuerySlots(dest) => {
                    let counts = self.slots;
                    let storage = h.storage(self.client)?;
                    let query = self.worker.spawn(move |cancel| {
                        cancel.check()?;
                        slot_index::any(storage.as_ref(), counts)
                    })?;
                    self.slot_query = Some((dest, query));
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::LateScreenUpdate(command) => {
                    let result = command.invoke(h, self.client, self.canvas, &mut self.registers, &mut self.late_screen);
                    let pending = self.late_screen.preparing();
                    if pending {
                        self.late_screen_resume = Some(script.pc);
                    }
                    if pending || result.is_err() {
                        script.pc = pc;
                        script.remaining += 1;
                    }
                    let refreshed = result.with_context(|| format!("ExHibit {}.rld+{pc:#x} action 271", self.name))?;
                    if pending || refreshed {
                        // Let the presenter observe the refresh before subsequent records.
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::LateAssign(assignment) => {
                    assignment.apply(&mut self.registers)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    0
                }
                Action::Change(change) => {
                    self.registers
                        .change(change)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    0
                }
                Action::Billboard(billboard) => {
                    if !self.billboard_shown {
                        self.billboard_fade = billboard.begin(h, self.client)?;
                        self.billboard_shown = true;
                    }
                    0
                }
                Action::V1CgBillboard(draw) => {
                    self.begin_cg_billboard(h, draw)?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::Surface(surface) => {
                    ensure!(self.opening_dlt.as_ref().is_none_or(|delta| delta.slot() != surface.slot),
                        "ExHibit billboard conflicts with opening delta layer");
                    self.panels
                        .define(h, self.client, surface)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    0
                }
                Action::PanelAnimation(spec) => {
                    self.animations
                        .begin(spec, &self.panels, now, self.controls.skip)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    if self.animations.waiting() {
                        return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                    }
                    0
                }
                Action::LeavePanelAnimation(kind, channel, wait) => {
                    self.animations.leave(kind, channel, wait);
                    if self.animations.waiting() {
                        return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                    }
                    0
                }
                Action::ControlInput(layer, enable, complement) => {
                    self.panels.control_input(layer, enable, complement);
                    0
                }
                Action::LeaveCompletedClip => {
                    if let Some(target) = self.magnify_target {
                        ensure!(
                            self.clip.is_none() && self.draw_scene[2..6] == target,
                            "ExHibit {}.rld+{pc:#x}: unsupported replacement of active magnification",
                            self.name
                        );
                        self.magnify_target = None;
                        1
                    } else {
                        0
                    }
                }
                Action::RetireV1Clip(wait_for_end) => {
                    if wait_for_end && self.clip.is_some() {
                        // tmExitMagnify waits in mode 0 while the render clock
                        // continues to advance. Keep the native record pending.
                        script.pc = pc;
                        script.remaining += 1;
                        return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                    }
                    if let Some(clip) = self.clip.take() {
                        self.draw_scene[2..6].copy_from_slice(&clip.crop());
                    }
                    u64::from(self.magnify_target.take().is_some())
                }
                Action::DefineSelect2011(mode, bytes) => {
                    self.select_view_2011.install(mode, &bytes);
                    continue;
                }
                Action::DefineViews(kind, extended, bytes) => {
                    self.view_definitions
                        .install(kind, extended, &bytes, self.abi)?;
                    0
                }
                Action::PlaceLoadPage(position) => {
                    self.view_definitions.place_load_page(position);
                    0
                }
                Action::LoadV1Entry(slot) => {
                    let bytes = &self.v1_entries.names()[slot];
                    let (name, asset_path, native_path) = v1_services::entry_path(bytes)?;
                    self.v1_resume = Some(v1_services::Resume {
                        next: script.start.remaining - script.remaining,
                        returns: std::mem::take(&mut script.returns),
                        block: script.block,
                        asset_path, native_path,
                    });
                    self.name = name;
                    self.script = None;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::DefineV1Dialog(dialog) => {
                    ensure!(self.v1_dialogs.len() < 65_536, "ExHibit dialog definition limit");
                    self.v1_dialogs.push(dialog);
                    0
                }
                Action::DefineV3Dialog(dialog) => {
                    ensure!(self.v3_dialogs.len() < 65_536, "ExHibit dialog definition limit");
                    self.v3_dialogs.push(dialog);
                    0
                }
                Action::PreCache(path, mib) => {
                    if let Some(path) = path {
                        let storage = h.storage(self.client)?;
                        self.v1_precache = Some(self.worker.spawn(move |cancel| {
                            cancel.check()?;
                            if let Ok(reader) = storage.open_read(&path) {
                                v1_precache::read(reader.as_ref(), mib, &cancel)?;
                            }
                            Ok(())
                        })?);
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::EntryNameV1(entry) => {
                    self.v1_entries.set(entry);
                    0
                }
                Action::ExperienceGate(gate) => {
                    if let Some((slot, item)) = gate.disabled_item(&self.registers)? {
                        self.panels.disable(
                            h,
                            self.client,
                            &self.worker,
                            i32::from(slot),
                            i32::from(item),
                        )?;
                        if self.panels.pending() {
                            return Ok(VmYield::BudgetExhausted);
                        }
                    }
                    0
                }
                Action::DisableButton(layer, item) => {
                    self.panels
                        .disable(h, self.client, &self.worker, layer, item)?;
                    if self.panels.pending() {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::SelectItem(button) => {
                    self.panels
                        .bind(h, self.client, &self.worker, button)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::WaitForSelection(group, dest) => {
                    self.selection = Some(select_items::Selection::new(group, dest));
                    return Ok(VmYield::WaitForInput);
                }
                Action::RemoveItems(group, layer, first, last) => {
                    self.panels
                        .erase_items(h, self.client, group, Some(layer), first, last)?;
                    if self.panels.pending() {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::ExitSelection(group) => {
                    self.panels
                        .erase_items(h, self.client, i32::from(group), None, 0, 255)?;
                    if self.panels.pending() {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::RemoveSurface(slot, duration) => {
                    ensure!(self.opening_dlt.as_ref().is_none_or(|delta| delta.slot() != slot),
                        "ExHibit removal conflicts with opening delta layer");
                    self.retire_billboard(h, slot, duration)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    if let Some(reason) = self.poll_cg_billboard(h)? {
                        return Ok(reason);
                    }
                    0
                }
                Action::Clip(target, duration, mode) => {
                    let id = self.draw_scene[0];
                    ensure!(
                        id >= 0 && self.movie.is_none(),
                        "unsupported ExHibit clip without a CG surface"
                    );
                    let asset = format!("res/g/ev/{:02}/{id:04}.gyu", id / 100);
                    let source = h.image_size(self.client, &asset)?;
                    let from = if let Some(previous) = &self.clip {
                        previous.crop()
                    } else if self.draw_scene[4] > 0 {
                        self.draw_scene[2..6].try_into()?
                    } else {
                        [0, 0, source[0] as i32, source[1] as i32]
                    };
                    self.magnify_target = (duration != 0).then_some(target);
                    let mut clip = clip::Clip::new(asset, source, from, target, duration, mode)?;
                    if clip.present(h, self.client, self.canvas)? {
                        self.draw_scene[2..6].copy_from_slice(&clip.crop());
                        self.clip = None;
                    } else {
                        self.clip = Some(clip);
                    }
                    0
                }
                Action::DefineMessageArea(area) => {
                    self.message_areas.define(area);
                    0
                }
                Action::DefineMidoriArea(definition) => {
                    self.midori_areas.begin(definition, h, self.client, &self.worker)?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::SelectMidoriFrame => {
                    if self.midori_areas
                        .select_frame(h, self.client, &mut self.registers, self.controls.skip)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))? {
                        return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                    }
                    0
                }
                Action::ChangeMidoriVisibility(visibility) => {
                    if self.midori_areas.change_visibility(
                        visibility, h, self.client, &self.registers, self.controls.skip
                    ).with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))? {
                        return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                    }
                    0
                }
                Action::DefineCharacter(definition) => {
                    ensure!(
                        self.name.eq_ignore_ascii_case("defChara"),
                        "unsupported ExHibit character redefinition outside defChara"
                    );
                    self.characters.define(definition)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    0
                }
                Action::DefineBody(definition) => {
                    ensure!(
                        self.name.eq_ignore_ascii_case("defChara"),
                        "unsupported ExHibit body definition outside defChara"
                    );
                    self.characters.define_body(definition)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    0
                }
                Action::DefineBody2011(definition) => {
                    ensure!(
                        self.name.eq_ignore_ascii_case("defChara"),
                        "unsupported ExHibit 2011 body definition outside defChara"
                    );
                    self.characters.define_body_2011(definition)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    0
                }
                Action::DefineDebug(definition) => {
                    self.debug_definition = Some(definition);
                    0
                }
                Action::DefineMidoriControlParts(parts) => {
                    self.midori_control_defs.set_parts(parts);
                    0
                }
                Action::DefineMidoriShortcut(definition) => {
                    self.midori_shortcuts.define(definition);
                    0
                }
                Action::DefineMidoriControlPart(part) => {
                    self.midori_control_defs.set_part(part);
                    0
                }
                Action::DefineNotification(notification) => {
                    self.midori_control_defs.set_notification(notification);
                    0
                }
                Action::DefineNotifications(notifications) => {
                    for notification in notifications {
                        self.midori_control_defs.set_notification(notification);
                    }
                    0
                }
                Action::DefineV3Control(definition) => {
                    self.midori_control_defs.set_control(*definition);
                    0
                }
                Action::BindMidoriControlSound(command) => {
                    self.midori_control_defs.sounds.apply(command);
                    0
                }
                Action::MidoriControlSounds(update) => {
                    self.midori_control_defs.update_sound(update);
                    0
                }
                Action::DefineMidoriMarker(definition) => {
                    self.midori_marker.begin(definition, h, self.client)?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::MidoriEntryName(assignment) => {
                    self.midori_entries.set(assignment);
                    0
                }
                Action::DefineLocalControls(definition) => {
                    self.local_controls.define(definition)?;
                    0
                }
                Action::LocalControlScope(open) => {
                    let mut banks = [false; 8];
                    banks[5] = self.controls.auto;
                    banks[6] = self.registers.read(Value::System(10))? & 15 != 0;
                    self.local_controls.change_scope(open, self.controls.skip, banks,
                        h, self.client, &self.worker)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    if self.local_controls.busy() { return Ok(VmYield::BudgetExhausted); }
                    0
                }
                Action::CloseV1MessageBackdrop(duration) => {
                    if self.message_backdrop.opened {
                        let id = self.selected_message_area.context("no selected ExHibit message area")?;
                        if duration != 0 {
                            self.animations.leave(panel_animation::Kind::Fade, 6, false);
                        }
                        self.message_areas.close(id, &mut self.message_backdrop, h, self.client,
                            if self.controls.skip { 0 } else { duration })?;
                        if self.message_backdrop.waiting() {
                            return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                        }
                    }
                    0
                }
                Action::OpenMessageBackdrop => {
                    let id = self.selected_message_area.context("no selected ExHibit message area")?;
                    self.message_areas.open(
                        id, &mut self.message_backdrop, h, self.client, &self.registers,
                        self.controls.skip,
                    ).with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    if self.message_backdrop.waiting() {
                        return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                    }
                    0
                }
                Action::MessageFrame(id) => {
                    if self.message_areas.has(id) || self.selected_message_area == Some(id) {
                        self.bind_v1_message_frame(id, h)?;
                        if self.message_backdrop.waiting() {
                            return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                        }
                    } else {
                        let storage = h.storage(self.client)?;
                        let task = self.worker.spawn(move |cancel| {
                            cancel.check()?;
                            let path = script_path("def")?;
                            let reader = storage.open_read(&path)?;
                            let bytes = crysirror_resource::read_all(reader.as_ref(), 8 << 20)?;
                            cancel.check()?;
                            message_area::Catalog::from_v1_definition(bytes)
                        })?;
                        self.pending_message_area = Some((id, task));
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::V1SceneFade(kind) => {
                    ensure!(
                        !self.controls.skip && self.panels.scene_empty()
                            && !self.animations.live() && self.movie.is_none()
                            && self.clip.is_none() && self.billboard_fade.is_none()
                            && self.opening_dlt.is_none()
                            && (kind != scene_fade::Kind::Title
                                || (self.name.eq_ignore_ascii_case("title") && self.last_selected_item == Some(10)
                                    && self.selected_message_area.is_none() && self.draw_scene[0] == 1246)),
                        "unsupported ExHibit fade scene state at {}.rld+{pc:#x}",
                        self.name
                    );
                    if kind == scene_fade::Kind::Title {
                        self.magnify_target = None;
                    }
                    self.scene_fade = Some(scene_fade::SceneFade::begin(
                        h, self.client, self.canvas, &self.audio, kind,
                    )?);
                    return Ok(VmYield::WaitUntil(now.saturating_add(1)));
                }
                Action::Draw(id, ms, crop, retain_clip) => {
                    let opening = clip::is_opening_draw(
                        script.abi, &self.name, pc, id, ms, crop, retain_clip,
                    );
                    if opening {
                        ensure!(
                            self.clip.is_none() && self.magnify_target.is_none()
                                && self.opening_dlt.is_none() && self.movie.is_none()
                                && !self.controls.skip,
                            "unsupported ExHibit opening CG state"
                        );
                    }
                    let mut clip_flag = self.draw_scene[1] & 0x10000;
                    let duration = self.controls.delay(ms);
                    let (paint, uv) = if id < 0 {
                        ensure!(self.clip.is_none() && self.magnify_target.is_none(),
                            "unsupported ExHibit solid draw while magnification owns the background");
                        (clip::solid_paint(id), None)
                    } else {
                        let asset = format!("res/g/ev/{:02}/{id:04}.gyu", id / 100);
                        if v1_scaled_draw::supports(crop, self.canvas) {
                            let work = self.begin_v1_scaled_draw(h, id, &asset, crop, retain_clip, duration);
                            let script = self.script.as_mut().context("missing ExHibit scaled draw script")?;
                            script.pc = pc;
                            script.remaining += 1;
                            work.with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                            return Ok(VmYield::BudgetExhausted);
                        }
                        let uv = if crop[2] > 0 {
                            let source = h.image_size(self.client, &asset)?;
                            clip_flag = if retain_clip && source != [16, 16] { 0x10000 } else { 0 };
                            Some(clip::draw_uv(crop, source, self.canvas, opening)?)
                        } else {
                            None
                        };
                        (json!({"kind":"image","asset":asset}), uv)
                    };
                    h.submit_engine(
                        self.client,
                        1,
                        &serde_json::to_vec(&clip::draw_commands(paint, self.canvas, uv, duration))?,
                    )?;
                    self.release_v1_scaled_image(h)?;
                    self.store_v1_draw(id, crop, clip_flag)?;
                    duration
                }
                Action::Wait(ms) => self.controls.delay(ms),
                Action::PrepareOpeningDlt(placement) => {
                    ensure!(self.opening_dlt.is_none() && !self.panels.is_bound(placement.slot),
                        "unsupported ExHibit replacement of a live delta or bound layer");
                    self.opening_dlt = Some(opening_dlt::OpeningDlt::prepare(h, self.client, placement)?);
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::StartOpeningDlt(level, duration) => {
                    ensure!(!self.controls.skip, "unsupported ExHibit skip during delta fade");
                    self.opening_dlt.as_mut().context("missing ExHibit delta")?
                        .start(h, self.client, level, duration)?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::StopOpeningDlt(channel, duration, wait) => {
                    if channel == 0 && let Some(delta) = &mut self.opening_dlt {
                        ensure!(!self.controls.skip && !wait && duration >= 0,
                            "unsupported ExHibit live delta exit during skip, native completion wait or retained layer");
                        delta.stop(now, duration as u32)?;
                        return Ok(VmYield::BudgetExhausted);
                    }
                    // exitAnimation returns immediately when the native channel has no object.
                    0
                }
                Action::Movie(movie) => {
                    self.movie = Some(movie::Playing::start(self.client, h, self.canvas, movie, &mut self.registers, &mut self.audio, &self.worker)?);
                    if self.audio.preparation_pending() {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::V1GalleryMovies(movies) => {
                    self.v1_gallery_movies = Some(movies);
                    if self.next_gallery_movie(h)? {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::PrepareV1Effect(slot, id) => {
                    self.audio.prepare_v1_effect(h, self.client, slot, id)?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::LeaveMovie(wait, fade) => {
                    self.leaving = Some((wait, fade));
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::End => {
                    ensure!(!self.native_definition, "unsupported ExHibit original startup after def.rld (camera baseline, local controls and release handoff)");
                    ensure!(self.opening_dlt.is_none(), "unsupported ExHibit end with live delta");
                    if script.abi == 3 {
                        self.registers.set_normal(6, 0)?;
                    }
                    self.halted = true;
                    return Ok(VmYield::Halted);
                }
                Action::Label | Action::IgnoredExperienceBank | Action::NativeMethodReturn => 0,
                Action::Block(block) => {
                    if let Some(resident) = script.resident.filter(|_| self.seeds.is_some_and(|seeds| seeds.checkpoint_layout))
                        && !block.marker && block.condition.matches(&self.registers)? {
                        let result = (|| -> Result<()> {
                            let id = if block.id == -1 { script.block.wrapping_add(1) } else { block.id };
                            let flag = block.checkpoint_flag.unwrap_or(self.checkpoint_flag);
                            let title = if block.checkpoint_skip.is_some() {
                                midori_checkpoint::title(&self.registers, &block.title, id)?
                            } else { block.title };
                            let scene = midori_checkpoint::Scene {
                                resident,
                                canvas: self.canvas,
                                actors: &self.midori_actor_slots,
                                background: &self.midori_background,
                                printer: &self.midori_areas,
                                marker: &self.midori_marker,
                                positions: self.midori_positions.as_ref().context("missing local position state")?,
                                entries: &self.midori_entries,
                                sound: if resident.late() {
                                    midori_sound_capture::SoundCapture::late_initial()
                                } else {
                                    midori_sound_capture::SoundCapture::from_owners(
                                        &self.midori_music, &self.midori_ambient,
                                    ).context("ExHibit sound scene is changing")?
                                },
                            };
                            let capture = scene.capture(&self.registers, flag, block.checkpoint_skip.is_some())?;
                            self.registers.checkpoint_v3(id, title, block.checkpoint_skip, capture)?;
                            self.checkpoint_flag = flag;
                            script.block = id;
                            Ok(())
                        })();
                        if result.is_err() {
                            script.pc = pc;
                            script.remaining += 1;
                        }
                        result.with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                        continue;
                    }
                    script
                        .visit_block(
                            block,
                            &mut self.registers,
                            &mut self.checkpoint_flag,
                            &self.draw_scene,
                            &self.audio.scene,
                            [self.message_backdrop.opened, self.local_controls.open()],
                        )
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    0
                }

                Action::LateFlow(flow) => {
                    flow.apply(script, &mut self.registers)?;
                    0
                }
                Action::Branch(op, id, when) => {
                    if when.matches(&self.registers)? {
                        script.branch(op, id)?;
                    }
                    0
                }
                Action::Transfer(name, when) => {
                    if !when.matches(&self.registers)? {
                        continue;
                    }
                    ensure!(self.opening_dlt.is_none(), "unsupported ExHibit transfer with live delta");
                    self.request_transferred_script(name);
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::Transfer2011(command) => {
                    if !command.when.matches(&self.registers)? {
                        continue;
                    }
                    script.pc = pc;
                    script.remaining += 1;
                    self.begin_midori_transfer(h, command)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
                    return Ok(VmYield::BudgetExhausted);
                }
            };
            if ms != 0 {
                self.deadline = now.saturating_add(ms);
                return Ok(VmYield::WaitUntil(
                    if self.clip.is_some()
                        || self.billboard_fade.is_some()
                        || self.animations.live()
                    {
                        now.saturating_add(1).min(self.deadline)
                    } else {
                        self.deadline
                    },
                ));
            }
        }
        Ok(VmYield::BudgetExhausted)
    }
}
impl VmAdapter for Vm {
    fn engine_id(&self) -> &'static str {
        "exhibit"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn preparation_pending(&self) -> bool {
        self.script.is_none()
            || self.late_screen.preparing()
            || self.midori_transfer.is_some()
            || self.autosave.is_some()
            || self.v1_dialogue.as_ref().is_some_and(|dialogue| !matches!(dialogue, V1Dialogue::Wait(_)))
            || matches!(self.v1_image_billboard, Some(v1_image_billboard::Pending::Loading(_)))
            || self.v1_scaled_draw.as_ref().is_some_and(v1_scaled_draw::Pending::preparing)
            || self.v1_precache.is_some()
            || self.slot_query.is_some()
            || self.panels.pending()
            || self.midori_marker.preparing()
            || self.midori_areas.pending()
            || self.midori_actor_slots.pending()
            || self.midori_background.preparing()
            || self.midori_music.preparing()
            || self.audio.preparation_pending()
    }
    fn step(
        &mut self,
        h: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        self.tick(h, input, budget)
            .map_err(|e| Error::Io(format!("{e:#}")))
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("ExHibit snapshot"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("ExHibit restore"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        // A prepared native register image only, independent of script T204.
        // In particular this does not unguard the 2011 block or write R61.
        let midori_audio_candidate = if self.script.as_ref().is_some_and(|s| s.resident == Some(profile::Resident::Midori2011)) {
            midori_sound_capture::SoundCapture::from_owners(&self.midori_music, &self.midori_ambient)
                .map(|capture| json!({"normal":capture.normal, "text":capture.text}))
        } else {
            None
        };
        let midori_status_candidate = if self.script.as_ref().is_some_and(|s| s.resident == Some(profile::Resident::Midori2011)) {
            midori_status_capture::StatusCapture::from_owners(&self.registers, &self.midori_areas, &self.midori_entries)
                .ok()
                .map(|capture| json!({
                    "printer_open": capture.printer_open.map(|write| [write.register as i32, write.mask, write.bits]),
                    "block_flag_0x40": ({
                        let w = midori_status_capture::MaskedNormal::authored_block_flag(self.checkpoint_flag);
                        [w.register as i32,w.mask,w.bits]
                    }),
                    "printer_socket": capture.printer_socket.map(|socket| json!({"normal":socket.index, "text":socket.packet})),
                    "entry_names": [json!(capture.entry_names.0), json!(String::from_utf8_lossy(&capture.entry_names.1))],
                    "experience": {
                        "base": capture.experience.base, "counts": capture.experience.counts,
                        "words": capture.experience.words, "nonzero": capture.experience.nonzero_words()
                    }
                }))
        } else {
            None
        };
        let midori_filter_candidate = if self.script.as_ref().is_some_and(|s| s.resident == Some(profile::Resident::Midori2011)) {
            self.registers.text(4).ok().and_then(|text| self.midori_actor_slots.capture_filter(text))
        } else {
            None
        };
        let mut snapshot = json!({
            "abi": self.abi,
            "late_screen_update_counter": self.late_screen.counter(),
            "scenario": self.name,
            "record_offset": self.script.as_ref().map(|script| script.pc),
            "committed_block": self.registers.read_normal(2).ok(),
            "waiting_for_selection": self.selection.is_some(),
            "last_selected_item": self.last_selected_item,
            "scene_fade_in_progress": self.scene_fade.is_some(),
            "message_area": self.selected_message_area,
            "message_backdrop_open": self.message_backdrop.opened,
            "local_controls_open": self.local_controls.open(),
            "dialogue_waiting": matches!(self.v1_dialogue, Some(V1Dialogue::Wait(_))),
            "dialogue_history_rows": self.v1_history.len(),
            "last_dialogue_ordinal": self.v1_history.back().map(|line| line.ordinal),
            "opening_dlt_live": self.opening_dlt.is_some(),
            "authored_characters": self.characters.len(),
            "authored_body_variants": self.characters.body_count(),
            "selection_definition_2011": (self.abi == 3).then_some(&self.select_view_2011),
            "view_definitions": &self.view_definitions,
            "v1_dialogs": (self.abi == 1).then_some(&self.v1_dialogs),
            "v3_dialog_count": self.v3_dialogs.len(),
            "v1_entry_name_bytes": (self.abi == 1).then(|| self.v1_entries.names()),
        });
        let serde_json::Value::Object(midori) = json!({
            "midori_entry_names": self.midori_entries.count(),
            "midori_area_geometry": self.midori_areas.geometries(),
            "midori_installed_frame": self.midori_areas.installed(),
            "midori_bgm_requested_and_audible": self.midori_music.ids(),
            "midori_audio_checkpoint_candidate": midori_audio_candidate,
            "midori_status_checkpoint_candidate": midori_status_candidate,
            "midori_filter_checkpoint_candidate": midori_filter_candidate,
            "midori_background_checkpoint_candidate": self.midori_background.capture(),
            "midori_background_crop": self.midori_background.current_crop(),
            "midori_debug_definition": self.debug_definition.as_ref().map(|state| json!({"mode":state.mode,"pairs":state.pairs})),
            "midori_control_parts": self.midori_control_defs.part_status().ok(),
            "midori_shortcuts": self.midori_shortcuts,
            "midori_control_sounds": self.midori_control_defs.sound_slots(),
            "midori_marker_geometry": self.midori_marker.geometry(),
            "midori_marker_checkpoint_candidate": self.midori_marker.capture(),
            "midori_local_position_checkpoint_candidate": self.midori_positions.as_ref().map(midori_positions::Work::capture),
            "midori_printer_depth": self.midori_areas.depth(),
            "midori_portrait_poses": self.midori_actor_slots.poses(),
            "midori_portrait_checkpoint_rows": self.midori_actor_slots.checkpoint_rows()
                .filter(|_| self.abi == 3 && self.script.is_some()),
        }) else {
            unreachable!("Midori diagnostics are a JSON object")
        };
        snapshot.as_object_mut().expect("ExHibit diagnostics are a JSON object").extend(midori);
        if !self.midori_control_defs.sounds.details.is_empty() {
            snapshot["midori_control_sound_bindings"] = json!(self.midori_control_defs.sounds.details);
        }
        if self.script.as_ref().and_then(|script| script.resident).is_some_and(profile::Resident::late) {
            snapshot["notification_slots"] = json!(self.midori_control_defs.notification_slots());
        }
        if self.abi == 3 {
            snapshot["local_control_definition_count"] = json!(self.midori_control_defs.control_slots().len());
        }
        serde_json::to_vec(&snapshot).unwrap_or_default()
    }
}
struct Factory;
impl Factory {
    fn scenario(
        client: ClientId,
        spec: &crysirror_runtime::model::ClientSpec,
    ) -> Result<Vm> {
        let config: Config = serde_json::from_value(spec.config.clone())?;
        ensure!(
            matches!(config.abi, 1 | 3),
            "unsupported ExHibit RLD version {} (scenario payload)",
            config.abi
        );
        ensure!(
            spec.program.is_empty() && config.canvas.iter().all(|v| (1..=16384).contains(v)),
            "invalid ExHibit project"
        );
        ensure!(
            config.slots.iter().all(|n| *n <= 100000),
            "invalid ExHibit slot count"
        );
        script_path(&config.entry)?;
        if let Some(class) = &config.save_class {
            ensure!(config.abi == 1, "invalid autosave profile for ExHibit v3");
            v1_save::system_path(class, Mount::Save)?;
        }
        let mut registers = Registers::for_scenario(config.limits, config.abi, config.canvas)?;
        registers.set_windows_version(config.native_windows_version);
        Ok(Vm {
            client,
            name: if config.native_bootstrap { "def".into() } else { config.entry },
            native_definition: config.native_bootstrap,
            canvas: config.canvas,
            script: None,
            worker: TaskPool::new("exhibit-rld", 1, 1)?,
            load: None,
            deadline: 0,
            abi: config.abi,
            sources: config.sources,
            seeds: None,
            late_screen: later_screen::Updates::default(),
            late_screen_resume: None,
            movie: None,
            opening_dlt: None,
            leaving: None,
            halted: false,
            checkpoint_flag: false,
            slots: config.slots,
            save_class: config.save_class,
            system_opened: false,
            midori_system_path: None,
            midori_transfer: None,
            autosave: None,
            slot_query: None,
            controls: Controls::default(),
            clip: None,
            magnify_target: None,
            animations: Default::default(),
            panels: select_items::Panels::for_characters(
                config.limits.map_or(0, |l| l.characters.saturating_add(3)),
            )?,
            selection: None,
            last_selected_item: None,
            billboard_shown: false,
            billboard_fade: None,
            scene_fade: None,
            v1_scaled_draw: None,
            v1_scaled_image: None,
            message_areas: Default::default(),
            selected_message_area: None,
            message_backdrop: Default::default(),
            local_controls: Default::default(),
            v1_entries: Default::default(),
            v1_resume: None,
            v1_voice_defs: None,
            v1_dialogue: None,
            v1_page_fade: None,
            v1_image_billboard: None,
            v1_history: Default::default(),
            v1_gallery_movies: None,
            v1_precache: None,
            v1_dialogs: Vec::new(),
            v3_dialogs: Vec::new(),
            view_definitions: Default::default(),
            select_view_2011: Default::default(),
            midori_entries: Default::default(),
            midori_ambient: Default::default(),
            midori_music: Default::default(),
            midori_actor_slots: midori_actor::Slots::for_view(config.canvas),
            midori_background: Default::default(),
            midori_areas: Default::default(),
            midori_positions: None,
            debug_definition: None,
            midori_marker: Default::default(),
            midori_control_defs: Default::default(),
            midori_shortcuts: Default::default(),
            characters: character::Catalog::new(
                config.limits.map_or(0, |l| l.characters.saturating_add(3)),
            ),
            pending_message_area: None,
            draw_scene: [
                -1,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                -1,
                -1,
                0,
                800 + (600 << 16),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ],
            registers,
            limits: config.limits,
            audio: sound::Audio::default(),
        })
    }
}
impl crysirror_runtime::runtime::VmFactory for Factory {
    fn create(
        &self,
        client: ClientId,
        spec: &crysirror_runtime::model::ClientSpec,
    ) -> Result<Box<dyn VmAdapter>> {
        Ok(Box::new(Self::scenario(client, spec)?))
    }
}
pub fn register(registry: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    registry.register("exhibit", Arc::new(Factory))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(records: &[u8], count: u32) -> Result<Script> {
        let mut bytes = b"\0DLR".to_vec();
        for value in [1, 16, count] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(records);
        Script::new(bytes, 1)
    }

    #[test]
    fn opening_say_retains_actor_ordinal_and_wait_at_dispatch() -> Result<()> {
        let mut record = vec![0x1c, 0, 7, 2];
        for value in printer::FIRST_WORDS {
            record.extend(value.to_le_bytes());
        }
        record.extend([0x8b, 0x4c, 0x8f, 0x71, 0, 0x96, 0x7b, 0]);
        let mut script = fixture(&record, 1)?;
        let Action::V1Say(line) = script.next()? else { bail!("expected opening dialogue") };
        assert_eq!((line.character, line.ordinal, line.wait), (5, 1, true));
        assert_eq!((script.pc, script.remaining), (16 + record.len(), 0));
        Ok(())
    }

    #[test]
    fn draw_wait_and_transfer_consume_exact_operands() -> Result<()> {
        let mut body = vec![0x4a, 0, 16, 0];
        for v in [321i32, 9, 0, 750, 0, 0, 0, 0, 0, 0, 0, -1, -1, 0, 0, 0] {
            body.extend(v.to_le_bytes());
        }
        body.extend([13, 0, 2, 0, 25, 0, 0, 0, 0, 0, 0, 0]);
        body.extend(b"\x11\0\0\x12next\0*\0");
        let mut script = fixture(&body, 3)?;
        assert_eq!(script.next()?, Action::Draw(321, 750, [0; 4], false));
        assert_eq!(script.next()?, Action::Wait(25));
        assert_eq!(
            script.next()?,
            Action::Transfer("next".into(), Condition::text(b"*")?)
        );
        assert_eq!(script.remaining, 0);
        assert_eq!(script.pc, script.bytes.len());
        Ok(())
    }

    #[test]
    fn label_call_and_return_resume_after_the_call() -> Result<()> {
        let mut body = vec![0x16, 0, 16, 5];
        for arg in std::iter::once(7i32).chain(std::iter::repeat_n(-1, 15)) {
            body.extend(arg.to_le_bytes());
        }
        body.extend([0; 5]);
        body.extend([0x18, 0, 0, 0, 0x2f, 0, 1, 0, 7, 0, 0, 0]);
        body.extend([0x17, 0, 16, 5]);
        for arg in std::iter::once(0i32).chain(std::iter::repeat_n(-1, 15)) {
            body.extend(arg.to_le_bytes());
        }
        body.extend([0; 5]);
        // A duplicate marker must not move the call target to the end.
        body.extend([0x2f, 0, 1, 0, 7, 0, 0, 0]);
        let mut script = fixture(&body, 5)?;
        let Action::Branch(op, id, cond) = script.next()? else {
            anyhow::bail!("expected call")
        };
        assert!(cond.matches(&Registers::default())?);
        script.branch(op, id)?;
        assert_eq!(script.next()?, Action::Label);
        let Action::Branch(op, id, _) = script.next()? else {
            anyhow::bail!("expected return")
        };
        script.branch(op, id)?;
        assert_eq!(script.next()?, Action::End);
        assert!(script.branch(0x14, 88).is_err());
        assert!(script.branch(0x17, 0).is_err());
        Ok(())
    }

    #[test]
    fn v3_record_origin_differs_and_execution_is_profile_scoped() -> Result<()> {
        let mut bytes = b"\0DLR".to_vec();
        for n in [3u32, 12, 1] {
            bytes.extend(n.to_le_bytes());
        }
        bytes.extend([0x4a, 0, 0, 0]);
        let mut script = Script::new(bytes, 3)?;
        assert_eq!(script.pc, 16);
        assert!(script.next().is_err());
        assert_eq!(script.remaining, 1);
        Ok(())
    }

    #[test]
    fn malformed_and_unimplemented_records_preserve_the_pc() -> Result<()> {
        for data in [
            b"\x11\0\0\x12../next\0*\0".as_slice(),
            b"\x11\0\0\x12next\0abc",
            b"\x0d\0\x02\0\x01\0",
            b"\x71\0\0\0",
        ] {
            let mut script = fixture(data, 1)?;
            assert!(script.next().is_err());
            assert_eq!((script.pc, script.remaining), (16, 1));
        }
        Ok(())
    }
}

#[cfg(test)]
mod native_state_tests {
    use super::*;
    use registers::{Bank, Limits};
    pub(super) const LIMITS: Limits = Limits {
        regs: 2048,
        sys: 4096,
        cg: 32,
        scene: 8,
        sound: 16,
        strings: 256,
        characters: 34,
        locals: 100,
        messages: 65,
    };

    pub(super) fn fresh(abi: u32) -> Result<Registers> {
        Registers::for_scenario(Some(LIMITS), abi, [1024, 576])
    }
    #[test]
    fn first_run_uses_the_configured_native_register_bases() -> Result<()> {
        for (abi, history, volume) in [(1, 137, 96), (3, 217, 0xc000c0)] {
            let r = fresh(abi)?;
            assert_eq!(r.read(Value::System(1))?, 1);
            assert_eq!(r.read(Value::System(15))?, 117);
            assert_eq!(r.read(Value::System(16))?, history);
            assert_eq!(r.read(Value::System(32))?, volume);
            assert_eq!(r.read(Value::Flag(2))?, 1);
            assert_eq!(r.read(Value::System(116))?, 128);
        }
        assert_eq!(fresh(3)?.read(Value::System(8))?, (412 << 16) + 348);
        Ok(())
    }
    #[test]
    fn entering_a_v1_block_captures_scene_sound_and_native_read_bits() -> Result<()> {
        let mut r = fresh(1)?;
        let mut draw = [0; 19];
        draw[0] = 13;
        let mut audio = sound::Audio::default();
        audio.scene[1..4].copy_from_slice(&[2, 200, 160]);
        r.change(Change::Experience(Bank::Scene, vec![4, 4]))?;
        r.mark_message(0)?;
        r.mark_message(31)?;
        r.mark_message(32)?;
        r.change(registers::assignment(6, b"0,77,0,22,*,")?)?;
        r.checkpoint_v1(9, b"gallery", &draw, &audio.scene, [true, true])?;
        assert_eq!(r.read(Value::Local(2))?, 9);
        assert_eq!(r.text(1)?, b"gallery");
        assert_eq!(r.read(Value::Local(10))?, 13);
        assert_eq!(r.read(Value::Local(29))?, -1);
        assert_eq!(r.read(Value::Local(62))?, 2);
        assert_eq!(r.read(Value::Local(77))?, 22);
        let base = r.read(Value::System(16))?;
        for (i, n) in [32, 8, 16, 3].into_iter().enumerate() {
            assert_eq!(r.read(Value::System(base + i as i32))?, n);
        }
        assert_eq!(r.read(Value::System(base + 4 + 32 + 4))?, 2);
        assert_eq!(r.read(Value::System(base + 4 + 32 + 8 + 16))?, 0x01000080);
        assert_eq!(r.read(Value::System(base + 5 + 32 + 8 + 16))?, 128);
        Ok(())
    }
    #[test]
    fn checkpoint_replaces_only_the_two_native_view_bits() -> Result<()> {
        let mut r = fresh(1)?;
        r.set_normal(4, 0x97)?;
        for bits in 0..4 {
            r.checkpoint_v1(1, b"", &[0; 19], &[0; 12], [bits & 1 != 0, bits & 2 != 0])?;
            assert_eq!(r.read(Value::Local(4))?, 0x94 | bits);
        }
        Ok(())
    }
    #[test]
    fn false_auto_numbered_block_lands_on_marker_without_entering_it() -> Result<()> {
        let mut bytes = b"\0DLR".to_vec();
        for n in [3u32, 12, 3] {
            bytes.extend(n.to_le_bytes());
        }
        bytes.extend(b"\x05\0\0\x21-1,64,-1,false,0,0,title\0");
        bytes.extend(b"\x18\0\0\0\x05\0\0\x212,1,-1,*,0,0,marker\0");
        let mut script = Script::new(bytes, 3)?;
        let mut regs = fresh(3)?;
        let mut flag = false;
        for _ in 0..2 {
            let Action::Block(b) = script.next()? else {
                bail!("expected block")
            };
            script.visit_block(b, &mut regs, &mut flag, &[0; 19], &[0; 12], [false; 2])?;
        }
        assert_eq!(script.remaining, 0);
        assert!(flag);
        assert_eq!(script.block, 1);
        let page = midori_status_capture::MaskedNormal::authored_block_flag(flag);
        assert_eq!((page.register, page.mask, page.bits), (4, 0x80, 0x80));
        assert_eq!(regs.read(Value::Local(4))?, 0);
        assert!(regs.text(1)?.is_empty());
        Ok(())
    }
    #[test]
    fn text_assignment_order_and_cp932_trail_bytes_are_preserved() -> Result<()> {
        let mut r = fresh(1)?;
        for s in [
            b"2,8,0,\x81\\,*,",
            b"2,8,1,A,*,".as_slice(),
            b"2,8,2,B,*,",
            b"2,8,3,\\,*,!",
        ] {
            r.change(registers::assignment_for_abi(10, s, 1)?)?;
        }
        assert_eq!(r.text(8)?, b"B\x81\\A");
        Ok(())
    }
}
