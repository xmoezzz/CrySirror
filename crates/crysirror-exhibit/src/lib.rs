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
mod midori_state;
mod midori_music;
mod midori_sound_capture;
mod midori_status_capture;
mod midori_ambient;
mod midori_actor;
mod midori_actor_image;
mod midori_color_filter;
mod midori_background;
mod midori_message;
mod midori_positions;
mod midori_prelude;
mod midori_control_parts;
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
#[cfg(test)]
mod v1_census;
#[cfg(test)]
mod v3_census;
#[cfg(test)]
mod scenario_load_tests;
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
    abi: u32,
    #[serde(default)]
    sources: Option<profile::Sources>,
    #[serde(default)]
    limits: Option<registers::Limits>,
    #[serde(default = "slot_index::default_counts")]
    slots: [usize; 3],
    canvas: [u32; 2],
}

struct Script {
    bytes: Vec<u8>,
    pc: usize,
    remaining: u32,
    abi: u32,
    start: Position,
    labels: Option<BTreeMap<i32, Position>>,
    returns: Vec<(Position, i32)>,
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
            start: Position { pc, remaining },
            labels: None,
            returns: vec![],
            block: 0,
        })
    }
    fn branch(&mut self, op: u16, target: i32) -> Result<()> {
        if op == 0x17 {
            let (frame, block) = self.returns.pop().context("RLD return without call")?;
            (self.pc, self.remaining, self.block) = (frame.pc, frame.remaining, block);
            return Ok(());
        }
        if self.labels.is_none() {
            self.labels = Some(branch::labels(&self.bytes, self.start, self.abi)?);
        }
        let to = self
            .labels
            .as_ref()
            .and_then(|l| l.get(&target))
            .with_context(|| format!("unknown RLD block {target}"))?;
        if op == 0x16 {
            ensure!(self.returns.len() < 65536, "RLD call stack limit");
            self.returns.push((
                Position {
                    pc: self.pc,
                    remaining: self.remaining,
                },
                self.block,
            ));
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
        ensure!(
            self.abi == 1
                || matches!(
                    op,
                    0x05..=0x08
                        | 0x0a
                        | 0x0b
                        | 0x0d
                        | 0x12
                        | 0x14
                        | 0x16
                        | 0x17
                        | 0x18
                        | 0x2c | 0x2f
                        | 0x30..=0x35 | 0xfb..=0xfd
                        | 0x38..=0x3a
                        | 0x3d
                        | 0x3e
                        | 0x46 | 0x47
                        | 0x48
                        | 0x49
                        | 0x4a
                        | 0x50
                        | 0x6c | 0x6d
                        | 0x8c
                        | 0xa7
                        | 0xbd
                        | 0xbf
                ),
            "unsupported ExHibit v{} execution opcode {op:#06x}",
            self.abi
        );
        let action = match (op, flags, values.as_slice()) {
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
                Action::DefineMidoriActorSlot(midori_actor::Definition::parse(raw)?)
            }
            (0x46, 1, []) => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Environment(environment::Definition::parse(raw)?)
            }
            (0xbf, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                if raw.starts_with(b"338,") {
                    Action::MidoriControlSounds(midori_control_parts::SoundUpdate::parse(raw)?)
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
                Action::DefineMidoriDebug(midori_prelude::DebugDefinition::parse(raw)?)
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
                if raw.starts_with(b"60,") {
                    Action::DefineMidoriMarker(midori_prelude::MarkerDefinition::parse(raw)?)
                } else if raw.starts_with(b"57,") {
                    Action::DefineMidoriControlPart(midori_control_parts::Part::parse(raw)?)
                } else {
                    Action::MidoriEntryName(midori_state::EntryAssignment::parse(raw)?)
                }
            }
            (0x0b, 1, []) if self.abi == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                registers::syscall(raw)?
            }
            (0x3d, 1, [table]) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Change(Change::MidoriDefineDb(midori_state::DbDefinition::parse(
                    *table, raw,
                )?))
            }
            (0x3e | 0xbd, 1, []) if self.abi == 3 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Change(if op == 0x3e {
                    Change::MidoriGetDb(midori_state::DbLookup::parse(raw)?)
                } else {
                    Change::MidoriSceneDone(midori_state::SceneDone::parse(raw)?)
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
                [id, 9, 0, ms, 0, 0, 0, x, y, w, h, -1, -1, flags, 0]
                | [id, 9, 0, ms, 0, 0, 0, x, y, w, h, -1, -1, flags, 0, 0],
            ) if *id >= 0
                && (0..=600_000).contains(ms)
                && matches!(flags, 0 | 65536)
                && [*x, *y, *w, *h].into_iter().all(|v| v >= 0) =>
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
            (0x12, 0, [1, _]) if self.abi == 1 => Action::OpenMessageBackdrop,
            (0x48, 0, [id]) if self.abi == 1 && (0..=6).contains(id) => {
                Action::MessageFrame(*id as u8)
            }
            (0x49, 1, []) if self.abi == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::DefineMessageArea(message_area::Area::parse(raw)?)
            }
            (0x4e, 0, [1200, 0x30001, 400, 0, 83]) if self.abi == 1 => Action::TitleSceneFade,
            (0x52, 0, [1, 1, _, _, 0, _] | [1, 1, _, _, 0, _, _, _, _, _]) if self.abi == 1 => {
                Action::LeaveCompletedClip
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
            (0x5f, 0, [slot, 0]) if self.abi == 1 && (0..127).contains(slot) => {
                Action::RemoveSurface(*slot as u8)
            }
            (0x5e, 0x11 | 0x21, []) if self.abi == 1 => {
                let (raw, end) = cstring(&self.bytes, at)?;
                at = end;
                Action::Surface(surfaces::Surface::image_csv(raw, flags == 0x21)?)
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
                    format!("res/g/ev/{:02}/{id:04}.mpg", id / 100)
                };
                Action::Movie(movie::Movie {
                    path,
                    sound: *sound,
                    rect: [*x, *y, *w, *h],
                })
            }
            (0x72, 0, [wait, fade]) => Action::LeaveMovie(*wait != 0, *fade),
            (0x76, 0, [0, 1239, 4, 0, 0, -1, -1, -2, -1]) if self.abi == 1 => Action::PrepareOpeningDlt,
            (0x77, 0, [0, 16, 800, -1]) if self.abi == 1 => Action::StartOpeningDlt,
            (0x1c, 2, values) if self.abi == 1 && values == printer::FIRST_WORDS => {
                let (speaker, end) = cstring(&self.bytes, at)?;
                let (body, _) = cstring(&self.bytes, end)?;
                let _checked = printer::OrdinarySay::decode(speaker, body)?;
                // Keep PC and operand count unchanged until the linked native
                // character-voice, history and hit-wait behavior can commit.
                bail!("unsupported ExHibit opening printer voice/history/hit-wait")
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
            (0x75, 0, [0, 800, 0]) if self.abi == 1 => Action::StopOpeningDlt,
            (0x18 | 0x8c, 0, []) => Action::End,
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
    Draw(i32, u64, [i32; 4], bool),
    TitleSceneFade,
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
    MidoriMusic(midori_music::Track),
    StopMidoriMusic(u32),
    MidoriEntryName(midori_state::EntryAssignment),
    DefineMidoriDebug(midori_prelude::DebugDefinition),
    DefineMidoriMarker(midori_prelude::MarkerDefinition),
    DefineMidoriControlPart(midori_control_parts::Part),
    DefineMidoriControlParts(midori_control_parts::Parts),
    DefineMidoriShortcut(midori_shortcuts::Define),
    MidoriControlSounds(midori_control_parts::SoundUpdate),
    LocalControlScope(bool),
    MessageFrame(u8),
    OpenMessageBackdrop,
    DefineMessageArea(message_area::Area),
    DefineMidoriArea(midori_message::AreaDefinition),
    SelectMidoriFrame,
    ChangeMidoriVisibility(midori_message::Visibility),
    Billboard(billboard::Billboard),
    Surface(surfaces::Surface),
    SelectItem(select_items::Button),
    WaitForSelection(u8, i32),
    ExitSelection(u8),
    RemoveSurface(u8),
    DisableButton(i32, i32),
    ControlInput(i32, bool, bool),
    LeaveCompletedClip,
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
    Block(branch::Block),
    Branch(u16, i32, Condition),
    Label,
    Movie(movie::Movie),
    LeaveMovie(bool, i32),
    PrepareOpeningDlt,
    StartOpeningDlt,
    StopOpeningDlt,
    End,
}

#[derive(Default)]
struct Controls {
    skip: bool,
    auto: bool,
}
impl Controls {
    fn delay(&self, ms: u64) -> u64 {
        if self.skip { 0 } else { ms }
    }
}
// Keep the capability path that actually produced the accepted script. A
// queued transfer name is not yet the resident's current scenario (T2).
struct ScenarioLoad {
    script: Script,
    seeds: Option<profile::Seeds>,
    path: VfsPath,
}

struct Vm {
    client: ClientId,
    name: String,
    canvas: [u32; 2],
    script: Option<Script>,
    worker: TaskPool,
    load: Option<Task<ScenarioLoad>>,
    abi: u32,
    sources: Option<profile::Sources>,
    seeds: Option<profile::Seeds>,
    deadline: u64,
    movie: Option<movie::Playing>,
    opening_dlt: Option<opening_dlt::OpeningDlt>,
    leaving: Option<(bool, i32)>,
    halted: bool,
    checkpoint_flag: bool,
    slots: [usize; 3],
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
    scene_fade: Option<scene_fade::SceneFade>,
    message_areas: message_area::Catalog,
    selected_message_area: Option<u8>,
    message_backdrop: message_area::Backdrop,
    local_controls: local_controls::Toolbar,
    characters: character::Catalog,
    midori_entries: midori_state::EntryNames,
    midori_ambient: midori_ambient::Ambient,
    midori_music: midori_music::Music,
    midori_actor_slots: midori_actor::Slots,
    midori_background: midori_background::Background,
    midori_areas: midori_message::Catalog,
    midori_positions: Option<midori_positions::Work>,
    midori_debug: Option<midori_prelude::DebugDefinition>,
    midori_marker: midori_prelude::Marker,
    midori_control_defs: midori_control_parts::Definitions,
    midori_shortcuts: midori_shortcuts::Catalog,
    pending_message_area: Option<(u8, Task<message_area::Catalog>)>,
    registers: Registers,
    audio: sound::Audio,
}
impl Vm {
    fn request_transferred_script(&mut self, name: String) {
        self.registers.enter_transferred_script();
        self.name = name;
        self.script = None;
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
            delta_ready = delta.present(h, self.client, &self.worker)?;
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
        if let Some(fade) = &mut self.scene_fade {
            if !fade.present(h, self.client, self.canvas, &mut self.audio, &mut self.draw_scene)? {
                return Ok(VmYield::WaitUntil(clock.saturating_add(1)));
            }
            self.scene_fade = None;
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
        if let Some((id, task)) = &mut self.pending_message_area {
            let Some(catalog) = task.try_take()? else {
                return Ok(VmYield::BudgetExhausted);
            };
            let id = *id;
            self.message_areas.merge_missing(catalog);
            self.message_areas.attach_inactive(id, h, self.client, &mut self.registers)?;
            self.selected_message_area = Some(id);
            self.pending_message_area = None;
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
        if let Some((wait, fade)) = self.leaving {
            if let Some(movie) = self.movie.as_mut() {
                if let Some(until) = movie.leave(self.client, h, wait, fade)? {
                    return Ok(VmYield::WaitUntil(until));
                }
            }
            self.movie = None;
            self.leaving = None;
        }
        if self.script.is_none() {
            if self.load.is_none() {
                let storage = h.storage(self.client)?;
                let path = script_path(&self.name)?;
                let (abi, sources, mut seeds, name) = (
                    self.abi,
                    self.sources.clone(),
                    self.seeds,
                    self.name.clone(),
                );
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
                    Ok(ScenarioLoad { script: Script::new(bytes, abi)?, seeds, path })
                })?);
            }
            if let Some(ScenarioLoad { script, seeds, path }) = self
                .load
                .as_mut()
                .context("missing scenario task")?
                .try_take()?
            {
                if script.abi == 3 {
                    self.registers.midori_scenario_loaded(&path)?;
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
                Action::DefineMidoriAmbient(def) => {
                    self.midori_ambient.define(def);
                    0
                }
                Action::MidoriMusic(track) => {
                    if self.midori_music.play(h, self.client, &self.registers, track, self.controls.skip)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))? {
                        return Ok(VmYield::BudgetExhausted);
                    }
                    0
                }
                Action::StopMidoriMusic(fade) => {
                    if self.midori_music.stop(h, self.client, fade, self.controls.skip)? {
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
                    self.midori_positions.as_mut().context("missing Midori local position work")?.apply(cmd);
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
                    ensure!(self.name.eq_ignore_ascii_case("def"), "unsupported Midori actor slot redefinition outside def");
                    self.midori_actor_slots.define(def)?;
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
                Action::Surface(surface) => {
                    ensure!(surface.slot != opening_dlt::SLOT || self.opening_dlt.is_none(),
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
                Action::ExitSelection(group) => {
                    self.panels.exit_group(group);
                    0
                }
                Action::RemoveSurface(slot) => {
                    ensure!(slot != opening_dlt::SLOT || self.opening_dlt.is_none(),
                        "ExHibit removal conflicts with opening delta layer");
                    self.panels
                        .remove(h, self.client, slot)
                        .with_context(|| format!("ExHibit {}.rld+{pc:#x}", self.name))?;
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
                Action::DefineMidoriDebug(definition) => {
                    self.midori_debug = Some(definition);
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
                    ensure!(!self.message_backdrop.opened || self.selected_message_area == Some(id),
                        "unsupported ExHibit active message area replacement");
                    if self.selected_message_area == Some(id) {
                        self.registers.set_normal(77, i32::from(id))?;
                    } else if self.message_areas.has(id) {
                        self.message_areas.attach_inactive(id, h, self.client, &mut self.registers)?;
                        self.selected_message_area = Some(id);
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
                Action::TitleSceneFade => {
                    ensure!(
                        self.name.eq_ignore_ascii_case("title") && self.last_selected_item == Some(10)
                            && !self.controls.skip && self.panels.scene_empty()
                            && !self.animations.live() && self.movie.is_none()
                            && self.clip.is_none() && self.billboard_fade.is_none()
                            && self.selected_message_area.is_none()
                            && self.draw_scene[0] == 1246,
                        "unsupported ExHibit title scene state at {}.rld+{pc:#x}",
                        self.name
                    );
                    self.magnify_target = None;
                    self.scene_fade = Some(scene_fade::SceneFade::begin(
                        h, self.client, self.canvas, &self.audio,
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
                    let asset = format!("res/g/ev/{:02}/{id:04}.gyu", id / 100);
                    let [w, hgt] = self.canvas;
                    let uv = if crop[2] > 0 {
                        let source = h.image_size(self.client, &asset)?;
                        clip_flag = if retain_clip && source != [16, 16] { 0x10000 } else { 0 };
                        Some(clip::draw_uv(crop, source, self.canvas, opening)?)
                    } else {
                        None
                    };
                    h.submit_engine(
                        self.client,
                        1,
                        &serde_json::to_vec(&clip::draw_commands(&asset, self.canvas, uv, ms))?,
                    )?;
                    self.clip = None;
                    self.magnify_target = None;
                    self.draw_scene[0] = id;
                    self.draw_scene[1] = clip_flag;
                    // 0x32580 snapshots the authored rectangle, not the
                    // intersected copy geometry or the released backup rect.
                    self.draw_scene[2..6].copy_from_slice(&crop);
                    self.draw_scene[11] = (w | (hgt << 16)) as i32;
                    if self.registers.configured() {
                        self.registers
                            .change(Change::Experience(registers::Bank::Cg, vec![id]))?;
                    }
                    ms
                }
                Action::Wait(ms) => self.controls.delay(ms),
                Action::PrepareOpeningDlt => {
                    ensure!(self.name.eq_ignore_ascii_case("F00A1101_2") && !self.controls.skip
                        && self.opening_dlt.is_none() && !self.panels.is_bound(opening_dlt::SLOT),
                        "unsupported ExHibit opening delta state");
                    self.opening_dlt = Some(opening_dlt::OpeningDlt::prepare(h, self.client, &self.worker)?);
                    if self.registers.configured() {
                        self.registers.change(Change::Experience(registers::Bank::Cg, vec![opening_dlt::ID]))?;
                    }
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::StartOpeningDlt => {
                    ensure!(!self.controls.skip, "unsupported ExHibit skip during opening delta");
                    self.opening_dlt.as_mut().context("missing ExHibit opening delta")?
                        .start(h, self.client, &self.worker)?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::StopOpeningDlt => {
                    ensure!(!self.controls.skip, "unsupported ExHibit skip during opening delta");
                    self.opening_dlt.as_mut().context("missing ExHibit opening delta")?.stop(now)?;
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::Movie(movie) => {
                    self.movie = Some(movie::Playing::start(self.client, h, self.canvas, movie)?);
                    0
                }
                Action::LeaveMovie(wait, fade) => {
                    self.leaving = Some((wait, fade));
                    return Ok(VmYield::BudgetExhausted);
                }
                Action::End => {
                    ensure!(self.opening_dlt.is_none(), "unsupported ExHibit end with live delta");
                    if script.abi == 3 {
                        self.registers.set_normal(6, 0)?;
                    }
                    self.halted = true;
                    return Ok(VmYield::Halted);
                }
                Action::Label => 0,
                Action::Block(block) => {
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
        let midori_audio_candidate = if self.abi == 3 && self.seeds.is_some() && self.script.is_some() {
            midori_sound_capture::SoundCapture::from_owners(&self.midori_music, &self.midori_ambient)
                .map(|capture| json!({"normal":capture.normal, "text":capture.text}))
        } else {
            None
        };
        let midori_status_candidate = if self.abi == 3 && self.seeds.is_some() && self.script.is_some() {
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
        let midori_filter_candidate = if self.abi == 3 && self.seeds.is_some() && self.script.is_some() {
            self.registers.text(4).ok().and_then(|text| self.midori_actor_slots.capture_filter(text))
        } else {
            None
        };
        serde_json::to_vec(&json!({
            "abi": self.abi,
            "scenario": self.name,
            "record_offset": self.script.as_ref().map(|script| script.pc),
            "waiting_for_selection": self.selection.is_some(),
            "last_selected_item": self.last_selected_item,
            "scene_fade_in_progress": self.scene_fade.is_some(),
            "message_area": self.selected_message_area,
            "message_backdrop_open": self.message_backdrop.opened,
            "local_controls_open": self.local_controls.open(),
            "opening_dlt_live": self.opening_dlt.is_some(),
            "authored_characters": self.characters.len(),
            "authored_body_variants": self.characters.body_count(),
            "midori_entry_names": self.midori_entries.count(),
            "midori_area_geometry": self.midori_areas.geometries(),
            "midori_installed_frame": self.midori_areas.installed(),
            "midori_bgm_requested_and_audible": self.midori_music.ids(),
            "midori_audio_checkpoint_candidate": midori_audio_candidate,
            "midori_status_checkpoint_candidate": midori_status_candidate,
            "midori_filter_checkpoint_candidate": midori_filter_candidate,
            "midori_background_checkpoint_candidate": self.midori_background.capture(),
            "midori_background_crop": self.midori_background.current_crop(),
            "midori_debug_definition": self.midori_debug.as_ref().map(|state| json!({"mode":state.mode,"pairs":state.pairs})),
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
        }))
        .unwrap_or_default()
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
        Ok(Vm {
            client,
            name: config.entry,
            canvas: config.canvas,
            script: None,
            worker: TaskPool::new("exhibit-rld", 1, 1)?,
            load: None,
            deadline: 0,
            abi: config.abi,
            sources: config.sources,
            seeds: None,
            movie: None,
            opening_dlt: None,
            leaving: None,
            halted: false,
            checkpoint_flag: false,
            slots: config.slots,
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
            message_areas: Default::default(),
            selected_message_area: None,
            message_backdrop: Default::default(),
            local_controls: Default::default(),
            midori_entries: Default::default(),
            midori_ambient: Default::default(),
            midori_music: Default::default(),
            midori_actor_slots: midori_actor::Slots::for_view(config.canvas),
            midori_background: Default::default(),
            midori_areas: Default::default(),
            midori_positions: None,
            midori_debug: None,
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
            registers: Registers::for_scenario(config.limits, config.abi, config.canvas)?,
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
    fn shape_checked_opening_say_does_not_advance_an_unimplemented_operation() -> Result<()> {
        let mut record = vec![0x1c, 0, 7, 2];
        for value in printer::FIRST_WORDS {
            record.extend(value.to_le_bytes());
        }
        record.extend([0x8b, 0x4c, 0x8f, 0x71, 0, 0x96, 0x7b, 0]);
        let mut script = fixture(&record, 1)?;
        let before = (script.pc, script.remaining);
        let error = script.next().expect_err("staged printer does not admit native say");
        assert!(error.to_string().contains("voice/history/hit-wait"));
        assert_eq!((script.pc, script.remaining), before);
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
    pub(super) fn fresh(abi: u32) -> Result<Registers> {
        Registers::for_scenario(
            Some(Limits {
                regs: 2048,
                sys: 4096,
                cg: 32,
                scene: 8,
                sound: 16,
                strings: 256,
                characters: 34,
                locals: 100,
                messages: 65,
            }),
            abi,
            [1024, 576],
        )
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
