// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! docs/api is generated from the live registry and specs. This test fails
//! when the checked-in reference is stale or an API signature change has no
//! CHANGES.md entry. Regenerate with
//! `CRYSIRROR_BLESS=1 cargo test -p crysirror-engine --test api_docs`.
use crysirror_engine::{
    api_doc::{self, sample, ApiInput, DataType},
    builtin,
    object::ClassRegistry,
    services::COMMAND_API,
};
use crysirror_scene::{canvas, components as c, layer, material, physics, ui, view, world};
use crysirror_vfx as vfx;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}
fn variants<T: DeserializeOwned>() -> Vec<String> {
    let err = serde_json::from_value::<T>(json!({"op": "__probe__"}))
        .err()
        .expect("probe accepted")
        .to_string();
    err.split("expected one of ")
        .nth(1)
        .expect("serde variant list")
        .split(',')
        .map(|s| s.trim().trim_matches('`').to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}
fn d(name: &'static str, used_by: &'static str, sample: Option<serde_json::Value>) -> DataType {
    DataType {
        name,
        used_by,
        sample,
    }
}

// Public reference aliases distinguish modular VFX declarations from older
// scene/puppet declarations. Rust and serialized field/variant names stay intact.
const VFX_ALIASES: &[(&str, &str)] = &[
    ("ParameterValue", "VfxParameterValue"),
    ("ScalarValue", "VfxScalarValue"),
    ("ScalarBinding", "VfxScalarBinding"),
    ("VectorValue", "VfxVectorValue"),
    ("VectorBinding", "VfxVectorBinding"),
    ("Interpolation", "VfxInterpolation"),
    ("ScalarKey", "VfxScalarKey"),
    ("Vec2Key", "VfxVec2Key"),
    ("ColorKey", "VfxColorKey"),
    ("ScalarCurve", "VfxScalarCurve"),
    ("Vec2Curve", "VfxVec2Curve"),
    ("ColorCurve", "VfxColorCurve"),
    ("SimulationTarget", "VfxSimulationTarget"),
    ("SimulationSpace", "VfxSimulationSpace"),
    ("OverflowPolicy", "VfxOverflowPolicy"),
    ("SpawnShape", "VfxSpawnShape"),
    ("Burst", "VfxBurst"),
    ("SpawnSpec", "VfxSpawnSpec"),
    ("InitSpec", "VfxInitSpec"),
    ("Module", "VfxModule"),
    ("BlendMode", "VfxBlendMode"),
    ("FacingMode", "VfxFacingMode"),
    ("MeshShape", "VfxMeshShape"),
    ("RenderSpec", "VfxRenderSpec"),
    ("EventKind", "VfxEventKind"),
    ("EventHandler", "VfxEventHandler"),
    ("EmitterSpec", "VfxEmitterSpec"),
    ("SystemSpec", "VfxSystemSpec"),
    ("ScreenSpriteTarget", "VfxScreenSpriteTarget"),
    ("ScreenSpriteOptions", "VfxScreenSpriteOptions"),
    ("SimulationClock", "VfxSimulationClock"),
    ("SequenceRng", "VfxSequenceRng"),
    ("SequenceExchange", "VfxSequenceExchange"),
    ("FallingAtlasSpec", "VfxFallingAtlasSpec"),
    ("SystemSequenceState", "VfxSystemSequenceState"),
    ("Particle", "VfxParticle"),
    ("ParticleEvent", "VfxParticleEvent"),
    ("VfxStats", "VfxStats"),
    ("WorldBudget", "VfxWorldBudget"),
];

fn vfx_type_aliases(ty: &str) -> String {
    let mut result = String::new();
    let mut word = String::new();
    let append = |word: &mut String, result: &mut String| {
        if word.is_empty() {
            return;
        }
        if word == "ParameterMap" {
            result.push_str("BTreeMap<String, VfxParameterValue>");
        } else {
            result.push_str(
                VFX_ALIASES
                    .iter()
                    .find(|(original, _)| *original == word)
                    .map_or(word.as_str(), |(_, alias)| *alias),
            );
        }
        word.clear();
    };
    for c in ty.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            append(&mut word, &mut result);
            result.push(c);
        }
    }
    append(&mut word, &mut result);
    result
}

fn vfx_doc_items() -> Vec<api_doc::RustItem> {
    let mut result = Vec::new();
    for file in ["schema", "simulation", "screen", "falling_atlas"] {
        let source = read(&format!("crates/crysirror-vfx/src/{file}.rs"));
        // Preserve serde's multiline tag/rename metadata for the deliberately
        // small generic parser, which normally sees single-line attributes.
        let mut normalized = String::new();
        let mut attribute = false;
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("#[") {
                attribute = true;
            }
            normalized.push_str(if attribute { trimmed } else { line });
            if !attribute || trimmed.ends_with(']') {
                attribute = false;
                normalized.push('\n');
            }
        }
        for mut item in api_doc::parse_rust_items(&normalized) {
            let Some((_, alias)) = VFX_ALIASES.iter().find(|(name, _)| *name == item.name) else {
                continue;
            };
            // The generic reference parser handles struct variants; supplement
            // the three tuple-value enums from their source declarations too.
            if matches!(
                item.name.as_str(),
                "ParameterValue" | "ScalarValue" | "VectorValue"
            ) {
                let declaration = source
                    .split_once(&format!("pub enum {} {{", item.name))
                    .expect("VFX enum declaration")
                    .1
                    .split_once('\n')
                    .unwrap()
                    .1
                    .split_once("\n}")
                    .unwrap()
                    .0;
                for (member, line) in item.members.iter_mut().zip(declaration.lines()) {
                    let (variant, payload) = line.trim().split_once('(').expect("tuple variant");
                    let payload = payload.trim_end_matches(',').trim_end_matches(')');
                    member.ty = if item.name == "ParameterValue" {
                        format!("value: {payload}")
                    } else {
                        payload.to_owned()
                    };
                    member.doc = if item.name == "ParameterValue" {
                        "Payload is serialized under `value`; `kind` selects this variant.".into()
                    } else if variant == "Constant" {
                        "Serialized as a bare number or vector, without a variant tag.".into()
                    } else {
                        "Serialized as the binding object, without a variant tag.".into()
                    };
                }
            }
            item.name = (*alias).to_owned();
            for member in &mut item.members {
                member.ty = vfx_type_aliases(&member.ty.replace("crate::screen::", "").replace("crate::falling_atlas::", ""));
            }
            result.push(item);
        }
    }
    result
}

fn generate() -> api_doc::ApiDocs {
    let mut classes = ClassRegistry::default();
    builtin::register(&mut classes).unwrap();
    let mut items = Vec::new();
    for f in [
        "crates/crysirror-scene/src/view.rs",
        "crates/crysirror-scene/src/components.rs",
        "crates/crysirror-scene/src/world.rs",
        "crates/crysirror-scene/src/layer.rs",
        "crates/crysirror-scene/src/layer/motion.rs",
        "crates/crysirror-scene/src/layer/crossfade.rs",
        "crates/crysirror-scene/src/layer/crossfade_effect.rs",
        "crates/crysirror-scene/src/canvas.rs",
        "crates/crysirror-core/src/service.rs",
        "crates/crysirror-core/src/service/font.rs",
        "crates/crysirror-image/src/associated.rs",
        "crates/crysirror-image/src/effects.rs",
        "crates/crysirror-image/src/sprite_alpha.rs",
        "crates/crysirror-scene/src/sprite.rs",
        "crates/crysirror-image/src/vector.rs",
        "crates/crysirror-image/src/transition.rs",
        "crates/crysirror-image/src/row_copy.rs",
        "crates/crysirror-audio/src/synth/mod.rs",
        "crates/crysirror-image/src/bitmap.rs",
        "crates/crysirror-resource/src/context.rs",
        "crates/crysirror-resource/src/transforms.rs",
        "crates/crysirror-resource/src/file.rs",
        "crates/crysirror-resource/src/lzss.rs",
        "crates/crysirror-resource/src/wave.rs",
        "crates/crysirror-resource/src/pe.rs",
        "crates/crysirror-resource/src/pe_menu.rs",
        "crates/crysirror-siglus-vm/src/message.rs",
        "crates/crysirror-runtime/src/media.rs",
        "crates/crysirror-runtime/src/assets_async.rs",
        "crates/crysirror-tasks/src/lib.rs",
        "crates/crysirror-tasks/src/table.rs",
        "crates/crysirror-engine/src/audio_resources_api.rs",
        "crates/crysirror-engine/src/audio_sequences_api.rs",
        "crates/crysirror-core/src/service/audio_sequence.rs",
        "crates/crysirror-core/src/input_buffer.rs",
        "crates/crysirror-audio/src/scene.rs",
        "crates/crysirror-audio/src/buffered.rs",
        "crates/crysirror-audio/src/decoder.rs",
        "crates/crysirror-audio/src/mixer.rs",
        "crates/crysirror-audio/src/doppler.rs",
        "crates/crysirror-audio/src/motion.rs",
        "crates/crysirror-audio/src/reverb.rs",
        "crates/crysirror-acoustics/src/reverb.rs",
        "crates/crysirror-acoustics/src/reflections.rs",
        "crates/crysirror-acoustics/src/schema.rs",
        "crates/crysirror-acoustics/src/geometry.rs",
        "crates/crysirror-engine/src/acoustics_api/instances.rs",
        "crates/crysirror-acoustics/src/binaural.rs",
        "crates/crysirror-acoustics/src/direct.rs",
        "crates/crysirror-scene/src/ui/menu.rs",
        "crates/crysirror-scene/src/ui.rs",
        "crates/crysirror-scene/src/ui/hit_mask.rs",
        "crates/crysirror-scene/src/material.rs",
        "crates/crysirror-scene/src/sequence.rs",
        "crates/crysirror-scene/src/physics.rs",
        "crates/crysirror-scene/src/lib.rs",
        "crates/crysirror-puppet/src/lib.rs",
    ] {
        items.extend(api_doc::parse_rust_items(&read(f)));
    }
    for item in &mut items {
        for member in &mut item.members {
            member.ty = member
                .ty
                .replace("crysirror_vfx::SystemSpec", "VfxSystemSpec");
        }
    }
    items.extend(vfx_doc_items());
    let config = |f: &str| {
        api_doc::parse_rust_items(&read(f))
            .into_iter()
            .find(|i| i.name == "Config")
    };
    let e = json!({});
    let extent = json!({"extent": [1, 1, 1]});
    let data = vec![
        d("ImageEffect","Bitmap.applyEffectAsync; shared worker effects",None),
        d("SpriteAlpha","ImageEffect.sprite_alpha.spec; reusable sprite alpha normalization",None),
        d("StripeReveal","ImageEffect.stripe_reveal.spec; reusable stripe/stagger threshold masks",None),
        d("SheetSequence","Rust shared sprite sheet timing and UV sequence",None),
        d("SheetLoop","SheetSequence.mode",None),
        d("WidgetInteraction","HostServices EngineQuery::UiInteraction; pointer/focus state and current-tick activation serial",None),
        d("VectorPath","ImageEffect.vector.path",None),
        d("PathCommand","VectorPath.commands",None),
        d("PathStroke","VectorPath.stroke",None),
        d("ImageTransition","Bitmap.transitionAsync options",None),
        d("SineRowCopy","Bitmap.rowCopyAsync options / native copyRaster",None),
        d("RegisterScore","Audio.prepareSourceAsync .opna assets",None),
        d("RegisterWrite","RegisterScore.writes (output sample clock)",None),
        d("SimpleTextPlacement","Text.positionSimple / TextMeasurement::position_simple",None),
        d("WindowPolicy","Ui.setWindowPolicy / UiSystem.window_policy",None),
        d("InputSample","Input.events samples; journal JSON u64 fields are decimal strings",None),
        d("InputReceipt","Input.events receipt / ackEvents; JSON epoch/sequence are decimal strings",None),
        d("InputBufferSnapshot","Input.captureEvents / restoreEvents; schema 1, JSON u64 fields are decimal strings",None),
        d("InputFrame","InputSample.frame; journal wire uses decimal strings for masks",None),
        d("CueSequenceCursor","Audio.captureCueSequence / restoreCueSequence",None),
        d("AudioSequenceCursor","HostServices::capture_audio_sequence / restore_audio_sequence",None),
        d("CueSequenceSpec","Audio.createCueSequence",sample::<crysirror_engine::CueSequenceSpec>(e.clone())),
        d("CueSequenceLine","CueSequenceSpec.lines",sample::<crysirror_engine::CueSequenceLine>(json!({"key":"cue"}))),
        d("CueSequenceMode","CueSequenceSpec.mode",None),
        d("CueSequenceClock","CueSequenceSpec.clock",None),
        d("CueSequenceTiming","CueSequenceSpec.timing",None),
        d("CueSequenceRandom","CueSequenceSpec.random",None),
        d("AudioSequenceHandle","Audio cue sequence operations / HostServices",None),
        d("AudioSequenceEmission","Audio.pollCueSequence",None),
        d("AudioSequenceInfo","Audio.cueSequence",None),
        d("DopplerSettings","Spatial.doppler",sample::<crysirror_audio::doppler::DopplerSettings>(e.clone())),
        d("DopplerCalculation","DopplerInfo.target / Rust doppler::calculate",None),
        d("DopplerInfo","Audio.dopplerVoice / Rust Mixer::doppler_info",None),
        d("MotionSettings","Audio.setMotionSettings / motionSettings",sample::<crysirror_audio::motion::MotionSettings>(e.clone())),
        d("MotionEstimate","Audio.dopplerVoice motion source/listener",None),
        d("MotionStatus","MotionEstimate.status",None),
        d("Spatial","VoiceSpec.spatial / Audio.playSfx.spatial",sample::<crysirror_audio::mixer::Spatial>(e.clone())),
        d("VoiceSpec","Audio.playSfx / Rust Mixer::play",sample::<crysirror_audio::mixer::VoiceSpec>(e.clone())),
        d("AudioResourceHandle","Audio.prepareSourceAsync / beginPlaySource / HostServices",None),
        d("AudioSourcePrepareSpec","Audio.beginPrepareSource / prepareSourceAsync options",sample::<crysirror_engine::AudioSourcePrepareSpec>(e.clone())),
        d("AudioTransformSpec","AudioSourcePrepareSpec.transform",sample::<crysirror_engine::AudioTransformSpec>(json!({"kind":"siglus_jitan","percent":200}))),
        d("AudioResourceTransform","Rust HostServices transformed resource preparation",None),
        d("AudioResourceInfo","Audio.sourceInfo / sourcePreparation / HostServices",None),
        d("AudioResourcePreparation","Rust HostServices.audio_resource_preparation",None),
        d("AudioMixGroupSettings","Audio.setMixGroup / Mixer / HostServices audio requests",None),
        d("AudioMixGroupInfo","Audio.mixGroup / nativeMixGroup / HostServices.audio_mix_group",None),
        d("AudioParameters","Rust HostServices/ChannelAudio playback parameters",None),
        d("PcmBus","AudioParameters.bus",None),
        d("AudioResumeSettings","Rust AudioRequest::ResumeWithOptions / Mixer::schedule_resume_with_settings",None),
        d("AudioResumePolicy","Rust AudioResumeSettings.policy / Mixer::schedule_resume_with_policy",None),
        d("AudioPlaybackInfo","Audio.playbackInfo / nativeChannelInfo / HostServices.audio_info",None),
        d("ReflectionTrackingSpec","Audio.trackVoiceReflections / playSfx.reflections",sample::<crysirror_acoustics::reflections::ReflectionTrackingSpec>(json!({}))),
        d("ReflectionOptions","Audio.beginVoiceReflections options / ReflectionQuery.options",sample::<crysirror_acoustics::reflections::ReflectionOptions>(json!({}))),
        d("ReflectionQuery","Audio.beginReflectionProbe",sample::<crysirror_acoustics::reflections::ReflectionQuery>(json!({"source":[0,0,2],"listener":[0,0,0]}))),
        d("ReflectionPath","ReflectionPaths.paths",None),
        d("ReflectionPaths","Audio.reflectionStatus / reflectionsVoice",None),
        d("ReverbSpec","ReverbSettings.room",sample::<crysirror_acoustics::reverb::ReverbSpec>(json!({}))),
        d("ReverbSettings","Audio.beginReverb",sample::<crysirror_audio::reverb::ReverbSettings>(json!({}))),
        d("ReverbSend","ReverbSettings.sends",sample::<crysirror_audio::reverb::ReverbSend>(json!({"bus":"sfx","gain":0.2}))),
        d("SceneVoicePhase","SceneVoiceStatus.phase",None),
        d("SceneVoiceStatus","Audio.channelStatus",None),
        d("AudioAsset","HostServices::begin_audio / AudioRequest::PlayAsset",None),
        d("AudioPreparePhase","AudioPreparation.phase",None),
        d("AudioPreparation","HostServices::audio_preparation",None),
        d("PcmAudioInfo","PcmAudio::info / AudioRequest::PlayPcm (Rust)",None),
        d("StreamBufferSpec","Rust BufferedSource::prepare; engine music uses defaults",sample::<crysirror_audio::buffered::StreamBufferSpec>(json!({}))),
        d("SourceBufferStatus","Audio.sourceStatus",None),
        d("SeekProgress","Rust Mixer::seek_progress",None),
        d("HrtfEarOrder","HrtfOptions.ear_order",Some(json!("left_right"))),
        d("HrtfOptions","Audio.beginHrtf options",sample::<crysirror_acoustics::binaural::HrtfOptions>(json!({}))),
        d("HrirAsset","Audio.beginHrtf JSON asset",sample::<crysirror_acoustics::binaural::HrirAsset>(json!({"sample_rate":48000,"measurements":[{"direction":[0,0,-1],"left":[1],"right":[1]}]}))),
        d("HrirMeasurement","HrirAsset.measurements",sample::<crysirror_acoustics::binaural::HrirMeasurement>(json!({"direction":[0,0,-1],"left":[1],"right":[1]}))),
        d("AcousticBackend","AcousticSceneSpec.backend",Some(json!("parry"))),
        d("AcousticMeshUpdate","Audio.beginUpdateAcousticScene / updateAcousticsAsync",sample::<crysirror_acoustics::AcousticMeshUpdate>(json!({"id":7,"enabled":false}))),
        d("AcousticGeometryStats","Audio.acousticStats scene.geometry / scene commit result",None),
        d("AcousticMeshBindingSpec","Audio.bindAcousticMesh (scripts also accept an Entity object)",sample::<crysirror_engine::AcousticMeshBindingSpec>(json!({"mesh":7,"entity":{"client":1,"id":20,"generation":1}}))),
        d("AcousticSceneSpec","Audio.beginAcousticScene asset JSON",sample::<crysirror_acoustics::AcousticSceneSpec>(json!({}))),
        d("AcousticMesh","AcousticSceneSpec.meshes",sample::<crysirror_acoustics::AcousticMesh>(json!({"id":1,"vertices":[],"triangles":[]}))),
        d("AcousticMaterial","AcousticSceneSpec.materials",sample::<crysirror_acoustics::AcousticMaterial>(json!({}))),
        d("AcousticSource","Audio.playSfx acoustics and AcousticQuery.settings",sample::<crysirror_acoustics::AcousticSource>(json!({}))),
        d("AcousticQuery","Audio.beginAcousticProbe",sample::<crysirror_acoustics::AcousticQuery>(json!({"source":[0,0,0],"listener":[1,0,0]}))),
        d("DirectPath","Audio.acousticStatus result and Audio.acousticVoice",None),

        d(
            "DecodeLimits",
            "Resources.defineProfile limits",
            sample::<crysirror_resource::DecodeLimits>(e.clone()),
        ),
        d(
            "TransformSpec",
            "Resources.defineProfile rules and native format pipelines",
            sample::<crysirror_resource::TransformSpec>(json!({"algorithm":"identity"})),
        ),
        d("LzssMode", "LzssParams.mode", Some(json!("bytewise"))),
        d(
            "LzssParams",
            "TransformSpec parameters for lzss",
            sample::<crysirror_resource::LzssParams>(e.clone()),
        ),
        d(
            "FileKind",
            "Resources file/audio decoder registry",
            Some(json!("file")),
        ),
        d(
            "WaveLoop",
            "WaveSpec.loops; native sample positions",
            sample::<crysirror_resource::WaveLoop>(json!({"start":0,"end":1,"play_count":0})),
        ),
        d(
            "WaveSpec",
            "Shared native resource WAVE serialization",
            sample::<crysirror_resource::WaveSpec>(e.clone()),
        ),
        d(
            "BitmapRect",
            "Bitmap.blit and Bitmap.stretch source/destination rectangles",
            Some(json!({"x":0,"y":0,"width":1,"height":1})),
        ),
        d(
            "PixelMath",
            "Bitmap.blit and Bitmap.stretch mode",
            Some(json!("copy")),
        ),
        d(
            "Sampling",
            "Bitmap.stretch sampling",
            Some(json!("nearest")),
        ),
        d(
            "BlitOptions",
            "Bitmap.blit options",
            sample::<crysirror_image::bitmap::BlitOptions>(
                json!({"source_rect":{"x":0,"y":0,"width":1,"height":1},"destination":[0,0]}),
            ),
        ),
        d(
            "StretchOptions",
            "Bitmap.stretch options",
            sample::<crysirror_image::bitmap::StretchOptions>(
                json!({"source_rect":{"x":0,"y":0,"width":1,"height":1},"destination":{"x":0,"y":0,"width":1,"height":1}}),
            ),
        ),
        d(
            "MessageWindow",
            "Siglus `Config.message_window` (ADR 0061)",
            Some(
                json!({"font":"default","rect":[20,340,600,120],"name_rect":[20,308,600,28],"size":24,"name_size":20,"spacing":[0,2],"color":[255,255,255,255],"name_color":[255,255,255,255]}),
            ),
        ),
        d(
            "AssociatedBlend",
            "`Bitmap.composite` options.blend",
            Some(json!("over")),
        ),
        d(
            "CoverageWrite",
            "`Bitmap.composite` options.coverage_write",
            Some(json!("preserve")),
        ),
        d(
            "AssociatedBlitOptions",
            "`Bitmap.composite` options",
            Some(
                json!({"source_rect":{"x":0,"y":0,"width":1,"height":1},"destination":[0,0],"blend":"over","weight":256,"coverage_write":"preserve"}),
            ),
        ),
        d(
            "RgbFilter",
            "`Bitmap.filterRgb` filter",
            Some(json!({"kind":"fade","rgb":[0,0,0],"weight":256})),
        ),
        d(
            "AssetKind",
            "`Assets.load(path, kind)`",
            Some(json!("model")),
        ),
        d("AssetState", "`Assets.status(id)`", None),
        d(
            "AssetTicket",
            "Rust async asset handle; store/client checked",
            None,
        ),
        d("AssetStatus", "`Assets.status(id)`", None),
        d("TaskStats", "`Assets.stats()` CPU and I/O pools", None),
        d(
            "Environment",
            "`World.setEnvironment`, `world.set_environment`",
            sample::<view::Environment>(e.clone()),
        ),
        d("Sun", "`Environment.sun`", sample::<view::Sun>(e.clone())),
        d("Fog", "`Environment.fog`", sample::<view::Fog>(e.clone())),
        d(
            "AutoExposure",
            "`Environment.auto_exposure`",
            sample::<view::AutoExposure>(e.clone()),
        ),
        d(
            "ScreenSpaceReflections",
            "`Environment.reflections` (ADR 0023)",
            sample::<view::ScreenSpaceReflections>(e.clone()),
        ),
        d(
            "RayTracedGi",
            "`Environment.ray_traced_gi` (ADR 0060)",
            sample::<view::RayTracedGi>(e.clone()),
        ),
        d(
            "GeometryLod",
            "`Environment.geometry_lod` (ADR 0026)",
            sample::<view::GeometryLod>(e.clone()),
        ),
        d(
            "VirtualShadows",
            "`Environment.virtual_shadows` (ADR 0027)",
            sample::<view::VirtualShadows>(e.clone()),
        ),
        d("Tonemap", "`Environment.tonemap`", None),
        d(
            "EntitySpec",
            "`World.spawn`, `ClientSpec.systems.entities`",
            sample::<world::EntitySpec>(json!({"id": 1, "name": "e"})),
        ),
        d(
            "Transform",
            "`EntitySpec.transform`",
            sample::<world::Transform>(e.clone()),
        ),
        d(
            "Material",
            "`EntitySpec.material`, `Entity.setMaterial`",
            sample::<material::MaterialSpec>(e.clone()),
        ),
        d("AlphaMode", "`Material.alpha_mode`", None),
        d("NormalEncoding", "`Material.normal_encoding`", None),
        d(
            "LightSpec",
            "`EntitySpec.light`, `Entity.setLight` (ADR 0018)",
            sample::<c::LightSpec>(e.clone()),
        ),
        d("LightShape", "`LightSpec.shape`", None),
        d("LightUnits", "`LightSpec.units`", None),
        d("Falloff", "`LightSpec.falloff`", None),
        d(
            "ReflectionProbeSpec",
            "`EntitySpec.reflection_probe`, `Entity.setReflectionProbe` (ADR 0024/0025)",
            sample::<c::ReflectionProbeSpec>(extent.clone()),
        ),
        d(
            "IrradianceVolumeSpec",
            "`EntitySpec.irradiance_volume`, `Entity.setIrradianceVolume` (ADR 0028)",
            sample::<c::IrradianceVolumeSpec>(extent.clone()),
        ),
        d(
            "ColliderSpec",
            "`EntitySpec.collider`, `Entity.setCollider` (ADR 0012)",
            sample::<c::ColliderSpec>(json!({"shape": {"kind": "sphere", "radius": 1}})),
        ),
        d("ColliderShape", "`ColliderSpec.shape`", None),
        d("BodyKind", "`ColliderSpec.body`", None),
        d(
            "CollisionGroups",
            "`ColliderSpec.groups`",
            sample::<c::CollisionGroups>(e.clone()),
        ),
        d(
            "SystemsSpec",
            "`ClientSpec.systems`",
            sample::<crysirror_scene::SystemsSpec>(e.clone()),
        ),
        d(
            "PhysicsScene",
            "World snapshot physics declarations (ADR 0045)",
            sample::<physics::PhysicsScene>(e.clone()),
        ),
        d(
            "ClientPhysicsScene",
            "`ClientSpec.systems.physics`, `Physics.gravity`, `WorldCommand.set_physics`",
            sample::<physics::ClientPhysicsScene>(e.clone()),
        ),
        d(
            "BodyForces",
            "`ClientPhysicsScene.forces`",
            sample::<physics::BodyForces>(json!({"generation":1})),
        ),
        d(
            "KinematicTarget",
            "`ClientPhysicsScene.kinematic_targets`",
            sample::<physics::KinematicTarget>(json!({"generation":1,"pose":{}})),
        ),
        d(
            "JointFrame",
            "`JointSpec` local frames and `KinematicTarget.pose`",
            sample::<physics::JointFrame>(e.clone()),
        ),
        d(
            "JointMotor",
            "`JointSpec.motor`",
            sample::<physics::JointMotor>(e.clone()),
        ),
        d(
            "JointSpec",
            "`Physics.createJoint`, `PhysicsJoint.setSpec`",
            None,
        ),
        d(
            "JointHandle",
            "Client-scoped generational physics joint identity",
            None,
        ),
        d("JointState", "`ClientPhysicsScene.joints`", None),
        d(
            "EmitterSpec",
            "`EntitySpec.particles`, `Entity.setParticles` (ADR 0014)",
            sample::<c::EmitterSpec>(e.clone()),
        ),
        d(
            "VfxSystemSpec",
            "`EntitySpec.vfx`, `Vfx.compile/play`, `Entity.setVfx` (ADR 0064)",
            sample::<vfx::SystemSpec>(e.clone()),
        ),
        d(
            "VfxEmitterSpec",
            "`VfxSystemSpec.emitters`; distinct from legacy `EmitterSpec`",
            sample::<vfx::EmitterSpec>(e.clone()),
        ),
        d(
            "VfxScreenSpriteTarget",
            "`VfxEmitterSpec.screen`; owned CPU sprite publication into a retained pixel-plane mesh layer",
            None,
        ),
        d(
            "VfxScreenSpriteOptions",
            "Rust screen-sprite packer canvas and anchor options",
            None,
        ),
        d("VfxSimulationClock", "`VfxSystemSpec.clock`; separate engine or explicit tick/wall-time sources", None),
        d("VfxSequenceRng", "`VfxSystemSpec.sequence_rng`; explicit serializable integer stream", None),
        d("VfxSequenceExchange", "`VfxSystemSpec.sequence_exchange`; fenced stream replacement acknowledged by workers", None),
        d("VfxFallingAtlasSpec", "`VfxEmitterSpec.falling_atlas`; bounded CPU pixel-motion and atlas node", None),
        d("VfxSystemSequenceState", "Worker-completed state queried through `Vfx.sequenceStatus`", None),
        d(
            "VfxParameterValue",
            "`VfxSystemSpec.parameters` typed values; `Vfx.setParameter` accepts the existing type's raw payload",
            None,
        ),
        d(
            "VfxScalarValue",
            "Emission rates and scalar `VfxModule` inputs",
            None,
        ),
        d(
            "VfxScalarBinding",
            "`VfxScalarValue` parameter binding object",
            sample::<vfx::ScalarBinding>(json!({"parameter":"strength"})),
        ),
        d(
            "VfxVectorValue",
            "Direction, force, position and axis inputs",
            None,
        ),
        d(
            "VfxVectorBinding",
            "`VfxVectorValue` parameter binding object",
            sample::<vfx::VectorBinding>(json!({"parameter":"direction"})),
        ),
        d("VfxSimulationTarget", "`VfxEmitterSpec.target`", None),
        d("VfxSimulationSpace", "`VfxEmitterSpec.space`", None),
        d(
            "VfxOverflowPolicy",
            "`VfxEmitterSpec.overflow`; GPU supports only drop_new",
            None,
        ),
        d(
            "VfxSpawnSpec",
            "`VfxEmitterSpec.spawn`",
            sample::<vfx::SpawnSpec>(e.clone()),
        ),
        d(
            "VfxBurst",
            "`VfxSpawnSpec.bursts`",
            sample::<vfx::Burst>(json!({"time":0,"count":1})),
        ),
        d(
            "VfxInitSpec",
            "`VfxEmitterSpec.initialize`; sizes are full world width/height",
            sample::<vfx::InitSpec>(e.clone()),
        ),
        d("VfxSpawnShape", "`VfxInitSpec.shape`", None),
        d(
            "VfxModule",
            "`VfxEmitterSpec.update` ordered typed module stack",
            None,
        ),
        d("VfxInterpolation", "Lifetime-curve interpolation", None),
        d(
            "VfxScalarKey",
            "`VfxScalarCurve.keys`",
            sample::<vfx::ScalarKey>(json!({"time":0,"value":1})),
        ),
        d(
            "VfxVec2Key",
            "`VfxVec2Curve.keys`",
            sample::<vfx::Vec2Key>(json!({"time":0,"value":[1,1]})),
        ),
        d(
            "VfxColorKey",
            "`VfxColorCurve.keys`",
            sample::<vfx::ColorKey>(json!({"time":0,"value":[1,1,1,1]})),
        ),
        d(
            "VfxScalarCurve",
            "Scalar lifetime curves",
            sample::<vfx::ScalarCurve>(json!({"keys":[{"time":0,"value":1},{"time":1,"value":0}]})),
        ),
        d(
            "VfxVec2Curve",
            "`VfxModule.size_over_life.curve`",
            sample::<vfx::Vec2Curve>(
                json!({"keys":[{"time":0,"value":[1,1]},{"time":1,"value":[0,0]}]}),
            ),
        ),
        d(
            "VfxColorCurve",
            "`VfxModule.color_over_life.curve`",
            sample::<vfx::ColorCurve>(
                json!({"keys":[{"time":0,"value":[1,1,1,1]},{"time":1,"value":[1,1,1,0]}]}),
            ),
        ),
        d(
            "VfxRenderSpec",
            "`VfxEmitterSpec.render`; actual Sprite, Mesh or CPU Ribbon renderer",
            None,
        ),
        d("VfxBlendMode", "`VfxRenderSpec.blend`", None),
        d("VfxFacingMode", "`VfxRenderSpec.sprite.facing`", None),
        d(
            "VfxMeshShape",
            "`VfxRenderSpec.mesh.mesh`; static geometry only",
            None,
        ),
        d(
            "VfxEventKind",
            "`VfxEventHandler.event`, `VfxParticleEvent.kind`",
            None,
        ),
        d(
            "VfxEventHandler",
            "`VfxEmitterSpec.events` CPU sub-emitter handlers",
            sample::<vfx::EventHandler>(json!({"event":"birth","emitter":"child","count":1})),
        ),
        d(
            "VfxParticle",
            "Device-independent Rust CPU snapshot attributes",
            None,
        ),
        d(
            "VfxParticleEvent",
            "`Vfx.drainEvents`; client-scoped CPU event records",
            None,
        ),
        d(
            "VfxStats",
            "`VfxRuntime::inspect(None).simulation`; Rust CPU diagnostics, omitted from client-scoped `Vfx.stats`",
            sample::<vfx::VfxStats>(serde_json::to_value(vfx::VfxStats::default()).unwrap()),
        ),
        d(
            "VfxWorldBudget",
            "Rust `CpuWorld::new`; checked CPU reservation/queue limits",
            sample::<vfx::WorldBudget>(e.clone()),
        ),
        d(
            "LayerSpec",
            "`Layers.put`, `ClientSpec.systems.layers`",
            None,
        ),
        d("Paint", "`LayerSpec.paint`", None),
        d("SubtreeTransitionEffect", "`LayerCommand.begin_crossfade_effect.effect`; reusable isolated subtree effects", None),
        d("RuleWipe", "Shared subtree rule image and soft-edge levels", None),
        d("ImageSampling", "`LayerSpec.sampling`; independently addressed image axes", sample::<layer::ImageSampling>(json!({"address_u":"repeat","address_v":"clamp_to_edge"}))),
        d("TextureAddressMode", "`ImageSampling.address_u` and `address_v`", None),
        d(
            "PositionedGlyph",
            "`Paint.glyph_run.glyphs`",
            sample::<layer::PositionedGlyph>(json!({"character":"あ","baseline":[0,20]})),
        ),
        d("GlyphRevealUnit", "`GlyphReveal.unit`", None),
        d(
            "GlyphReveal",
            "`Paint.glyph_run.reveal`",
            sample::<layer::GlyphReveal>(json!({"interval_ms":40})),
        ),
        d("GlyphRevealStatus", "Engine glyph reveal query", None),
        d("GlyphBlur", "`Paint.glyph_run.blur`; shared coverage resource", sample::<layer::GlyphBlur>(json!({"radius":2,"level":256,"kernel":"disk_outline"}))),
        d("GlyphCoverageKernel", "`GlyphBlur.kernel`; defaults to weighted_blur for existing records", None),
        d(
            "CanvasSurface",
            "`Canvases.get`",
            sample::<canvas::CanvasSurface>(json!({"width":640,"height":480,"nodes":[]})),
        ),
        d("CanvasNode", "`CanvasSurface.nodes`", None),
        d(
            "CanvasCommand",
            "`Canvases.apply`, `ServiceBatch.canvases`",
            None,
        ),
        d(
            "ServiceBatch",
            "Engine service mutation batch",
            sample::<crysirror_scene::ServiceBatch>(e.clone()),
        ),
        d(
            "MediaRequest",
            "`HostServices.media_request` (script `Media.play` options use `loop`, `audio`, `gain`)",
            None,
        ),
        d("MediaPhase", "`HostServices.media_info`", None),
        d("MediaInfo", "`HostServices.media_info`", None),
        d(
            "MediaOptions",
            "Rust shared media service options (script uses `loop` instead of `looping`)",
            sample::<crysirror_runtime::media::MediaOptions>(e.clone()),
        ),
        d("MediaState", "`MediaStatus.state`", None),
        d("MediaClock", "`MediaOptions.clock`, `Media.setClock`; engine or sampled external time", None),
        d("MediaStatus", "`Media.status`", None),
        d(
            "LayerFilter",
            "`Layers.setFilter` (ADR 0021)",
            sample::<layer::LayerFilter>(e.clone()),
        ),
        d("Blend", "`LayerSpec.blend`", None),
        d("ToneCurve", "`LayerFilter.tone_curve`", None),
        d("LayerMask", "`LayerFilter.mask`", None),
        d("Distortion", "`LayerFilter.distortion`", None),
        d("TransitionSpec", "`Layers.transition` (ADR 0022)", None),
        d("TransitionEffect", "`TransitionSpec.effect`", None),
        d("Easing", "`TransitionSpec.easing`", None),
        d(
            "WidgetSpec",
            "`Ui.put`, `ClientSpec.systems.widgets`",
            sample::<ui::WidgetSpec>(json!({"id": 1, "rect": [0, 0, 1, 1]})),
        ),
        d("WidgetKind", "`WidgetSpec.kind`", None),
        d("Flow", "`WidgetSpec.flow`", None),
        d("PuppetCommand", "`Puppets.*`, `layer.puppet`", None),
        d(
            "PuppetRig",
            "`Paint.puppet.asset` (engine rig JSON, ADR 0029)",
            None,
        ),
        d("Parameter", "`PuppetRig.parameters`", None),
        d("Deformer", "`PuppetRig.deformers`", None),
        d("DeformerKey", "`Deformer.keyforms[].keys`", None),
        d("Part", "`PuppetRig.parts`", None),
        d("PartKey", "`Part.keyforms[].keys`", None),
        d("PartBlend", "`Part.blend`", None),
        d("Motion", "`PuppetRig.motions`", None),
        d("Curve", "`Motion.curves`", None),
        d("Pendulum", "`PuppetRig.physics`", None),
        d("SequenceSpec", "`World.playSequence` (ADR 0040)", None),
        d("Track", "`SequenceSpec.tracks`", None),
        d("TrackProperty", "`Track.property`", None),
        d(
            "KeyInterpolation",
            "`Track.interpolation`, `CameraTrack.interpolation`",
            None,
        ),
        d("TrackNoise", "`Track.noise`, `CameraTrack.shake`", None),
        d("CameraTrack", "`SequenceSpec.camera`", None),
        d("CameraKey", "`CameraTrack.keys`", None),
        d("SequenceEvent", "`SequenceSpec.events`", None),
        d(
            "WorldCommand",
            "World mutation batches (`ServiceBatch.world`)",
            None,
        ),
        d(
            "LayerCommand",
            "Layer mutation batches (`ServiceBatch.layers`)",
            None,
        ),
        d("UiCommand", "UI mutation batches (`ServiceBatch.ui`)", None),
        d(
            "MenuSpec",
            "UI.setMenu / UiCommand.set_menu",
            sample::<ui::menu::MenuSpec>(
                json!({"id":1,"rect":[0,0,640,24],"font":"default","items":[]}),
            ),
        ),
        d(
            "MenuItemSpec",
            "MenuSpec.items",
            sample::<ui::menu::MenuItemSpec>(json!({"id":2,"label":"File"})),
        ),
        d(
            "OperationStatus",
            "Background Text/Save/Resources operation polling",
            None,
        ),
        d("OperationState", "OperationStatus.state", None),
        d("FontJob", "HostServices.font_library asynchronous registration ticket", None),
        d("FontHandle", "FontLibrary client-owned immutable face lease", None),
        d("FontLoadSource", "FontLoadSpec.source capability path or immutable bytes", None),
        d("FontLoadSpec", "FontLibrary.begin", None),
        d("FontInfo", "FontLoadState.Ready / FontLibrary.resolve", None),
        d("FontLoadState", "FontLibrary.status", None),
        d("MotionProperty", "Layer/UI motion track component", None),
        d("MotionTrackSpec", "Layers.beginMotion / Ui.beginMotion", sample::<layer::motion::MotionTrackSpec>(json!({"property":"x","to":100,"duration":20}))),
        d("MotionTrackStatus", "MotionStatus.tracks", None),
        d("MotionStatus", "Layers.motionStatus / Ui.motionStatus / EngineQuery", None),
        d("WidgetHitMask", "WidgetSpec.hit_mask / Ui.setHitMask", sample::<ui::WidgetHitMask>(json!({"size":[1,1],"coverage":[255],"threshold":1}))),
        d("MenuTemplate", "Shared PE standard menu template parser", None),
        d("MenuTemplateItem", "MenuTemplate.items recursive native menu data", None),
        d("PeMenuResource", "PeData.menu_templates resource identity/language/template", None),
        d("PeResourceName", "PeMenuResource.name native resource identity", None),
        d("UiEvent", "Ui.events and native engine event queries", None),
        d("WidgetId", "UiEvent.widget client/local identity", None),
    ];
    api_doc::generate(&ApiInput {
        classes: &classes,
        globals: builtin::GLOBALS,
        commands: COMMAND_API,
        command_variants: vec![
            ("canvas", variants::<canvas::CanvasCommand>()),
            ("world", variants::<world::WorldCommand>()),
            ("layer", variants::<layer::LayerCommand>()),
            ("ui", variants::<ui::UiCommand>()),
        ],
        command_enums: vec![
            ("canvas", "CanvasCommand"),
            ("world", "WorldCommand"),
            ("layer", "LayerCommand"),
            ("ui", "UiCommand"),
        ],
        data,
        adapters: vec![
            ("avg32".into(), config("crates/crysirror-avg32/src/lib.rs")),
            ("bgi", config("crates/crysirror-bgi/src/lib.rs")),
            ("circus", config("crates/crysirror-circus/src/adapter.rs")),
            ("catsystem2", config("crates/crysirror-catsystem2/src/adapter.rs")),
            ("krkr", config("crates/crysirror-krkr/src/adapter.rs")),
            ("nscripter", config("crates/crysirror-nscripter/src/lib.rs")),
            ("rfvp", config("crates/crysirror-rfvp/src/adapter.rs")),
            (
                "reallive",
                config("crates/crysirror-reallive/src/adapter.rs"),
            ),
            (
                "siglus",
                config("crates/crysirror-siglus-vm/src/adapter.rs"),
            ),
        ],
        items,
    })
}

#[test]
fn api_reference_is_current_and_every_signature_change_is_logged() {
    let docs = generate();
    let dir = root().join("docs/api");
    let files = [
        ("REFERENCE.md", &docs.markdown),
        ("api.json", &docs.json),
        ("crysirror.d.ts", &docs.typescript),
    ];
    if std::env::var_os("CRYSIRROR_BLESS").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        for (f, s) in files {
            std::fs::write(dir.join(f), s).unwrap();
        }
    }
    let mut stale = Vec::new();
    for (f, s) in files {
        if std::fs::read_to_string(dir.join(f)).ok().as_deref() != Some(s.as_str()) {
            stale.push(f);
        }
    }
    assert!(
        stale.is_empty(),
        "docs/api {stale:?} are stale; run CRYSIRROR_BLESS=1 cargo test -p crysirror-engine --test api_docs"
    );
    let changes = std::fs::read_to_string(dir.join("CHANGES.md")).unwrap_or_default();
    assert!(
        changes.contains(&format!("`{}`", docs.fingerprint)),
        "API signature changed: add an entry with fingerprint `{}` to docs/api/CHANGES.md",
        docs.fingerprint
    );
}

#[test]
fn every_class_property_and_method_is_documented() {
    let mut classes = ClassRegistry::default();
    builtin::register(&mut classes).unwrap();
    let missing = api_doc::undocumented(&classes);
    assert!(missing.is_empty(), "undocumented API: {missing:?}");
}

#[test]
fn every_listed_data_type_has_source_docs_and_samples_where_given() {
    let docs = generate();
    assert!(
        !docs.markdown.contains("(no source declaration found)"),
        "a data type or adapter config was not found in source"
    );
    let v: serde_json::Value = serde_json::from_str(&docs.json).unwrap();
    // Specs given a minimal sample must parse it: a renamed field or a new
    // required field shows up here instead of as an empty defaults column.
    for name in [
        "Environment",
        "EntitySpec",
        "IrradianceVolumeSpec",
        "ColliderSpec",
        "WidgetSpec",
        "VfxSystemSpec",
        "VfxEmitterSpec",
        "VfxSpawnSpec",
        "VfxInitSpec",
        "VfxScalarBinding",
        "VfxVectorBinding",
        "VfxScalarCurve",
        "VfxVec2Curve",
        "VfxColorCurve",
        "VfxEventHandler",
        "VfxStats",
        "VfxWorldBudget",
    ] {
        let entry = v["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == name)
            .unwrap();
        assert!(
            entry["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| !f["default"].is_null()),
            "{name}: sample did not parse"
        );
    }
}

#[test]
fn modular_vfx_reference_keeps_legacy_types_distinct_and_explains_wire_values() {
    let docs = generate();
    let value: serde_json::Value = serde_json::from_str(&docs.json).unwrap();
    let data = value["data"].as_array().unwrap();
    let entry = |name: &str| data.iter().find(|d| d["name"] == name).unwrap();
    let field = |name: &str, member: &str| {
        entry(name)["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == member)
            .unwrap()
    };
    for (_, alias) in VFX_ALIASES {
        assert!(
            !entry(alias)["fields"].as_array().unwrap().is_empty(),
            "{alias} has no source members"
        );
    }
    assert_eq!(
        field("EntitySpec", "vfx")["rust_type"],
        "Option<VfxSystemSpec>"
    );
    assert_eq!(
        field("VfxSystemSpec", "emitters")["rust_type"],
        "Vec<VfxEmitterSpec>"
    );
    assert_eq!(
        field("VfxSystemSpec", "parameters")["rust_type"],
        "BTreeMap<String, VfxParameterValue>"
    );
    assert_eq!(field("VfxEmitterSpec", "target")["default"], "cpu");
    assert_eq!(
        field("VfxEmitterSpec", "render")["default"]["kind"],
        "sprite"
    );
    assert_eq!(
        field("VfxParameterValue", "vector")["rust_type"],
        "value: [f32; 3]"
    );
    assert_eq!(
        field("VfxScalarValue", "Parameter")["rust_type"],
        "VfxScalarBinding"
    );
    assert!(field("VfxScalarValue", "Constant")["doc"]
        .as_str()
        .unwrap()
        .contains("without a variant tag"));
    assert_eq!(field("EmitterSpec", "rate")["rust_type"], "f32");
    assert!(docs.typescript.contains("class Vfx"));
    assert!(docs.typescript.contains("vfx: Vfx"));
}

#[test]
fn native_capabilities_and_config_are_in_public_reference() {
    let docs = generate();
    for term in [
        "Canvases",
        "Media",
        "glyph_run",
        "GlyphReveal",
        "ordered_dither",
        "set_title",
        "set_cursor_visible",
        "text_field",
        "insert_text",
        "backspace",
        "focus",
        "font_alias",
        "start_scene",
    ] {
        assert!(docs.markdown.contains(term), "public API missing {term}");
    }
    for op in variants::<canvas::CanvasCommand>() {
        assert!(
            COMMAND_API
                .iter()
                .any(|(system, command, _)| *system == "canvas" && *command == op),
            "canvas command missing: {op}"
        );
    }
    let config = api_doc::parse_rust_items(&read("crates/crysirror-reallive/src/adapter.rs"))
        .into_iter()
        .find(|i| i.name == "Config")
        .expect("RealLive Config declaration");
    assert_eq!(
        config
            .members
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["root", "font_alias", "encoding", "start_scene"]
    );
}

#[test]
fn documented_text_input_methods_mutate_engine_owned_state_without_native_vm() {
    use crysirror_engine::{App, Value};
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    std::fs::write(
        root.path().join("project.json"),
        serde_json::to_vec(&json!({
            "schema":2,"name":"ui-api","content_revision":"1","canvas":[64,64],"assets":"assets",
            "clients":[{"name":"main","adapter":"crysirror-script-v1","game":"test"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        crysirror_runtime::Runtime::load(
            &root.path().join("project.json"),
            &root.path().join("state"),
        )
        .unwrap(),
        "main",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let ui = app.context().singleton("Ui").unwrap();
    let mut call = |name: &str, args: serde_json::Value| {
        let args = args
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .map(Value::from_json)
            .collect::<Vec<_>>();
        app.context().call(ui, name, &args)
    };
    call(
        "put",
        json!([{"id":7,"rect":[0,0,60,20],"kind":"text_field","font":"default","max_length":2}]),
    )
    .unwrap();
    call("focusTextField", json!([7])).unwrap();
    call("insertText", json!(["あ😀x"])).unwrap();
    assert!(call("insertText", json!(["\n"])).is_err());
    call("backspace", json!([7])).unwrap();
    call("insertText", json!(["界"])).unwrap();
    call("setTitle", json!(["engine UI"])).unwrap();
    call("setCursorVisible", json!([false])).unwrap();
    let frozen=call("snapshotLayers",json!([7])).unwrap().to_json();
    assert!(frozen.as_array().unwrap().iter().any(|layer|layer["paint"]["text"]=="あ界"));
    assert!(call("snapshotLayers",json!([999])).is_err());
    let ui = &app.runtime().unwrap().host.engine.ui;
    let owner = ui.focus.unwrap().owner;
    assert_eq!(ui.get(owner, 7).unwrap().text, "あ界");
    assert_eq!(ui.title, Some((owner, "engine UI".into())));
    assert_eq!(ui.cursor, Some((owner, false)));
}

#[test]
fn audio_transport_keeps_existing_three_argument_typescript_calls_compatible() {
    let ts = generate().typescript;
    assert!(ts.contains("scheduleVoiceResume(handle: Record<string, any>, delay_ms: number, fade_ms: number): void;"));
    assert!(ts.contains("scheduleVoiceResumeWithOptions(handle: Record<string, any>, delay_ms: number, fade_ms: number, options: Record<string, any>): void;"));
}

#[test]
fn documented_subtree_effect_is_engine_owned_without_a_native_vm() {
    use crysirror_engine::{App,Value};
    let root=tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    std::fs::write(root.path().join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"subtree-api","content_revision":"1","assets":"assets","canvas":[64,64],"clients":[{"name":"main","adapter":"crysirror-script-v1","game":"test"}]})).unwrap()).unwrap();
    let rt=crysirror_runtime::Runtime::load(&root.path().join("project.json"),&root.path().join("state")).unwrap();
    let mut app=App::new(rt,"main").unwrap(); app.init().unwrap(); app.start().unwrap();
    let cx=app.context(); let layers=cx.singleton("Layers").unwrap();
    for (id,color) in [(1,[255,0,0,255]),(2,[0,0,255,255])] {
        cx.call(layers,"put",&[Value::from_json(json!({"id":id,"rect":[0,0,32,32],"paint":{"kind":"solid","color":color}}))]).unwrap();
    }
    let args=[Value::Int(1),Value::Int(2),Value::Int(1000),Value::from_json(json!({"kind":"blur","before":[2,1],"after":[0,2]}))];
    cx.call(layers,"beginCrossfadeEffect",&args).unwrap();
    let status=cx.call(layers,"crossfadeStatus",&[Value::Int(1)]).unwrap().to_json();
    assert_eq!(status["effect"]["kind"],"blur");
    assert_eq!(status["effect"]["before"],json!([2,1]));
    assert_eq!(status["duration_ms"],1000);
    cx.call(layers,"endCrossfade",&[Value::Int(1)]).unwrap();
    assert!(cx.call(layers,"crossfadeStatus",&[Value::Int(1)]).unwrap().to_json().is_null());
    let invalid=[Value::Int(1),Value::Int(2),Value::Int(1000),Value::from_json(json!({"kind":"blur","before":[129,1],"after":[0,2]}))];
    assert!(cx.call(layers,"beginCrossfadeEffect",&invalid).is_err());
    assert!(cx.call(layers,"crossfadeStatus",&[Value::Int(1)]).unwrap().to_json().is_null());
}
