// SPDX-License-Identifier: MPL-2.0
use crate::{path::VfsPath, Error, Result};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectId {
    pub client: ClientId,
    pub local: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub [u8; 4]); // Straight alpha in the public API.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRun {
    pub text: String,
    pub font: String,
    pub size_px: u16,
    pub bounds: Rect,
    pub color: Color,
    /// Opaque profile label. A shaper must reject unknown layout policies.
    pub layout_profile: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sprite {
    pub asset: VfsPath,
    pub bounds: Rect,
    pub tint: Color,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Visual {
    Text(TextRun),
    Sprite(Sprite),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualNode {
    pub z: i32,
    pub visual: Visual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioVoice {
    pub asset: VfsPath,
    pub looping: bool,
    /// Integer gain avoids a NaN/non-deterministic serialization surface.
    pub gain_milli: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    PutVisual {
        id: u32,
        node: VisualNode,
    },
    RemoveVisual {
        id: u32,
    },
    PlayAudio {
        channel: u32,
        voice: AudioVoice,
    },
    StopAudio {
        channel: u32,
    },
    /// Change a playing voice's gain without restarting it (spatial audio).
    SetAudioGain {
        channel: u32,
        gain_milli: u16,
    },
    /// Explicit extension gate, not a silently ignored draw call.
    Require3d,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub visual_count: usize,
    pub voice_count: usize,
    pub text_bytes: usize,
    pub batch_commands: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            visual_count: 4096,
            voice_count: 128,
            text_bytes: 1024 * 1024,
            batch_commands: 4096,
        }
    }
}

/// The serializable service model, not a replacement for an adapter's VM state.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SceneState {
    visuals: BTreeMap<ObjectId, VisualNode>,
    voices: BTreeMap<ObjectId, AudioVoice>,
    frame: u64,
    voice_epochs: BTreeMap<ObjectId, u64>,
    next_voice_epoch: u64,
}

impl SceneState {
    pub fn visuals(&self) -> &BTreeMap<ObjectId, VisualNode> {
        &self.visuals
    }
    pub fn voices(&self) -> &BTreeMap<ObjectId, AudioVoice> {
        &self.voices
    }
    pub fn frame(&self) -> u64 {
        self.frame
    }
    pub fn voice_epoch(&self, id: ObjectId) -> u64 {
        self.voice_epochs.get(&id).copied().unwrap_or(0)
    }
    /// Used only after a complete service snapshot has been validated.
    pub fn restore_frame(&mut self, frame: u64) {
        self.frame = frame;
    }
    pub fn advance_frame(&mut self) -> Result<()> {
        self.frame = self
            .frame
            .checked_add(1)
            .ok_or(Error::Limit("frame counter"))?;
        Ok(())
    }
    pub fn clear_client(&mut self, client: ClientId) {
        self.visuals.retain(|id, _| id.client != client);
        self.voices.retain(|id, _| id.client != client);
        self.voice_epochs.retain(|id, _| id.client != client);
    }
    /// Stable render order independent of map insertion order.
    pub fn draw_order(&self) -> Vec<(ObjectId, &VisualNode)> {
        let mut out: Vec<_> = self.visuals.iter().map(|(id, node)| (*id, node)).collect();
        out.sort_by_key(|(id, node)| (node.z, *id));
        out
    }
    /// Atomic at the service model boundary. No platform side effects are issued
    /// during validation. Cloning is deliberate in this reference implementation.
    pub fn apply_batch(
        &mut self,
        client: ClientId,
        commands: &[Command],
        limits: Limits,
    ) -> Result<()> {
        if commands.len() > limits.batch_commands {
            return Err(Error::Limit("command batch"));
        }
        let mut staged = self.clone();
        for command in commands {
            staged.apply_one(client, command.clone(), limits)?;
        }
        *self = staged;
        Ok(())
    }
    fn apply_one(&mut self, client: ClientId, command: Command, limits: Limits) -> Result<()> {
        match command {
            Command::PutVisual { id, node } => {
                let key = ObjectId { client, local: id };
                match &node.visual {
                    Visual::Text(t) => {
                        if t.size_px == 0 || t.font.is_empty() || t.layout_profile.is_empty() {
                            return Err(Error::Invalid("text style"));
                        }
                        if t.text.len() > limits.text_bytes {
                            return Err(Error::Limit("text bytes"));
                        }
                        validate_rect(t.bounds)?;
                    }
                    Visual::Sprite(s) => validate_rect(s.bounds)?,
                }
                if !self.visuals.contains_key(&key) && self.visuals.len() >= limits.visual_count {
                    return Err(Error::Limit("visual count"));
                }
                self.visuals.insert(key, node);
                let text_total = self.visuals.values().try_fold(0usize, |sum, node| {
                    let bytes = match &node.visual {
                        Visual::Text(t) => t.text.len(),
                        _ => 0,
                    };
                    sum.checked_add(bytes)
                        .ok_or(Error::Limit("aggregate text bytes"))
                })?;
                if text_total > limits.text_bytes {
                    return Err(Error::Limit("aggregate text bytes"));
                }
            }
            Command::RemoveVisual { id } => {
                self.visuals.remove(&ObjectId { client, local: id });
            }
            Command::PlayAudio { channel, voice } => {
                if voice.gain_milli > 1000 {
                    return Err(Error::Invalid("gain outside 0..1000"));
                }
                let key = ObjectId {
                    client,
                    local: channel,
                };
                if !self.voices.contains_key(&key) && self.voices.len() >= limits.voice_count {
                    return Err(Error::Limit("voice count"));
                }
                self.next_voice_epoch = self
                    .next_voice_epoch
                    .checked_add(1)
                    .ok_or(Error::Limit("voice generations"))?;
                self.voice_epochs.insert(key, self.next_voice_epoch);
                self.voices.insert(key, voice);
            }
            Command::StopAudio { channel } => {
                let id = ObjectId {
                    client,
                    local: channel,
                };
                self.voices.remove(&id);
                self.voice_epochs.remove(&id);
            }
            Command::SetAudioGain {
                channel,
                gain_milli,
            } => {
                if gain_milli > 1000 {
                    return Err(Error::Invalid("gain outside 0..1000"));
                }
                let voice = self
                    .voices
                    .get_mut(&ObjectId {
                        client,
                        local: channel,
                    })
                    .ok_or(Error::NotFound)?;
                voice.gain_milli = gain_milli;
            }
            Command::Require3d => {
                return Err(Error::Unsupported(
                    "dynamic 3D command service is not implemented",
                ))
            }
        }
        Ok(())
    }
}

fn validate_rect(rect: Rect) -> Result<()> {
    if rect.width == 0 || rect.height == 0 {
        return Err(Error::Invalid("empty rectangle"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{string::ToString, vec};
    fn text(id: u32) -> Command {
        Command::PutVisual {
            id,
            node: VisualNode {
                z: 0,
                visual: Visual::Text(TextRun {
                    text: "synthetic".to_string(),
                    font: "test".to_string(),
                    size_px: 16,
                    bounds: Rect {
                        x: 0,
                        y: 0,
                        width: 80,
                        height: 20,
                    },
                    color: Color([255; 4]),
                    layout_profile: "fixture".to_string(),
                }),
            },
        }
    }
    #[test]
    fn client_ids_prevent_legacy_handle_collisions() {
        let mut s = SceneState::default();
        for id in [1, 2] {
            s.apply_batch(ClientId(id), &[text(0)], Limits::default())
                .unwrap();
        }
        assert_eq!(s.visuals().len(), 2);
        s.clear_client(ClientId(1));
        assert_eq!(s.visuals().len(), 1);
    }
    #[test]
    fn a_bad_batch_does_not_half_apply() {
        let mut s = SceneState::default();
        assert!(s
            .apply_batch(
                ClientId(1),
                &[text(0), Command::Require3d],
                Limits::default()
            )
            .is_err());
        assert!(s.visuals().is_empty());
    }
    #[test]
    fn updates_are_allowed_at_capacity() {
        let mut s = SceneState::default();
        let limits = Limits {
            visual_count: 1,
            ..Limits::default()
        };
        s.apply_batch(ClientId(1), &[text(0)], limits).unwrap();
        s.apply_batch(ClientId(1), &[text(0)], limits).unwrap();
        assert!(s.apply_batch(ClientId(1), &[text(1)], limits).is_err());
    }
    #[test]
    fn draw_order_is_stable() {
        let mut s = SceneState::default();
        s.apply_batch(ClientId(1), &vec![text(8), text(2)], Limits::default())
            .unwrap();
        assert_eq!(
            s.draw_order()
                .iter()
                .map(|(id, _)| id.local)
                .collect::<Vec<_>>(),
            vec![2, 8]
        );
    }

    #[test]
    fn replaying_the_same_sound_changes_generation() {
        let mut s = SceneState::default();
        let voice = AudioVoice {
            asset: crate::path::VfsPath::new(crate::path::Mount::Asset, "sound.wav").unwrap(),
            looping: false,
            gain_milli: 1000,
        };
        let command = Command::PlayAudio { channel: 0, voice };
        let key = ObjectId {
            client: ClientId(1),
            local: 0,
        };
        s.apply_batch(ClientId(1), &[command.clone()], Limits::default())
            .unwrap();
        let first = s.voice_epoch(key);
        s.apply_batch(ClientId(1), &[command], Limits::default())
            .unwrap();
        assert!(s.voice_epoch(key) > first);
        // Gain changes keep the epoch (no restart).
        let epoch = s.voice_epoch(key);
        s.apply_batch(
            ClientId(1),
            &[Command::SetAudioGain {
                channel: 0,
                gain_milli: 250,
            }],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(s.voice_epoch(key), epoch);
        assert_eq!(s.voices()[&key].gain_milli, 250);
        s.apply_batch(
            ClientId(1),
            &[Command::StopAudio { channel: 0 }],
            Limits::default(),
        )
        .unwrap();
        assert_eq!(s.voice_epoch(key), 0);
    }
    #[test]
    fn failed_audio_batch_does_not_advance_generation() {
        let mut s = SceneState::default();
        let voice = AudioVoice {
            asset: crate::path::VfsPath::new(crate::path::Mount::Asset, "sound.wav").unwrap(),
            looping: false,
            gain_milli: 1000,
        };
        let key = ObjectId {
            client: ClientId(1),
            local: 0,
        };
        assert!(s
            .apply_batch(
                ClientId(1),
                &[Command::PlayAudio { channel: 0, voice }, Command::Require3d],
                Limits::default()
            )
            .is_err());
        assert_eq!(s.voice_epoch(key), 0);
        assert!(s.voices().is_empty());
    }
}
