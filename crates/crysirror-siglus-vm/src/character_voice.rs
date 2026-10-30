// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native CHRKOE grouping/default policy; actual voice gains belong to Audio.
//! Reference: siglus_rs 5f358ef runtime/forms/syscom.rs config_default_chrkoe,
//! chrkoe_chara_numbers, chrkoe_route and SET/GET_CHARAKOE_* handlers.
use crate::runtime::{CommandContext, Value, forms::codes::syscom_op as op};
use anyhow::{Context, Result, ensure};

fn count(ctx: &CommandContext) -> usize {
    ctx.tables
        .gameexe
        .as_ref()
        .and_then(|c| c.get_usize("CHRKOE.CNT"))
        .unwrap_or(64)
        .min(256)
}
fn defaults(ctx: &CommandContext, index: usize) -> crate::runtime::globals::ConfigChrKoeState {
    let entry = ctx
        .tables
        .gameexe
        .as_ref()
        .and_then(|c| c.get_indexed_entry("CHRKOE", index));
    let get = |item| {
        entry
            .and_then(|e| e.item_unquoted(item))
            .and_then(|s| s.parse::<i64>().ok())
    };
    crate::runtime::globals::ConfigChrKoeState {
        onoff: get(3).is_none_or(|v| v != 0),
        volume: get(4).unwrap_or(255).clamp(0, 255),
    }
}
pub(crate) fn initialize(ctx: &mut CommandContext) {
    ctx.globals.syscom.original_config.chrkoe =
        (0..count(ctx)).map(|index| defaults(ctx, index)).collect();
}
pub(crate) fn routing(ctx: &CommandContext, chara: i64) -> Result<(bool, u8)> {
    if chara < 0 {
        return Ok((true, 255));
    }
    let mut enabled = true;
    let mut gain = 255i64;
    if let Some(cfg) = &ctx.tables.gameexe {
        for index in 0..count(ctx) {
            let Some(entry) = cfg.get_indexed_entry("CHRKOE", index) else {
                continue;
            };
            ensure!(
                entry.value.len() <= 8192,
                "native character group list budget"
            );
            let Some(open) = entry.value.find('(') else {
                continue;
            };
            let close = entry
                .value
                .rfind(')')
                .context("native character group closing parenthesis")?;
            ensure!(close > open, "native character group list");
            let list = entry.value[open + 1..close]
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            ensure!(list.len() <= 256, "native character group member budget");
            let mut matched = false;
            for item in list {
                let value = if let Some(hex) = item.strip_prefix("0x") {
                    i64::from_str_radix(hex, 16)?
                } else {
                    item.parse()?
                };
                matched |= value == chara;
            }
            if matched {
                let state = ctx
                    .globals
                    .syscom
                    .original_config
                    .chrkoe
                    .get(index)
                    .copied()
                    .unwrap_or_else(|| defaults(ctx, index));
                enabled &= state.onoff;
                gain = gain * state.volume.clamp(0, 255) / 255;
            }
        }
    }
    Ok((enabled, gain as u8))
}
pub(crate) fn voice_gain(ctx: &CommandContext) -> Result<f32> {
    let (enabled, gain) = routing(ctx, ctx.globals.sound_routing.koe_chara_no)?;
    Ok(if !enabled && !ctx.globals.sound_routing.koe_ex_flag {
        0.
    } else {
        f32::from(gain) / 255.
    })
}
pub(crate) fn refresh(ctx: &mut CommandContext) -> Result<()> {
    if ctx.koe.current_no() < 0 || !ctx.koe.is_playing_any() {
        return Ok(());
    }
    let (enabled, gain) = routing(ctx, ctx.globals.sound_routing.koe_chara_no)?;
    if !enabled && !ctx.globals.sound_routing.koe_ex_flag {
        ctx.koe.stop(None)?;
    } else {
        ctx.koe.set_character_gain(gain)?;
    }
    Ok(())
}
pub(crate) fn dispatch(ctx: &mut CommandContext, command: i32, args: &[Value]) -> Result<bool> {
    let (volume, action) = match command {
        op::SET_CHARAKOE_ONOFF => (false, 0),
        op::SET_CHARAKOE_ONOFF_DEFAULT => (false, 1),
        op::GET_CHARAKOE_ONOFF => (false, 2),
        op::SET_CHARAKOE_VOLUME => (true, 0),
        op::SET_CHARAKOE_VOLUME_DEFAULT => (true, 1),
        op::GET_CHARAKOE_VOLUME => (true, 2),
        _ => return Ok(false),
    };
    ensure!(
        args.len() == if action == 0 { 2 } else { 1 },
        "native CHRKOE argument count"
    );
    let index = args[0].as_i64().context("native character group index")?;
    let count = count(ctx);
    let default = if (0..count as i64).contains(&index) {
        Some(defaults(ctx, index as usize))
    } else {
        None
    };
    if action == 2 {
        let state = usize::try_from(index)
            .ok()
            .and_then(|i| ctx.globals.syscom.original_config.chrkoe.get(i))
            .copied();
        ctx.push(Value::Int(state.map_or(0, |s| {
            if volume {
                s.volume.clamp(0, 255)
            } else {
                i64::from(s.onoff)
            }
        })));
    } else {
        let value = if action == 0 {
            args[1].as_i64().context("native character group value")?
        } else {
            0
        };
        while ctx.globals.syscom.original_config.chrkoe.len() < count {
            let next = defaults(ctx, ctx.globals.syscom.original_config.chrkoe.len());
            ctx.globals.syscom.original_config.chrkoe.push(next);
        }
        ctx.globals.syscom.original_config.chrkoe.truncate(count);
        if let Some(default) = default {
            let state = &mut ctx.globals.syscom.original_config.chrkoe[index as usize];
            if volume {
                state.volume = if action == 1 {
                    default.volume
                } else {
                    value.clamp(0, 255)
                };
            } else {
                state.onoff = if action == 1 {
                    default.onoff
                } else {
                    value != 0
                };
            }
        }
        refresh(ctx)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn character_groups_multiply_and_live_gain_is_an_engine_audio_request() {
        let mut ctx = CommandContext::new("asset".into());
        ctx.tables.gameexe = Some(std::sync::Arc::new(
            crate::gameexe::GameexeConfig::from_text(
                "#CHRKOE.CNT=2\n#CHRKOE.000=\"A\",0,\"\",1,128,(7)\n#CHRKOE.001=\"B\",0,\"\",1,64,(7,8)\n",
            ),
        ));
        initialize(&mut ctx);
        assert_eq!(routing(&ctx, 7).unwrap(), (true, 32));
        assert_eq!(routing(&ctx, 8).unwrap(), (true, 64));
        assert_eq!(routing(&ctx, -1).unwrap(), (true, 255));
        ctx.globals.sound_routing.koe_chara_no = 7;
        ctx.koe.play(1, 100).unwrap();
        refresh(&mut ctx).unwrap();
        assert!(std::mem::take(&mut ctx.koe.pending).entries().iter().any(|(_,op)|matches!(op,crate::bgm_bridge::Operation::Control(crysirror_core::service::AudioRequest::SetGain {channel:3,gain,..}) if (*gain-32./255.).abs()<1e-6)));
        dispatch(
            &mut ctx,
            op::SET_CHARAKOE_ONOFF,
            &[Value::Int(1), Value::Int(0)],
        )
        .unwrap();
        assert!(
            !ctx.koe.is_playing_any(),
            "normal disabled character stops its shared voice"
        );
        assert_eq!(voice_gain(&ctx).unwrap(), 0.);
        ctx.globals.sound_routing.koe_ex_flag = true;
        assert!(
            (voice_gain(&ctx).unwrap() - 32. / 255.).abs() < 1e-6,
            "EXKOE ignores on/off but retains group volume"
        );
        dispatch(
            &mut ctx,
            op::SET_CHARAKOE_VOLUME,
            &[Value::Int(0), Value::Int(999)],
        )
        .unwrap();
        assert_eq!(ctx.globals.syscom.original_config.chrkoe[0].volume, 255);
        dispatch(&mut ctx, op::SET_CHARAKOE_VOLUME_DEFAULT, &[Value::Int(0)]).unwrap();
        assert_eq!(ctx.globals.syscom.original_config.chrkoe[0].volume, 128);
    }
}
