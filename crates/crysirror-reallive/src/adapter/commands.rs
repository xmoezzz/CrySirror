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
                20 => {}
                21 | 22 => bail!(
                    "manual native presentation requires engine surface visibility transactions"
                ),
                _ => bail!("unsupported screen operation"),
            },
            (1, 31) => m.yield_frame = true,
            (1, 33) => self.graphics(m, h, c)?,
            (0, 3) => self.message(m, h, c)?,
            (1, 10) if c.op.opcode == 100 => {
                let s = m.str_param(c, 0)?;
                self.text(m, h, &s)?;
            }
            (1, 20 | 21 | 22 | 23) => self.sound(m, h, c)?,
            (1, 26) => self.movie(m, h, c)?,
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
                ensure!(
                    mask || h.image_is_opaque(self.client, &asset).map_err(err)?,
                    "non-mask copying a transparent image needs a shared alpha-override API"
                );
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
                let layer = self.image_layer(
                    asset,
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
                ensure!(
                    matches!(style, 0 | 1 | 4 | 5) || time == 0,
                    "transition style {style} needs a shared engine implementation"
                );
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
                        self.default_grp.clone()
                    } else {
                        name.clone()
                    };
                    let asset = self.resource(m, "image", &resource)?;
                    let size = h.image_size(self.client, &asset).map_err(err)?;
                    ensure!(
                        op == 74 || h.image_is_opaque(self.client, &asset).map_err(err)?,
                        "non-mask opening a transparent image needs shared alpha override"
                    );
                    let r = src.unwrap_or([0., 0., size[0] as f32, size[1] as f32]);
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
                    ops.push(json!({"op":if full{"set"}else{"draw"},"id":1,"layer":layer}));
                    ops.push(json!({"op":"copy","source":1,"destination":0,"rect":[0,0,self.width,self.height],"position":[0,0],"opacity":255,"replace":true}));
                    if op == 73 && name != "???" {
                        self.default_grp = name;
                    }
                }
                self.canvas(h, ops)?;
                if time > 0 && style != 1 {
                    let effect = if matches!(style, 4 | 5) {
                        let values = if c.params.len() >= 17 {
                            m.int_params_from(c, 1)?
                        } else {
                            let index = m.int_param_or(c, 1, 0)?;
                            m.gameexe.ints(&format!("SEL.{index:03}"))
                        };
                        json!({"kind":"ordered_dither","cell":[values.get(10).copied().unwrap_or(1).max(1),values.get(11).copied().unwrap_or(1).max(1)],"diagonal":style==5,"steps":1+values.get(9).copied().unwrap_or(0).max(0)})
                    } else {
                        json!({"kind":"crossfade"})
                    };
                    self.submit(h,json!({"layers":[{"op":"transition","transition":{"effect":effect,"duration_ms":time}}]}))?;
                    self.wait = Some(Wait::Time {
                        until: h.now_ms().saturating_add(time as u64),
                        cancel: false,
                    });
                }
            }
            100 | 101 | 102 => {
                ensure!(
                    op == 100,
                    "external copy masks require a shared canvas mask API"
                );
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
                let replace = pos == [0., 0.]
                    && rect == [0., 0., target[0] as f32, target[1] as f32]
                    && alpha == 255;
                self.canvas(h,vec![json!({"op":"copy","source":source,"destination":dst,"rect":rect,"position":pos,"opacity":alpha,"replace":replace})])?;
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
                let bytes = self.resources.as_ref().unwrap().read(&asset)?;
                let fade = m
                    .int_param_or(c, if module == 20 { 1 } else { 2 }, 0)?
                    .max(0) as f32;
                ensure!(
                    module != 20
                        || m.int_param_or(c, 2, 0)? <= 0
                        || !h.audio_playing(self.client, channel).map_err(err)?,
                    "outgoing music fade needs shared queued replacement API"
                );
                h.submit_audio(
                    self.client,
                    AudioRequest::Play {
                        channel,
                        source: EncodedAudio {
                            bytes: Arc::from(bytes),
                            frame_limit,
                        },
                        intro: None,
                        parameters: AudioParameters {
                            looping: if module == 20 { op != 2 } else { op == 2 },
                            loop_start,
                            start_ms,
                            fade_ms: fade,
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
                if module == 20 && op == 1 || module == 21 && op == 1 {
                    self.wait = Some(Wait::Audio { channel });
                }
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
            101 => {
                let i = if c.params.len() > 1 { 1 } else { 0 };
                let size = m.int_param(c, i)?;
                ensure!((1..=512).contains(&size), "native text size");
                self.sizes.insert(self.window, size);
            }
            151 => self.clear_window(h, self.window)?,
            161 => self.hide_text(h)?,
            152 | 162 => {
                self.hide_text(h)?;
                self.pen.clear();
                m.message_page_empty = true;
            }
            201 => {
                let size = self.font_size(m);
                let pen = self.pen.entry(self.window).or_insert([0, 0]);
                pen[0] = 0;
                pen[1] += size + 10;
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
    fn clear_window(&mut self, h: &mut dyn HostServices, window: u32) -> Result<()> {
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
        require_supported_text_window(&m.gameexe, &prefix)?;
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
        let pen = self.pen.entry(self.window).or_insert([0, 0]);
        for ch in text.chars() {
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
            if pen[0] + cell > width {
                pen[0] = 0;
                pen[1] += height;
            }
            ensure!(
                pen[1] + size <= rows * height,
                "native page full requires paragraph policy"
            );
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
        let mut backing = vec![];
        if m.gameexe.int(&(prefix.clone() + "WAKU_SETNO")) == Some(-1) {
            let attr = m.gameexe.ints(&(prefix.clone() + "ATTR"));
            if attr.len() >= 4 {
                backing.push(json!({"op":"put","layer":{"id":id+2,"rect":[bx,y,width+left+size,window_h],"order":[999,0],"paint":{"kind":"solid","color":[attr[0].clamp(0,255),attr[1].clamp(0,255),attr[2].clamp(0,255),255]},"blend":if attr.get(4).copied().unwrap_or(0)==0{"subtract"}else{"normal"},"opacity":attr[3].clamp(0,255)}}));
            }
        }
        let speed = if self.fast_text {
            0
        } else {
            m.gameexe.int("INIT_MESSAGE_SPEED").unwrap_or(30).max(0)
        };
        let paint = json!({"kind":"glyph_run","font":self.config.font_alias,"size":size,"color":color,"glyphs":glyphs,"reveal":{"unit":"unicode_scalar","interval_ms":speed,"initial_visible":old+1}});
        backing.push(json!({"op":"put","layer":{"id":id,"rect":[0,0,self.width,self.height],"order":[1000,1],"paint":paint}}));
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
            let y = (indices.len() as i32) * (size + 4);
            indices.push(index);
            ids.push(id);
            enabled_choices.push(enabled);
            ops.push(json!({"op":"put","widget":{"id":id,"kind":"button","rect":[0,y,self.width,size+4],"text":text,"font":self.config.font_alias,"text_size":size,"enabled":enabled,"color":[0,0,0,160],"z":1000}}));
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
