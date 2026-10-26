// SPDX-License-Identifier: AGPL-3.0-or-later
//! `krkr` VmAdapter: runs original TJS2 scripts on the vendored Kirakira VM
//! and translates KiriKiri native objects onto engine Layer/Canvas services.
use crate::{
    host::{Batch, KrkrHost},
    natives::{self, Rt, has_member},
    storage::Storage,
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame, buttons},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use krkr_tjs2::runtime::{ObjectHandle, Runtime, Variant};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Game directory inside the client's asset root ("" is the root).
    #[serde(default)]
    pub root: String,
    /// First script executed, KiriKiri's `startup.tjs`.
    #[serde(default = "startup")]
    pub startup: String,
    /// Project font alias used for every KiriKiri font face.
    #[serde(default = "font")]
    pub font_alias: String,
    /// Logical canvas size; must equal the project canvas.
    pub canvas: [u32; 2],
}
fn startup() -> String {
    "startup.tjs".into()
}
fn font() -> String {
    "default".into()
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if !self.root.is_empty() {
            let p = VfsPath::new(Mount::Asset, &self.root).map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(
                p.relative() == self.root,
                "canonical KiriKiri root required"
            );
        }
        ensure!(
            !self.startup.is_empty() && !self.font_alias.is_empty(),
            "startup and font alias required"
        );
        ensure!(
            self.canvas.iter().all(|v| (1..=8192).contains(v)),
            "KiriKiri canvas size"
        );
        Ok(())
    }
}

pub struct KrkrVm {
    client: ClientId,
    config: Config,
    rt: Option<Rt>,
    last_down: u64,
    last_pointer: [i32; 2],
    pressed: Option<usize>,
    frames: u64,
    last_error: Option<String>,
    halted: bool,
    events: BTreeMap<&'static str, u64>,
}

impl KrkrVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            rt: None,
            last_down: 0,
            last_pointer: [-1, -1],
            pressed: None,
            frames: 0,
            last_error: None,
            halted: false,
            events: BTreeMap::new(),
        })
    }

    fn boot(&mut self, h: &mut dyn HostServices) -> Result<()> {
        let files = h.storage(self.client).map_err(err)?;
        let storage = Storage::new(files, &self.config.root)?;
        ensure!(
            storage.resolve(&self.config.startup).is_some(),
            "KiriKiri startup script `{}` absent",
            self.config.startup
        );
        let mut host = KrkrHost::new(self.config.canvas, self.config.font_alias.clone());
        host.storage = Some(storage);
        host.images = Some(h.image_metadata(self.client).map_err(err)?);
        host.text_measurement = Some(h.text_measurement(self.client).map_err(err)?);
        host.now = h.now_ms();
        let mut rt = Runtime::with_host(host);
        natives::install(&mut rt);
        let startup = self.config.startup.clone();
        let result = natives::exec_storage(&mut rt, &startup);
        self.rt = Some(rt);
        result
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("execute {startup}"))?;
        self.flush(h)
    }

    /// Submit queued engine commands in the order TJS issued them.
    fn flush(&mut self, h: &mut dyn HostServices) -> Result<()> {
        let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
        rt.host_mut().sync();
        let batches = std::mem::take(&mut rt.host_mut().batches);
        for batch in batches {
            let payload = match batch {
                Batch::Layers(ops) => json!({"layers": ops}),
                Batch::Canvases(ops) => json!({"canvases": ops}),
            };
            h.submit_engine(self.client, 1, &serde_json::to_vec(&payload)?)
                .map_err(err)
                .with_context(|| format!("engine batch {}", truncate(&payload.to_string())))?;
        }
        Ok(())
    }

    fn call(
        &mut self,
        target: ObjectHandle,
        method: &'static str,
        args: Vec<Variant>,
    ) -> Result<()> {
        let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
        if !rt.object_valid(target) || !has_member(rt, target, method) {
            return Ok(());
        }
        *self.events.entry(method).or_default() += 1;
        rt.call_object_method(target, method, args)
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("TJS event {method}"))
    }

    fn input(&mut self, input: InputFrame) -> Result<()> {
        let rt = self.rt.as_ref().context("KiriKiri runtime absent")?;
        let window = rt.host().windows.keys().next().copied().map(ObjectHandle);
        let Some(window) = window else {
            self.last_down = input.buttons_down;
            return Ok(());
        };
        let (x, y) = (i64::from(input.pointer_x), i64::from(input.pointer_y));
        let hit = rt.host().hit(x, y);
        if [input.pointer_x, input.pointer_y] != self.last_pointer {
            self.last_pointer = [input.pointer_x, input.pointer_y];
            self.call(
                window,
                "onMouseMove",
                vec![
                    Variant::Integer(x),
                    Variant::Integer(y),
                    Variant::Integer(0),
                ],
            )?;
            if let Some((layer, lx, ly)) = hit {
                self.call(
                    ObjectHandle(layer),
                    "onMouseMove",
                    vec![
                        Variant::Integer(lx),
                        Variant::Integer(ly),
                        Variant::Integer(0),
                    ],
                )?;
            }
        }
        for (bit, button) in [(buttons::MOUSE_LEFT, 0), (buttons::MOUSE_RIGHT, 1)] {
            let down = input.buttons_down & bit != 0;
            let was = self.last_down & bit != 0;
            let pressed = (down && !was) || (input.buttons_pressed & bit != 0 && !was);
            let released = !down && (was || input.buttons_pressed & bit != 0);
            let args = |px, py| {
                vec![
                    Variant::Integer(px),
                    Variant::Integer(py),
                    Variant::Integer(button),
                    Variant::Integer(0),
                ]
            };
            if pressed {
                self.call(window, "onMouseDown", args(x, y))?;
                if let Some((layer, lx, ly)) = hit {
                    self.call(ObjectHandle(layer), "onMouseDown", args(lx, ly))?;
                }
                if button == 0 {
                    self.pressed = hit.map(|h| h.0);
                }
            }
            if released {
                self.call(window, "onMouseUp", args(x, y))?;
                if let Some((layer, lx, ly)) = hit {
                    self.call(ObjectHandle(layer), "onMouseUp", args(lx, ly))?;
                }
                if button == 0 {
                    self.call(
                        window,
                        "onClick",
                        vec![Variant::Integer(x), Variant::Integer(y)],
                    )?;
                    if let (Some(p), Some((layer, lx, ly))) = (self.pressed.take(), hit) {
                        if p == layer {
                            self.call(
                                ObjectHandle(layer),
                                "onClick",
                                vec![Variant::Integer(lx), Variant::Integer(ly)],
                            )?;
                        }
                    }
                }
            }
        }
        for (bit, key) in [
            (buttons::ENTER, 13),
            (buttons::SPACE, 32),
            (buttons::ESCAPE, 27),
            (buttons::UP, 38),
            (buttons::DOWN, 40),
            (buttons::LEFT, 37),
            (buttons::RIGHT, 39),
        ] {
            if input.buttons_pressed & bit != 0 {
                self.call(
                    window,
                    "onKeyDown",
                    vec![Variant::Integer(key), Variant::Integer(0)],
                )?;
            }
        }
        self.last_down = input.buttons_down;
        Ok(())
    }

    fn timers(&mut self, now: u64) -> Result<()> {
        let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
        let mut due = Vec::new();
        for (h, t) in rt.host_mut().timers.iter_mut() {
            if !t.enabled || now < t.next {
                continue;
            }
            let missed = (now - t.next) / t.interval + 1;
            t.next += missed * t.interval;
            due.push((*h, missed.min(t.capacity.max(1))));
        }
        for (h, count) in due {
            for _ in 0..count {
                // A handler may disable or invalidate its own timer.
                let live = self
                    .rt
                    .as_ref()
                    .unwrap()
                    .host()
                    .timers
                    .get(&h)
                    .is_some_and(|t| t.enabled);
                if !live {
                    break;
                }
                self.call(ObjectHandle(h), "onTimer", vec![])?;
            }
        }
        let handlers = natives::continuous_handlers(self.rt.as_mut().unwrap());
        for f in handlers {
            let rt = self.rt.as_mut().unwrap();
            rt.call_function(f, vec![Variant::Integer(now as i64)])
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("continuous handler")?;
        }
        Ok(())
    }

    fn execute(&mut self, h: &mut dyn HostServices, input: InputFrame) -> Result<VmYield> {
        if let Some(e) = &self.last_error {
            bail!("KiriKiri cannot resume after a failed step: {e}");
        }
        if self.halted {
            return Ok(VmYield::Halted);
        }
        if self.rt.is_none() {
            self.boot(h)?;
        }
        let now = h.now_ms();
        self.rt.as_mut().unwrap().host_mut().now = now;
        self.input(input)?;
        self.timers(now)?;
        self.flush(h)?;
        self.frames += 1;
        if self.rt.as_ref().unwrap().host().exit_requested {
            self.halted = true;
            return Ok(VmYield::Halted);
        }
        Ok(VmYield::BudgetExhausted)
    }

    /// Script log lines (Debug.message, System.inform); for tests and tools.
    pub fn log(&self) -> Vec<String> {
        self.rt
            .as_ref()
            .map(|rt| rt.host().log.clone())
            .unwrap_or_default()
    }
}

fn truncate(s: &str) -> String {
    s.chars().take(240).collect()
}

fn err(e: Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

impl VmAdapter for KrkrVm {
    fn engine_id(&self) -> &'static str {
        "krkr"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        h: &mut dyn HostServices,
        i: InputFrame,
        b: u32,
    ) -> crysirror_core::Result<VmYield> {
        if b == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        match self.execute(h, i) {
            Ok(y) => Ok(y),
            Err(e) => {
                let message = format!("{e:#}");
                self.last_error = Some(message.clone());
                Err(Error::Io(message))
            }
        }
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("KiriKiri VM checkpoint not implemented"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("KiriKiri VM checkpoint not implemented"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        let host = self.rt.as_ref().map(|rt| rt.host());
        serde_json::to_vec(&json!({
            "vm": "kirakira-tjs2",
            "frames": self.frames,
            "layers": host.map(|h| h.layers.len()),
            "timers": host.map(|h| h.timers.len()),
            "engine_api_calls": host.map(|h| h.api_calls),
            "events": self.events,
            "log_tail": host.map(|h| h.log.iter().rev().take(16).cloned().collect::<Vec<_>>()),
            "error": self.last_error,
            "rendering": "engine retained Layer/Canvas",
            "owns_framebuffer": false,
        }))
        .unwrap_or_default()
    }
}

pub struct KrkrFactory;
impl crysirror_runtime::runtime::VmFactory for KrkrFactory {
    fn create(
        &self,
        c: ClientId,
        s: &crysirror_runtime::model::ClientSpec,
    ) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            s.program.is_empty(),
            "KiriKiri executes original TJS2 scripts"
        );
        Ok(Box::new(KrkrVm::new(
            c,
            serde_json::from_value(s.config.clone())?,
        )?))
    }
}

pub fn register(r: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    r.register("krkr", Arc::new(KrkrFactory))
}
