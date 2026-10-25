// SPDX-License-Identifier: MPL-2.0
//! Optional device implementation. No VM/runtime/filesystem ownership.
use crate::{AudioDevice, VoiceStart};
use anyhow::{Context, Result};
use crysirror_core::state::ObjectId;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::{collections::BTreeMap, io::Cursor};

#[derive(Default)]
pub struct RodioDevice {
    // Sinks drop before their stream, including on normal host teardown.
    voices: BTreeMap<ObjectId, Sink>,
    stream: Option<(OutputStream, OutputStreamHandle)>,
}
impl AudioDevice for RodioDevice {
    fn start(&mut self, id: ObjectId, request: VoiceStart) -> Result<()> {
        let decoder = Decoder::new(Cursor::new(request.bytes))
            .context("unsupported or corrupt audio asset")?;
        if self.stream.is_none() {
            self.stream =
                Some(OutputStream::try_default().context(
                    "no audio device; use --no-audio for intentionally silent operation",
                )?);
        }
        let handle = &self.stream.as_ref().context("audio stream")?.1;
        let sink = Sink::try_new(handle)?;
        sink.set_volume(f32::from(request.gain_milli) / 1000.);
        if request.paused {
            sink.pause();
        }
        if request.voice.looping {
            sink.append(decoder.repeat_infinite());
        } else {
            sink.append(decoder);
        }
        self.voices.insert(id, sink);
        Ok(())
    }
    fn stop(&mut self, id: ObjectId) {
        if let Some(sink) = self.voices.remove(&id) {
            sink.stop();
        }
    }
    fn set_gain(&mut self, id: ObjectId, gain: u16) {
        if let Some(sink) = self.voices.get(&id) {
            sink.set_volume(f32::from(gain) / 1000.);
        }
    }
    fn pause(&mut self, id: ObjectId) {
        if let Some(sink) = self.voices.get(&id) {
            sink.pause();
        }
    }
    fn resume(&mut self, id: ObjectId) {
        if let Some(sink) = self.voices.get(&id) {
            sink.play();
        }
    }
}
