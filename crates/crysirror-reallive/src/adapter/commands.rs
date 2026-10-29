// SPDX-License-Identifier: MPL-2.0
//! Opcode meanings and native layout policy mapped to public engine operations.
use super::*;
impl RealLiveVm {
    pub(super) fn command(
        &mut self,
        m: &mut Machine,
        h: &mut dyn HostServices,
        input: InputFrame,
        c: &Command,
    ) -> Result<Next> {
        if matches!(c.kind, CommandKind::Select(_)) {
            return self.selection(m, h, c);
        }
        match (c.op.modtype, c.op.module) {
            (1, 4) => self.system(m, h, input, c)?,
            (0, 4) => match c.op.opcode {
                303 => m.yield_frame = true,
                _ => bail!("unsupported native event-loop operation"),
            },
            (1, 30) => match c.op.opcode {
                0 => self.draw_stack.clear(),
                3 => m.store = self.draw_stack.len() as i32,
                // Auto / semi-auto show every draw; manual shows refreshes only.
                20 | 21 => self.set_manual(h, false)?,
                22 => self.set_manual(h, true)?,
                _ => bail!("unsupported screen operation"),
            },
            (1, 31) => {
                if self.manual {
                    self.refresh_manual(h)?;
                }
                m.yield_frame = true;
            }
            (1, 33) => self.graphics(m, h, c)?,
            (0, 3) => self.message(m, h, c)?,
            (1, 10) if c.op.opcode == 100 => {
                let s = m.str_param(c, 0)?;
                self.text(m, h, &s)?;
            }
            (1, 20 | 21 | 22 | 23) => self.sound(m, h, c)?,
            (1, 26) => self.movie(m, h, c)?,
            (2, 1) => self.dll(m, c)?,
            (1, 40) => self.haikei(m, h, c)?,
            _ => bail!("unsupported native syscall {}", c.op),
        }
        Ok(Next::Advance)
    }
    fn system(
        &mut self,
        m: &mut Machine,
        h: &mut dyn HostServices,
        input: InputFrame,
        c: &Command,
    ) -> Result<()> {
        let now = h.now_ms();
        match c.op.opcode {
            0 => {
                self.title = m.str_param(c, 0)?;
                self.submit(h, json!({"ui":[{"op":"set_title","text":self.title}]}))?;
            }
            2 => {
                let t = m.str_target_param(c, 0)?;
                m.write_string(t, self.title.clone())?;
            }
            100 | 101 => {
                self.wait = Some(Wait::Time {
                    until: now.saturating_add(m.int_param(c, 0)?.max(0) as u64),
                    cancel: c.op.opcode == 101,
                })
            }
            110 | 120 => {
                let i = m.int_param_or(c, 0, 0)?;
                ensure!((0..256).contains(&i), "timer index");
                self.timers[usize::from(c.op.opcode == 120)][i as usize] = now;
            }
            111..=114 | 121..=124 => {
                let layer = usize::from(c.op.opcode >= 120);
                let query = c.op.opcode % 10 == 4;
                let i = m.int_param_or(c, if query { 0 } else { 1 }, 0)?;
                ensure!((0..256).contains(&i), "timer index");
                let start = self.timers[layer][i as usize];
                if query {
                    m.store = now.saturating_sub(start).min(i32::MAX as u64) as i32;
                } else {
                    let timeout = m.int_param(c, 0)?.max(0) as u64;
                    let cancel = matches!(c.op.opcode % 10, 2 | 3);
                    if now < start.saturating_add(timeout) {
                        self.wait = Some(Wait::Time {
                            until: start.saturating_add(timeout),
                            cancel,
                        });
                    } else if cancel {
                        m.store = 0;
                    }
                }
            }
            106 => self.wait = Some(Wait::Click { record: None }),
            130 => self.button_state = [0; 2],
            131 => {
                let x = m.int_target_param(c, 0)?;
                let y = m.int_target_param(c, 1)?;
                self.wait = Some(Wait::Click {
                    record: Some((x, y)),
                });
            }
            133 | 202 | 138 => {
                let values = [
                    input.pointer_x,
                    input.pointer_y,
                    self.button_state[0],
                    self.button_state[1],
                ];
                for (i, v) in
                    values
                        .into_iter()
                        .enumerate()
                        .take(if c.op.opcode == 133 { 4 } else { 2 })
                {
                    let t = m.int_target_param(c, i)?;
                    m.set_target(t, v)?;
                }
            }
            204 | 205 => {
                self.hide_cursor = c.op.opcode == 205;
                self.submit(
                    h,
                    json!({"ui":[{"op":"set_cursor_visible","visible":!self.hide_cursor}]}),
                )?;
            }
            210 | 211 => self.wipe_closes = c.op.opcode == 210,
            215 => self.grp_closes = true,
            216 => self.grp_closes = false,
            320 => {
                let mut top = m.stack.pop().context("empty native stack")?;
                m.stack.clear();
                top.kind = FrameKind::Root;
                m.stack.push(top);
            }
            1300 | 1310 => {
                let i = m.int_param(c, 0)?.max(0) as usize;
                let target = m.str_target_param(c, 1)?;
                m.write_string(target, m.memory.name(c.op.opcode == 1310, i).to_owned())?;
            }
            1301 | 1311 => {
                let i = m.int_param(c, 0)?.max(0) as usize;
                let text = m.str_param(c, 1)?;
                m.memory.set_name(c.op.opcode == 1311, i, text);
            }
            1302 | 1303 | 1312 | 1313 => self.name_entry(m, h, c)?,
            1211 | 1212 | 1213 => {
                if c.params.is_empty() {
                    self.menu = c.op.opcode == 1211;
                } else {
                    let i = m.int_param(c, 0)?;
                    self.syscom.insert(
                        i,
                        if c.op.opcode == 1211 {
                            1
                        } else if c.op.opcode == 1212 {
                            0
                        } else {
                            2
                        },
                    );
                }
            }
            _ => bail!("unsupported native system operation {}", c.op),
        }
        Ok(())
    }
    fn graphics(&mut self, m: &mut Machine, h: &mut dyn HostServices, c: &Command) -> Result<()> {
        let raw = c.op.opcode % 1000;
        let rec = c.op.opcode >= 1000;
        let op = match raw {
            52..=58 => raw + 20,
            62..=68 => raw + 10,
            82..=88 => raw - 10,
            _ => raw,
        };
        let n = c.params.len();
        match op {
            15 => {
                let id = m.int_param(c, 0)?;
                let w = m.int_param(c, 1)?;
                let hh = m.int_param(c, 2)?;
                self.canvas(
                    h,
                    vec![json!({"op":"create","id":id,"width":w,"height":hh})],
                )?;
            }
            31 => {
                let id = m.int_param(c, 0)?;
                let color = rgb(m, c, 1)?;
                self.canvas(h, vec![json!({"op":"clear","id":id,"color":color})])?;
            }
            50 | 51 | 70 | 71 => {
                let name = m.str_param(c, 0)?;
                let dc = m.int_param(c, 1)?;
                let asset = self.resource(m, "image", &name)?;
                let size = h.image_size(self.client, &asset).map_err(err)?;
                let mask = matches!(op, 51 | 71);
                let (rect, pos, alpha) = if n >= 8 {
                    (
                        read_rect(m, c, 2, rec)?,
                        [m.int_param(c, 6)? as f32, m.int_param(c, 7)? as f32],
                        m.int_param_or(c, 8, 255)?.clamp(0, 255) as u8,
                    )
                } else {
                    (
                        [0., 0., size[0] as f32, size[1] as f32],
                        [0., 0.],
                        if n == 3 {
                            m.int_param(c, 2)?.clamp(0, 255) as u8
                        } else {
                            255
                        },
                    )
                };
                // Upstream blits clip the source rectangle to the image.
                let rect = clip_source(rect, size);
                let layer = self.image_layer(
                    asset.clone(),
                    [pos[0], pos[1], rect[2], rect[3]],
                    Some(uv(rect, size)),
                    alpha,
                );
                let mut ops = vec![];
                let extent = if !mask && dc > 1 && n <= 3 {
                    ops.push(json!({"op":"create","id":dc,"width":size[0],"height":size[1]}));
                    size
                } else {
                    let extent = self.extent(h, dc)?;
                    ensure!(
                        dc <= 1
                            || (pos[0] + rect[2] <= extent[0] as f32
                                && pos[1] + rect[3] <= extent[1] as f32),
                        "native DC growth needs shared preserving canvas resize"
                    );
                    extent
                };
                let replace = !mask
                    && alpha == 255
                    && pos == [0., 0.]
                    && rect[2] >= extent[0] as f32
                    && rect[3] >= extent[1] as f32;
                // Upstream Blend::Copy keeps the image's alpha: a whole-surface
                // `set` is exactly that; a partial copy over existing pixels
                // would need an engine region replace.
                ensure!(
                    mask || replace || h.image_is_opaque(self.client, &asset).map_err(err)?,
                    "non-mask partial copy of a transparent image needs an engine region replace"
                );
                ops.push(json!({"op":if replace{"set"}else{"draw"},"id":dc,"layer":layer}));
                self.canvas(h, ops)?;
            }
            72 | 73 | 74 | 76 => {
                let is_buffer = op == 72;
                let name = if is_buffer {
                    String::new()
                } else {
                    m.str_param(c, 0)?
                };
                let (src, pos, time, style, alpha) =
                    display(m, c, 1, self.width, self.height, rec)?;
                if self.grp_closes || (time > 0 && style != 1 && self.wipe_closes) {
                    self.hide_text(h)?;
                }
                let mut ops = Vec::new();
                if is_buffer {
                    let dc = m.int_param(c, 0)?;
                    let size = self.extent(h, dc)?;
                    ops.push(json!({"op":"copy","source":dc,"destination":0,"rect":src.unwrap_or([0.,0.,size[0]as f32,size[1]as f32]),"position":pos,"opacity":alpha,"replace":false}));
                } else if name == "?" {
                    let size = self.extent(h, 1)?;
                    ops.push(json!({"op":"copy","source":1,"destination":0,"rect":src.unwrap_or([0.,0.,size[0]as f32,size[1]as f32]),"position":pos,"opacity":alpha,"replace":false}));
                } else {
                    let resource = if name == "???" {
                        m.sys.default_grp.clone()
                    } else {
                        name.clone()
                    };
                    let asset = self.resource(m, "image", &resource)?;
                    let size = h.image_size(self.client, &asset).map_err(err)?;
                    // Non-mask opens copy the image (Blend::Copy); DC 0 is shown
                    // without its alpha, so transparent pixels show their colour
                    // (black in practice): back them with opaque black.
                    let backed = op != 74 && !h.image_is_opaque(self.client, &asset).map_err(err)?;
                    let r = clip_source(src.unwrap_or([0., 0., size[0] as f32, size[1] as f32]), size);
                    let layer = self.image_layer(
                        asset,
                        [pos[0], pos[1], r[2], r[3]],
                        Some(uv(r, size)),
                        alpha,
                    );
                    let full = op == 73
                        && pos == [0., 0.]
                        && r[2] >= self.width as f32
                        && r[3] >= self.height as f32;
                    if !full {
                        ops.push(json!({"op":"copy","source":0,"destination":1,"rect":[0,0,self.width,self.height],"position":[0,0],"opacity":255,"replace":true}));
                    }
                    if backed {
                        ops.push(json!({"op":"fill","id":1,"rect":[pos[0], pos[1], r[2], r[3]],"color":[0,0,0,255],"replace":true}));
                    }
                    ops.push(json!({"op":if full{"set"}else{"draw"},"id":1,"layer":layer}));
                    ops.push(json!({"op":"copy","source":1,"destination":0,"rect":[0,0,self.width,self.height],"position":[0,0],"opacity":255,"replace":true}));
                    if op == 73 && name != "???" {
                        m.sys.default_grp = name;
                    }
                }
                self.canvas(h, ops)?;
                // grpDisplay / grpOpenBg promote background objects (upstream
                // `open_onto_screen`); they join the transition's after-image.
                if matches!(op, 72 | 73) {
                    m.sys.gfx.promote_objects();
                }
                self.sync_objects(m, h)?;
                if time > 0 && style != 1 {
                    let mut values = if c.params.len() >= 17 {
                        m.int_params_from(c, 1)?
                    } else {
                        let index = m.int_param_or(c, 1, 0)?;
                        m.gameexe.ints(&format!("SEL.{index:03}"))
                    };
                    values.resize(16, 0);
                    values[6] = time;
                    values[7] = style;
                    self.sel_transition(h, &values)?;
                }
            }
            100 | 101 | 102 => {
                // 100 copies (Blend::Copy); 101/102 blend by the source alpha
                // (Blend::Mask) — upstream `copy_blend`.
                let (source, rect, pos, dst, at) = if n >= 8 {
                    (
                        m.int_param(c, 4)?,
                        read_rect(m, c, 0, rec)?,
                        [m.int_param(c, 5)? as f32, m.int_param(c, 6)? as f32],
                        m.int_param(c, 7)?,
                        8,
                    )
                } else {
                    let src = m.int_param(c, 0)?;
                    let size = self.extent(h, src)?;
                    (
                        src,
                        [0., 0., size[0] as f32, size[1] as f32],
                        [0., 0.],
                        m.int_param(c, 1)?,
                        2,
                    )
                };
                let alpha = m.int_param_or(c, at, 255)?.clamp(0, 255) as u8;
                let target = self.extent(h, dst)?;
                // grpCopy at full opacity replaces the region (upstream
                // Blend::Copy); the engine drops what it covers.
                let _ = target;
                let replace = op == 100 && alpha == 255;
                self.canvas(h,vec![json!({"op":"copy","source":source,"destination":dst,"rect":rect,"position":pos,"opacity":alpha,"replace":replace})])?;
            }
            // freeDC(dc): DC 0 stays, DC 1 is emptied at screen size, others
            // are released (upstream `Graphics::free_dc`).
            16 => {
                let dc = m.int_param(c, 0)?;
                ensure!((0..=255).contains(&dc), "invalid DC {dc}");
                match dc {
                    0 => {}
                    1 => self.canvas(h, vec![json!({"op":"create","id":1,"width":self.width,"height":self.height})])?,
                    _ => self.canvas(h, vec![json!({"op":"create","id":dc,"width":1,"height":1})])?,
                }
            }
            // grpStretchBlt / grpMaskStretchBlt(src rect, src, dst rect, dst[, alpha])
            401 | 409 => {
                let from = read_rect(m, c, 0, rec)?;
                let source = m.int_param(c, 4)?;
                let to = read_rect(m, c, 5, rec)?;
                let target = m.int_param(c, 9)?;
                let alpha = if n > 10 { m.int_param(c, 10)? } else { 255 }.clamp(0, 255);
                if from[2] > 0. && from[3] > 0. && to[2] > 0. && to[3] > 0. {
                    let (sx, sy) = (to[2] / from[2], to[3] / from[3]);
                    let matrix = [sx, 0., 0., sy, to[0] - from[0] * sx, to[1] - from[1] * sy];
                    // Blend::Copy replaces the area; partial opacity blends over it.
                    let replace = op == 401 && alpha == 255;
                    self.canvas(h, vec![json!({"op":"transform_copy","source":source,"destination":target,"rect":from,"matrix":matrix,"replace":replace,"opacity":alpha})])?;
                }
            }
            // grpInvert / grpMono([area,] dc[, alpha]), grpColour([area,] dc, r, g, b)
            // / grpLight([area,] dc, level): upstream grp 300..303 as an
            // engine region filter (apply_tint: + toward white, - toward black).
            300..=303 => {
                let width = match op {
                    300 | 301 => 1,
                    302 => 4,
                    _ => 2,
                };
                let (area, at) = if n >= width + 4 { (Some(read_rect(m, c, 0, rec)?), 4) } else { (None, 0) };
                let dc = m.int_param(c, at)?;
                let (filter, alpha) = match op {
                    300 | 301 => {
                        let alpha = if n > at + 1 { m.int_param(c, at + 1)? } else { 255 }.clamp(0, 255);
                        (if op == 300 { json!({"invert": 1.0}) } else { json!({"mono": 1.0}) }, alpha)
                    }
                    _ => {
                        let rgb = if op == 302 {
                            [m.int_param(c, at + 1)?, m.int_param(c, at + 2)?, m.int_param(c, at + 3)?]
                        } else {
                            [m.int_param(c, at + 1)?; 3]
                        };
                        ensure!(rgb[0] == rgb[1] && rgb[1] == rgb[2], "grpColour with unequal channels needs an engine per-channel tint");
                        let v = rgb[0].clamp(-255, 255) as f32 / 255.;
                        (json!({"bright": v.max(0.), "dark": (-v).max(0.)}), 255)
                    }
                };
                let rect = match area {
                    Some(r) => r,
                    None => {
                        let size = self.extent(h, dc)?;
                        [0., 0., size[0] as f32, size[1] as f32]
                    }
                };
                if rect[2] > 0. && rect[3] > 0. && alpha > 0 {
                    self.canvas(h, vec![json!({"op":"filter","id":dc,"rect":rect,"filter":filter,"opacity":alpha})])?;
                }
            }
            // grpMulti(file | dc, effect..., compositors...) — upstream grp 75/77.
            75 | 77 => {
                use crate::vm::expr::Expr;
                let plain = c.params.iter().take_while(|p| !matches!(p.value, Expr::Special { .. })).count();
                let mut values = if plain >= 17 {
                    (1..17).map(|i| m.int_param(c, i)).collect::<Result<Vec<_>>>()?
                } else if plain >= 2 {
                    let sel = m.int_param(c, 1)?;
                    m.gameexe.ints(&format!("{}.{sel:03}", if rec { "SELR" } else { "SEL" }))
                } else {
                    vec![]
                };
                if values.is_empty() {
                    values = vec![0, 0, self.width as i32 - 1, self.height as i32 - 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0];
                }
                values.resize(16, 0);
                let src_rect = |v: &[i32]| -> [f32; 4] {
                    if rec {
                        [v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32]
                    } else {
                        [v[0].min(v[2]) as f32, v[1].min(v[3]) as f32, (v[0].abs_diff(v[2]) + 1) as f32, (v[1].abs_diff(v[3]) + 1) as f32]
                    }
                };
                let mut ops = vec![json!({"op":"clear","id":1,"color":[0,0,0,255]})];
                if op == 75 {
                    let name = m.str_param(c, 0)?;
                    let name = if name == "???" { m.sys.default_grp.clone() } else { m.sys.default_grp = name.clone(); name };
                    let asset = self.resource(m, "image", &name)?;
                    let size = h.image_size(self.client, &asset).map_err(err)?;
                    let r = clip_source(src_rect(&values), size);
                    ops.push(json!({"op":"draw","id":1,"layer":self.image_layer(asset, [values[4] as f32, values[5] as f32, r[2], r[3]], Some(uv(r, size)), 255)}));
                } else {
                    let dc = m.int_param(c, 0)?;
                    let r = src_rect(&values);
                    ops.push(json!({"op":"copy","source":dc,"destination":1,"rect":r,"position":[values[4],values[5]],"opacity":255,"replace":true}));
                }
                for index in plain..c.params.len() {
                    let Expr::Special { tag, pieces } = c.params[index].value.clone() else { continue };
                    let Some(first) = pieces.first() else { continue };
                    let name = m.eval_str(first)?;
                    let numbers = pieces[1..].iter().map(|p| m.eval_int(p)).collect::<Result<Vec<_>>>()?;
                    let asset = self.resource(m, "image", &name)?;
                    let size = h.image_size(self.client, &asset).map_err(err)?;
                    let (r, dest, alpha) = match (tag, numbers.as_slice()) {
                        (0, _) => ([0., 0., size[0] as f32, size[1] as f32], [0., 0.], 255),
                        (1 | 2, [effect, rest @ ..]) => {
                            let mut v = m.gameexe.ints(&format!("{}.{effect:03}", if rec { "SELR" } else { "SEL" }));
                            v.resize(16, 0);
                            (src_rect(&v), [v[4] as f32, v[5] as f32], rest.first().copied().unwrap_or(if v[14] == 0 { 255 } else { v[14] }))
                        }
                        (3 | 4, [a, b, cc, d, dx, dy, rest @ ..]) => (src_rect(&[*a, *b, *cc, *d]), [*dx as f32, *dy as f32], rest.first().copied().unwrap_or(255)),
                        _ => continue,
                    };
                    let r = clip_source(r, size);
                    ops.push(json!({"op":"draw","id":1,"layer":self.image_layer(asset, [dest[0], dest[1], r[2], r[3]], Some(uv(r, size)), alpha.clamp(0, 255) as u8)}));
                }
                ops.push(json!({"op":"copy","source":1,"destination":0,"rect":[0,0,self.width,self.height],"position":[0,0],"opacity":255,"replace":true}));
                if self.grp_closes {
                    self.hide_text(h)?;
                }
                self.canvas(h, ops)?;
                m.sys.gfx.promote_objects();
                self.sync_objects(m, h)?;
                if values[7] != 1 {
                    self.sel_transition(h, &values)?;
                }
            }
            // shake(spec): `#SHAKE.spec` (else `#SHAKE.000`), waited for.
            32 => {
                let spec = m.int_param(c, 0)?;
                let mut values = m.gameexe.ints(&format!("SHAKE.{spec:03}"));
                if values.is_empty() {
                    values = m.gameexe.ints("SHAKE.000");
                }
                let steps = values.chunks_exact(3).map(|c| (c[0], c[1], c[2])).collect();
                self.wait = Some(Wait::Shake { steps, start: h.now_ms() });
            }
            // grpZoom(from, to, src, dest, time)
            402 => {
                let from = read_rect(m, c, 0, rec)?;
                let to = read_rect(m, c, 4, rec)?;
                let source = u32::try_from(m.int_param(c, 8)?)?;
                let dest = read_rect(m, c, 9, rec)?;
                let time = m.int_param(c, 13)?.max(0) as u64;
                self.wait = Some(Wait::Zoom { source, from, to, dest, start: h.now_ms(), time });
            }
            // grpOutline / grpFill(([area,] dc, r, g, b[, alpha])), upstream grp 200/201.
            200 | 201 => {
                let (area, at) = if n >= 8 {
                    (Some(read_rect(m, c, 0, rec)?), 4)
                } else {
                    (None, 0)
                };
                let dc = m.int_param(c, at)?;
                let color = rgb(m, c, at + 1)?;
                let alpha = if n > at + 4 { m.int_param(c, at + 4)? } else { 255 }.clamp(0, 255) as u8;
                let rect = match area {
                    Some(r) => r,
                    None => {
                        let size = self.extent(h, dc)?;
                        [0., 0., size[0] as f32, size[1] as f32]
                    }
                };
                if rect[2] > 0. && rect[3] > 0. {
                    let [x, y, w, hh] = rect;
                    let parts = if op == 201 {
                        vec![rect]
                    } else {
                        vec![
                            [x, y, w, 1.],
                            [x, y + hh - 1., w, 1.],
                            [x, y + 1., 1., hh - 2.],
                            [x + w - 1., y + 1., 1., hh - 2.],
                        ]
                    };
                    let ops = parts
                        .into_iter()
                        .filter(|r| r[2] > 0. && r[3] > 0.)
                        .map(|r| json!({"op":"fill","id":dc,"rect":r,"color":[color[0],color[1],color[2],alpha],"replace":alpha==255}))
                        .collect();
                    self.canvas(h, ops)?;
                }
            }
            403 => {
                let (color, time) = match n {
                    1 | 2 => {
                        let color = m.int_param(c, 0)?;
                        (colour(&m.gameexe, color), m.int_param_or(c, 1, 0)?)
                    }
                    3 | 4 => (rgb(m, c, 0)?, m.int_param_or(c, 3, 0)?),
                    _ => bail!("area fade requires shared region transition"),
                };
                self.canvas(h, vec![json!({"op":"clear","id":0,"color":color})])?;
                if time > 0 {
                    self.submit(h,json!({"layers":[{"op":"transition","transition":{"effect":{"kind":"crossfade"},"duration_ms":time}}]}))?;
                    self.wait = Some(Wait::Time {
                        until: h.now_ms() + time as u64,
                        cancel: false,
                    });
                }
            }
            _ => bail!("unsupported native graphics operation {}", c.op),
        }
        self.draw_stack.push(c.op.to_string());
        if self.draw_stack.len() > 256 {
            self.draw_stack.remove(0);
        }
        Ok(())
    }
    fn movie(&mut self, m: &mut Machine, h: &mut dyn HostServices, c: &Command) -> Result<()> {
        match c.op.opcode {
            0 | 1 | 2 | 20 => {
                let name = m.str_param(c, 0)?;
                let path = self.resource(m, "movie", &name)?;
                let texture = h
                    .media_request(
                        self.client,
                        MediaRequest::Play {
                            id: 0,
                            asset: path,
                            looping: c.op.opcode == 2,
                            audio: true,
                            gain: 1.,
                        },
                    )
                    .map_err(err)?;
                let coords = m.int_params_from(c, 1)?;
                let rect = if coords.len() >= 4 && coords[..4].iter().any(|v| *v != 0) {
                    read_rect(m, c, 1, false)?
                } else {
                    [0., 0., self.width as f32, self.height as f32]
                };
                let mut layer = self.image_layer(texture.relative().into(), rect, None, 255);
                layer["id"] = json!(500000);
                layer["order"] = json!([5000, 0]);
                self.layer(h, layer)?;
                if matches!(c.op.opcode, 1 | 20) {
                    self.wait = Some(Wait::Media {
                        cancel: c.op.opcode == 20,
                    });
                }
            }
            3 | 21 => {
                self.wait = Some(Wait::Media {
                    cancel: c.op.opcode == 21,
                })
            }
            4 => {
                m.store =
                    i32::from(h.media_info(self.client, 0).is_ok_and(|s| {
                        matches!(s.phase, MediaPhase::Loading | MediaPhase::Playing)
                    }))
            }
            5 => self.stop_movie(h)?,
            _ => bail!("unsupported native movie operation"),
        }
        Ok(())
    }
    fn sound(&mut self, m: &mut Machine, h: &mut dyn HostServices, c: &Command) -> Result<()> {
        let module = c.op.module;
        let op = c.op.opcode;
        let channel = match module {
            20 => 0,
            21 => 100 + m.int_param_or(c, if op <= 2 { 1 } else { 0 }, 0)?.max(0) as u32,
            22 => 32,
            23 => 1,
            _ => unreachable!(),
        };
        match (module, op) {
            (20, 0..=2) | (21, 0..=2) => {
                let mut name = m.str_param(c, 0)?;
                let mut loop_start = 0;
                let start_ms = 0;
                let mut frame_limit = None;
                if module == 20 {
                    for e in m.gameexe.filter("DSTRACK") {
                        let strings = e.strs();
                        if strings
                            .get(1)
                            .or(strings.first())
                            .is_some_and(|s| s.eq_ignore_ascii_case(&name))
                        {
                            let v = e.ints();
                            name = strings[0].to_string();
                            loop_start = v.get(2).copied().unwrap_or(0).max(0) as u64;
                            if v.first().copied().unwrap_or(0) != 0 {
                                bail!(
                                    "nonzero native music segment start needs shared segment source API"
                                );
                            }
                            frame_limit = v.get(1).copied().filter(|v| *v > 0).map(|v| v as u64);
                            break;
                        }
                    }
                }
                let asset = self.resource(m, if module == 20 { "bgm" } else { "wav" }, &name)?;
                let fade = m
                    .int_param_or(c, if module == 20 { 1 } else { 2 }, 0)?
                    .max(0) as f32;
                // bgmPlay(file, fadein, fadeout): the playing track fades out
                // as the new one starts (engine replacement fade).
                let replace_fade = if module == 20 { m.int_param_or(c, 2, 0)?.max(0) as f32 } else { 0. };
                let ticket = h
                    .begin_audio(
                        self.client,
                        AudioRequest::PlayAsset {
                            channel,
                            source: AudioAsset {
                                path: VfsPath::new(Mount::Asset, &asset).map_err(err)?,
                                frame_limit,
                            },
                            intro: None,
                            parameters: AudioParameters {
                                looping: if module == 20 { op != 2 } else { op == 2 },
                                loop_start,
                                start_ms,
                                fade_ms: fade,
                                replace_fade_ms: replace_fade,
                                bus: if module == 20 {
                                    PcmBus::Music
                                } else {
                                    PcmBus::Sfx
                                },
                                ..Default::default()
                            },
                        },
                    )
                    .map_err(err)?;
                self.pending_audio = Some((
                    ticket,
                    ((module == 20 || module == 21) && op == 1).then_some(channel),
                ));
                self.poll_audio(h)?;
                return Ok(());
            }
            (20 | 21, 3) => self.wait = Some(Wait::Audio { channel }),
            (20 | 21, 4 | 7) => {
                m.store = i32::from(h.audio_playing(self.client, channel).map_err(err)?)
            }
            (20 | 21, 5) | (23, 5) => {
                if module == 21 && c.params.is_empty() {
                    for channel in 100..116 {
                        h.submit_audio(
                            self.client,
                            AudioRequest::Stop {
                                channel,
                                fade_ms: 0.,
                            },
                        )
                        .map_err(err)?;
                    }
                } else {
                    h.submit_audio(
                        self.client,
                        AudioRequest::Stop {
                            channel,
                            fade_ms: 0.,
                        },
                    )
                    .map_err(err)?;
                }
            }
            (21, 20) => {
                for channel in 100..116 {
                    h.submit_audio(
                        self.client,
                        AudioRequest::Stop {
                            channel,
                            fade_ms: 0.,
                        },
                    )
                    .map_err(err)?;
                }
            }
            (20, 105 | 106) => {
                h.submit_audio(
                    self.client,
                    AudioRequest::Stop {
                        channel,
                        fade_ms: m.int_param_or(c, 0, 1000)?.max(0) as f32,
                    },
                )
                .map_err(err)?;
                if op == 106 {
                    self.wait = Some(Wait::Audio { channel });
                }
            }
            // koePlay family (upstream `modules::sound::koe` / `voice_pcm`):
            // voice `id` is sample id % 100000 of archive id / 100000.
            (23, 0 | 1 | 7..=10) => {
                let id = m.int_param(c, 0)?;
                let character = if Machine::param_count(c) >= 2 { Some(m.int_param(c, 1)?) } else { None };
                let honour = matches!(op, 0 | 1 | 7);
                let settings = &m.sys.settings;
                let muted = settings.koe_mode == 2
                    || !settings.enabled[crate::vm::settings::channel::KOE]
                    || (honour && character.is_some_and(|ch| settings.use_koe.get(&ch) == Some(&false)));
                h.submit_audio(self.client, AudioRequest::Stop { channel, fade_ms: 0. }).map_err(err)?;
                if muted {
                    return Ok(());
                }
                let (file, sample) = (id / 100_000, id % 100_000);
                let archive = self.resources.as_ref().context("RealLive resources")?.borrow_mut().find(&m.gameexe, "koe", &format!("z{file:04}")).ok();
                let asset = match archive {
                    // A voice archive (KOEPAC/OVK/NWK) is an engine container:
                    // the project mounts it (`real-live/koepac` etc.) and its
                    // samples are ordinary audio assets.
                    Some(archive) => {
                        let storage = self.resources.as_ref().context("RealLive resources")?.borrow().storage.clone();
                        let point = storage.container_mount(&archive).map_err(err)?.with_context(|| {
                            format!("reallive: voice archive {archive} is not mounted (mount it with format real-live/koepac, ovk or nwk)")
                        })?;
                        let names = storage.list(Mount::Asset, &point).map_err(err)?;
                        let stem = format!("sample{sample:05}");
                        let name = names
                            .iter()
                            .find(|n| n.split('.').next() == Some(stem.as_str()))
                            .with_context(|| format!("reallive: voice {id} is not in {archive}"))?;
                        format!("{point}/{name}")
                    }
                    None => {
                        let loose = format!("z{file:04}{sample:05}");
                        self.resource(m, "koe", &loose)
                            .or_else(|_| self.resource(m, "koe", &format!("{file:04}/{loose}")))
                            .with_context(|| format!("reallive: voice {id} not found"))?
                    }
                };
                let volume = settings_volume(&m.sys.settings, crate::vm::settings::channel::KOE);
                let ticket = h
                    .begin_audio(
                        self.client,
                        AudioRequest::PlayAsset {
                            channel,
                            source: AudioAsset { path: VfsPath::new(Mount::Asset, &asset).map_err(err)?, frame_limit: None },
                            intro: None,
                            parameters: AudioParameters { gain: volume, bus: PcmBus::Voice, ..Default::default() },
                        },
                    )
                    .map_err(err)?;
                self.pending_audio = Some((ticket, matches!(op, 1 | 7 | 9 | 10).then_some(channel)));
                self.poll_audio(h)?;
                return Ok(());
            }
            // se(number): `#SE.number` = file, played once over other SEs
            // (upstream `Sound::se_play`; none while fast-forwarding).
            (22, 0) => {
                let number = m.int_param(c, 0)?;
                let file = m.gameexe.get(&format!("SE.{number:03}")).and_then(|e| e.str(0).map(str::to_owned)).unwrap_or_default();
                if file.is_empty() || m.sys.should_fast_forward() {
                    return Ok(());
                }
                let asset = self.resource(m, "wav", &file)?;
                let slot = 32 + (self.se_serial % 16);
                self.se_serial = self.se_serial.wrapping_add(1);
                let gain = settings_volume(&m.sys.settings, crate::vm::settings::channel::SE);
                let ticket = h
                    .begin_audio(
                        self.client,
                        AudioRequest::PlayAsset {
                            channel: slot,
                            source: AudioAsset { path: VfsPath::new(Mount::Asset, &asset).map_err(err)?, frame_limit: None },
                            intro: None,
                            parameters: AudioParameters { gain, bus: PcmBus::Sfx, ..Default::default() },
                        },
                    )
                    .map_err(err)?;
                self.pending_audio = Some((ticket, None));
                self.poll_audio(h)?;
                return Ok(());
            }
            (23, 3 | 6) => self.wait = Some(Wait::Audio { channel }),
            (23, 4) => m.store = i32::from(h.audio_playing(self.client, channel).map_err(err)?),
            // wavStop / wavStopEx, wavFadeOut(channel[, time = 1000]).
            (21, 9 | 10) => {
                h.submit_audio(self.client, AudioRequest::Stop { channel, fade_ms: 0. }).map_err(err)?;
            }
            (21, 105 | 106) => {
                let fade = m.int_param_or(c, 1, 1000)?.max(0) as f32;
                h.submit_audio(self.client, AudioRequest::Stop { channel, fade_ms: fade }).map_err(err)?;
            }
            // bgmSetVolume / wavSetVolume(volume[, time]), *VolMax / *VolMin([time]).
            (20, 12) | (21, 12) => {
                let at = usize::from(module == 21);
                let volume = m.int_param(c, at)?.clamp(0, 255) as f32 / 255.;
                let fade = m.int_param_or(c, at + 1, 0)?.max(0) as f32;
                h.submit_audio(self.client, AudioRequest::SetGain { channel, gain: volume, fade_ms: fade }).map_err(err)?;
            }
            (20, 13 | 14) | (21, 13 | 14) => {
                let at = usize::from(module == 21);
                let fade = m.int_param_or(c, at, 0)?.max(0) as f32;
                let gain = if op == 13 { 1. } else { 0. };
                h.submit_audio(self.client, AudioRequest::SetGain { channel, gain, fade_ms: fade }).map_err(err)?;
            }
            // bgmPause / bgmUnPause.
            (20, 9 | 10) => {
                h.submit_audio(self.client, AudioRequest::Pause { channel, paused: op == 9, fade_ms: 0. }).map_err(err)?;
            }
            _ => bail!("unsupported native audio operation {}", c.op),
        }
        self.api_calls += 1;
        Ok(())
    }
    fn message(&mut self, m: &mut Machine, h: &mut dyn HostServices, c: &Command) -> Result<()> {
        match c.op.opcode {
            102 => {
                let w = m.int_param(c, 0)?;
                ensure!((0..64).contains(&w), "native text window");
                self.window = w as u32;
            }
            17 | 15 | 205 | 206 | 207 | 210 => {
                self.wait = Some(Wait::Pause {
                    kind: c.op.opcode,
                    window: self.window,
                })
            }
            103 => self.fast_text = true,
            104 => self.fast_text = false,
            100 | 105 => self.color = m.int_param(c, 0)?.max(0) as u32,
            // FontSize([size]): no size returns to the window's default.
            101 => {
                if c.params.is_empty() {
                    self.sizes.remove(&self.window);
                } else {
                    let i = if c.params.len() > 1 { 1 } else { 0 };
                    let size = m.int_param(c, i)?;
                    ensure!((1..=512).contains(&size), "native text size");
                    self.sizes.insert(self.window, size);
                }
            }
            // Upstream msg: 151 closes the active window (frame included),
            // 152 clears its text, 161 closes every window, 162 clears all.
            151 => {
                self.close_window(h, self.window)?;
                m.message_page_empty = true;
            }
            152 => {
                self.clear_window(h, self.window)?;
                m.message_page_empty = true;
            }
            161 => {
                self.hide_text(h)?;
                m.message_page_empty = true;
            }
            162 => {
                let windows: Vec<u32> = self.pen.keys().copied().collect();
                for w in windows {
                    self.clear_window(h, w)?;
                }
                m.message_page_empty = true;
            }
            // br: the next line (upstream `hard_break`); par: a new line only
            // when the current one has text (upstream `paragraph`).
            201 | 3 | 202 => {
                let line = self.window_geometry(m, self.window).line;
                let pen = self.pen.entry(self.window).or_insert([0, 0]);
                if c.op.opcode == 201 || pen[0] > 0 {
                    pen[0] = 0;
                    pen[1] += line;
                }
            }
            _ => bail!("unsupported native message operation {}", c.op),
        }
        Ok(())
    }
    fn hide_text(&mut self, h: &mut dyn HostServices) -> Result<()> {
        let mut ops = vec![];
        for w in 0..64 {
            for i in 0..3 {
                let id = 10000 + w * 4 + i;
                if !self.query(h, EngineQuery::Layer(id))?.is_null() {
                    ops.push(json!({"op":"remove","id":id}));
                }
            }
        }
        for id in std::mem::take(&mut self.waku_ids) {
            ops.push(json!({"op":"remove","id":id}));
        }
        self.submit(h, json!({"layers":ops}))?;
        self.pen.clear();
        Ok(())
    }
    pub(super) fn stop_movie(&mut self, h: &mut dyn HostServices) -> Result<()> {
        match h.media_info(self.client, 0) {
            Ok(_) => {
                h.media_request(self.client, MediaRequest::Stop { id: 0 })
                    .map_err(err)?;
            }
            Err(Error::NotFound) => {}
            Err(e) => return Err(err(e)),
        }
        if !self.query(h, EngineQuery::Layer(500000))?.is_null() {
            self.submit(h, json!({"layers":[{"op":"remove","id":500000}]}))?;
        }
        Ok(())
    }
    /// Close one window: its text and its frame (upstream `close_window`).
    pub(super) fn close_window(&mut self, h: &mut dyn HostServices, window: u32) -> Result<()> {
        self.clear_window(h, window)?;
        let frame = 20000 + window * 8..20000 + window * 8 + 8;
        let ops: Vec<Value> = self
            .waku_ids
            .iter()
            .filter(|id| frame.contains(id))
            .map(|id| json!({"op":"remove","id":id}))
            .collect();
        self.waku_ids.retain(|id| !frame.contains(id));
        if !ops.is_empty() {
            self.submit(h, json!({ "layers": ops }))?;
        }
        Ok(())
    }
    pub(super) fn clear_window(&mut self, h: &mut dyn HostServices, window: u32) -> Result<()> {
        let mut ops = vec![];
        for id in (10000 + window * 4)..(10003 + window * 4) {
            if !self.query(h, EngineQuery::Layer(id))?.is_null() {
                ops.push(json!({"op":"remove","id":id}));
            }
        }
        self.submit(h, json!({"layers":ops}))?;
        self.pen.remove(&window);
        Ok(())
    }
    pub(super) fn finish_pause(
        &mut self,
        m: &mut Machine,
        h: &mut dyn HostServices,
        kind: u16,
        window: u32,
    ) -> Result<()> {
        self.sizes.remove(&window);
        self.fast_text = false;
        self.color = 0;
        let r = m
            .gameexe
            .int(&format!("WINDOW.{window:03}.R_COMMAND_MOD"))
            .unwrap_or(0)
            != 0;
        if matches!(kind, 15 | 205 | 206) {
            return Ok(());
        }
        if matches!(kind, 17 | 207) && r {
            let size = self.font_size(m);
            let rep = m.gameexe.ints(&format!("WINDOW.{window:03}.MOJI_REP"));
            let p = self.pen.entry(window).or_insert([0, 0]);
            p[0] = 0;
            p[1] += size + rep.get(1).copied().unwrap_or(0);
        } else {
            self.clear_window(h, window)?;
            m.message_page_empty = true;
            if kind != 207 {
                let active = self.window;
                for w in self.pen.keys().copied().collect::<Vec<_>>() {
                    if w != active {
                        self.clear_window(h, w)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn name_entry(&mut self, m: &mut Machine, h: &mut dyn HostServices, c: &Command) -> Result<()> {
        let mut fields = vec![];
        if matches!(c.op.opcode, 1302 | 1312) {
            fields.push((m.int_param(c, 0)?, m.str_param(c, 1)?));
        } else {
            for at in 0..c.params.len() {
                let pieces = m.complex_param(c, at)?;
                ensure!(pieces.len() >= 2, "invalid native name field");
                fields.push((m.eval_int(&pieces[0])?, m.eval_str(&pieces[1])?));
            }
        }
        let local = c.op.opcode >= 1310;
        let max = m.gameexe.int("NAME_MAXLEN").unwrap_or(6).clamp(6, 10);
        let mut ops = vec![];
        let mut ids = vec![];
        for (at, (index, label)) in fields.into_iter().enumerate() {
            let id = 610000 + at as u32;
            ops.push(json!({"op":"put","widget":{"id":id+1000,"kind":"label","rect":[80,120+at*80,640,32],"text":label,"font":self.config.font_alias,"text_size":24}}));
            ops.push(json!({"op":"put","widget":{"id":id,"kind":"text_field","rect":[80,152+at*80,640,36],"text":m.memory.name(local,index.max(0)as usize),"max_length":max,"font":self.config.font_alias,"text_size":24}}));
            ids.push((index, id));
        }
        if !ids.is_empty() {
            // The original dialog's OK: commits even when the field lost focus.
            let y = 152 + ids.len() * 80;
            ops.push(json!({"op":"put","widget":{"id":NAME_OK,"kind":"button","rect":[560,y,160,36],"text":"OK","font":self.config.font_alias,"text_size":24}}));
            ops.push(json!({"op":"focus","id":ids[0].1}));
            self.submit(h, json!({"ui":ops}))?;
            self.wait = Some(Wait::Name { local, fields: ids });
        }
        Ok(())
    }
    fn font_size(&self, m: &Machine) -> i32 {
        self.sizes
            .get(&self.window)
            .copied()
            .unwrap_or_else(|| {
                m.gameexe
                    .int(&format!("WINDOW.{:03}.MOJI_SIZE", self.window))
                    .unwrap_or(24)
            })
            .clamp(1, 512)
    }
    pub(super) fn text(
        &mut self,
        m: &mut Machine,
        h: &mut dyn HostServices,
        text: &str,
    ) -> Result<()> {
        let text = interpret_names(text, |local, index| m.memory.name(local, index).to_owned());
        let size = self.font_size(m);
        let prefix = format!("WINDOW.{:03}.", self.window);
        let pos = m.gameexe.ints(&(prefix.clone() + "POS"));
        let pad = m.gameexe.ints(&(prefix.clone() + "MOJI_POS"));
        let rep = m.gameexe.ints(&(prefix.clone() + "MOJI_REP"));
        let count = m.gameexe.ints(&(prefix.clone() + "MOJI_CNT"));
        let left = pad.get(2).copied().unwrap_or(0);
        let top = pad.first().copied().unwrap_or(0);
        let columns = count.first().copied().unwrap_or(25).max(1);
        let rows = count.get(1).copied().unwrap_or(3).max(1);
        let pitch = size + rep.first().copied().unwrap_or(0);
        let height = size + rep.get(1).copied().unwrap_or(0);
        let width = columns * pitch;
        let bx = pos.get(1).copied().unwrap_or(0);
        let by = pos.get(2).copied().unwrap_or(0);
        let window_h = rows * height + top + pad.get(1).copied().unwrap_or(0);
        let y = if pos.first().is_some_and(|n| *n == 2 || *n == 3) {
            self.height as i32 - window_h - by
        } else {
            by
        };
        let id = 10000 + self.window * 4;
        let mut glyphs = preserved_glyph_styles(&self.query(h, EngineQuery::Layer(id))?)?;
        let old = glyphs.len();
        let metrics = h
            .font_metrics(&self.config.font_alias, size as f32, '字')
            .map_err(err)?;
        let color = colour(&m.gameexe, self.color as i32);
        let kinsoku = m.gameexe.int(&(prefix.clone() + "KINSOKU_USE")).unwrap_or(1) != 0;
        let pen = self.pen.entry(self.window).or_insert([0, 0]);
        let page_was_empty = *pen == [0, 0];
        let mut overflow: Option<usize> = None;
        for (rest_from, ch) in text.char_indices() {
            if ch == '\n' {
                pen[0] = 0;
                pen[1] += height;
                continue;
            }
            let cell = if ch.is_ascii() || ('\u{ff61}'..='\u{ff9f}').contains(&ch) {
                size / 2
            } else {
                size
            };
            // Upstream `needs_break`: kinsoku characters (and spaces) may hang
            // one cell past the line; a run of them stays with its character.
            let extended = width + size;
            let half = |c: char| if c.is_ascii() || ('\u{ff61}'..='\u{ff9f}').contains(&c) { size / 2 } else { size };
            let squeeze = kinsoku && (is_kinsoku(ch) || ch == ' ');
            let wrap = if squeeze && pen[0] + cell <= extended {
                false
            } else if pen[0] + cell > width {
                true
            } else if kinsoku && !squeeze {
                let mut end = pen[0] + cell;
                let mut wrap = false;
                for next in text[rest_from + ch.len_utf8()..].chars() {
                    if !is_kinsoku(next) {
                        break;
                    }
                    end += half(next);
                    if end > extended {
                        wrap = true;
                        break;
                    }
                }
                wrap
            } else {
                false
            };
            if wrap {
                pen[0] = 0;
                pen[1] += height;
            }
            if pen[1] + size > rows * height {
                // The window is full: show this page, then continue the
                // speech on a fresh one after a click (upstream PageFull).
                overflow = Some(rest_from);
                break;
            }
            glyphs.push(native_glyph(
                ch,
                [
                    bx + left + pen[0],
                    y + top + pen[1] + metrics.ascent.round() as i32,
                ],
                size,
                color,
            ));
            pen[0] += cell + rep.first().copied().unwrap_or(0);
        }
        if let Some(at) = overflow {
            ensure!(!(page_was_empty && at == 0), "RealLive text window too small for one line");
            self.page_rest = Some(text[at..].to_owned());
        }
        let mut backing = self.waku_layers(m, &prefix, [bx, y])?;
        if m.gameexe.int(&(prefix.clone() + "WAKU_SETNO")) == Some(-1) {
            let attr = m.gameexe.ints(&(prefix.clone() + "ATTR"));
            if attr.len() >= 4 {
                backing.push(json!({"op":"put","layer":{"id":id+2,"parent":SCENE_ROOT,"rect":[bx,y,width+left+size,window_h],"order":[999,0],"paint":{"kind":"solid","color":[attr[0].clamp(0,255),attr[1].clamp(0,255),attr[2].clamp(0,255),255]},"blend":if attr.get(4).copied().unwrap_or(0)==0{"subtract"}else{"normal"},"opacity":attr[3].clamp(0,255)}}));
            }
        }
        let speed = if self.fast_text {
            0
        } else {
            m.gameexe.int("INIT_MESSAGE_SPEED").unwrap_or(30).max(0)
        };
        let paint = json!({"kind":"glyph_run","font":self.config.font_alias,"size":size,"color":color,"glyphs":glyphs,"reveal":{"unit":"unicode_scalar","interval_ms":speed,"initial_visible":old+1}});
        backing.push(json!({"op":"put","layer":{"id":id,"parent":SCENE_ROOT,"rect":[0,0,self.width,self.height],"order":[1000,1],"paint":paint}}));
        if m.gameexe.int(&(prefix + "MOJI_SHADOW")).unwrap_or(1) != 0 {
            let mut shadow = backing.last().unwrap()["layer"].clone();
            shadow["id"] = json!(id + 1);
            shadow["order"] = json!([1000, 0]);
            shadow["rect"] = json!([1, 1, self.width, self.height]);
            black_glyph_shadow(&mut shadow)?;
            backing.push(json!({"op":"put","layer":shadow}));
        }
        self.submit(h, json!({"layers":backing}))?;
        m.message_page_empty = false;
        self.wait = Some(Wait::Reveal { layer: id });
        Ok(())
    }
    fn selection(
        &mut self,
        m: &mut Machine,
        h: &mut dyn HostServices,
        c: &Command,
    ) -> Result<Next> {
        let CommandKind::Select(select) = &c.kind else {
            bail!("selection without native choice data")
        };
        let window = select
            .window
            .as_ref()
            .map(|v| m.eval_int(v))
            .transpose()?
            .unwrap_or(self.window as i32);
        let size = m
            .gameexe
            .int(&format!("WINDOW.{window:03}.MOJI_SIZE"))
            .unwrap_or(24)
            .clamp(1, 512);
        // Choices are lines of a text window (upstream `lay_out_in_window`):
        // below the text already shown, or on a cleared page if they do not
        // fit; `SELCOM_USE` adds the `SELCOM_MOJIPOS` indent.
        let g = self.window_geometry(m, window as u32);
        let visible = select.options.len() as i32;
        let pen = self.pen.get(&(window as u32)).copied().unwrap_or([0, 0]);
        let mut first_row = if pen == [0, 0] { 0 } else { pen[1] / g.line.max(1) + i32::from(pen[0] > 0) };
        let prefix = format!("WINDOW.{window:03}.");
        if first_row + visible > g.rows.max(1) {
            self.clear_window(h, window as u32)?;
            // More choices than rows: grow upward so all stay on screen.
            first_row = (g.rows.max(1) - visible).min(0);
        }
        let frame = self.waku_layers(m, &prefix, [g.window_x, g.window_y])?;
        if !frame.is_empty() {
            self.submit(h, json!({ "layers": frame }))?;
        }
        let indent = if m.gameexe.int(&(prefix.clone() + "SELCOM_USE")).unwrap_or(0) != 0 {
            m.gameexe.int(&(prefix + "SELCOM_MOJIPOS")).unwrap_or(0).max(0)
        } else {
            0
        };
        let mut indices = Vec::new();
        let mut ids = Vec::new();
        let mut enabled_choices = Vec::new();
        let mut ops = Vec::new();
        for (index, option) in select.options.iter().enumerate() {
            let mut enabled = true;
            let mut hidden = false;
            for condition in &option.conditions {
                let applies = condition
                    .condition
                    .as_ref()
                    .map(|v| m.eval_int(v))
                    .transpose()?
                    .unwrap_or(1)
                    != 0;
                if applies {
                    match condition.effect {
                        b'0' | b'4' => {}
                        b'1' | b'3' => enabled = false,
                        b'2' => hidden = true,
                        _ => bail!("unknown native choice condition"),
                    }
                }
            }
            if hidden {
                continue;
            }
            let text = m.eval_str(&option.text)?;
            let id = 600000 + indices.len() as u32;
            let row = indices.len() as i32;
            // `SELCOM_USE=1`: the choices are lines of the text window
            // (upstream `select::draw_window_items`); otherwise a plain list.
            let rect = [g.x + indent, g.y + g.luby + (first_row + row) * g.line - g.rep_y / 2, g.width - indent, g.line];
            indices.push(index);
            ids.push(id);
            enabled_choices.push(enabled);
            let color = [0, 0, 0, 0];
            ops.push(json!({"op":"put","widget":{"id":id,"kind":"button","rect":rect,"text":text,"font":self.config.font_alias,"text_size":size,"enabled":enabled,"color":color,"z":1000}}));
        }
        ensure!(!indices.is_empty(), "empty native selection");
        self.submit(h, json!({"ui":ops}))?;
        self.wait = Some(Wait::Choice {
            indices,
            ids,
            enabled: enabled_choices,
        });
        Ok(Next::Advance)
    }
}
fn rgb(m: &mut Machine, c: &Command, at: usize) -> Result<[u8; 4]> {
    Ok([
        m.int_param(c, at)?.clamp(0, 255) as u8,
        m.int_param(c, at + 1)?.clamp(0, 255) as u8,
        m.int_param(c, at + 2)?.clamp(0, 255) as u8,
        255,
    ])
}
fn colour(exe: &Gameexe, n: i32) -> [u8; 4] {
    let v = exe.ints(&format!("COLOR_TABLE.{n:03}"));
    [
        v.first().copied().unwrap_or(255).clamp(0, 255) as u8,
        v.get(1).copied().unwrap_or(255).clamp(0, 255) as u8,
        v.get(2).copied().unwrap_or(255).clamp(0, 255) as u8,
        255,
    ]
}
fn read_rect(m: &mut Machine, c: &Command, at: usize, rec: bool) -> Result<[f32; 4]> {
    let a = m.int_param(c, at)?;
    let b = m.int_param(c, at + 1)?;
    let cc = m.int_param(c, at + 2)?;
    let d = m.int_param(c, at + 3)?;
    Ok(if rec {
        [a as f32, b as f32, cc as f32, d as f32]
    } else {
        [
            a.min(cc) as f32,
            b.min(d) as f32,
            (a.abs_diff(cc) + 1) as f32,
            (b.abs_diff(d) + 1) as f32,
        ]
    })
}
/// `r` ([x, y, w, h]) intersected with an image of `size`.
fn clip_source(r: [f32; 4], size: [u32; 2]) -> [f32; 4] {
    let x0 = r[0].clamp(0., size[0] as f32);
    let y0 = r[1].clamp(0., size[1] as f32);
    let x1 = (r[0] + r[2]).clamp(x0, size[0] as f32);
    let y1 = (r[1] + r[3]).clamp(y0, size[1] as f32);
    [x0, y0, x1 - x0, y1 - y0]
}
fn uv(r: [f32; 4], size: [u32; 2]) -> [f32; 4] {
    [
        r[0] / size[0] as f32,
        r[1] / size[1] as f32,
        (r[0] + r[2]) / size[0] as f32,
        (r[1] + r[3]) / size[1] as f32,
    ]
}
fn display(
    m: &mut Machine,
    c: &Command,
    at: usize,
    w: u32,
    h: u32,
    rec: bool,
) -> Result<(Option<[f32; 4]>, [f32; 2], i32, i32, u8)> {
    let remaining = c.params.len() - at;
    let mut v = if remaining >= 16 {
        m.int_params_from(c, at)?
    } else {
        let index = m.int_param_or(c, at, 0)?;
        let r = m.gameexe.ints(&format!("SEL.{index:03}"));
        if r.is_empty() {
            vec![
                0,
                0,
                w as i32 - 1,
                h as i32 - 1,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                255,
                0,
            ]
        } else {
            r
        }
    };
    v.resize(16, 0);
    let mut rect = if rec && remaining >= 15 {
        [v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32]
    } else {
        [
            v[0] as f32,
            v[1] as f32,
            (v[2] - v[0] + 1) as f32,
            (v[3] - v[1] + 1) as f32,
        ]
    };
    let mut pos = [v[4] as f32, v[5] as f32];
    let mut alpha = if v[14] == 0 {
        255
    } else {
        v[14].clamp(0, 255) as u8
    };
    if remaining == 2 {
        alpha = m.int_param(c, at + 1)?.clamp(0, 255) as u8;
    }
    if remaining == 7 || remaining == 8 {
        rect = read_rect(m, c, at + 1, rec)?;
        pos = [
            m.int_param(c, at + 5)? as f32,
            m.int_param(c, at + 6)? as f32,
        ];
        if remaining == 8 {
            alpha = m.int_param(c, at + 7)?.clamp(0, 255) as u8;
        }
    }
    Ok((
        if rect[0] == 0. && rect[1] == 0. && rect[2] <= 1. && rect[3] <= 1. {
            None
        } else {
            Some(rect)
        },
        pos,
        v[6],
        v[7],
        alpha,
    ))
}

// Native name-variable syntax from pinned siglus_rs textout.rs; no rendering.
fn interpret_names(text: &str, name: impl Fn(bool, usize) -> String) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let letter = |c: char| ('Ａ'..='Ｚ').contains(&c);
    while i < chars.len() {
        let c = chars[i];
        if (c == '＊' || c == '％') && chars.get(i + 1).copied().is_some_and(letter) {
            let mut letters = String::from(chars[i + 1]);
            let mut j = i + 2;
            if chars.get(j).copied().is_some_and(letter) {
                letters.push(chars[j]);
                j += 1;
            }
            let value = crate::vm::memory::Memory::name_index(&letters)
                .map(|index| name(c == '％', index))
                .unwrap_or_default();
            if let Some(digit) = chars.get(j).filter(|d| ('０'..='９').contains(d)) {
                let nth = *digit as usize - '０' as usize;
                out.extend(value.chars().nth(nth));
                j += 1;
            } else {
                out.push_str(&value);
            }
            i = j;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

// Native page append semantics: previously drawn characters keep the style
// in force when they were written, even when the containing run changes.
fn preserved_glyph_styles(layer: &Value) -> Result<Vec<Value>> {
    if layer.is_null() {
        return Ok(Vec::new());
    }
    let paint: crysirror_scene::layer::Paint = serde_json::from_value(layer["paint"].clone())?;
    let crysirror_scene::layer::Paint::GlyphRun {
        size,
        color,
        bold,
        mut glyphs,
        ..
    } = paint
    else {
        bail!("native text layer no longer glyph run");
    };
    for glyph in &mut glyphs {
        glyph.size.get_or_insert(size);
        glyph.color.get_or_insert(color);
        glyph.bold.get_or_insert(bold);
    }
    glyphs
        .into_iter()
        .map(|glyph| Ok(serde_json::to_value(glyph)?))
        .collect()
}
fn native_glyph(character: char, baseline: [i32; 2], size: i32, color: [u8; 4]) -> Value {
    json!({"character":character.to_string(),"baseline":baseline,"size":size,"color":color,"bold":false})
}
fn black_glyph_shadow(layer: &mut Value) -> Result<()> {
    let paint = &mut layer["paint"];
    paint["color"] = json!([0, 0, 0, 255]);
    for glyph in paint["glyphs"]
        .as_array_mut()
        .context("native shadow glyphs absent")?
    {
        glyph["color"] = json!([0, 0, 0, 255]);
    }
    Ok(())
}
impl RealLiveVm {
    /// Module 1:40 (upstream `scr::bgr` / `load_haikei`): DC 0 is cleared to
    /// black, the background drawn at the haikei position, objects promoted,
    /// and the `#SEL` transition runs from the previous screen.
    fn haikei(&mut self, m: &mut Machine, h: &mut dyn HostServices, c: &Command) -> Result<()> {
        let op = c.op.opcode;
        let count = Machine::param_count(c);
        let (name, sel, time, promote, overlays) = match op {
            10 | 11 | 20 | 21 | 101 => {
                let mut count = count;
                let time = if matches!(op, 20 | 21 | 101) && count >= 2 {
                    count -= 1;
                    Some(m.int_param(c, count)?)
                } else {
                    None
                };
                let (name, sel) = if count <= 1 {
                    (None, m.int_param_or(c, 0, 0)?)
                } else {
                    (Some(m.str_param(c, 0)?), m.int_param(c, 1)?)
                };
                (name, sel, time, !matches!(op, 11 | 21), 0)
            }
            // bgrMulti(file[, sel], (file[, x, y])...): overload 1 carries the
            // `#SEL` number before the overlays.
            100 => {
                let with_sel = c.params.get(1).is_some_and(|p| !matches!(p.value, crate::vm::expr::Expr::Special { .. }));
                let sel = if with_sel { m.int_param(c, 1)? } else { 0 };
                (Some(m.str_param(c, 0)?), sel, None, true, c.params.len().saturating_sub(1 + usize::from(with_sel)))
            }
            _ => bail!("unsupported native background operation {}", c.op),
        };
        let name = name.map(|n| {
            if !n.is_empty() && n.chars().all(|ch| ch == '?') { m.sys.default_bgr.clone() } else { n }
        });
        let mut ops = vec![json!({"op":"clear","id":0,"color":[0,0,0,255]})];
        if let Some(name) = &name {
            m.sys.default_bgr.clone_from(name);
            let hik = self.resources.as_ref().context("RealLive resources")?.borrow_mut().file("", &format!("{name}.hik"))?;
            ensure!(hik.is_none(), "RealLive HIK animated backgrounds need an engine mapping");
            let asset = self.resource(m, "image", name)?;
            let size = h.image_size(self.client, &asset).map_err(err)?;
            ops.push(json!({"op":"draw","id":0,"layer":self.image_layer(asset, [0., 0., size[0] as f32, size[1] as f32], None, 255)}));
        }
        let first_overlay = c.params.len() - overlays;
        for index in first_overlay..c.params.len() {
            let pieces = m.complex_param(c, index)?;
            let Some(first) = pieces.first() else { continue };
            let file = m.eval_str(first)?;
            let x = match pieces.get(1) { Some(e) => m.eval_int(e)?, None => 0 };
            let y = match pieces.get(2) { Some(e) => m.eval_int(e)?, None => 0 };
            let asset = self.resource(m, "image", &file)?;
            let size = h.image_size(self.client, &asset).map_err(err)?;
            ops.push(json!({"op":"draw","id":0,"layer":self.image_layer(asset, [x as f32, y as f32, size[0] as f32, size[1] as f32], None, 255)}));
        }
        if self.grp_closes {
            self.hide_text(h)?;
        }
        self.canvas(h, ops)?;
        if promote {
            m.sys.gfx.promote_objects();
        }
        self.sync_objects(m, h)?;
        let mut values = m.gameexe.ints(&format!("SEL.{sel:03}"));
        values.resize(16, 0);
        if let Some(t) = time {
            values[6] = t;
        }
        self.sel_transition(h, &values)
    }

    /// Start the engine transition a `#SEL` row describes (time at 6,
    /// style at 7, pattern size at 10/11, steps at 9).
    fn sel_transition(&mut self, h: &mut dyn HostServices, values: &[i32]) -> Result<()> {
        let get = |i: usize| values.get(i).copied().unwrap_or(0);
        let (time, style) = (get(6), get(7));
        if time <= 0 || style == 1 {
            return Ok(());
        }
        let effect = match style {
            0 | 50 | 54 => json!({"kind":"crossfade"}),
            4 | 5 => json!({"kind":"ordered_dither","cell":[get(10).max(1),get(11).max(1)],"diagonal":style==5,"steps":1+get(9).max(0)}),
            _ => {
                // Every other reveal style: the engine's rule image for it.
                let key = values.iter().take(12).map(|v| v.to_string()).collect::<Vec<_>>().join(",");
                let rule = if self.rules.contains_key(&key) {
                    None
                } else {
                    crysirror_image::reallive_wipe::reveal_rule(values, self.width, self.height)
                };
                if !self.rules.contains_key(&key) && rule.is_none() {
                    // Scroll/squash/slide/zoom/ripple styles move pictures; the
                    // engine has no motion transition for them yet. Crossfade,
                    // and record the substitution in diagnostics.
                    *self.approximated.entry(style).or_default() += 1;
                    self.submit(h, json!({"layers":[{"op":"transition","transition":{"effect":{"kind":"crossfade"},"duration_ms":time}}]}))?;
                    self.wait = Some(Wait::Time { until: h.now_ms().saturating_add(time as u64), cancel: false });
                    return Ok(());
                }
                let asset = match self.rules.get(&key) {
                    Some(asset) => asset.clone(),
                    None => {
                        let (rgba, _) = rule.context("reveal rule")?;
                        let id = 0x5245_0000 + self.rules.len() as u32;
                        let path = h.upload_rgba(self.client, id, self.width, self.height, &rgba).map_err(err)?;
                        let asset = path.relative().to_owned();
                        self.rules.insert(key, asset.clone());
                        asset
                    }
                };
                json!({"kind":"mask","asset":asset,"fade":get(9).clamp(0, 8)})
            }
        };
        self.submit(h, json!({"layers":[{"op":"transition","transition":{"effect":effect,"duration_ms":time}}]}))?;
        self.wait = Some(Wait::Time { until: h.now_ms().saturating_add(time as u64), cancel: false });
        Ok(())
    }

    /// Text-area geometry of a window, as `text` lays glyphs out.
    fn window_geometry(&self, m: &Machine, window: u32) -> WindowGeometry {
        let prefix = format!("WINDOW.{window:03}.");
        let exe = &m.gameexe;
        let size = exe.int(&(prefix.clone() + "MOJI_SIZE")).unwrap_or(24).clamp(1, 512);
        let pos = exe.ints(&(prefix.clone() + "POS"));
        let pad = exe.ints(&(prefix.clone() + "MOJI_POS"));
        let rep = exe.ints(&(prefix.clone() + "MOJI_REP"));
        let count = exe.ints(&(prefix + "MOJI_CNT"));
        let left = pad.get(2).copied().unwrap_or(0);
        let top = pad.first().copied().unwrap_or(0);
        let columns = count.first().copied().unwrap_or(25).max(1);
        let rows = count.get(1).copied().unwrap_or(3).max(1);
        let pitch = size + rep.first().copied().unwrap_or(0);
        let line = size + rep.get(1).copied().unwrap_or(0);
        let bx = pos.get(1).copied().unwrap_or(0);
        let by = pos.get(2).copied().unwrap_or(0);
        let window_h = rows * line + top + pad.get(1).copied().unwrap_or(0);
        let y = if pos.first().is_some_and(|n| *n == 2 || *n == 3) { self.height as i32 - window_h - by } else { by };
        let luby = exe.int(&format!("WINDOW.{window:03}.LUBY_SIZE")).unwrap_or(0);
        WindowGeometry { window_x: bx, window_y: y, x: bx + left, y: y + top, width: columns * pitch, line, rows, rep_y: rep.get(1).copied().unwrap_or(0), luby }
    }

    /// The window frame (`#WAKU.set.pattern`, upstream `textout::waku` and
    /// `draw_window` for non-nine-slice types): BACK tinted by the window
    /// attribute (subtractive for filter 0), NAME drawn over it.
    fn waku_layers(&mut self, m: &Machine, prefix: &str, at: [i32; 2]) -> Result<Vec<Value>> {
        let exe = &m.gameexe;
        let set = exe.int(&format!("{prefix}WAKU_SETNO")).unwrap_or(0);
        if set < 0 {
            return Ok(vec![]);
        }
        let waku_mod = exe.int(&format!("{prefix}WAKU_MOD")).unwrap_or(0);
        let pattern = if waku_mod != 0 {
            exe.int(&format!("{prefix}WAKU_NO")).unwrap_or(0)
        } else {
            m.sys.settings.waku_all
        };
        let key = |part: &str| format!("WAKU.{set:03}.{pattern:03}.{part}");
        let kind = exe.int(&key("TYPE")).or_else(|| exe.int(&format!("WAKU.{set:03}.TYPE")));
        ensure!(!matches!(kind, Some(3 | 4)), "RealLive nine-slice window frames (WAKU type 3/4) need an engine nine-slice paint");
        let attr = if exe.int(&format!("{prefix}ATTR_MOD")).unwrap_or(0) != 0 {
            crate::vm::settings::WindowAttr::from_slice(&exe.ints(&format!("{prefix}ATTR"))).unwrap_or_default()
        } else {
            m.sys.settings.window_attr
        };
        let window = (prefix.trim_start_matches("WINDOW.").trim_end_matches('.').parse::<u32>()).unwrap_or(0);
        let base = 20000 + window * 8;
        let loader = m.sys.gfx.loader.clone().context("RealLive resources")?;
        let image = |name: Option<&str>| -> Result<Option<Rc<crate::vm::graphics::Image>>> {
            match name.filter(|n| !n.is_empty()) {
                Some(n) => Ok(Some(loader.image(n)?)),
                None => Ok(None),
            }
        };
        let back = image(exe.str(&key("BACK")))?;
        let main = image(exe.str(&key("NAME")))?;
        let colour = [attr.r, attr.g, attr.b].map(|c| c.clamp(0, 255) as f32 / 255.);
        let strength = attr.alpha.clamp(0, 255);
        let sprite = |id: u32, img: &crate::vm::graphics::Image, order: i32| {
            json!({"id": id, "parent": SCENE_ROOT, "rect": [at[0], at[1], img.width, img.height], "paint": {"kind":"image","asset":img.asset}, "order": [998, order]})
        };
        let mut layers = vec![];
        if let Some(back) = &back {
            let mut tint = sprite(base, back, 0);
            tint["filter"] = json!({"color_rate": 1.0, "color": colour});
            tint["opacity"] = json!(strength);
            if attr.filter == 0 {
                // d - m + colour * m: subtract the mask, then add the colour.
                let mut cut = sprite(base + 1, back, 0);
                cut["filter"] = json!({"color_rate": 1.0, "color": [1.0, 1.0, 1.0]});
                cut["opacity"] = json!(strength);
                cut["blend"] = json!("subtract");
                tint["blend"] = json!("add");
                tint["order"] = json!([998, 1]);
                layers.push(cut);
            }
            layers.push(tint);
        }
        if let Some(main) = &main {
            layers.push(sprite(base + 2, main, 2));
        }
        self.waku_ids.extend(layers.iter().filter_map(|l| l["id"].as_u64()).map(|id| id as u32));
        self.waku_ids.sort_unstable();
        self.waku_ids.dedup();
        Ok(layers.into_iter().map(|layer| json!({"op":"put","layer":layer})).collect())
    }
}

#[allow(dead_code)]
fn require_supported_text_window(exe: &Gameexe, prefix: &str) -> Result<()> {
    // Pinned WindowConfig::from_gameexe uses set 0 when this key is absent.
    // A nonnegative set selects a WAKU resource; it cannot silently become an
    // unframed window until that skin policy is mapped to shared engine draws.
    let set = exe.int(&format!("{prefix}WAKU_SETNO")).unwrap_or(0);
    ensure!(
        set == -1,
        "native text window WAKU_SETNO={set} requires a shared window skin API"
    );
    Ok(())
}

#[cfg(test)]
mod text_style_tests {
    use super::*;
    #[test]
    fn appended_page_keeps_prior_defaults_and_explicit_glyph_overrides() {
        let previous = json!({"paint":{"kind":"glyph_run","font":"project-font","size":24,"color":[255,0,0,255],"bold":true,
            "glyphs":[{"character":"A","baseline":[0,24]},
            {"character":"B","baseline":[12,24],"size":12,"color":[0,0,255,255],"bold":false}]}});
        let mut glyphs = preserved_glyph_styles(&previous).unwrap();
        glyphs.push(native_glyph('新', [24, 24], 32, [0, 255, 0, 255]));
        let mut next = json!({"paint":{"kind":"glyph_run","font":"project-font","size":32,"color":[0,255,0,255],"glyphs":glyphs}});
        let saved = next.clone();
        let typed: crysirror_scene::layer::Paint =
            serde_json::from_value(next["paint"].clone()).unwrap();
        typed.validate().unwrap();
        assert_eq!(next["paint"]["glyphs"][0]["size"], 24.);
        assert_eq!(next["paint"]["glyphs"][0]["color"], json!([255, 0, 0, 255]));
        assert_eq!(next["paint"]["glyphs"][0]["bold"], true);
        assert_eq!(next["paint"]["glyphs"][1]["size"], 12.);
        assert_eq!(next["paint"]["glyphs"][1]["color"], json!([0, 0, 255, 255]));
        assert_eq!(next["paint"]["glyphs"][1]["bold"], false);
        assert_eq!(next["paint"]["glyphs"][2]["size"], 32);
        assert_eq!(next["paint"]["glyphs"][2]["color"], json!([0, 255, 0, 255]));
        assert_eq!(next["paint"]["glyphs"][2]["bold"], false);
        black_glyph_shadow(&mut next).unwrap();
        for glyph in next["paint"]["glyphs"].as_array().unwrap() {
            assert_eq!(glyph["color"], json!([0, 0, 0, 255]));
        }
        assert_eq!(
            next["paint"]["glyphs"][0]["size"],
            saved["paint"]["glyphs"][0]["size"]
        );
        assert_eq!(
            previous["paint"]["glyphs"][0].get("size"),
            None,
            "normalization does not mutate the inspected engine state"
        );
    }
    #[test]
    fn absent_text_layer_is_empty_but_conflicting_engine_paint_fails() {
        assert!(preserved_glyph_styles(&Value::Null).unwrap().is_empty());
        assert!(
            preserved_glyph_styles(&json!({"paint":{"kind":"solid"}}))
                .unwrap_err()
                .to_string()
                .contains("no longer glyph run")
        );
        let defaults = json!({"paint":{"kind":"glyph_run","font":"project-font","size":20,"glyphs":[{"character":"字","baseline":[0,20]}]}});
        let glyphs = preserved_glyph_styles(&defaults).unwrap();
        assert_eq!(glyphs[0]["color"], json!([255, 255, 255, 255]));
        assert_eq!(glyphs[0]["bold"], false);
    }
    #[test]
    fn resource_window_skins_fail_explicitly_including_native_default_zero() {
        for config in [
            "",
            "#WINDOW.002.WAKU_SETNO=0\n",
            "#WINDOW.002.WAKU_SETNO=4\n",
        ] {
            assert!(
                require_supported_text_window(&Gameexe::parse(config), "WINDOW.002.")
                    .unwrap_err()
                    .to_string()
                    .contains("shared window skin API")
            );
        }
        assert!(
            require_supported_text_window(
                &Gameexe::parse("#WINDOW.002.WAKU_SETNO=-1\n"),
                "WINDOW.002."
            )
            .is_ok()
        );
    }
}

impl RealLiveVm {
    /// `LoadDLL` / `UnloadDLL` / `CallDLL` (2:1), as upstream `modules::dll`.
    /// The EF00/PT00/DT00 emulation is the vendored `reallive-dll` crate.
    fn dll(&mut self, m: &mut Machine, c: &Command) -> Result<()> {
        match c.op.opcode {
            10 => {
                let slot = m.int_param(c, 0)?;
                let name = m.str_param(c, 1)?;
                m.sys.dlls.load(slot, &name);
            }
            11 => {
                let slot = m.int_param_or(c, 0, 0)?;
                m.sys.dlls.unload(slot);
            }
            12 => {
                let slot = m.int_param(c, 0)?;
                let mut args = [0; 5];
                for (i, arg) in args.iter_mut().enumerate() {
                    *arg = m.int_param_or(c, i + 1, 0)?;
                }
                let resources = self.resources.clone().context("RealLive resources")?;
                let mut resources = resources.borrow_mut();
                let mut dlls = std::mem::take(&mut m.sys.dlls);
                let Machine { memory, stack, sys, .. } = &mut *m;
                let mut sys = DllSystem { rng: sys, resources: &mut resources };
                let frame = &mut stack.last_mut().context("empty native stack")?.vars;
                let mut banks = Banks { memory, frame };
                // Restore the slots even when the call fails.
                let result = dlls.call(slot, &mut sys, &mut banks, args);
                m.sys.dlls = dlls;
                m.store = result?;
            }
            _ => bail!("unsupported native DLL operation {}", c.op),
        }
        Ok(())
    }
}

/// The original System calls a DLL makes: the script RNG and its own file.
struct DllSystem<'a> {
    rng: &'a mut crate::vm::sys_state::Sys,
    resources: &'a mut Resources,
}
impl reallive_dll::System for DllSystem<'_> {
    fn random(&mut self, low: i32, high: i32) -> i32 {
        self.rng.random(low, high)
    }
    fn root_file(&mut self, name: &str) -> Result<Vec<u8>> {
        let path = self
            .resources
            .file("", name)?
            .with_context(|| format!("reallive: {name} not found in the game directory"))?;
        self.resources.read(&path)
    }
}

/// intA–intF, intG, intZ of the running machine (upstream `dll::Banks`).
struct Banks<'a> {
    memory: &'a mut crate::vm::memory::Memory,
    frame: &'a mut crate::vm::memory::FrameMemory,
}
impl reallive_dll::DllMemory for Banks<'_> {
    fn bank(&mut self, bank: u8) -> Result<&mut [i32]> {
        self.memory.words_mut(bank, self.frame)
    }
}

/// A channel group's player volume (`#VOLUME` settings, enable switch).
fn settings_volume(s: &crate::vm::settings::Settings, index: usize) -> f32 {
    if !s.enabled[index] {
        return 0.;
    }
    s.volume[index].clamp(0, 255) as f32 / 255.
}

/// Where a text window and its text area are on screen.
struct WindowGeometry {
    window_x: i32,
    window_y: i32,
    x: i32,
    y: i32,
    width: i32,
    line: i32,
    rows: i32,
    rep_y: i32,
    luby: i32,
}

/// UI id of the name-entry OK button.
pub(super) const NAME_OK: u32 = 619_999;

// Copied from siglus_rs `textout.rs` (5f358ef).
/// Characters that may not start a line.
pub fn is_kinsoku(c: char) -> bool {
    matches!(
        c,
        '、' | '。'
            | '，'
            | '．'
            | '：'
            | '；'
            | '？'
            | '！'
            | '゛'
            | '゜'
            | 'ヽ'
            | 'ヾ'
            | 'ゝ'
            | 'ゞ'
            | '々'
            | 'ー'
            | '）'
            | '］'
            | '｝'
            | '」'
            | '』'
            | '〕'
            | '〉'
            | '》'
            | '】'
            | '’'
            | '”'
            | 'ぁ'
            | 'ぃ'
            | 'ぅ'
            | 'ぇ'
            | 'ぉ'
            | 'っ'
            | 'ゃ'
            | 'ゅ'
            | 'ょ'
            | 'ゎ'
            | 'ァ'
            | 'ィ'
            | 'ゥ'
            | 'ェ'
            | 'ォ'
            | 'ッ'
            | 'ャ'
            | 'ュ'
            | 'ョ'
            | 'ヮ'
            | 'ヵ'
            | 'ヶ'
            | '・'
            | '…'
            | '‥'
            | '～'
            | '─'
            | ','
            | '.'
            | '!'
            | '?'
            | ')'
            | ']'
            | '}'
    )
}
