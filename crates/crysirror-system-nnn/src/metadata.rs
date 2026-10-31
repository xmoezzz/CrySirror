// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Native story/film metadata. This is original script/save policy, not a
//! renderer, text layout, UI system or a substitute for engine operations.
use crate::machine::{Machine, NativeCall, Payload, Step};
use crate::Program;
use anyhow::{ensure, Result};
#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub story_name: Vec<u8>,
    pub film_name: Vec<u8>,
}
impl Metadata {
    /// Commit only original native metadata callbacks. Callers must not use
    /// this to acknowledge graphics, sound, input, configuration or waits.
    pub fn commit(&mut self, machine: &mut Machine, call: &NativeCall) -> Result<()> {
        let (is_story, text) = match &call.payload {
            Payload::StoryName(text) => (true, text),
            Payload::FilmName(text) => (false, text),
            _ => anyhow::bail!("System-NNN callback needs an engine semantic mapping"),
        };
        ensure!(text.len() <= 1024 * 1024, "System-NNN metadata text budget");
        // The native callback copies at most 62 bytes, not Unicode characters,
        // then writes two NUL terminators in its 64-byte save-policy buffer.
        // Keep raw byte truncation, including a possible multibyte lead byte.
        // Validate and consume the exact pending receipt before changing policy.
        machine.acknowledge(call)?;
        if is_story {
            self.story_name = text[..text.len().min(62)].to_vec();
        } else {
            self.film_name = text[..text.len().min(62)].to_vec();
        }
        Ok(())
    }
    /// Execute metadata and internal control only, yielding every other native
    /// operation untouched. `budget` counts both dispatch and native receipts.
    pub fn advance(
        &mut self,
        machine: &mut Machine,
        program: &Program,
        budget: u32,
    ) -> Result<Step> {
        ensure!((1..=10000).contains(&budget), "System-NNN metadata budget");
        for _ in 0..budget {
            match machine.poll(program, 1)? {
                Step::Native(call)
                    if matches!(call.payload, Payload::StoryName(_) | Payload::FilmName(_)) =>
                {
                    self.commit(machine, &call)?
                }
                Step::Budget => {}
                other => return Ok(other),
            }
        }
        Ok(Step::Budget)
    }
}
