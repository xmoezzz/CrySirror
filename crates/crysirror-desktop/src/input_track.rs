// SPDX-License-Identifier: MPL-2.0
//! Deterministic frontend input for native VM/service regression runs.
use anyhow::{ensure, Context, Result};
use crysirror_core::service::{buttons, InputFrame};
use serde::Deserialize;
use std::{io::Read, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    frame: u32,
    #[serde(default)]
    down: Option<Vec<String>>,
    #[serde(default)]
    pressed: Vec<String>,
    #[serde(default)]
    pointer: Option<[i32; 2]>,
}
#[derive(Default)]
pub struct InputTrack(Vec<(u32, InputFrame)>);
impl InputTrack {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 1024 * 1024, "input track exceeds 1 MiB");
        Self::parse(&bytes).with_context(|| format!("input track {}", path.display()))
    }
    fn parse(bytes: &[u8]) -> Result<Self> {
        let entries: Vec<Entry> = serde_json::from_slice(bytes)?;
        ensure!(entries.len() <= 100_000, "input track event limit");
        let mut state = InputFrame::default();
        let mut result = Vec::new();
        let bits = |names: &[String]| -> Result<u64> {
            ensure!(
                names.len() <= buttons::NAMES.len(),
                "input track button limit"
            );
            names.iter().try_fold(0, |bits, name| {
                let bit = buttons::NAMES
                    .iter()
                    .find(|(n, _)| *n == name)
                    .with_context(|| format!("unknown input key '{name}'"))?
                    .1;
                ensure!(bits & bit == 0, "duplicate input key '{name}'");
                Ok(bits | bit)
            })
        };
        for entry in entries {
            ensure!(
                result.last().is_none_or(|(f, _)| *f < entry.frame),
                "input frames must strictly increase"
            );
            if let Some(down) = entry.down {
                state.buttons_down = bits(&down)?;
            }
            state.buttons_pressed = bits(&entry.pressed)?;
            if let Some([x, y]) = entry.pointer {
                state.pointer_x = x;
                state.pointer_y = y;
            }
            result.push((entry.frame, state));
        }
        Ok(Self(result))
    }
    pub fn at(&self, frame: u32) -> InputFrame {
        let count = self.0.partition_point(|(f, _)| *f <= frame);
        if count == 0 {
            return InputFrame::default();
        }
        let (when, mut state) = self.0[count - 1];
        if when != frame {
            state.buttons_pressed = 0;
        }
        state
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn holds_pointer_and_keys_but_not_pulses() {
        let track = InputTrack::parse(br#"[{"frame":2,"down":["mouse_left"],"pressed":["4"],"pointer":[200,330]},{"frame":5,"down":[]}]"#).unwrap();
        assert_eq!(track.at(1), InputFrame::default());
        assert_eq!(track.at(2).buttons_pressed, buttons::KEY_4);
        assert_eq!(track.at(4).buttons_down, buttons::MOUSE_LEFT);
        assert_eq!(track.at(4).buttons_pressed, 0);
        assert_eq!((track.at(6).pointer_x, track.at(6).pointer_y), (200, 330));
        assert_eq!(track.at(6).buttons_down, 0);
    }
    #[test]
    fn rejects_ambiguous_or_unknown_events() {
        for json in [
            r#"[{"frame":1},{"frame":1}]"#,
            r#"[{"frame":2},{"frame":1}]"#,
            r#"[{"frame":0,"pressed":["unknown"]}]"#,
            r#"[{"frame":0,"down":["4","4"]}]"#,
            r#"[{"frame":0,"extra":true}]"#,
        ] {
            assert!(InputTrack::parse(json.as_bytes()).is_err(), "{json}");
        }
    }
}
