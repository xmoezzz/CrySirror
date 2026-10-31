// SPDX-License-Identifier: MPL-2.0
//! Passive client-scoped tooltip resources and delayed engine presentation.
//! Text extents come from shared TextMeasurement, never character estimates.
use crate::layer::{rectangle, Paint, Primitive};
use anyhow::{ensure, Context, Result};
use crysirror_core::service::{TextMeasurement, UNWRAPPED_TEXT_LAYOUT};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
fn text_size() -> u16 {
    16
}
fn delay() -> u64 {
    500
}
fn lifetime() -> u64 {
    5000
}
fn offset() -> [f32; 2] {
    [12., 20.]
}
fn padding() -> [u16; 2] {
    [6, 4]
}
fn foreground() -> [u8; 4] {
    [0, 0, 0, 255]
}
fn background() -> [u8; 4] {
    [255, 255, 225, 255]
}
fn border() -> [u8; 4] {
    [70, 70, 60, 255]
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TooltipSpec {
    pub id: u32,
    pub text: String,
    pub font: String,
    #[serde(default = "text_size")]
    pub text_size: u16,
    #[serde(default = "delay")]
    pub delay_ms: u64,
    #[serde(default = "lifetime")]
    pub lifetime_ms: u64,
    #[serde(default = "offset")]
    pub offset: [f32; 2],
    #[serde(default = "padding")]
    pub padding: [u16; 2],
    #[serde(default = "foreground")]
    pub color: [u8; 4],
    #[serde(default = "background")]
    pub background: [u8; 4],
    #[serde(default = "border")]
    pub border: [u8; 4],
}
impl TooltipSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.id > 0
                && !self.text.is_empty()
                && self.text.len() <= 4096
                && !self.text.chars().any(char::is_control),
            "tooltip requires bounded single-line plaintext"
        );
        ensure!(
            !self.font.is_empty() && self.font.len() <= 512 && (8..=128).contains(&self.text_size),
            "tooltip font/size"
        );
        ensure!(
            self.delay_ms <= 60000
                && (1..=60000).contains(&self.lifetime_ms)
                && self.padding.iter().all(|p| *p <= 64)
                && self
                    .offset
                    .iter()
                    .all(|p| p.is_finite() && p.abs() <= 8192.),
            "tooltip timing/style bounds"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tooltip {
    spec: TooltipSpec,
    extent: [f32; 2],
}
#[derive(Clone, Debug)]
struct Active {
    id: u32,
    position: [f32; 2],
    elapsed: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TooltipSystem {
    clients: BTreeMap<u32, BTreeMap<u32, Tooltip>>,
    /// Derived preparations may be published before an atomic command batch.
    #[serde(skip)]
    prepared: Vec<(u32, TooltipSpec, [f32; 2])>,
    /// Hover/held state is transient and never resurrected by restoration.
    #[serde(skip)]
    active: BTreeMap<u32, Active>,
}
impl TooltipSystem {
    pub fn is_empty(&self) -> bool {
        self.clients.values().all(BTreeMap::is_empty)
    }
    pub fn client_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.clients.keys().copied()
    }
    pub fn get(&self, owner: u32, id: u32) -> Option<&TooltipSpec> {
        self.clients.get(&owner)?.get(&id).map(|t| &t.spec)
    }
    /// Prepare exact engine-font bounds without mutating retained tooltip state.
    /// The owner calls this before submitting PutTooltip; failures cannot commit
    /// half a UI command batch. Replacing a preparation cannot change live text.
    pub fn prepare(
        &mut self,
        owner: u32,
        spec: &TooltipSpec,
        text: &dyn TextMeasurement,
    ) -> Result<()> {
        spec.validate()?;
        let existing = self
            .prepared
            .iter()
            .position(|(o, s, _)| *o == owner && s == spec);
        ensure!(
            self.prepared.len() < 4096 || existing.is_some(),
            "tooltip preparation limit"
        );
        let measured = text
            .measure_simple(&spec.font, f32::from(spec.text_size), &spec.text)
            .map_err(|e| anyhow::anyhow!("tooltip text measurement: {e}"))?;
        ensure!(
            measured.advance.is_finite()
                && measured.advance >= 0.
                && measured.line_height.is_finite()
                && measured.line_height > 0.,
            "tooltip text metrics"
        );
        let extent = [measured.advance.max(1.), measured.line_height];
        ensure!(
            extent.iter().all(|x| x.is_finite() && *x <= 8192.),
            "tooltip measured extent limit"
        );
        if let Some(i) = existing {
            self.prepared[i].2 = extent;
        } else {
            self.prepared.push((owner, spec.clone(), extent));
        }
        Ok(())
    }
    pub fn put(&mut self, owner: u32, spec: TooltipSpec) -> Result<()> {
        spec.validate()?;
        let extent = self
            .prepared
            .iter()
            .find(|(o, s, _)| *o == owner && s == &spec)
            .map(|(_, _, extent)| *extent)
            .context("tooltip text must be prepared through shared TextMeasurement")?;
        ensure!(
            self.clients.values().map(BTreeMap::len).sum::<usize>() < 4096
                || self.get(owner, spec.id).is_some(),
            "tooltip resource limit"
        );
        let id = spec.id;
        self.clients
            .entry(owner)
            .or_default()
            .insert(id, Tooltip { spec, extent });
        self.hide(owner, Some(id));
        Ok(())
    }
    pub fn remove(&mut self, owner: u32, id: u32) -> Result<()> {
        ensure!(
            self.clients
                .get_mut(&owner)
                .and_then(|o| o.remove(&id))
                .is_some(),
            "tooltip resource missing"
        );
        self.hide(owner, Some(id));
        // Preparations belong to the entire pending atomic command batch;
        // a later Put of this id in that batch must retain its preparation.
        Ok(())
    }
    pub fn show(&mut self, owner: u32, id: u32, position: [f32; 2]) -> Result<()> {
        ensure!(
            position.iter().all(|p| p.is_finite() && p.abs() <= 100000.),
            "tooltip position"
        );
        ensure!(self.get(owner, id).is_some(), "tooltip resource missing");
        self.active.insert(
            owner,
            Active {
                id,
                position,
                elapsed: 0,
            },
        );
        Ok(())
    }
    pub fn hide(&mut self, owner: u32, id: Option<u32>) {
        if self
            .active
            .get(&owner)
            .is_some_and(|a| id.is_none_or(|id| id == a.id))
        {
            self.active.remove(&owner);
        }
    }
    pub fn clear_prepared(&mut self) {
        self.prepared.clear();
    }
    pub fn reset_input(&mut self) {
        self.active.clear();
    }
    pub fn advance(&mut self, delta_ms: u64) {
        self.active.retain(|owner, a| {
            a.elapsed = a.elapsed.saturating_add(delta_ms);
            self.clients
                .get(owner)
                .and_then(|o| o.get(&a.id))
                .is_some_and(|t| a.elapsed < t.spec.delay_ms + t.spec.lifetime_ms)
        });
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.clients.len() <= 64
                && self.clients.values().map(BTreeMap::len).sum::<usize>() <= 4096,
            "tooltip owner/resource limit"
        );
        for tips in self.clients.values() {
            for (id, t) in tips {
                t.spec.validate()?;
                ensure!(
                    *id == t.spec.id
                        && t.extent
                            .iter()
                            .all(|x| x.is_finite() && *x > 0. && *x <= 8192.),
                    "tooltip measured state"
                );
            }
        }
        for (owner, a) in &self.active {
            ensure!(
                self.get(*owner, a.id).is_some() && a.position.iter().all(|x| x.is_finite()),
                "tooltip active state"
            );
        }
        Ok(())
    }
    pub fn frame(&self, canvas: [u32; 2]) -> Result<Vec<Primitive>> {
        self.validate()?;
        let mut out = Vec::new();
        let [cw, ch] = canvas.map(|n| n as f32);
        ensure!(
            cw > 0. && ch > 0. && cw <= 8192. && ch <= 8192.,
            "tooltip canvas bounds"
        );
        for (owner, a) in &self.active {
            let t = &self.clients[owner][&a.id];
            let s = &t.spec;
            if a.elapsed < s.delay_ms {
                continue;
            }
            let [px, py] = s.padding.map(f32::from);
            let w = (t.extent[0] + 2. * px + 2.).min(cw);
            let h = (t.extent[1] + 2. * py + 2.).min(ch);
            let x = (a.position[0] + s.offset[0]).clamp(0., (cw - w).max(0.));
            let y = (a.position[1] + s.offset[1]).clamp(0., (ch - h).max(0.));
            let clip = Some([0., 0., cw, ch]);
            let order = [i32::MAX - 32, *owner as i32];
            out.push(rectangle(
                *owner,
                s.id,
                240,
                [x, y, w, h],
                Paint::Solid { color: s.border },
                clip,
                order,
            ));
            out.push(rectangle(
                *owner,
                s.id,
                241,
                [x + 1., y + 1., (w - 2.).max(1.), (h - 2.).max(1.)],
                Paint::Solid {
                    color: s.background,
                },
                clip,
                order,
            ));
            out.push(rectangle(
                *owner,
                s.id,
                242,
                [x + px + 1., y + py + 1., t.extent[0], t.extent[1]],
                Paint::Text {
                    text: s.text.clone(),
                    font: s.font.clone(),
                    size: s.text_size,
                    color: s.color,
                    layout: UNWRAPPED_TEXT_LAYOUT.into(),
                },
                Some([x, y, w, h]),
                order,
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{UiCommand, UiSystem};
    use crysirror_core::{
        service::{InputFrame, TextExtent},
        state::ClientId,
    };
    struct Metrics;
    impl TextMeasurement for Metrics {
        fn measure_simple(
            &self,
            font: &str,
            size: f32,
            text: &str,
        ) -> crysirror_core::Result<TextExtent> {
            assert_eq!((font, size, text), ("fixture", 16., "保存"));
            Ok(TextExtent {
                advance: 37.,
                line_height: 19.,
                ascent: 14.,
                descent: 5.,
            })
        }
    }
    fn spec() -> TooltipSpec {
        serde_json::from_value(serde_json::json!({"id":7,"text":"保存","font":"fixture","delay_ms":100,"lifetime_ms":200})).unwrap()
    }
    #[test]
    fn exact_measurement_delay_expiry_and_canvas_clamping_are_engine_owned() {
        let mut ui = UiSystem::default();
        let tip = spec();
        assert!(ui
            .apply_batch(
                ClientId(1),
                &[UiCommand::PutTooltip {
                    tooltip: tip.clone()
                }]
            )
            .is_err());
        ui.tooltips.prepare(1, &tip, &Metrics).unwrap();
        ui.apply_batch(
            ClientId(1),
            &[
                UiCommand::PutTooltip { tooltip: tip },
                UiCommand::ShowTooltip {
                    id: 7,
                    position: [99., 79.],
                },
            ],
        )
        .unwrap();
        ui.advance_motions(99);
        assert!(ui.frame([100, 80]).unwrap().is_empty());
        ui.advance_motions(1);
        let frame = ui.frame([100, 80]).unwrap();
        assert_eq!(frame.len(), 3);
        assert!(
            matches!(&frame[2].paint,Paint::Text{text,font,..} if text=="保存"&&font=="fixture")
        );
        assert_eq!(frame[0].extent, [51., 29.]);
        for p in &frame {
            assert!(p.quad[0][0] >= 0. && p.quad[0][1] >= 0.);
        }
        // A tooltip never registers an input target or consumes the click.
        let (actions, consumed) = ui
            .tick(
                InputFrame {
                    buttons_down: 1,
                    buttons_pressed: 1,
                    pointer_x: 90,
                    pointer_y: 70,
                },
                [100, 80],
            )
            .unwrap();
        assert!(actions.is_empty() && !consumed && ui.events.is_empty());
        ui.advance_motions(200);
        assert!(ui.frame([100, 80]).unwrap().is_empty());
    }
    #[test]
    fn ownership_atomic_replacement_and_restore_do_not_resurrect_hover() {
        let mut ui = UiSystem::default();
        let tip = spec();
        ui.tooltips.prepare(1, &tip, &Metrics).unwrap();
        assert!(ui
            .apply_batch(
                ClientId(1),
                &[
                    UiCommand::PutTooltip {
                        tooltip: tip.clone()
                    },
                    UiCommand::ShowTooltip {
                        id: 999,
                        position: [0., 0.]
                    }
                ]
            )
            .is_err());
        assert!(ui.tooltips.get(1, 7).is_none());
        ui.apply_batch(
            ClientId(1),
            &[
                UiCommand::PutTooltip {
                    tooltip: tip.clone(),
                },
                UiCommand::ShowTooltip {
                    id: 7,
                    position: [0., 0.],
                },
            ],
        )
        .unwrap();
        assert!(ui.tooltips.show(2, 7, [0., 0.]).is_err());
        assert!(ui.tooltips.remove(2, 7).is_err());
        ui.advance_motions(100);
        assert!(!ui.frame([100, 80]).unwrap().is_empty());
        let restored: UiSystem = serde_json::from_slice(&serde_json::to_vec(&ui).unwrap()).unwrap();
        assert!(restored.tooltips.get(1, 7).is_some());
        assert!(restored.frame([100, 80]).unwrap().is_empty());
        ui.tooltips.prepare(1, &tip, &Metrics).unwrap();
        // Removal followed by redefinition in one atomic batch retains the
        // preparation for the later definition.
        ui.apply_batch(
            ClientId(1),
            &[
                UiCommand::RemoveTooltip { id: 7 },
                UiCommand::PutTooltip { tooltip: tip },
            ],
        )
        .unwrap();
        assert!(ui.frame([100, 80]).unwrap().is_empty());
    }
}
