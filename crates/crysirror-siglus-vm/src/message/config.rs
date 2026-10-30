// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded native Gameexe policy, never a substitute renderer or font engine.
use super::*;
use crate::gameexe::GameexeConfig;

pub(super) fn tuple<const N: usize>(value: Option<&str>, fallback: [i32; N]) -> Result<[i32; N]> {
    let Some(value) = value else {
        return Ok(fallback);
    };
    let parts = value
        .split(|c: char| c == ',' || c.is_ascii_whitespace() || c == '(' || c == ')')
        .filter(|s| !s.is_empty())
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    parts
        .try_into()
        .map_err(|_| anyhow::anyhow!("native MWND configuration tuple requires {N} integers"))
}
pub(super) fn scalar(value: Option<&str>, fallback: i32) -> Result<i32> {
    value
        .map(|s| {
            s.trim()
                .trim_matches('"')
                .parse::<i32>()
                .map_err(Into::into)
        })
        .unwrap_or(Ok(fallback))
}
pub(super) fn color(cfg: &GameexeConfig, number: i32) -> Result<[u8; 4]> {
    ensure!((0..4096).contains(&number), "native MWND color index");
    let raw = cfg
        .get_indexed_value("COLOR_TABLE", number as usize)
        .or_else(|| cfg.get_indexed_field("COLOR_TABLE", number as usize, "RGB"))
        .or_else(|| cfg.get_indexed_field("COLOR_TABLE", number as usize, "COLOR"));
    let defaults = [
        [255, 255, 255],
        [0, 0, 0],
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [255, 255, 0],
        [255, 0, 255],
        [0, 255, 255],
    ];
    let rgb = tuple(
        raw,
        defaults.get(number as usize).copied().unwrap_or([255; 3]),
    )?;
    Ok([
        rgb[0].clamp(0, 255) as u8,
        rgb[1].clamp(0, 255) as u8,
        rgb[2].clamp(0, 255) as u8,
        255,
    ])
}
pub(crate) fn initialize(ctx: &mut CommandContext) -> Result<()> {
    if ctx.ui.window.is_some() {
        return Ok(());
    }
    let Some(cfg) = ctx.tables.gameexe.as_ref() else {
        return Ok(());
    };
    let get = |field| cfg.get_indexed_field("MWND", 0, field);
    // Do not turn an arbitrary configuration into a visually incomplete plain
    // profile. Each unsupported mechanism remains a named native error.
    for (key, fallback) in [("TATEYOKO_MODE", 0), ("MWND.DEFAULT_MWND_NO", 0)] {
        ensure!(
            scalar(cfg.get_unquoted(key), fallback)? == 0,
            "native Gameexe {key} requires an additional engine text profile"
        );
    }
    for field in [
        "NOVEL_MODE",
        "EXTEND_TYPE",
        "NAME_DISP_MODE",
        "NAME_BRACKET",
        "NAME_EXTEND_TYPE",
        "NAME_WINDOW_ALIGN",
    ] {
        ensure!(
            scalar(get(field), 0)? == 0,
            "native Gameexe MWND.000.{field} is not mapped by the horizontal message profile"
        );
    }
    super::decoration::validate(ctx)?;
    let waku = scalar(get("WAKU_NO"), 0)?;
    let mut waku_file = String::new();
    let mut filter_file = String::new();
    let mut filter_color = None;
    let mut filter_margin = [0; 4];
    let mut filter_config_color = false;
    let mut filter_config_tr = false;
    if waku >= 0 {
        let index = waku as usize;
        ensure!(
            scalar(cfg.get_indexed_field("WAKU", index, "EXTEND_TYPE"), 0)? == 0,
            "native extended WAKU layout is not mapped"
        );
        let field = |name| cfg.get_indexed_field("WAKU", index, name);
        waku_file = cfg
            .get_indexed_field_unquoted("WAKU", index, "WAKU_FILE")
            .unwrap_or("")
            .to_owned();
        filter_file = cfg
            .get_indexed_field_unquoted("WAKU", index, "FILTER_FILE")
            .unwrap_or("")
            .to_owned();
        filter_margin = tuple(field("FILTER_MARGIN"), [0; 4])?;
        if !waku_file.is_empty() || !filter_file.is_empty() || field("FILTER_COLOR").is_some() {
            let color =
                tuple(field("FILTER_COLOR"), [0, 0, 255, 128])?.map(|v| v.clamp(0, 255) as u8);
            filter_color = Some((color[0], color[1], color[2], color[3]));
            filter_config_color = scalar(field("FILTER_CONFIG_COLOR"), 1)? != 0;
            filter_config_tr = scalar(field("FILTER_CONFIG_TR"), 1)? != 0;
        }
    }
    let pos = tuple(get("WINDOW_POS"), [50, 400])?;
    let window_size = tuple(get("WINDOW_SIZE"), [700, 150])?;
    let open_type = scalar(get("OPEN_ANIME_TYPE"), 0)?;
    let open_time = scalar(get("OPEN_ANIME_TIME"), 0)?;
    let close_type = scalar(get("CLOSE_ANIME_TYPE"), 0)?;
    let close_time = scalar(get("CLOSE_ANIME_TIME"), 0)?;
    ensure!(
        (0..=5).contains(&open_type)
            && (0..=5).contains(&close_type)
            && open_time >= 0
            && close_time >= 0,
        "unsupported native Gameexe MWND animation"
    );
    let message = tuple(get("MESSAGE_POS"), [20, 20])?;
    let cnt = tuple(get("MOJI_CNT"), [26, 3])?;
    let size = scalar(get("MOJI_SIZE"), 25)?;
    let spacing = tuple(get("MOJI_SPACE"), [-1, 10])?;
    ensure!(
        (1..=512).contains(&size) && cnt.iter().all(|v| (1..=4096).contains(v)),
        "native MWND grid bounds"
    );
    let extent = [
        size.checked_mul(cnt[0])
            .and_then(|n| {
                spacing[0]
                    .checked_mul(cnt[0] - 1)
                    .and_then(|s| n.checked_add(s))
            })
            .context("MWND width overflow")?,
        size.checked_mul(cnt[1])
            .and_then(|n| {
                spacing[1]
                    .checked_mul(cnt[1] - 1)
                    .and_then(|s| n.checked_add(s))
            })
            .context("MWND height overflow")?,
    ];
    let name_pos = tuple(get("NAME_WINDOW_POS"), [0, -100])?;
    let name_msg = tuple(get("NAME_MESSAGE_POS"), [8, 8])?;
    let name_rep = tuple(get("NAME_MESSAGE_POS_REP"), [0, 0])?;
    let name_size = scalar(get("NAME_MOJI_SIZE"), 16)?;
    let mut name_bounds = tuple(get("NAME_WINDOW_SIZE"), [300, 100])?;
    let name_spacing = tuple(get("NAME_MOJI_SPACE"), [-1, 8])?;
    // A zero native name-window dimension means the text's own grid extent;
    // it is not an empty engine Layer rectangle.
    if name_bounds[0] == 0 {
        let count = scalar(get("NAME_MOJI_CNT"), 8)?;
        ensure!((1..=4096).contains(&count), "native name grid count");
        name_bounds[0] = name_size
            .checked_mul(count)
            .and_then(|n| {
                name_spacing[0]
                    .checked_mul(count - 1)
                    .and_then(|s| n.checked_add(s))
            })
            .context("native name grid overflow")?;
    }
    if name_bounds[1] == 0 {
        name_bounds[1] = name_size;
    }
    let global_color = scalar(cfg.get_unquoted("MWND.MOJI_COLOR"), 0)?;
    let resolved_color = |field| -> Result<[u8; 4]> {
        let n = scalar(get(field), -1)?;
        color(cfg, if n < 0 { global_color } else { n })
    };
    let add = |a: i32, b: i32| a.checked_add(b).context("native MWND coordinate overflow");
    let profile = MessageWindow {
        font: "default".into(),
        rect: [
            add(pos[0], message[0])?,
            add(pos[1], message[1])?,
            extent[0],
            extent[1],
        ],
        name_rect: [
            add(add(add(pos[0], name_pos[0])?, name_msg[0])?, name_rep[0])?,
            add(add(add(pos[1], name_pos[1])?, name_msg[1])?, name_rep[1])?,
            name_bounds[0],
            name_bounds[1],
        ],
        size: u16::try_from(size)?,
        name_size: u16::try_from(name_size)?,
        spacing,
        color: resolved_color("MOJI_COLOR")?,
        name_color: resolved_color("NAME_MOJI_COLOR")?,
    };
    profile.validate()?;
    let render_values = [
        ("ORDER", 1),
        ("FILTER_LAYER_REP", 0),
        ("WAKU_LAYER_REP", 1),
        ("MOJI_LAYER_REP", 5),
        ("SHADOW_LAYER_REP", 3),
        ("FUCHI_LAYER_REP", 4),
    ]
    .map(|(k, d)| scalar(cfg.get_unquoted(&format!("MWND.{k}")), d))
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    let config_filter = tuple(cfg.get("CONFIG.FILTER_COLOR"), [0, 0, 0, 128])?;
    ctx.ui.window = Some(profile);
    ctx.ui.name_spacing = Some(name_spacing);
    ctx.tables.mwnd_render.order = i64::from(render_values[0]);
    ctx.tables.mwnd_render.filter_layer_rep = i64::from(render_values[1]);
    ctx.tables.mwnd_render.waku_layer_rep = i64::from(render_values[2]);
    ctx.tables.mwnd_render.moji_layer_rep = i64::from(render_values[3]);
    ctx.tables.mwnd_render.shadow_layer_rep = i64::from(render_values[4]);
    ctx.tables.mwnd_render.fuchi_layer_rep = i64::from(render_values[5]);
    for (key, value) in [84, 91, 92, 93].into_iter().zip(config_filter) {
        ctx.globals
            .syscom
            .config_int
            .entry(key)
            .or_insert(i64::from(value));
    }
    let native = super::ops::state_mut(ctx);
    native.window_pos = Some((i64::from(pos[0]), i64::from(pos[1])));
    native.window_size = Some((i64::from(window_size[0]), i64::from(window_size[1])));
    native.open_anime_type = i64::from(open_type);
    native.open_anime_time = i64::from(open_time);
    native.close_anime_type = i64::from(close_type);
    native.close_anime_time = i64::from(close_time);
    native.waku_file = waku_file;
    native.filter_file = filter_file;
    native.filter_color = filter_color;
    native.filter_margin = Some((
        i64::from(filter_margin[0]),
        i64::from(filter_margin[1]),
        i64::from(filter_margin[2]),
        i64::from(filter_margin[3]),
    ));
    native.filter_config_color = filter_config_color;
    native.filter_config_tr = filter_config_tr;
    native.initialized_from_gameexe = true;
    if waku >= 0 {
        super::buttons::initialize(ctx, waku as usize)?;
    }
    Ok(())
}
