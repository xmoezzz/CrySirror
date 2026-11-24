// SPDX-License-Identifier: MPL-2.0
//! SystemUI's persistent TrophyAchieveEffect queue and its serialized NGUI panel.
use super::*;
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Effects {
    pub pending: VecDeque<i32>,
    pub widgets: Vec<Widget>,
    motion: Option<(u64, Phase)>,
    timing: (u64, f32, u64),
}
#[derive(Clone, Copy)]
enum Phase {
    In,
    Hold,
    Out,
}
impl Effects {
    pub fn update(
        &mut self,
        now: u64,
        fading: bool,
        game: &mut Assets,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        script: &AdvKag,
        files: &GameFiles,
        sound: &mut Sound,
    ) -> Result<()> {
        if self.motion.is_none()
            && !fading
            && let Some(&id) = self.pending.front()
        {
            let mut tjs = script.tjs.lock().map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let t = tjs
                .runtime
                .host_mut()
                .state
                .retail
                .table("trophy_list.nei")?;
            let r = (1..t.rows)
                .find(|&r| t.int(0, r) == id)
                .context("missing trophy")?;
            let rarity = match t.int(1, r) {
                2 => "silver",
                3 => "gold",
                _ => "bronze",
            };
            let mut w = game.scene(op, host, client, &format!("Trophy/{rarity}"))?;
            let item = t.string(9, r);
            w.retain(|w| !item.is_empty() || !w.path.ends_with("/ItemFixText"));
            for piece in &mut w {
                if let Some(label) = &mut piece.label {
                    if piece.path.ends_with("/TitleLabel") {
                        label.set_text(&format!("『{}』", t.string(2, r)));
                    } else if piece.path.ends_with("/LineupItemLabel") {
                        label.set_text(&if item.is_empty() {
                            String::new()
                        } else {
                            format!("『{item}』")
                        });
                    }
                }
                piece.depth += 10000;
            }
            self.timing = game
                .trophy_timing
                .context("missing TrophyAchieveEffect component")?;
            sound.play(host, client, files, Kind::System, "SE015.ogg", 0., false)?;
            self.widgets = w;
            self.pending.pop_front();
            self.motion = Some((now, Phase::In));
        }
        if let Some((since, phase)) = self.motion {
            let age = now.saturating_sub(since);
            match phase {
                Phase::In if !fading && age >= self.timing.0 => {
                    self.motion = Some((now, Phase::Hold))
                }
                Phase::Hold if age >= self.timing.2 => self.motion = Some((now, Phase::Out)),
                Phase::Out if age >= self.timing.0 => {
                    self.motion = None;
                    self.widgets.clear();
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub fn transform(&self, now: u64) -> (f32, u8) {
        let (dx, alpha) = self.motion.map_or((0., 0.), |(start, phase)| {
            let t = (now.saturating_sub(start) as f32 / self.timing.0.max(1) as f32).clamp(0., 1.);
            let eased = 1. - (1. - t).powi(3);
            match phase {
                Phase::In => (self.timing.1 * eased, eased),
                Phase::Hold => (self.timing.1, 1.),
                Phase::Out => (self.timing.1, 1. - eased),
            }
        });
        (dx, (255. * alpha).round() as u8)
    }
}
