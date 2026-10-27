// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared bounded OPNA synthesis. No UK2 VM, Kira mixer, device or ambient I/O.
//! `.pmd`/`.mmm` select the PMD-M OPNA profile; `.opna` is a schema-1 JSON
//! register score. Decoders are worker-owned through the normal audio pipeline.
mod opna;
mod pmd;
use crate::decoder::{AudioSource, StereoFrame};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_WRITES: usize = 262_144;
const CLOCK: u64 = 7_987_200;
/// A register write immediately before the given output sample. Input order
/// resolves simultaneous writes. This clock is independent of render frames.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterWrite {
    pub frame: u64,
    pub port: u8,
    pub register: u8,
    pub value: u8,
}
/// Engine-owned finite Yamaha YM2608 score, serialized as a `.opna` JSON asset.
/// FM/SSG and procedural rhythm only; no chip ROM, ADPCM, FM3 special mode or
/// hardware timer/status interface. This is a sound model, not silicon parity.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterScore {
    pub schema: u8,
    pub output_rate: u32,
    pub frames: u64,
    pub writes: Vec<RegisterWrite>,
}
impl RegisterScore {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "unknown OPNA score schema");
        ensure!(
            (8000..=192000).contains(&self.output_rate),
            "OPNA output rate"
        );
        ensure!(
            self.frames > 0 && self.frames <= u64::from(self.output_rate) * 3600,
            "OPNA duration budget"
        );
        ensure!(self.writes.len() <= MAX_WRITES, "OPNA write budget");
        let mut previous = 0;
        for w in &self.writes {
            ensure!(
                w.frame >= previous && w.frame < self.frames,
                "OPNA writes must be ordered inside duration"
            );
            validate_register(w.port as usize, w.register, w.value)?;
            previous = w.frame;
        }
        Ok(())
    }
}
pub(super) fn validate_register(port: usize, reg: u8, value: u8) -> Result<()> {
    ensure!(port <= 1, "OPNA bank must be 0 or 1");
    let channel = reg & 3;
    let fm = channel < 3
        && ((0x30..=0x8f).contains(&reg) || matches!(reg & 0xfc, 0xa0 | 0xa4 | 0xb0 | 0xb4));
    let special = port == 0
        && (reg <= 13
            || matches!(reg, 0x10 | 0x11 | 0x18..=0x1d | 0x22 | 0x28)
            || (reg == 0x29 && value == 0x83));
    // Normal SSG-EG is an exact no-op. Unsupported nonzero SSG-EG shapes fail.
    let intact_eg = (0x90..=0x9f).contains(&reg) && channel < 3 && value == 0;
    ensure!(
        fm || special || intact_eg,
        "unsupported OPNA register/mode: bank {port}, register {reg:#x}, value {value:#x}"
    );
    Ok(())
}
fn validate_pmd(bytes: &[u8]) -> Result<()> {
    ensure!(
        (25..=65537).contains(&bytes.len()),
        "PMD score must contain a complete 16-bit directory"
    );
    ensure!(bytes[0] == 0, "unsupported PMD file/driver variant");
    let md = &bytes[1..];
    for i in 0..12 {
        let at = u16::from_le_bytes([md[i * 2], md[i * 2 + 1]]) as usize;
        if at == 0 {
            continue;
        }
        ensure!(
            (24..md.len()).contains(&at),
            "PMD directory target outside score"
        );
        if i == 9 {
            ensure!(
                md[at] == 0x80,
                "PMD ADPCM part J needs an unsupported sample bank"
            );
        }
    }
    if md[0] != 24 {
        ensure!(md.len() >= 26, "PMD voice directory missing");
        let at = u16::from_le_bytes([md[24], md[25]]) as usize;
        ensure!(at >= 26 && at < md.len(), "PMD voice table outside score");
    }
    Ok(())
}
enum Generator {
    Registers {
        score: Arc<RegisterScore>,
        next: usize,
        chip: opna::Opna,
    },
    Pmd {
        bytes: Arc<[u8]>,
        driver: pmd::Pmd,
        tail: Option<u32>,
    },
}
/// An independent shared decoder lease. Construction/seek/render belong on
/// workers. Existing BufferedSource supplies bounded prefetch and completions.
pub struct SynthSource {
    generator: Generator,
    rate: u32,
    position: u64,
    phase: u64,
    previous: StereoFrame,
    current: StereoFrame,
}
impl SynthSource {
    pub fn registers(score: RegisterScore) -> Result<Self> {
        score.validate()?;
        let rate = score.output_rate;
        Ok(Self::new(
            Generator::Registers {
                score: Arc::new(score),
                next: 0,
                chip: opna::Opna::new(),
            },
            rate,
        ))
    }
    pub fn pmd(bytes: Arc<[u8]>) -> Result<Self> {
        validate_pmd(&bytes)?;
        let mut driver = pmd::Pmd::new();
        driver.start(&bytes);
        Ok(Self::new(
            Generator::Pmd {
                bytes,
                driver,
                tail: None,
            },
            48000,
        ))
    }
    /// PMD's original Timer-A fade. Modern voice fades remain mixer controls.
    pub fn fade_out(&mut self, speed: u8) -> Result<()> {
        match &mut self.generator {
            Generator::Pmd { driver, .. } => {
                driver.fade_out(speed);
                Ok(())
            }
            _ => bail!("driver fade requires a PMD source"),
        }
    }
    fn new(generator: Generator, rate: u32) -> Self {
        Self {
            generator,
            rate,
            position: 0,
            phase: 0,
            previous: [0.; 2],
            current: [0.; 2],
        }
    }
    fn sample(&mut self) -> Result<Option<StereoFrame>> {
        match &mut self.generator {
            Generator::Registers { score, next, chip } => {
                if self.position >= score.frames {
                    return Ok(None);
                }
                while let Some(w) = score.writes.get(*next).filter(|w| w.frame == self.position) {
                    chip.write(w.port as usize, w.register, w.value);
                    *next += 1;
                }
            }
            Generator::Pmd { driver, tail, .. } => {
                if driver.ended() || !driver.is_playing() {
                    let left = tail.get_or_insert(self.rate);
                    if *left == 0 {
                        return Ok(None);
                    }
                    *left -= 1;
                }
            }
        }
        // Exact rational chip clock / 144; no rounded 55,466 Hz timeline drift.
        let divisor = u64::from(self.rate) * 144;
        self.phase += CLOCK;
        while self.phase >= divisor {
            self.phase -= divisor;
            self.previous = self.current;
            let v = match &mut self.generator {
                Generator::Registers { chip, .. } => {
                    let v = chip.sample();
                    if let Some(e) = &chip.fault {
                        bail!("{e}")
                    }
                    v
                }
                Generator::Pmd { driver, .. } => {
                    let v = driver.sample();
                    if let Some(e) = driver.error() {
                        bail!("{e}")
                    }
                    v
                }
            };
            self.current = [v.0, v.1];
        }
        let t = self.phase as f32 / divisor as f32;
        let v =
            std::array::from_fn(|i| self.previous[i] + (self.current[i] - self.previous[i]) * t);
        self.position += 1;
        Ok(Some(v))
    }
}
impl AudioSource for SynthSource {
    fn channel_count(&self) -> Option<u16> {
        Some(2)
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn frame_count(&self) -> Option<u64> {
        match &self.generator {
            Generator::Registers { score, .. } => Some(score.frames),
            _ => None,
        }
    }
    fn read_position(&self) -> Option<u64> {
        Some(self.position)
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        if let Generator::Pmd { driver, .. } = &mut self.generator {
            driver.begin_block();
        }
        let limit = out.len().min(4096);
        let mut n = 0;
        while n < limit {
            let Some(v) = self.sample()? else { break };
            out[n] = v;
            n += 1;
        }
        Ok(n)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        ensure!(
            frame <= u64::from(self.rate) * 120,
            "synth seek replay exceeds 120 seconds"
        );
        if let Some(end) = self.frame_count() {
            ensure!(frame <= end, "synth seek past end");
        }
        match &mut self.generator {
            Generator::Registers { chip, next, .. } => {
                *chip = opna::Opna::new();
                *next = 0;
            }
            Generator::Pmd {
                bytes,
                driver,
                tail,
            } => {
                *driver = pmd::Pmd::new();
                driver.start(bytes);
                *tail = None;
            }
        }
        self.position = 0;
        self.phase = 0;
        self.previous = [0.; 2];
        self.current = [0.; 2];
        let mut scratch = [[0.; 2]; 4096];
        while self.position < frame {
            let n = (frame - self.position).min(4096) as usize;
            ensure!(self.read(&mut scratch[..n])? > 0, "synth seek past EOF");
        }
        Ok(())
    }
}
pub fn is_extension(ext: Option<&str>) -> bool {
    ext.is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "opna" | "pmd" | "mmm"))
}
pub fn open(bytes: Arc<[u8]>, ext: Option<&str>) -> Result<Option<Box<dyn AudioSource>>> {
    if !is_extension(ext) {
        return Ok(None);
    }
    ensure!(bytes.len() <= MAX_BYTES, "synth encoded byte budget");
    let source = if ext.context("synth extension")?.eq_ignore_ascii_case("opna") {
        SynthSource::registers(serde_json::from_slice(&bytes).context("OPNA register score")?)?
    } else {
        SynthSource::pmd(bytes)?
    };
    Ok(Some(Box::new(source)))
}
