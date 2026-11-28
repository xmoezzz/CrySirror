// SPDX-License-Identifier: MPL-2.0
//! V1 RetouchPrintManager writable text surfaces (body 52, separate name 54).
use anyhow::Result;
use crysirror_core::{service::HostServices, state::ClientId};
use serde_json::{Value, json};

pub(super) const BODY_SLOT: u32 = 52;
pub(super) const NAME_SLOT: u32 = 54;

#[derive(Clone, Copy)]
pub(super) struct Page {
    pub body: [i32; 4],
    pub name: Option<[i32; 4]>,
}

impl Page {
    fn planes(self, with_name: bool) -> impl Iterator<Item = (u32, [i32; 4])> {
        std::iter::once((BODY_SLOT, self.body)).chain(
            self.name
                .filter(|_| with_name)
                .map(|rect| (NAME_SLOT, rect)),
        )
    }

    pub fn cleared(self, with_name: bool) -> impl Iterator<Item = Value> {
        self.planes(with_name).map(|(slot, rect)| {
            json!({"op":"put","layer":{
                "id":256 + slot,"order":[2, slot],"rect":rect,"opacity":255,
                "paint":{"kind":"solid","color":[0,0,0,0]}
            }})
        })
    }

    pub fn clear(
        self,
        host: &mut dyn HostServices,
        client: ClientId,
        with_name: bool,
    ) -> Result<()> {
        submit(host, client, self.cleared(with_name))
    }
}

fn submit(
    host: &mut dyn HostServices,
    client: ClientId,
    ops: impl Iterator<Item = Value>,
) -> Result<()> {
    let layers: Vec<_> = ops.collect();
    host.submit_engine(client, 1, &serde_json::to_vec(&json!({"layers":layers}))?)?;
    Ok(())
}

pub(super) struct ClearFade {
    page: Page,
    since: u64,
    duration: u32,
    level: u8,
}

impl ClearFade {
    pub fn new(page: Page, since: u64, duration: u32) -> Self {
        Self {
            page,
            since,
            duration,
            level: 16,
        }
    }

    pub fn present(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<bool> {
        let age = u32::try_from(host.now_ms().saturating_sub(self.since)).unwrap_or(u32::MAX);
        let level = crate::panel_animation::fade_level(16, 0, self.duration, age) as u8;
        if level == 0 {
            // leaveClear always clears the configured name printer and restores
            // both descriptor levels, even for a timed "body-only" clear.
            self.page.clear(host, client, true)?;
            return Ok(true);
        }
        if self.level != level {
            submit(host, client, self.page.planes(true).map(|(slot, _)| {
                json!({"op":"set_opacity","id":256 + slot,"opacity":level.saturating_mul(16)})
            }))?;
            self.level = level;
        }
        Ok(false)
    }
}
