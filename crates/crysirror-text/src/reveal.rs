// SPDX-License-Identifier: MPL-2.0
//! Font-independent, saved grapheme reveal clock and VN event delivery.
use crate::rich::{RichNode, RichText};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevealEvent {
    Grapheme {
        index: usize,
        bytes: Range<usize>,
        text: String,
    },
    Marker(String),
    Finished,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevealState {
    /// Content + marker + rate identity, preventing restore onto another line.
    pub fingerprint: String,
    pub revealed: usize,
    pub remainder_us: u64,
    pub paused: bool,
    pub finished_emitted: bool,
    pub markers_delivered: usize,
}
pub struct Typewriter {
    text: String,
    graphemes: Vec<Range<usize>>,
    markers: Vec<(usize, String)>,
    interval_us: u64,
    state: RevealState,
}
impl Typewriter {
    pub fn new(document: &RichText, graphemes_per_second: f64) -> Result<Self> {
        document.validate()?;
        ensure!(
            graphemes_per_second.is_finite() && (0.01..=10000.).contains(&graphemes_per_second),
            "typewriter rate"
        );
        let text = document.logical_text();
        let graphemes = text
            .grapheme_indices(true)
            .map(|(i, g)| i..i + g.len())
            .collect::<Vec<_>>();
        let mut markers = Vec::new();
        let mut offset = 0usize;
        for n in &document.nodes {
            match n {
                RichNode::Text { text, .. } => offset += text.len(),
                RichNode::Ruby { base, .. } => offset += base.len(),
                RichNode::Icon { .. } => offset += 3,
                RichNode::Event { name } => {
                    let boundary = graphemes.partition_point(|g| g.end <= offset);
                    ensure!(
                        offset == 0
                            || graphemes
                                .get(boundary.wrapping_sub(1))
                                .is_some_and(|g| g.end == offset),
                        "text marker splits an extended grapheme"
                    );
                    markers.push((boundary, name.clone()))
                }
            }
        }
        let interval_us = (1_000_000. / graphemes_per_second).round() as u64;
        let mut hash = blake3::Hasher::new();
        hash.update(&(text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
        hash.update(&interval_us.to_le_bytes());
        for (at, name) in &markers {
            hash.update(&(*at as u64).to_le_bytes());
            hash.update(&(name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
        }
        let fingerprint = hash.finalize().to_hex().to_string();
        Ok(Self {
            text,
            graphemes,
            markers,
            interval_us,
            state: RevealState {
                fingerprint,
                revealed: 0,
                remainder_us: 0,
                paused: false,
                finished_emitted: false,
                markers_delivered: 0,
            },
        })
    }
    pub fn state(&self) -> RevealState {
        self.state.clone()
    }
    pub fn restore(&mut self, state: RevealState) -> Result<()> {
        ensure!(
            state.fingerprint == self.state.fingerprint,
            "typewriter content/rate mismatch"
        );
        ensure!(
            state.revealed <= self.graphemes.len()
                && state.remainder_us < self.interval_us
                && state.markers_delivered <= self.markers.len(),
            "invalid typewriter state"
        );
        ensure!(
            !state.finished_emitted || state.revealed == self.graphemes.len(),
            "premature typewriter completion"
        );
        ensure!(
            self.markers
                .iter()
                .take(state.markers_delivered)
                .all(|(n, _)| *n <= state.revealed),
            "premature typewriter marker"
        );
        self.state = state;
        Ok(())
    }
    pub fn pause(&mut self, paused: bool) {
        self.state.paused = paused;
    }
    pub fn revealed(&self) -> usize {
        self.state.revealed
    }
    pub fn total(&self) -> usize {
        self.graphemes.len()
    }
    pub fn visible_text(&self) -> &str {
        let end = self
            .graphemes
            .get(self.state.revealed.wrapping_sub(1))
            .map_or(0, |r| r.end);
        &self.text[..end]
    }
    fn markers(&mut self, out: &mut Vec<RevealEvent>) {
        while let Some((at, name)) = self.markers.get(self.state.markers_delivered) {
            if *at > self.state.revealed {
                break;
            }
            out.push(RevealEvent::Marker(name.clone()));
            self.state.markers_delivered += 1;
        }
    }
    fn reveal_to(&mut self, count: usize, emit: bool) -> Vec<RevealEvent> {
        let mut out = Vec::new();
        self.markers(&mut out);
        while self.state.revealed < count.min(self.graphemes.len()) {
            let i = self.state.revealed;
            self.state.revealed += 1;
            if emit {
                let r = self.graphemes[i].clone();
                out.push(RevealEvent::Grapheme {
                    index: i,
                    text: self.text[r.clone()].into(),
                    bytes: r,
                });
            }
            self.markers(&mut out);
        }
        if self.state.revealed == self.graphemes.len() && !self.state.finished_emitted {
            out.push(RevealEvent::Finished);
            self.state.finished_emitted = true;
        }
        out
    }
    pub fn advance(&mut self, delta_ms: u64) -> Result<Vec<RevealEvent>> {
        ensure!(delta_ms <= 60_000, "typewriter step exceeds 60 seconds");
        if self.state.paused {
            return Ok(Vec::new());
        }
        let total = self.state.remainder_us + delta_ms * 1000;
        let count = (total / self.interval_us) as usize;
        self.state.remainder_us = total % self.interval_us;
        Ok(self.reveal_to(self.state.revealed.saturating_add(count), true))
    }
    pub fn finish(&mut self, emit_graphemes: bool) -> Vec<RevealEvent> {
        self.reveal_to(self.graphemes.len(), emit_graphemes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rich::TextStyle;
    #[test]
    fn combining_emoji_pause_chunking_markers_and_restore() {
        let d = RichText::parse(
            "[event=start]e\u{301}👩‍👩‍👧‍👦[event=voice]終",
            TextStyle::new("unloaded", 16.),
        )
        .unwrap();
        let mut w = Typewriter::new(&d, 10.).unwrap();
        assert_eq!(w.total(), 3);
        let a = w.advance(100).unwrap();
        assert!(matches!(&a[0],RevealEvent::Marker(n) if n=="start"));
        assert_eq!(w.visible_text(), "e\u{301}");
        w.pause(true);
        assert!(w.advance(1000).unwrap().is_empty());
        w.pause(false);
        w.advance(50).unwrap();
        let state = w.state();
        let mut restored = Typewriter::new(&d, 10.).unwrap();
        restored.restore(state).unwrap();
        assert_eq!(w.advance(50).unwrap(), restored.advance(50).unwrap());
        assert!(w.visible_text().ends_with('👦'));
        assert_eq!(w.finish(false), vec![RevealEvent::Finished]);
        assert!(w.finish(true).is_empty());
        let mut invalid = w.state();
        invalid.revealed = 99;
        assert!(w.restore(invalid).is_err());
        let other = RichText::plain("different", TextStyle::new("unloaded", 16.));
        let mut other = Typewriter::new(&other, 10.).unwrap();
        assert!(other.restore(w.state()).is_err());
    }
}
