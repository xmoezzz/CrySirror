// SPDX-License-Identifier: MPL-2.0
//! RLD movie channel and its independent optional channel-1 effect.
use crate::{
    native_numbers::integer,
    registers::{Bank, Change, Registers, Value},
    sound::{Audio, Sound},
};
use anyhow::{Result, bail};
use crysirror_core::{
    service::{AudioRequest, HostServices, MediaPhase, MediaRequest},
    state::ClientId,
};
use crysirror_tasks::TaskPool;
use serde_json::json;

const CHANNEL: u32 = 1;
const LAYER: u32 = 2;
const EFFECT_CHANNEL: u32 = 15;

#[derive(Debug, PartialEq)]
pub(super) struct Movie {
    pub cg: Option<i32>,
    pub path: String,
    pub sound: i32,
    pub rect: [i32; 4],
}

impl Movie {
    pub fn path(id: i32) -> String {
        format!(
            "res/g/ev/{}/{}.mpg",
            crate::v1_services::decimal(id / 100, 2),
            crate::v1_services::decimal(id, 4)
        )
    }
}

/// liteDrawMovie (0x60670) consumes four nonempty dotted fields per
/// movie, ignores fields three and four, and discards incomplete tails.
#[derive(Debug, PartialEq)]
pub(super) struct Gallery {
    data: Vec<u8>,
    offset: usize,
}
impl Gallery {
    pub fn new(raw: &[u8]) -> Self {
        Self {
            data: raw.to_vec(),
            offset: 0,
        }
    }
    fn token(&mut self) -> Option<&[u8]> {
        let tail = self.data.get(self.offset..)?;
        let start = self.offset + tail.iter().position(|&b| b != b'.')?;
        let end = self.data[start..]
            .iter()
            .position(|&b| b == b'.')
            .map_or(self.data.len(), |i| start + i);
        self.offset = end;
        Some(&self.data[start..end])
    }
    pub fn next(&mut self) -> Option<Movie> {
        loop {
            let id = integer(self.token()?);
            let sound = integer(self.token()?).max(-1);
            self.token()?;
            self.token()?;
            if id >= 0 {
                return Some(Movie {
                    cg: Some(id),
                    path: Movie::path(id),
                    sound,
                    rect: [0; 4],
                });
            }
        }
    }
}
pub(super) struct Playing {
    opened: u64,
    has_sound: bool,
    removed: bool,
}
impl Playing {
    pub fn start(
        client: ClientId,
        h: &mut dyn HostServices,
        canvas: [u32; 2],
        movie: Movie,
        registers: &mut Registers,
        audio: &mut Audio,
        worker: &TaskPool,
    ) -> Result<Self> {
        if let Some(id) = movie.cg {
            registers.read(Value::Seen(Bank::Cg, id))?;
        }
        let [x, y, w, height] = movie.rect;
        let w = if w == 0 { canvas[0] as i32 } else { w };
        let height = if height == 0 {
            canvas[1] as i32
        } else {
            height
        };
        let at = h.media_request(
            client,
            MediaRequest::Play {
                id: CHANNEL,
                asset: movie.path,
                looping: false,
                audio: true,
                gain: 1.0,
            },
        )?;
        let result = (|| -> Result<()> {
            let layer = json!({"layers":[{"op":"put","layer":{"id":LAYER,"order":[1,0],"rect":[x,y,w,height],"paint":{"kind":"image","asset":at.relative()}}}]});
            h.submit_engine(client, 1, &serde_json::to_vec(&layer)?)?;
            if movie.sound >= 0 {
                audio.issue(
                    h,
                    client,
                    registers,
                    worker,
                    Sound::Effect {
                        id: movie.sound,
                        levels: [256; 2],
                        slot: 1,
                        wait: false,
                    },
                )?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = h.submit_engine(
                client,
                1,
                &serde_json::to_vec(&json!({"layers":[{"op":"remove","id":LAYER}]}))?,
            );
            let _ = h.media_request(client, MediaRequest::Remove { id: CHANNEL });
            return Err(error);
        }
        if let Some(id) = movie.cg {
            registers.change(Change::Experience(Bank::Cg, vec![id]))?;
        }
        Ok(Self {
            opened: h.now_ms(),
            has_sound: movie.sound >= 0,
            removed: false,
        })
    }
    /// leaveMovie enforces a one-second startup interval before even a
    /// nonblocking leave. With sound duration zero it then waits for the WAV.
    pub fn leave(
        &mut self,
        client: ClientId,
        h: &mut dyn HostServices,
        wait: bool,
        fade: i32,
    ) -> Result<Option<u64>> {
        let now = h.now_ms();
        if now < self.opened.saturating_add(1000) {
            return Ok(Some(self.opened.saturating_add(1000)));
        }
        if !self.removed {
            match h.media_info(client, CHANNEL)?.phase {
                MediaPhase::Failed => bail!("ExHibit movie decoder failed"),
                MediaPhase::Loading | MediaPhase::Playing if wait => {
                    return Ok(Some(now.saturating_add(1)));
                }
                _ => {}
            }
            h.submit_engine(
                client,
                1,
                &serde_json::to_vec(&json!({"layers":[{"op":"remove","id":LAYER}]}))?,
            )?;
            h.media_request(client, MediaRequest::Remove { id: CHANNEL })?;
            self.removed = true;
        }
        if self.has_sound {
            if fade == 0 && h.audio_playing(client, EFFECT_CHANNEL)? {
                return Ok(Some(now.saturating_add(1)));
            }
            h.submit_audio(
                client,
                AudioRequest::Stop {
                    channel: EFFECT_CHANNEL,
                    fade_ms: fade.max(0) as f32,
                },
            )?;
        }
        Ok(None)
    }
}
