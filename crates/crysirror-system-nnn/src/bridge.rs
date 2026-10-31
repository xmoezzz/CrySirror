// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Native semantic state and scoped owner calls. Layer/Canvas/Audio services
//! own all pixels, composition and playback. No original engine is hosted.
use crate::{
    machine::{Machine, NativeCall, Payload},
    metadata::Metadata,
    native_effects::{FadeInState, SunaParameters, SunaState},
};
use anyhow::{bail, ensure, Result};
use crysirror_core::image_processing::ImageResource;
use crysirror_core::{
    service::{
        AudioRequest, HostServices, InputFrame, PatternJob, PatternOutput, PatternProcessState,
        PatternRequest,
    },
    state::ClientId,
};
use crysirror_image::temporal_pattern::{PatternRng, TemporalPatternSpec};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Explicit reviewed legacy callback ABI, not inferred from an arbitrary SPT.
/// The reviewed original module SHA and complete callback evidence are in SOURCE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackProfile {
    Mugen2Legacy,
}
/// Existing engine-owned scene root plus private adapter resource identifiers.
/// Caller initializes the root/backdrop before running this bounded profile.
#[derive(Clone, Copy, Debug)]
pub struct SceneBindings {
    pub root: u32,
    pub overlap_canvas: u32,
    pub effect_base: u32,
    pub size: [u32; 2],
    pub layers: u16,
    pub music_channel: u32,
}
pub struct Bridge {
    pub metadata: Metadata,
    pub api_calls: u64,
    pub overlap_captures: u64,
    pub option_off: i32,
    pub message_window_visible: bool,
    pub next_message_type: i32,
    pub next_window_mode: i32,
    pub last_message_id: Option<i32>,
    pub message_have_voice: bool,
    client: ClientId,
    bindings: SceneBindings,
    active_effects: BTreeSet<i32>,
    previous_effects: BTreeMap<i32, i32>,
    profile: CallbackProfile,
    patterns: BTreeMap<i32, SunaState>,
    pending_pattern: Option<(NativeCall, i32, SunaParameters, PatternJob)>,
    layer_visible: BTreeMap<i32, bool>,
    fades: BTreeMap<i32, FadeInState>,
    greys: BTreeMap<i32, crate::native_effects::LayerGreyState>,
    chara_trans: BTreeMap<i32, crate::native_chara_trans::CharaTransState>,
    old_lamps: BTreeMap<i32, crate::native_old_lamp::OldLampState>,
    buras: BTreeMap<i32, crate::native_bura::BuraState>,
    audio: Option<crate::audio_bridge::AudioBridge>,
    text: Option<crate::native_text::NativeTextBridge>,
    selection: Option<crate::native_ui::NativeSelectionBridge>,
    overlap_wait: Option<crate::native_overlap::NativeOverlap>,
    draw_wait: Option<crate::native_draw::NativeDraw>,
    frame_canvas_created: bool,
    frame_time_ms: u64,
    display_path: Option<String>,
    last_frame: Option<Arc<dyn ImageResource>>,
    overlap_frame: Option<Arc<dyn ImageResource>>,
    images: Option<crate::image_bridge::ImageBridge>,
    image_paths: BTreeMap<i32, String>,
    picture_assignments: BTreeSet<i32>,
    rectangles: BTreeMap<i32, crate::native_geometry::NativeRects>,
    fill_effects: BTreeMap<i32, [u8; 3]>,
    passive_counts: BTreeMap<i32, (i32, i32)>,
    functions: Option<crate::config::Names>,
    skip: crate::native_skip::NativeSkipPolicy,
}
impl Bridge {
    pub fn new(
        client: ClientId,
        profile: CallbackProfile,
        bindings: SceneBindings,
    ) -> Result<Self> {
        ensure!(
            client.0 != 0 && bindings.root != 0 && bindings.overlap_canvas != 0,
            "System-NNN presentation identity"
        );
        ensure!(
            bindings.size.iter().all(|v| (1..=8192).contains(v)) && bindings.layers == 16,
            "System-NNN presentation bounds"
        );
        let end = bindings
            .effect_base
            .checked_add(u32::from(bindings.layers))
            .ok_or_else(|| anyhow::anyhow!("System-NNN effect identifiers"))?;
        ensure!(
            !(bindings.effect_base..end).contains(&bindings.root),
            "System-NNN root/effect collision"
        );
        Ok(Self {
            metadata: Metadata::default(),
            api_calls: 0,
            overlap_captures: 0,
            option_off: 0,
            message_window_visible: false,
            next_message_type: 3,
            next_window_mode: -1,
            last_message_id: None,
            message_have_voice: false,
            client,
            bindings,
            active_effects: BTreeSet::new(),
            previous_effects: BTreeMap::new(),
            profile,
            patterns: BTreeMap::new(),
            pending_pattern: None,
            layer_visible: BTreeMap::new(),
            fades: BTreeMap::new(),
            greys: BTreeMap::new(),
            chara_trans: BTreeMap::new(),
            old_lamps: BTreeMap::new(),
            buras: BTreeMap::new(),
            audio: None,
            text: None,
            selection: None,
            overlap_wait: None,
            draw_wait: None,
            frame_canvas_created: false,
            frame_time_ms: 50,
            display_path: None,
            last_frame: None,
            overlap_frame: None,
            images: None,
            image_paths: BTreeMap::new(),
            picture_assignments: BTreeSet::new(),
            rectangles: BTreeMap::new(),
            fill_effects: BTreeMap::new(),
            passive_counts: BTreeMap::new(),
            functions: None,
            skip: crate::native_skip::NativeSkipPolicy::default(),
        })
    }
    /// Configuration tables are native byte-name/number policy, never an
    /// arbitrary host callback registry. Unknown names still fail at their PC.
    pub fn configure_functions(&mut self, functions: crate::config::Names) -> Result<()> {
        ensure!(
            self.functions.is_none(),
            "NNN function policy already configured"
        );
        ensure!(
            functions.tokens.len() % 2 == 0,
            "NNN function name/label pairs"
        );
        self.functions = Some(functions);
        Ok(())
    }
    pub fn skip_policy(&self) -> &crate::native_skip::NativeSkipPolicy {
        &self.skip
    }
    fn message_control(&self) -> crate::native_message_control::NativeMessageControl {
        crate::native_message_control::NativeMessageControl {
            last_message_id: self.last_message_id,
            next_type: self.next_message_type,
            next_window: self.next_window_mode,
            window_visible: self.message_window_visible,
            has_voice: self.message_have_voice,
        }
    }
    fn adopt_message_control(
        &mut self,
        policy: crate::native_message_control::NativeMessageControl,
    ) {
        self.last_message_id = policy.last_message_id;
        self.next_message_type = policy.next_type;
        self.next_window_mode = policy.next_window;
        self.message_window_visible = policy.window_visible;
        self.message_have_voice = policy.has_voice;
    }
    /// Pending task readiness is independent from an original timer or input
    /// wait. This query never polls a receipt, executes code or consumes input.
    pub fn dependency_pending(&self) -> bool {
        self.pending_pattern.is_some()
            || self.images.as_ref().is_some_and(|s| s.dependency_pending())
            || self.audio.as_ref().is_some_and(|s| s.pending())
            || self.text.as_ref().is_some_and(|s| s.dependency_pending())
            || self
                .selection
                .as_ref()
                .is_some_and(|s| s.dependency_pending())
            || self
                .draw_wait
                .as_ref()
                .is_some_and(|s| s.dependency_pending())
            || self
                .overlap_wait
                .as_ref()
                .is_some_and(|s| s.dependency_pending())
    }
    pub fn wait_kind(&self, call: Option<&NativeCall>) -> &'static str {
        if self.dependency_pending() {
            return "resource";
        }
        if self.draw_wait.is_some() || self.overlap_wait.is_some() {
            return "timer";
        }
        if call.is_some_and(|call| {
            matches!(
                call.payload,
                Payload::Message { .. } | Payload::Selection { .. }
            )
        }) {
            return "input";
        }
        "owner"
    }
    pub fn previous_effect(&self, slot: i32) -> Option<i32> {
        self.previous_effects.get(&slot).copied()
    }
    pub fn configure_audio(&mut self, audio: crate::audio_bridge::AudioBridge) -> Result<()> {
        ensure!(
            audio.client() == self.client,
            "NNN audio bridge foreign client"
        );
        ensure!(self.audio.is_none(), "NNN audio bridge already configured");
        self.audio = Some(audio);
        Ok(())
    }
    pub fn pattern(&self, slot: i32) -> Option<&SunaState> {
        self.patterns.get(&slot)
    }
    pub fn picture(&self, slot: i32) -> Option<&crate::image_bridge::PreparedPicture> {
        self.images.as_ref()?.pictures.get(&slot)
    }
    pub fn set_frame_time(&mut self, value: u64) -> Result<()> {
        ensure!((1..=10000).contains(&value), "NNN original frameTime range");
        self.frame_time_ms = value;
        Ok(())
    }
    pub fn effect_counter(&self, slot: i32) -> Option<(i32, i32)> {
        self.passive_counts
            .get(&slot)
            .copied()
            .or_else(|| {
                self.patterns
                    .get(&slot)
                    .map(|s| (s.count, s.parameters.frames))
            })
            .or_else(|| self.fades.get(&slot).map(|s| (s.count, s.frames)))
            .or_else(|| self.greys.get(&slot).map(|s| (s.count, s.frames)))
            .or_else(|| self.chara_trans.get(&slot).map(|s| (s.count, s.frames)))
            .or_else(|| self.old_lamps.get(&slot).map(|s| (s.count, 999999)))
            .or_else(|| self.buras.get(&slot).map(|s| (s.count, s.frames)))
    }
    pub fn configure_resources(
        &mut self,
        resources: crate::resource_bridge::NativeResources,
        names: crate::native_resources::ResourceNames,
        levels: u32,
    ) -> Result<()> {
        ensure!(self.images.is_none(), "NNN image bridge already configured");
        self.images = Some(crate::image_bridge::ImageBridge::new(
            self.client,
            names,
            levels,
            resources,
        )?);
        Ok(())
    }
    pub fn configure_text(&mut self, text: crate::native_text::NativeTextBridge) -> Result<()> {
        ensure!(
            self.text.is_none()
                && text.profile.client == self.client
                && text.profile.root == self.bindings.root,
            "NNN text bridge scope/already configured"
        );
        self.text = Some(text);
        Ok(())
    }
    pub fn configure_selection(
        &mut self,
        selection: crate::native_ui::NativeSelectionBridge,
    ) -> Result<()> {
        ensure!(
            self.selection.is_none()
                && selection.profile.client == self.client
                && selection.profile.root == self.bindings.root,
            "NNN selection bridge scope/already configured"
        );
        self.selection = Some(selection);
        Ok(())
    }
    pub fn try_commit_with_context(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        program: &crate::Program,
        input: InputFrame,
        host: &mut dyn HostServices,
    ) -> Result<bool> {
        machine.validate_receipt(call)?;
        if matches!(call.payload, Payload::Message { .. }) {
            ensure!(self.patterns.is_empty()&&self.fades.is_empty()&&self.greys.is_empty()&&self.chara_trans.is_empty()&&self.old_lamps.is_empty()&&self.buras.is_empty(),"NNN ongoing native effects during message wait require shared frame/window dependency recomposition");
            let text = self
                .text
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("NNN native message profile not prepared"))?;
            if let Some(background) = &self.last_frame {
                text.set_background(background.clone())?;
            }
            if !text.pending_message() {
                let default = call.code == 0x22220001;
                let visible = match self.next_window_mode {
                    0 => false,
                    1 => true,
                    _ => default,
                };
                text.set_next_window_visibility(visible)?;
                self.next_window_mode = -1;
                if let Payload::Message { serial, .. } = &call.payload {
                    self.last_message_id = Some(*serial as i32);
                }
            }
            let before = text.api_calls;
            let result = text.try_commit(machine, call, host, input);
            self.message_window_visible = text.state.window_visible;
            self.api_calls += text.api_calls - before;
            return result;
        }
        if matches!(call.payload, Payload::Selection { .. }) {
            let selection = self
                .selection
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("NNN native selection profile not prepared"))?;
            let before = selection.api_calls;
            let result = selection.try_commit(machine, call, program, host);
            self.api_calls += selection.api_calls - before;
            return result;
        }
        self.try_commit(machine, call, host)
    }
    /// Nonblocking owner-side resource dependency. No native receipt or global
    /// random state is committed until the scoped engine worker has succeeded.
    pub fn try_commit(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
    ) -> Result<bool> {
        machine.validate_receipt(call)?;
        if call.identify == 0x66660006 && call.code == 0x22220005 {
            return self.try_overlap(machine, call, host);
        }
        if call.identify == 0x66660006 && call.code == 0x22220004 {
            return self.try_draw(machine, call, host);
        }
        if matches!(call.payload, Payload::Image { .. }) {
            return self.try_image(machine, call, host);
        }
        if matches!(call.code, 0x33330007 | 0x33330008 | 0x33330009) {
            if let Some(audio) = self.audio.as_mut() {
                let before = audio.api_calls;
                let result = audio.try_commit(machine, call, host);
                self.api_calls += audio.api_calls - before;
                return result;
            }
        }
        if self.pending_pattern.is_none() {
            if let Payload::Words { count, words } = &call.payload {
                if call.code == 0x33330013 && *count >= 2 && words[1] == 29 {
                    let slot = words[0];
                    self.layer_id(slot)?;
                    let parameters = SunaParameters::parse(&words[2..], self.bindings.size)?;
                    let candidate = machine.native_variables_candidate(call)?;
                    let rng = candidate.random_state().ok_or_else(|| {
                        anyhow::anyhow!("SUNA requires original initialized shared random state")
                    })?;
                    let spec = TemporalPatternSpec {
                        size: self.bindings.size,
                        cell: [2, 2],
                    };
                    spec.validate()?;
                    let job = host.begin_pattern(
                        self.client,
                        PatternRequest::Create {
                            spec: serde_json::to_vec(&spec)?.into(),
                            rng: serde_json::to_vec(&rng)?.into(),
                        },
                    )?;
                    self.api_calls += 1;
                    self.pending_pattern = Some((call.clone(), slot, parameters, job));
                } else {
                    self.commit(machine, call, host)?;
                    return Ok(true);
                }
            } else {
                self.commit(machine, call, host)?;
                return Ok(true);
            }
        }
        let (receipt, slot, parameters, job) = self.pending_pattern.as_mut().unwrap();
        ensure!(
            receipt == call,
            "System-NNN resource receipt changed while pending"
        );
        match job.poll()? {
            PatternProcessState::Pending => Ok(false),
            PatternProcessState::Failed(error) => bail!("SUNA engine pattern creation: {error}"),
            PatternProcessState::Ready(PatternOutput::Resource {
                resource,
                rng_after,
            }) => {
                ensure!(
                    resource.client() == self.client && resource.info().size == self.bindings.size,
                    "SUNA scoped engine resource"
                );
                let slot = *slot;
                let parameters = parameters.clone();
                let rng: PatternRng = serde_json::from_slice(&rng_after)?;
                rng.validate()?;
                let mut candidate = machine.native_variables_candidate(call)?;
                candidate.replace_random_state(rng)?;
                self.restore_picture_source(slot, host)?;
                machine.acknowledge_with_variables(call, candidate)?;
                self.fill_effects.remove(&slot);
                self.fades.remove(&slot);
                self.greys.remove(&slot);
                self.chara_trans.remove(&slot);
                self.old_lamps.remove(&slot);
                self.buras.remove(&slot);
                self.passive_counts.remove(&slot);
                // Native CAllEffect owns one CEffectSuna object shared by all
                // effect29 layers. SetParam reinitializes its one pattern,
                // rather than giving every layer a private random texture.
                for state in self.patterns.values_mut() {
                    state.resource = resource.clone();
                }
                self.patterns.insert(
                    slot,
                    SunaState {
                        parameters,
                        resource,
                        count: 0,
                    },
                );
                self.previous_effects.insert(slot, 29);
                self.pending_pattern = None;
                Ok(true)
            }
            PatternProcessState::Ready(_) => bail!("SUNA engine returned image for creation"),
        }
    }
    fn try_image(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
    ) -> Result<bool> {
        if let Payload::Image {
            preload: true,
            slot,
            ..
        } = &call.payload
        {
            ensure!(!self.picture_assignments.contains(slot),"NNN active-picture preload needs resource rebinding preserving original effect rectangles");
        }
        let images = self
            .images
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("NNN scoped image resources not prepared"))?;
        let before = images.api_calls;
        let result = images.prepare(machine, call, host);
        self.api_calls += images.api_calls - before;
        let Some(picture) = result? else {
            return Ok(false);
        };
        if !picture.preload {
            let id = self.layer_id(picture.slot)?;
            let size = picture.display.size();
            let rectangles = crate::native_geometry::NativeRects::natural(size);
            let published = host.publish_image_resource(self.client, id, picture.display)?;
            self.api_calls += 1;
            if !self.fill_effects.contains_key(&picture.slot) {
                let op = self.picture_operation(
                    picture.slot,
                    size,
                    published.path.relative(),
                    &rectangles,
                )?;
                if let Err(error) = self.submit(host, json!({"layers":[op]})) {
                    let _ = host.release_image_resource(self.client, published.path.relative());
                    return Err(error);
                }
            }
            machine.acknowledge(call)?;
            self.images.as_mut().unwrap().accept(call)?;
            if let Some(previous) = self
                .image_paths
                .insert(picture.slot, published.path.relative().to_owned())
            {
                host.release_image_resource(self.client, &previous)?;
            }
            self.active_effects.insert(picture.slot);
            self.picture_assignments.insert(picture.slot);
            self.rectangles.insert(picture.slot, rectangles);
        } else {
            machine.acknowledge(call)?;
            self.images.as_mut().unwrap().accept(call)?;
        }
        Ok(true)
    }
    fn picture_operation(
        &self,
        slot: i32,
        size: [u32; 2],
        path: &str,
        rectangles: &crate::native_geometry::NativeRects,
    ) -> Result<Value> {
        let (rect, uv) = rectangles.plain_layer(size)?;
        Ok(
            json!({"op":"put","layer":{"id":self.layer_id(slot)?,"parent":self.bindings.root,"rect":rect,"uv":uv,"order":[slot,0],"visible":self.layer_visible.get(&slot).copied().unwrap_or(true),"paint":{"kind":"image","asset":path}}}),
        )
    }
    fn restore_picture_source(&mut self, slot: i32, host: &mut dyn HostServices) -> Result<()> {
        if !self.fill_effects.contains_key(&slot) {
            return Ok(());
        }
        let id = self.layer_id(slot)?;
        if self.picture_assignments.contains(&slot) {
            let picture = self
                .images
                .as_ref()
                .and_then(|images| images.pictures.get(&slot))
                .ok_or_else(|| anyhow::anyhow!("NNN assigned picture resource absent"))?;
            let size = picture.display.size();
            let path = self
                .image_paths
                .get(&slot)
                .ok_or_else(|| anyhow::anyhow!("NNN assigned picture publication absent"))?;
            let rectangles = self
                .rectangles
                .get(&slot)
                .cloned()
                .unwrap_or_else(|| crate::native_geometry::NativeRects::natural(size));
            let op = self.picture_operation(slot, size, path, &rectangles)?;
            self.submit(host, json!({"layers":[op]}))?;
            self.active_effects.insert(slot);
        } else if self.active_effects.contains(&slot) {
            self.submit(host, json!({"layers":[{"op":"remove","id":id}]}))?;
            self.active_effects.remove(&slot);
        }
        Ok(())
    }
    fn frame_effects(&self) -> BTreeMap<i32, crate::native_presentation::NativeFrameEffect> {
        use crate::native_presentation::NativeFrameEffect;
        let mut effects = BTreeMap::new();
        for (slot, state) in &self.patterns {
            effects.insert(*slot, NativeFrameEffect::Suna(state.clone()));
        }
        for (slot, state) in &self.fades {
            effects.insert(*slot, NativeFrameEffect::Fade(state.clone()));
        }
        for (slot, state) in &self.greys {
            effects.insert(*slot, NativeFrameEffect::Grey(state.clone()));
        }
        for (slot, state) in &self.chara_trans {
            let picture = self
                .images
                .as_ref()
                .and_then(|images| images.pictures.get(slot))
                .filter(|_| self.picture_assignments.contains(slot));
            let source = picture.map(|p| {
                if p.has_mask {
                    p.raw.clone()
                } else {
                    p.opaque.clone()
                }
            });
            let rectangles = self.rectangles.get(slot).cloned().unwrap_or_else(|| {
                crate::native_geometry::NativeRects::natural(self.bindings.size)
            });
            effects.insert(
                *slot,
                NativeFrameEffect::Chara(crate::native_presentation::CharaTransFrame {
                    state: state.clone(),
                    source,
                    rectangles,
                }),
            );
        }
        for (slot, state) in &self.old_lamps {
            let source = self
                .images
                .as_ref()
                .and_then(|images| images.pictures.get(slot))
                .filter(|_| {
                    self.picture_assignments.contains(slot)
                        && self.layer_visible.get(slot) != Some(&false)
                })
                .map(|p| p.display.clone());
            let r = self.rectangles.get(slot).cloned().unwrap_or_else(|| {
                crate::native_geometry::NativeRects::natural(self.bindings.size)
            });
            effects.insert(
                *slot,
                NativeFrameEffect::Lamp(crate::native_presentation::OldLampFrame {
                    slot: *slot,
                    state: state.clone(),
                    source,
                    destination: [r.destination[0], r.destination[1]],
                }),
            );
        }
        for (slot, state) in &self.buras {
            effects.insert(*slot, NativeFrameEffect::Bura(state.clone()));
        }
        effects
    }
    fn visible_effects(
        &self,
        effects: &BTreeMap<i32, crate::native_presentation::NativeFrameEffect>,
    ) -> Result<BTreeSet<i32>> {
        use crate::native_presentation::NativeFrameEffect;
        let visible: BTreeSet<_> = effects
            .keys()
            .filter(|slot| self.layer_visible.get(slot) != Some(&false))
            .copied()
            .collect();
        for slot in &visible {
            match &effects[slot] {
                NativeFrameEffect::Paint(_) => bail!("NNN paint DTO in persistent effect state"),
                NativeFrameEffect::Suna(state) => {
                    state.opacity256()?;
                }
                NativeFrameEffect::Fade(state) => {
                    state.linear_image_effect()?;
                }
                NativeFrameEffect::Grey(state) => {
                    state.linear_image_effect()?;
                }
                NativeFrameEffect::Chara(_) => {}
                NativeFrameEffect::Lamp(_) => {}
                NativeFrameEffect::Bura(state) => {
                    state.image_effect(self.bindings.size)?;
                }
            }
        }
        Ok(visible)
    }
    /// Ordinary pictures at/above the first screen effect cannot be flattened
    /// into the lower Canvas. They remain ordered shared image graph inputs.
    fn frame_underlays(
        &self,
        effects: &BTreeMap<i32, crate::native_presentation::NativeFrameEffect>,
        visible: &BTreeSet<i32>,
    ) -> Result<BTreeMap<i32, crysirror_core::image_processing::ImageCompositeLayer>> {
        use crysirror_core::image_processing::{
            ImageCompositeLayer, ImageCompositeMath, ImageInput,
        };
        let mut result = BTreeMap::new();
        let Some(first) = visible.first() else {
            return Ok(result);
        };
        for slot in self.active_effects.range(*first..) {
            if self.layer_visible.get(slot) == Some(&false)
                || self.chara_trans.contains_key(slot)
                || self.old_lamps.contains_key(slot)
            {
                continue;
            }
            let layer = if let Some(color) = self.fill_effects.get(slot) {
                let [w, h] = self.bindings.size;
                ImageCompositeLayer {
                    input: Box::new(ImageInput::Solid {
                        size: [w, h],
                        color: [color[0], color[1], color[2], 255],
                    }),
                    source: [0, 0, w as i32, h as i32],
                    destination: [0, 0],
                    clip: None,
                    opacity: 255,
                    math: ImageCompositeMath::CopyRgb,
                }
            } else if self.picture_assignments.contains(slot) {
                let picture = self
                    .images
                    .as_ref()
                    .and_then(|i| i.pictures.get(slot))
                    .ok_or_else(|| anyhow::anyhow!("NNN ordered picture resource missing"))?;
                let background = *slot < 4 || *slot == 14;
                let image = if background {
                    picture.opaque.clone()
                } else if picture.has_mask {
                    picture.raw.clone()
                } else {
                    picture.display.clone()
                };
                let r =
                    self.rectangles.get(slot).cloned().unwrap_or_else(|| {
                        crate::native_geometry::NativeRects::natural(image.size())
                    });
                ensure!(
                    r.destination_from == r.destination_to,
                    "NNN ordered moving picture needs native frame rectangle interpolation"
                );
                let [x, y, w, h] = r.destination;
                let [sx, sy, _, _] = r.source;
                ImageCompositeLayer {
                    input: Box::new(ImageInput::Resource(image)),
                    source: [sx, sy, w, h],
                    destination: [x, y],
                    clip: None,
                    opacity: 255,
                    math: if background {
                        ImageCompositeMath::CopyRgb
                    } else {
                        ImageCompositeMath::MaskSeparate256
                    },
                }
            } else {
                ensure!(
                    effects.contains_key(slot),
                    "NNN ordered layer lacks picture/fill/effect"
                );
                continue;
            };
            result.insert(*slot, layer);
        }
        Ok(result)
    }
    fn prepare_frame_canvas(
        &mut self,
        host: &mut dyn HostServices,
        first: Option<i32>,
    ) -> Result<u32> {
        let [w, h] = self.bindings.size;
        let canvas = self
            .bindings
            .overlap_canvas
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("NNN frame Canvas identifier overflow"))?;
        let mut canvases = vec![if self.frame_canvas_created {
            json!({"op":"restore","id":canvas,"surface":{"width":w,"height":h,"nodes":[]}})
        } else {
            json!({"op":"create","id":canvas,"width":w,"height":h})
        }];
        canvases.push(json!({"op":"clear","id":canvas,"color":[0,0,0,255]}));
        for slot in &self.active_effects {
            if first.is_some_and(|first| *slot >= first)
                || self.chara_trans.contains_key(slot)
                || self.old_lamps.contains_key(slot)
            {
                continue;
            }
            canvases.push(json!({"op":"copy_subtree","source":self.layer_id(*slot)?,"destination":canvas,"rect":[0,0,w,h],"position":[0,0]}));
        }
        self.submit(host, json!({"canvases":canvases}))?;
        self.frame_canvas_created = true;
        Ok(canvas)
    }
    fn try_draw(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
    ) -> Result<bool> {
        use crate::native_presentation::{NativeFrameEffect, NativeFrameWork};
        if self.draw_wait.is_none() {
            let mut policy = self.message_control();
            let visible = policy.before_print(self.message_window_visible);
            ensure!(!self.text.as_ref().is_some_and(|text|text.state.window_visible&&visible!=text.state.window_visible),"NNN DRAW live message-window refresh requires native window dependency composition");
            self.adopt_message_control(policy);
            let Payload::Words { count: 1, words } = &call.payload else {
                bail!("NNN native DRAW payload")
            };
            ensure!(
                words.len() == 1 && words[0] >= 0 && words[0] <= 864000,
                "NNN native DRAW count bounds"
            );
            let effects = self.frame_effects();
            let visible = self.visible_effects(&effects)?;
            let random = machine
                .native_variables_candidate(call)?
                .random_state()
                .ok_or_else(|| anyhow::anyhow!("NNN DRAW requires original shared random state"))?;
            let underlays = self.frame_underlays(&effects, &visible)?;
            let canvas = self.prepare_frame_canvas(host, visible.first().copied())?;
            let display = self.bindings.effect_base + u32::from(self.bindings.layers);
            ensure!(display != self.bindings.root, "NNN display/root collision");
            let work = NativeFrameWork::begin_draw(
                self.client,
                canvas,
                crate::native_presentation::ordered_frame_plan(&effects, &visible, &underlays),
                random,
                matches!(&call.payload,Payload::Words{words,..} if words[0]>0),
                host,
            )?;
            self.api_calls += work.api_calls;
            self.draw_wait = Some(crate::native_draw::NativeDraw::begin(
                self.client,
                display,
                self.bindings.layers,
                self.bindings.size,
                self.frame_time_ms,
                call,
                work,
                effects,
                visible,
                underlays,
                self.display_path.take(),
                host.now_ms(),
            )?);
        }
        let wait = self.draw_wait.as_mut().unwrap();
        let before = wait.api_calls;
        let result = wait.poll(machine, call, host);
        self.api_calls += wait.api_calls - before;
        if let Some(result) = result? {
            if let Payload::Words { words, .. } = &call.payload {
                let frames = words[0];
                if frames > 0 {
                    for (count, max) in self.passive_counts.values_mut() {
                        if *count < *max {
                            *count = count.saturating_add(frames).min(*max);
                        }
                    }
                }
            }
            self.patterns.clear();
            self.fades.clear();
            self.greys.clear();
            self.chara_trans.clear();
            self.old_lamps.clear();
            self.buras.clear();
            for (slot, effect) in result.effects {
                match effect {
                    NativeFrameEffect::Paint(_) => {
                        bail!("NNN paint DTO returned as semantic state")
                    }
                    NativeFrameEffect::Suna(state) => {
                        self.patterns.insert(slot, state);
                    }
                    NativeFrameEffect::Fade(state) => {
                        self.fades.insert(slot, state);
                    }
                    NativeFrameEffect::Grey(state) => {
                        self.greys.insert(slot, state);
                    }
                    NativeFrameEffect::Chara(frame) => {
                        self.chara_trans.insert(slot, frame.state);
                    }
                    NativeFrameEffect::Lamp(frame) => {
                        self.old_lamps.insert(slot, frame.state);
                    }
                    NativeFrameEffect::Bura(state) => {
                        self.buras.insert(slot, state);
                    }
                }
            }
            self.last_frame = Some(result.frame.target);
            self.display_path = result.display_path;
            self.draw_wait = None;
            return Ok(true);
        }
        Ok(false)
    }
    fn try_overlap(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
    ) -> Result<bool> {
        use crate::native_presentation::NativeFrameWork;
        if self.overlap_wait.is_none() {
            let Payload::Words { count, words } = &call.payload else {
                bail!("NNN overlap payload")
            };
            ensure!(
                *count >= 2
                    && words.len() >= 2
                    && words[0] == 0
                    && words[1] > 0
                    && words[1] <= 864000,
                "NNN unsupported native overlap type/length"
            );
            let effects = self.frame_effects();
            let visible = self.visible_effects(&effects)?;
            let underlays = self.frame_underlays(&effects, &visible)?;
            let effects =
                crate::native_presentation::ordered_frame_plan(&effects, &visible, &underlays);
            let candidate = machine.native_variables_candidate(call)?;
            let random = candidate.random_state().ok_or_else(|| {
                anyhow::anyhow!("NNN frame requires original initialized global random state")
            })?;
            let canvas = self.prepare_frame_canvas(host, visible.first().copied())?;
            let display = self.bindings.effect_base + u32::from(self.bindings.layers);
            ensure!(display != self.bindings.root, "NNN display/root collision");
            let work = NativeFrameWork::begin(
                self.client,
                self.bindings.overlap_canvas,
                canvas,
                self.overlap_frame.clone(),
                effects,
                random,
                host,
            )?;
            self.api_calls += work.api_calls;
            self.overlap_wait = Some(crate::native_overlap::NativeOverlap::begin(
                self.client,
                display,
                self.bindings.layers,
                self.bindings.size,
                self.frame_time_ms,
                call,
                work,
                self.display_path.take(),
                host.now_ms(),
            )?);
        }
        let wait = self.overlap_wait.as_mut().unwrap();
        let before = wait.api_calls;
        let result = wait.poll(machine, call, host);
        self.api_calls += wait.api_calls - before;
        if let Some(result) = result? {
            if let Some(pattern) = result.frame.shared_pattern {
                for state in self.patterns.values_mut() {
                    state.resource = pattern.clone();
                }
            }
            self.last_frame = Some(result.frame.target);
            self.display_path = result.display_path;
            self.overlap_wait = None;
            return Ok(true);
        }
        Ok(false)
    }
    fn layer_id(&self, slot: i32) -> Result<u32> {
        ensure!(
            (0..i32::from(self.bindings.layers)).contains(&slot),
            "System-NNN effect layer bounds"
        );
        Ok(self.bindings.effect_base + slot as u32)
    }
    fn submit(&mut self, host: &mut dyn HostServices, batch: Value) -> Result<()> {
        host.submit_engine(self.client, 1, &serde_json::to_vec(&batch)?)?;
        self.api_calls += 1;
        Ok(())
    }
    /// Only a real successful owner operation consumes its exact native receipt.
    /// Errors preserve both the pending PC and native policy state.
    pub fn commit(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
    ) -> Result<()> {
        machine.validate_receipt(call)?;
        if matches!(call.payload, Payload::StoryName(_) | Payload::FilmName(_)) {
            return self.metadata.commit(machine, call);
        }
        if let Payload::VariableWrites { writes } = &call.payload {
            let ops=writes.iter().filter(|write|self.active_effects.contains(&i32::from(write.layer)))
                .map(|write|Ok(json!({"op":"set_visible","id":self.layer_id(i32::from(write.layer))?,"visible":write.value==0})))
                .collect::<Result<Vec<_>>>()?;
            if !ops.is_empty() {
                self.submit(host, json!({"layers":ops}))?;
            }
            for write in writes {
                self.layer_visible
                    .insert(i32::from(write.layer), write.value == 0);
            }
            return machine.acknowledge(call);
        }
        if let Payload::UserCall { command: false, .. } = &call.payload {
            let table = self.functions.as_ref().ok_or_else(|| {
                anyhow::anyhow!("NNN original user-function table not configured")
            })?;
            let at = (call.code as usize)
                .checked_mul(2)
                .ok_or_else(|| anyhow::anyhow!("NNN function index overflow"))?;
            let name = table.tokens.get(at).ok_or_else(|| {
                anyhow::anyhow!("NNN original function index {} outside table", call.code)
            })?;
            if name.eq_ignore_ascii_case(b"stopf5") {
                machine.acknowledge(call)?;
                self.skip.stop_f5();
                return Ok(());
            }
            bail!("unsupported System-NNN native function pc={:#x} index={} name={}; exact pending call retained",call.pc,call.code,String::from_utf8_lossy(name));
        }
        ensure!(
            call.identify == 0x66660007,
            "System-NNN callback needs another semantic mapping"
        );
        match (self.profile, call.code, &call.payload) {
            // Original legacy ReceiveScriptCommand 0x541af0 has no branch for
            // this code; original ExecScript 0x4ea210 passes its scalar then advances.
            (CallbackProfile::Mugen2Legacy, 0x33330052, Payload::Scalar(_)) => {}
            (_, 0x33330012, Payload::Empty) => {
                let mut ops = self
                    .active_effects
                    .iter()
                    .map(|s| Ok(json!({"op":"remove","id":self.layer_id(*s)?})))
                    .collect::<Result<Vec<_>>>()?;
                if let Some(text) = &self.text {
                    ops.extend(text.clear_operations()?);
                }
                self.submit(host, json!({"layers":ops}))?;
                if let Some(text) = &mut self.text {
                    text.accept_clear()?;
                }
                self.active_effects.clear();
                self.patterns.clear();
                self.fades.clear();
                self.greys.clear();
                self.chara_trans.clear();
                self.old_lamps.clear();
                self.buras.clear();
                self.fill_effects.clear();
                self.passive_counts.clear();
                self.picture_assignments.clear();
                self.rectangles.clear();
            }
            (_, 0x33330011, Payload::Empty) => {
                let [w, h] = self.bindings.size;
                let mut ops = vec![if self.overlap_captures == 0 {
                    json!({"op":"create","id":self.bindings.overlap_canvas,"width":w,"height":h})
                } else {
                    json!({"op":"restore","id":self.bindings.overlap_canvas,"surface":{"width":w,"height":h,"nodes":[]}})
                }];
                ops.push(if let Some(path)=&self.display_path{
                    json!({"op":"set","id":self.bindings.overlap_canvas,"layer":{"id":1,"rect":[0,0,w,h],"paint":{"kind":"image","asset":path}}})
                }else{json!({"op":"copy_subtree","source":self.bindings.root,"destination":self.bindings.overlap_canvas,"rect":[0,0,w,h],"position":[0,0]})});
                self.submit(host, json!({"canvases":ops}))?;
                self.overlap_captures += 1;
                self.overlap_frame = self.last_frame.clone();
            }
            (_, code @ (0x3333001b | 0x3333001c), Payload::Words { count, words }) => {
                ensure!(
                    *count as usize == words.len(),
                    "NNN original message control parameter span"
                );
                let mut policy = self.message_control();
                if code == 0x3333001b {
                    policy.start_message(words, &self.skip)?;
                } else {
                    policy.start_koma(words, &self.skip)?;
                }
                machine.acknowledge(call)?;
                self.adopt_message_control(policy);
                return Ok(());
            }
            (_, 0x33330007, Payload::Words { count: 1, words }) if words == &[-1] => {
                // No source has been admitted by this bounded bridge. A live
                // source needs original fade-unit/settings policy, not a guessed0ms stop.
                ensure!(
                    !host.audio_playing(self.client, self.bindings.music_channel)?,
                    "System-NNN active music needs native fade/settings mapping"
                );
                host.submit_audio(
                    self.client,
                    AudioRequest::Stop {
                        channel: self.bindings.music_channel,
                        fade_ms: 0.,
                    },
                )?;
                self.api_calls += 1;
            }
            (_, 0x3333005a, Payload::Scalar(value)) => {
                self.option_off = if *value == -1 { 0 } else { *value };
            }
            (_, 0x33330016, Payload::Words { count, words }) => {
                ensure!(
                    *count as usize == words.len() && words.len() >= 9,
                    "NNN SETEFFECTRECT requires slot and eight original coordinates"
                );
                let slot = words[0];
                self.layer_id(slot)?;
                let mut rectangles = self.rectangles.get(&slot).cloned().unwrap_or_else(|| {
                    crate::native_geometry::NativeRects::natural(self.bindings.size)
                });
                rectangles.set_destination_pair(
                    words[1..5].try_into().unwrap(),
                    words[5..9].try_into().unwrap(),
                );
                if self.picture_assignments.contains(&slot)
                    && !self.fill_effects.contains_key(&slot)
                {
                    let picture = self
                        .images
                        .as_ref()
                        .and_then(|i| i.pictures.get(&slot))
                        .ok_or_else(|| anyhow::anyhow!("NNN assigned geometry picture missing"))?;
                    let path = self.image_paths.get(&slot).ok_or_else(|| {
                        anyhow::anyhow!("NNN assigned geometry publication missing")
                    })?;
                    let op =
                        self.picture_operation(slot, picture.display.size(), path, &rectangles)?;
                    self.submit(host, json!({"layers":[op]}))?;
                }
                machine.acknowledge(call)?;
                self.rectangles.insert(slot, rectangles);
                return Ok(());
            }
            (_, 0x33330014, Payload::Words { count: 1, words }) => {
                let id = self.layer_id(words[0])?;
                let ops = if self.active_effects.contains(&words[0]) {
                    vec![json!({"op":"remove","id":id})]
                } else {
                    vec![]
                };
                self.submit(host, json!({"layers":ops}))?;
                self.active_effects.remove(&words[0]);
                self.patterns.remove(&words[0]);
                self.fades.remove(&words[0]);
                self.greys.remove(&words[0]);
                self.chara_trans.remove(&words[0]);
                self.old_lamps.remove(&words[0]);
                self.buras.remove(&words[0]);
                self.fill_effects.remove(&words[0]);
                self.passive_counts.remove(&words[0]);
                self.picture_assignments.remove(&words[0]);
                self.rectangles.remove(&words[0]);
            }
            (_, 0x33330013, Payload::Words { count, words }) if *count >= 2 && words[1] == 54 => {
                let slot = words[0];
                let id = self.layer_id(slot)?;
                // EFFECT_FILL's first parameter is reserved. Absent RGB uses0;
                // green inherits red, blue inherits green; stored bytes wrap.
                let red = words.get(3).copied().unwrap_or(0) as u8;
                let green = words.get(4).copied().unwrap_or(i32::from(red)) as u8;
                let blue = words.get(5).copied().unwrap_or(i32::from(green)) as u8;
                let [w, h] = self.bindings.size;
                self.submit(host,json!({"layers":[{"op":"put","layer":{"id":id,"parent":self.bindings.root,"visible":self.layer_visible.get(&slot).copied().unwrap_or(true),"rect":[0,0,w,h],"paint":{"kind":"solid","color":[red,green,blue,255]},"order":[slot,0]}}]}))?;
                self.active_effects.insert(slot);
                self.previous_effects.insert(slot, 54);
                self.patterns.remove(&slot);
                self.fades.remove(&slot);
                self.greys.remove(&slot);
                self.chara_trans.remove(&slot);
                self.old_lamps.remove(&slot);
                self.buras.remove(&slot);
                self.fill_effects.insert(slot, [red, green, blue]);
                self.passive_counts.insert(slot, (0, 9999));
            }
            (_, 0x33330013, Payload::Words { count, words }) if *count >= 2 && words[1] == 1 => {
                ensure!(
                    *count as usize == words.len() && words.len() <= 258,
                    "MAE native parameter span"
                );
                self.layer_id(words[0])?;
                // Reviewed legacy SetEffectScript0x4c18e0 returns immediately
                // for native1: no SetParam, count reset or command replacement.
                // This is the explicit keep-current operation, not an unknown ACK.
            }
            (_, 0x33330013, Payload::Words { count, words }) if *count >= 2 && words[1] == 74 => {
                ensure!(*count as usize == words.len(), "BURA payload count");
                let slot = words[0];
                self.layer_id(slot)?;
                let state = crate::native_bura::BuraState::parse(&words[2..], self.bindings.size)?;
                self.restore_picture_source(slot, host)?;
                self.fill_effects.remove(&slot);
                self.patterns.remove(&slot);
                self.fades.remove(&slot);
                self.greys.remove(&slot);
                self.chara_trans.remove(&slot);
                self.old_lamps.remove(&slot);
                self.passive_counts.remove(&slot);
                self.buras.insert(slot, state);
                self.previous_effects.insert(slot, 74);
            }
            (_, 0x33330013, Payload::Words { count, words }) if *count >= 2 && words[1] == 109 => {
                ensure!(*count as usize == words.len(), "OLDLAMP payload count");
                let slot = words[0];
                self.layer_id(slot)?;
                let state = crate::native_old_lamp::OldLampState::parse(&words[2..])?;
                self.restore_picture_source(slot, host)?;
                self.fill_effects.remove(&slot);
                self.patterns.remove(&slot);
                self.fades.remove(&slot);
                self.greys.remove(&slot);
                self.chara_trans.remove(&slot);
                self.passive_counts.remove(&slot);
                self.buras.remove(&slot);
                self.old_lamps.insert(slot, state);
                self.previous_effects.insert(slot, 109);
            }
            (_, 0x33330013, Payload::Words { count, words }) if *count >= 2 && words[1] == 9 => {
                ensure!(*count as usize == words.len(), "CTRANS payload count");
                let slot = words[0];
                self.layer_id(slot)?;
                let state = crate::native_chara_trans::CharaTransState::parse(
                    &words[2..],
                    self.bindings.size,
                )?;
                self.restore_picture_source(slot, host)?;
                self.fill_effects.remove(&slot);
                self.patterns.remove(&slot);
                self.fades.remove(&slot);
                self.greys.remove(&slot);
                self.passive_counts.remove(&slot);
                self.old_lamps.remove(&slot);
                self.buras.remove(&slot);
                self.chara_trans.insert(slot, state);
                self.previous_effects.insert(slot, 9);
            }
            (_, 0x33330013, Payload::Words { count, words }) if *count >= 2 && words[1] == 129 => {
                let slot = words[0];
                self.layer_id(slot)?;
                let grey = crate::native_effects::LayerGreyState::parse(&words[2..])?;
                grey.linear_image_effect()?;
                self.restore_picture_source(slot, host)?;
                self.fill_effects.remove(&slot);
                self.patterns.remove(&slot);
                self.fades.remove(&slot);
                self.passive_counts.remove(&slot);
                self.chara_trans.remove(&slot);
                self.old_lamps.remove(&slot);
                self.buras.remove(&slot);
                self.greys.insert(slot, grey);
                self.previous_effects.insert(slot, 129);
            }
            (_, 0x33330013, Payload::Words { count, words })
                if *count >= 2 && matches!(words[1], 10 | 11) =>
            {
                let slot = words[0];
                self.layer_id(slot)?;
                let fade = if words[1] == 11 {
                    FadeInState::parse_out(&words[2..])?
                } else {
                    FadeInState::parse(&words[2..])?
                };
                self.restore_picture_source(slot, host)?;
                self.fill_effects.remove(&slot);
                self.patterns.remove(&slot);
                self.passive_counts.remove(&slot);
                self.greys.remove(&slot);
                self.chara_trans.remove(&slot);
                self.old_lamps.remove(&slot);
                self.buras.remove(&slot);
                self.fades.insert(slot, fade);
                self.previous_effects.insert(slot, words[1]);
            }
            (_, 0x33330013, Payload::Words { count, words }) if *count >= 2 && words[1] == 0 => {
                let slot = words[0];
                self.layer_id(slot)?;
                self.restore_picture_source(slot, host)?;
                self.fill_effects.remove(&slot);
                self.patterns.remove(&slot);
                self.fades.remove(&slot);
                self.greys.remove(&slot);
                self.chara_trans.remove(&slot);
                self.old_lamps.remove(&slot);
                self.buras.remove(&slot);
                self.passive_counts.insert(slot, (0, 99999));
                self.previous_effects.insert(slot, 0);
            }
            _ => bail!(
                "unsupported System-NNN native pc={:#x} code={:#x}; exact pending call retained",
                call.pc,
                call.code
            ),
        }
        machine.acknowledge(call)
    }
}
