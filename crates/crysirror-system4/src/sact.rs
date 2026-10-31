// Copyright (C) 2026 CrySirror contributors. GPL-2.0-or-later.
//! Native SACT handle/initialization policy. Canvas, layers, fonts and audio are
//! existing CrySirror services; no SDL/gfx/font/mixer implementation is embedded.
use anyhow::{ensure, Context, Result};
use crysirror_core::{service::HostServices, state::ClientId};
use serde_json::json;
#[derive(Clone, Debug)]
pub struct Sact {
    owner: Option<ClientId>,
    pub canvas: [u32; 2],
    pub font_aliases: [String; 2],
    pub initialized: bool,
    pub font_ready: bool,
    pub audio_ready: bool,
    pub calls: u64,
}
impl Sact {
    pub fn new(canvas: [u32; 2], font_aliases: [String; 2]) -> Result<Self> {
        ensure!(
            canvas.iter().all(|n| (1..=8192).contains(n)),
            "System4 native canvas dimensions"
        );
        ensure!(
            font_aliases.iter().all(|n| !n.is_empty() && n.len() <= 256),
            "System4 native font aliases"
        );
        Ok(Self {
            owner: None,
            canvas,
            font_aliases,
            initialized: false,
            font_ready: false,
            audio_ready: false,
            calls: 0,
        })
    }
    pub fn init(&mut self, client: ClientId, host: &mut dyn HostServices) -> Result<i32> {
        ensure!(
            self.owner.is_none_or(|owner| owner == client),
            "foreign System4 SACT owner"
        );
        if self.initialized {
            self.calls += 1;
            return Ok(1);
        }
        // Font initialization in the reference actually loads two faces. A
        // provider's mere presence must not stand in for published font data.
        let library = host.font_library(client)?;
        for alias in &self.font_aliases {
            library
                .resolve(alias)?
                .with_context(|| format!("System4 font `{alias}` is not published"))?;
        }
        host.text_measurement(client)?;
        // Runtime owns the already constructed device-free channel mixer. No
        // device or prepared voice is fabricated to acknowledge initialization.
        host.audio_playing(client, 0)?;
        let [w, h] = self.canvas;
        let batch = json!({"canvases":[
            {"op":"create","id":1,"width":w,"height":h},
            {"op":"clear","id":1,"color":[0,0,0,255]}
        ],"layers":[
            {"op":"put","layer":{"id":1,"rect":[0,0,w,h],"paint":{"kind":"solid","color":[0,0,0,255]},"order":[-2147483648,0]}},
            {"op":"put","layer":{"id":16,"rect":[0,0,w,h],"paint":{"kind":"solid","color":[0,0,0,0]},"visible":false}}
        ]});
        host.submit_engine(client, 1, &serde_json::to_vec(&batch)?)?;
        self.owner = Some(client);
        self.font_ready = true;
        self.audio_ready = true;
        self.initialized = true;
        self.calls += 1;
        Ok(1)
    }
}
