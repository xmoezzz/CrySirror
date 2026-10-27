// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Original event declarations, waits and channel policy. Timing, random draws
//! and dependency cursors belong to CrySirror's shared sequence service.
use crate::runtime::{
    CommandContext, Value, constants as c,
    globals::{
        PCM_EVENT_TYPE_LOOP, PCM_EVENT_TYPE_ONESHOT, PCM_EVENT_TYPE_RANDOM, PcmEventLine,
        PcmEventState,
    },
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::service::*;
pub(crate) const LIMIT: usize = 128;
fn named(args: &[Value], id: i32, default: i64) -> i64 {
    args.iter()
        .find_map(|v| match v {
            Value::NamedArg { id: n, value } if *n == id => value.as_i64(),
            _ => None,
        })
        .unwrap_or(default)
}
pub(crate) fn spec(st: &PcmEventState) -> Result<AudioSequenceSpec> {
    let mode = match st.event_type {
        PCM_EVENT_TYPE_ONESHOT => AudioSequenceMode::Once,
        PCM_EVENT_TYPE_LOOP => AudioSequenceMode::Loop,
        PCM_EVENT_TYPE_RANDOM => AudioSequenceMode::Weighted,
        _ => bail!("inactive/unknown Siglus PCMEVENT type"),
    };
    ensure!(
        st.pcm_buf_no >= -1,
        "unsupported Siglus PCMEVENT negative sound target"
    );
    for l in &st.lines {
        crate::sfx_bridge::key(&l.file_name)?;
    }
    let spec = AudioSequenceSpec {
        mode,
        clock: if st.real_flag {
            AudioSequenceClock::Real
        } else {
            AudioSequenceClock::Game
        },
        timing: AudioSequenceTiming::Reset,
        random: AudioSequenceRandom::Modulo32,
        soften_repeat: true,
        include_duration: st.time_type,
        lines: st
            .lines
            .iter()
            .map(|l| AudioSequenceLine {
                key: l.file_name.clone(),
                weight: l.probability.max(0) as u32,
                min_ms: i64::from(l.min_time),
                max_ms: i64::from(l.max_time),
            })
            .collect(),
    };
    spec.validate()?;
    Ok(spec)
}
pub(crate) fn dispatch(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let chain = ctx
        .vm_call
        .as_ref()
        .context("Siglus PCMEVENT call metadata missing")?
        .element
        .clone();
    ensure!(
        (3..=4).contains(&chain.len()) && chain[1] == ctx.ids.elm_array,
        "unsupported Siglus PCMEVENT element chain"
    );
    let index = chain[2].max(0) as usize;
    ensure!(index < LIMIT, "Siglus PCMEVENT index exceeds 128");
    let form = ctx.ids.form_global_pcm_event;
    let list = ctx.globals.pcm_event_lists.entry(form).or_default();
    if list.len() <= index {
        list.resize_with(index + 1, PcmEventState::default);
    }
    if chain.len() == 3 {
        return Ok(true);
    }
    let op = chain[3];
    match op {
        c::PCMEVENT_START_ONESHOT | c::PCMEVENT_START_LOOP | c::PCMEVENT_START_RANDOM => {
            for v in args {
                if let Value::NamedArg { id, value } = v {
                    ensure!(
                        matches!(id, 3 | 4 | 5 | 6 | 11 | 12) && value.as_i64().is_some(),
                        "unsupported Siglus PCMEVENT named argument {id}"
                    );
                }
            }
            let random = op == c::PCMEVENT_START_RANDOM;
            let mut lines = vec![];
            for v in args {
                let (key, numbers) = match v {
                    Value::Str(s) => (s.as_str(), &[][..]),
                    Value::List(v) => {
                        let Some(k) = v.first().and_then(Value::as_str).filter(|s| !s.is_empty())
                        else {
                            continue;
                        };
                        (k, &v[1..])
                    }
                    _ => continue,
                };
                let get =
                    |i: usize, d: i64| numbers.get(i).and_then(Value::as_i64).unwrap_or(d) as i32;
                let probability = if random { get(0, 1) } else { 0 };
                let offset = usize::from(random);
                let min_time = get(offset, 0);
                let max_time = get(offset + 1, i64::from(min_time));
                lines.push(PcmEventLine {
                    file_name: key.into(),
                    probability,
                    min_time,
                    max_time,
                });
            }
            let mut st = PcmEventState::default();
            st.lines = lines;
            st.start(
                if random {
                    PCM_EVENT_TYPE_RANDOM
                } else if op == c::PCMEVENT_START_LOOP {
                    PCM_EVENT_TYPE_LOOP
                } else {
                    PCM_EVENT_TYPE_ONESHOT
                },
                args.first()
                    .and_then(|v| match v {
                        Value::Int(n) => Some(*n as i32),
                        _ => None,
                    })
                    .unwrap_or(-1),
                named(args, 3, 2).clamp(-1, 31) as i32,
                named(args, 6, -1) as i32,
                named(args, 4, 0) != 0,
                named(args, 5, 0) != 0,
                named(args, 12, 0) != 0,
                true,
                named(args, 11, 0) != 0,
            );
            // Original out-of-range volume category falls back to PCM, not a clamp.
            let v = named(args, 3, 2);
            st.volume_type = if (-1..32).contains(&v) { v as i32 } else { 2 };
            spec(&st)?;
            if st.pcm_buf_no >= 0 {
                crate::pcmch_bridge::check_event_channel(ctx, st.pcm_buf_no as usize)?;
            }
            ctx.globals.pcm_event_lists.get_mut(&form).unwrap()[index] = st;
            ctx.bgm
                .pending
                .push(crate::bgm_bridge::Operation::EventInvalidate { form, index })?;
        }
        c::PCMEVENT_STOP => {
            let st = ctx.globals.pcm_event_lists[&form][index].clone();
            if args.first().and_then(Value::as_i64).unwrap_or(0) != 0 {
                if st.pcm_buf_no >= 0 {
                    crate::pcmch_bridge::stop_event_channel(ctx, st.pcm_buf_no as usize)?;
                } else if st.is_active() {
                    ctx.pcm.stop(None)?;
                }
            }
            ctx.globals.pcm_event_lists.get_mut(&form).unwrap()[index].reinit();
            ctx.bgm
                .pending
                .push(crate::bgm_bridge::Operation::EventInvalidate { form, index })?;
        }
        c::PCMEVENT_CHECK => ctx.push(Value::Int(i64::from(
            ctx.globals.pcm_event_lists[&form][index].is_active(),
        ))),
        c::PCMEVENT_WAIT | c::PCMEVENT_WAIT_KEY => {
            if ctx.globals.pcm_event_lists[&form][index].event_type == PCM_EVENT_TYPE_ONESHOT {
                let key = op == c::PCMEVENT_WAIT_KEY;
                ctx.wait.wait_pcm_event(form, index, key, key);
            }
        }
        _ => bail!("unsupported Siglus PCMEVENT opcode {op}"),
    }
    Ok(true)
}
pub(crate) fn play(ctx: &mut CommandContext, st: &PcmEventState, key: &str) -> Result<u32> {
    if st.pcm_buf_no < 0 {
        ctx.pcm.play_event(key)?;
        Ok(crate::sfx_bridge::PCM_CHANNEL)
    } else {
        let ch = st.pcm_buf_no as usize;
        crate::pcmch_bridge::play_event(ctx, ch, key, st)?;
        Ok(crate::pcmch_bridge::channel(ch))
    }
}
