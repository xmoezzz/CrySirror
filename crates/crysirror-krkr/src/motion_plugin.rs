// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `motionplayer.dll` / `motionplayer_nod3d.dll` `Motion` namespace, first
//! profile: resource, clock and worker-handle mapping onto the engine-owned
//! motion sampler and dynamic puppet mesh publication. Unsupported native
//! features fail visibly; no private rasterizer or composed screen is used.
//!
//! Member names and constant values follow Kirakira c451918
//! (`krkr-plugins/src/motion_player.rs`, recovered from the reference DLL).
use crate::natives::{int, object_arg, read_only};
use crate::natives::{methods, string, this_of, unsupported, Rt};
use crysirror_core::{service::HostServices, state::ClientId};
use crysirror_puppet::PuppetFrame;
use crysirror_puppet_eluna::motion::{
    MotionClock, MotionJob, MotionModel, MotionSample, MotionStep, MotionTiming, MotionWorkers,
};
use eluna::EmoteStaticScene;
use eluna::psb::PsbValue;
use crysirror_tjs2::{
    runtime::{Closure, ObjectHandle, Variant},
    Result, TjsError,
};
use std::{collections::BTreeMap, sync::Arc};

const CONSTANTS: &[(&str, i64)] = &[
    ("PlayFlagForce", 1),
    ("PlayFlagJoin", 2),
    ("PlayFlagChain", 4),
    ("PlayFlagAsCan", 8),
    ("PlayFlagStealth", 16),
    ("MaskModeAlpha", 1),
    ("MaskModeStencil", 2),
    // The nod3d profile: the game rewrites this from its own useD3D probe.
    ("enableD3D", 0),
];

/// Player methods of the reference registration that need the puppet mapping.
const PLAYER_METHODS: &[&str] = &[
    "stop",
    "setCameraOffset",
    "releaseSyncWait",
    "setVariable",
    "getVariable",
    "modifyRoot",
    "setCoord",
    "setZoom",
    "setSlant",
    "setRotate",
    "setScale",
    "getLayerNames",
    "play",
    "clear",
    "draw",
    "drawAffine",
    "getLayerMotion",
    "getLayerGetter",
    "getLayerSetter",
    "getLayerGetterList",
    "getCommandList",
    "getVariableRange",
    "initPhysics",
    "startWind",
    "stopWind",
    "serialize",
    "unserialize",
    "setColor",
    "getMainTimelineLabelList",
    "getDiffTimelineLabelList",
    "getPlayingTimelineInfoList",
    "playTimeline",
    "stopTimeline",
    "fadeOutTimeline",
    "setTimelineBlendRatio",
    "getTimelineBlendRatio",
    "getLoopTimeline",
    "getTimelinePlaying",
    "getVariableFrameList",
    "skip",
    "skipToSync",
    "progress",
    "setDrawAffineTranslateMatrix",
    "captureCanvas",
    "unloadUnusedTextures",
    "getSubImageLayers",
    "isExistMotion",
    "findMotion",
    "findSource",
];

struct Resource {
    model: Arc<MotionModel>,
    textures: Vec<String>,
    /// This model's share of the client texture budget, returned when the
    /// last manager entry, player or draw job drops it.
    _budget: TextureLease,
}
/// Live motion texture bytes/count of one client.
#[derive(Default)]
struct TextureLedger {
    bytes: std::sync::atomic::AtomicU64,
    count: std::sync::atomic::AtomicU64,
}
struct TextureLease {
    ledger: Arc<TextureLedger>,
    bytes: u64,
    count: u64,
}
impl Drop for TextureLease {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering::AcqRel;
        self.ledger.bytes.fetch_sub(self.bytes, AcqRel);
        self.ledger.count.fetch_sub(self.count, AcqRel);
    }
}
struct Loaded {
    refs: usize,
    resource: Arc<Resource>,
}
struct Player {
    manager: usize,
    object: String,
    animation: String,
    resource: Option<Arc<Resource>>,
    clock: MotionClock,
    variables: BTreeMap<String, f32>,
    affine: [f32; 6],
    opacity: f32,
    visible: bool,
    completion: i64,
    /// The last native step of this player and what it stepped (model,
    /// object, animation); the next sample continues from it.
    step: Option<(StepKey, Arc<EmoteStaticScene>)>,
}
type StepKey = (usize, String, String);
fn step_key(model: &Arc<MotionModel>, object: &str, animation: &str) -> StepKey {
    (Arc::as_ptr(model) as usize, object.to_owned(), animation.to_owned())
}
struct PendingLoad {
    manager: usize,
    name: String,
    result: ObjectHandle,
    job: MotionJob<Arc<MotionModel>>,
}
struct DrawJob {
    target: ObjectHandle,
    player: usize,
    key: StepKey,
    resource: Arc<Resource>,
    job: MotionJob<MotionStep>,
}
#[derive(Default)]
pub struct State {
    managers: BTreeMap<usize, BTreeMap<String, Loaded>>,
    players: BTreeMap<usize, Player>,
    adaptors: BTreeMap<usize, usize>,
    workers: Option<MotionWorkers>,
    loading: Option<PendingLoad>,
    /// Target layer → (target, player, resource, sample).
    requested: BTreeMap<usize, (ObjectHandle, usize, Arc<Resource>, MotionSample)>,
    drawing: BTreeMap<usize, DrawJob>,
    published: std::collections::BTreeSet<usize>,
    clears: BTreeMap<usize, [f32; 2]>,
    next_texture: u32,
    /// Textures of models still referenced (budget is for live models; a
    /// load/unload cycle, e.g. while skipping, returns its share).
    textures: Arc<TextureLedger>,
    cancelled_load: bool,
}
impl State {
    pub(crate) fn has_frame(&self, layer: usize) -> bool {
        self.pending_frame(layer) || self.published.contains(&layer)
    }
    pub(crate) fn pending_frame(&self, layer: usize) -> bool {
        self.requested.contains_key(&layer) || self.drawing.contains_key(&layer)
    }

    pub(crate) fn invalidate(&mut self, id: usize) {
        self.managers.remove(&id);
        self.players.remove(&id);
        self.adaptors.remove(&id);
        self.drawing.remove(&id);
        self.published.remove(&id);
        self.requested.remove(&id);
        self.clears.remove(&id);
        if self
            .loading
            .as_ref()
            .is_some_and(|p| p.manager == id || p.result.0 == id)
        {
            self.loading.take();
            self.cancelled_load = true;
        }
    }

    fn workers(&mut self) -> Result<&MotionWorkers> {
        if self.workers.is_none() {
            self.workers = Some(MotionWorkers::new().map_err(err)?);
        }
        Ok(self.workers.as_ref().unwrap())
    }
}

fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(format!("Motion: {e:#}"))
}

pub(crate) fn value(rt: &mut Rt, v: &PsbValue) -> Variant {
    match v {
        PsbValue::Null => Variant::Void,
        PsbValue::Bool(b) => Variant::Integer(*b as i64),
        PsbValue::Int(i) => Variant::Integer(*i),
        PsbValue::Float(f) => Variant::Real(f64::from(*f)),
        PsbValue::Double(d) => Variant::Real(*d),
        PsbValue::String(s) => Variant::String(s.clone()),
        PsbValue::List(items) => {
            let items = items.iter().map(|v| value(rt, v)).collect();
            Variant::Object(rt.alloc_array_object(items))
        }
        PsbValue::Object(fields) => {
            let h = rt.alloc_dictionary_object();
            for (k, v) in fields {
                let v = value(rt, v);
                rt.set_object_member(h, k.clone(), v);
            }
            Variant::Object(h)
        }
        // Resources/compiler tags are not plain metadata values.
        _ => Variant::Void,
    }
}

fn class(
    rt: &mut Rt,
    motion: ObjectHandle,
    name: &'static str,
    init: crate::natives::Native,
) -> ObjectHandle {
    let c = rt.alloc_native_constructor(move |rt: &mut Rt, this: Option<ObjectHandle>, a| {
        let existing = this
            .map(|h| rt.bound_this(h).unwrap_or(h))
            .filter(|h| *h != rt.global_handle());
        let h = existing.unwrap_or_else(|| rt.alloc_ordinary_object());
        let motion = rt.global_member("Motion").object_handle().expect("Motion");
        if let Variant::Object(class) = rt.object_member(motion, name) {
            if rt.object_super_class(h).is_none() && class != h {
                rt.set_object_super_class(h, class);
            }
        }
        rt.add_object_class_info(h, name);
        init(rt, Some(h), a)?;
        Ok(Variant::Object(h))
    });
    rt.add_object_class_info(c, name);
    rt.set_object_member(c, name, Variant::Closure(Closure::new(c, None)));
    rt.set_object_member(motion, name, Variant::Object(c));
    c
}

pub fn install(rt: &mut Rt) {
    if matches!(rt.global_member("Motion"), Variant::Object(_)) {
        return;
    }
    let motion = rt.alloc_ordinary_object();
    rt.add_object_class_info(motion, "Motion");
    rt.set_global_member("Motion", Variant::Object(motion));
    for (n, v) in CONSTANTS {
        rt.set_object_member(motion, *n, Variant::Integer(*v));
    }
    methods(
        rt,
        motion,
        &[("doAlphaMaskOperation", |_, _, _| {
            Err(unsupported(
                "Motion.doAlphaMaskOperation",
                "shared alpha-mask operation",
            ))
        })],
    );

    let rm = class(rt, motion, "ResourceManager", |rt, t, _| {
        let h = this_of(rt, t, "Motion.ResourceManager")?;
        rt.host_mut().motion.managers.entry(h.0).or_default();
        Ok(Variant::Void)
    });
    methods(
        rt,
        rm,
        &[
            ("load", |rt, t, a| {
                let h = this_of(rt, t, "ResourceManager.load")?;
                let name = string(&a, 0, "ResourceManager.load")?;
                load(rt, h, &name)
            }),
            ("unload", |rt, t, a| {
                let h = this_of(rt, t, "ResourceManager.unload")?;
                let name = string(&a, 0, "ResourceManager.unload")?;
                let m = rt.host_mut().motion.managers.entry(h.0).or_default();
                if let Some(l) = m.get_mut(&name) {
                    l.refs -= 1;
                    if l.refs == 0 {
                        m.remove(&name);
                    }
                }
                Ok(Variant::Void)
            }),
            // Parsed sources are owned by load references; there is no extra cache.
            ("clearCache", |_, _, _| Ok(Variant::Void)),
            ("finalize", |rt, t, _| {
                let h = this_of(rt, t, "ResourceManager.finalize")?;
                rt.host_mut().motion.managers.remove(&h.0);
                Ok(Variant::Void)
            }),
        ],
    );

    for name in ["Player", "EmotePlayer"] {
        let c = class(rt, motion, name, |rt, t, a| {
            let h = this_of(rt, t, "Motion.Player")?;
            let manager = a
                .first()
                .and_then(object_arg)
                .map(|v| rt.bound_this(v).unwrap_or(v))
                .map(|v| v.0)
                .unwrap_or(0);
            rt.host_mut().motion.players.insert(
                h.0,
                Player {
                    manager,
                    object: String::new(),
                    animation: String::new(),
                    resource: None,
                    clock: MotionClock {
                        tick: 0.,
                        speed: 1.,
                        playing: false,
                        timing: MotionTiming {
                            duration: 0.,
                            loop_start: None,
                        },
                    },
                    variables: BTreeMap::new(),
                    affine: [1., 0., 0., 1., 0., 0.],
                    opacity: 1.,
                    visible: true,
                    completion: 0,
                    step: None,
                },
            );
            Ok(Variant::Void)
        });
        for m in PLAYER_METHODS {
            rt.register_object_native(c, *m, move |_: &mut Rt, _, _| {
                Err(unsupported(
                    &format!("Motion.Player.{m}"),
                    "Motion playback mapped onto the shared puppet service",
                ))
            });
        }
        install_player(rt, c);
    }
    for name in ["SeparateLayerAdaptor", "LayerAdaptor"] {
        let c = class(rt, motion, name, |rt, t, a| {
            let h = this_of(rt, t, "Motion.LayerAdaptor")?;
            let owner = match a.first() {
                Some(Variant::Closure(c)) => c.object,
                Some(Variant::Object(h)) => *h,
                _ => return Err(err("LayerAdaptor requires owner layer")),
            };
            let owner = rt.bound_this(owner).unwrap_or(owner);
            let layer = rt.host().layer(owner)?.clone();
            rt.host_mut().create_layer(h, layer.window, Some(owner.0))?;
            if let Some(class) = rt.global_member("Layer").object_handle() {
                rt.set_object_super_class(h, class)
            }
            rt.add_object_class_info(h, "Layer");
            rt.host_mut().layer_mut(h)?.kind = 2;
            rt.host_mut().motion.adaptors.insert(h.0, owner.0);
            refit(rt, h)?;
            rt.register_object_native(h, "getSubImageLayers", |_: &mut Rt, _, _| Ok(Variant::Void));
            Ok(Variant::Void)
        });
        methods(
            rt,
            c,
            &[
                ("getSubImageLayers", |_, _, _| Ok(Variant::Void)),
                ("finalize", |rt, t, _| {
                    let h = this_of(rt, t, "LayerAdaptor.finalize")?;
                    rt.host_mut().motion.adaptors.remove(&h.0);
                    rt.host_mut().motion.requested.remove(&h.0);
                    rt.host_mut().motion.drawing.remove(&h.0);
                    Ok(Variant::Void)
                }),
            ],
        );
    }
}

fn resource_object(rt: &mut Rt, name: &str, resource: Option<&Resource>) -> Variant {
    let r = rt.alloc_ordinary_object();
    rt.add_object_class_info(r, "MotionResource");
    rt.set_object_member(r, "name", Variant::String(name.into()));
    let metadata = resource
        .and_then(|r| r.model.psb().root.field("metadata"))
        .map(|m| value(rt, m))
        .unwrap_or(Variant::Void);
    rt.set_object_member(r, "metadata", metadata);
    Variant::Object(r)
}
fn load(rt: &mut Rt, h: ObjectHandle, name: &str) -> Result<Variant> {
    let existing = rt
        .host_mut()
        .motion
        .managers
        .entry(h.0)
        .or_default()
        .get_mut(name)
        .map(|l| {
            l.refs += 1;
            l.resource.clone()
        });
    if let Some(resource) = existing {
        return Ok(resource_object(rt, name, Some(&resource)));
    }
    if !rt.host().can_suspend {
        return Err(unsupported(
            "Motion.ResourceManager.load in callback",
            "resumable loading boundary",
        ));
    }
    if rt.host().motion.loading.is_some() {
        return Err(err("motion load already pending"));
    }
    let bytes = rt.read_storage_binary(name, "").map_err(err)?;
    let job = rt.host_mut().motion.workers()?.load(bytes).map_err(err)?;
    let result = resource_object(rt, name, None);
    rt.host_mut().motion.loading = Some(PendingLoad {
        manager: h.0,
        name: name.into(),
        result: result.object_handle().unwrap(),
        job,
    });
    rt.request_suspend();
    Ok(result)
}
fn player(rt: &Rt, h: ObjectHandle) -> Result<&Player> {
    rt.host()
        .motion
        .players
        .get(&h.0)
        .ok_or_else(|| err("player was finalized"))
}
fn player_mut(rt: &mut Rt, h: ObjectHandle) -> Result<&mut Player> {
    rt.host_mut()
        .motion
        .players
        .get_mut(&h.0)
        .ok_or_else(|| err("player was finalized"))
}
fn real(args: &[Variant], i: usize, default: f64) -> Result<f64> {
    match args.get(i) {
        None | Some(Variant::Void) => Ok(default),
        Some(v) => v.to_real(),
    }
}
fn install_player(rt: &mut Rt, c: ObjectHandle) {
    for key in ["chara", "motion"] {
        rt.register_object_native_property(
            c,
            key,
            move |rt: &mut Rt, t| {
                let h = this_of(rt, t, key)?;
                let p = player(rt, h)?;
                Ok(Variant::String(if key == "chara" {
                    p.object.clone()
                } else {
                    p.animation.clone()
                }))
            },
            move |rt: &mut Rt, t, v| {
                let h = this_of(rt, t, key)?;
                let s = v.to_tjs_string()?;
                if key == "motion" {
                    return Err(unsupported(
                        "Motion.Player.motion assignment",
                        "play(motion,flags)",
                    ));
                }
                player_mut(rt, h)?.object = s;
                Ok(())
            },
        );
    }
    for key in ["playing", "allplaying", "animating"] {
        rt.register_object_native_property(
            c,
            key,
            move |rt: &mut Rt, t| {
                let h = this_of(rt, t, key)?;
                Ok(Variant::Integer(player(rt, h)?.clock.playing as i64))
            },
            read_only,
        );
    }
    for key in [
        "speed",
        "tickCount",
        "frameTickCount",
        "lastTime",
        "frameLastTime",
        "loopTime",
        "frameLoopTime",
        "opacity",
        "visible",
        "completionType",
        "colorWeight",
        "maskMode",
    ] {
        rt.register_object_native_property(
            c,
            key,
            move |rt: &mut Rt, t| {
                let h = this_of(rt, t, key)?;
                let p = player(rt, h)?;
                let n = match key {
                    "speed" => p.clock.speed,
                    "tickCount" => p.clock.tick / 0.06,
                    "frameTickCount" => p.clock.tick,
                    "lastTime" => p.clock.timing.duration / 0.06,
                    "frameLastTime" => p.clock.timing.duration,
                    "loopTime" => p.clock.timing.loop_start.map(|v| v / 0.06).unwrap_or(-1.),
                    "frameLoopTime" => p.clock.timing.loop_start.unwrap_or(-1.),
                    "opacity" => f64::from(p.opacity) * 255.,
                    "visible" => p.visible as i32 as f64,
                    "completionType" => p.completion as f64,
                    "colorWeight" => 4286611584.,
                    "maskMode" => 1.,
                    _ => unreachable!(),
                };
                Ok(Variant::Real(n))
            },
            move |rt: &mut Rt, t, v| {
                let h = this_of(rt, t, key)?;
                let n = v.to_real()?;
                if !n.is_finite() {
                    return Err(err("nonfinite player property"));
                }
                let p = player_mut(rt, h)?;
                match key {
                    "speed" if n >= 0. => p.clock.speed = n,
                    "tickCount" if n >= 0. => p.clock.tick = n * 0.06,
                    "frameTickCount" if n >= 0. => p.clock.tick = n,
                    "opacity" if (0. ..=255.).contains(&n) => p.opacity = n as f32 / 255.,
                    "visible" => p.visible = v.is_truthy(),
                    "completionType" if n == 0. || n == 1. || n == 2. => p.completion = n as i64,
                    "colorWeight" if n == 4286611584. => {}
                    "maskMode" if n == 1. || n == 2. => {}
                    _ => {
                        return Err(unsupported(
                            &format!("Motion.Player.{key}={n}"),
                            "supported motion property profile",
                        ))
                    }
                }
                Ok(())
            },
        );
    }
    rt.register_object_native_property(
        c,
        "resourceManager",
        |rt: &mut Rt, t| {
            let h = this_of(rt, t, "resourceManager")?;
            Ok(Variant::Object(ObjectHandle(player(rt, h)?.manager)))
        },
        read_only,
    );
    rt.register_object_native_property(
        c,
        "variableKeys",
        |rt: &mut Rt, t| {
            let h = this_of(rt, t, "variableKeys")?;
            let a = player(rt, h)?
                .variables
                .keys()
                .cloned()
                .map(Variant::String)
                .collect();
            Ok(Variant::Object(rt.alloc_array_object(a)))
        },
        read_only,
    );
    rt.register_object_native_property(
        c,
        "outline",
        |_, _| Ok(Variant::Void),
        |_, _, v| {
            if matches!(v, Variant::Void | Variant::Null) {
                Ok(())
            } else {
                Err(unsupported(
                    "Motion.Player.outline",
                    "shared puppet outline effect",
                ))
            }
        },
    );
    methods(
        rt,
        c,
        &[
            ("play", |rt, t, a| {
                let h = this_of(rt, t, "Motion.Player.play")?;
                let animation = string(&a, 0, "play")?;
                let flags = int(&a, 1, 1)?;
                if flags & !1 != 0 {
                    return Err(unsupported(
                        "Motion.Player.play flags",
                        "timeline joining/chaining",
                    ));
                }
                let p = player(rt, h)?;
                let manager = p.manager;
                let object = p.object.clone();
                let resource = rt
                    .host()
                    .motion
                    .managers
                    .get(&manager)
                    .and_then(|m| {
                        m.values().find(|l| {
                            let chara = if object.is_empty() {
                                l.resource.model.base_object()
                            } else {
                                &object
                            };
                            l.resource.model.timing(chara, &animation).is_ok()
                        })
                    })
                    .map(|l| l.resource.clone())
                    .ok_or_else(|| err(format!("motion {object}/{animation} not loaded")))?;
                let object = if object.is_empty() {
                    resource.model.base_object().to_owned()
                } else {
                    object
                };
                let timing = resource.model.timing(&object, &animation).map_err(err)?;
                let p = player_mut(rt, h)?;
                let speed = p.clock.speed;
                p.clock = MotionClock::new(timing);
                p.clock.speed = speed;
                p.resource = Some(resource);
                p.animation = animation;
                p.object = object;
                // A new play starts a new step sequence.
                p.step = None;
                Ok(Variant::Void)
            }),
            ("stop", |rt, t, _| {
                let h = this_of(rt, t, "stop")?;
                player_mut(rt, h)?.clock.playing = false;
                Ok(Variant::Void)
            }),
            ("progress", progress),
            ("frameProgress", |rt, t, a| {
                progress(rt, t, vec![Variant::Real(real(&a, 0, 0.)? / 0.06)])
            }),
            ("skipToSync", |rt, t, _| {
                let h = this_of(rt, t, "skipToSync")?;
                let p = player_mut(rt, h)?;
                p.clock.tick = p.clock.timing.duration;
                Ok(Variant::Void)
            }),
            ("setVariable", |rt, t, a| {
                let h = this_of(rt, t, "setVariable")?;
                let name = string(&a, 0, "setVariable")?;
                let n = real(&a, 1, 0.)?;
                if real(&a, 2, 0.)? != 0. {
                    return Err(unsupported(
                        "Motion.Player.setVariable tween",
                        "shared parameter tween",
                    ));
                }
                if !n.is_finite() {
                    return Err(err("nonfinite variable"));
                }
                player_mut(rt, h)?.variables.insert(name, n as f32);
                Ok(Variant::Void)
            }),
            ("getVariable", |rt, t, a| {
                let h = this_of(rt, t, "getVariable")?;
                let name = string(&a, 0, "getVariable")?;
                Ok(Variant::Real(
                    player(rt, h)?.variables.get(&name).copied().unwrap_or(0.) as f64,
                ))
            }),
            ("setDrawAffineTranslateMatrix", |rt, t, a| {
                if a.len() != 6 {
                    return Err(TjsError::bad_param_count());
                }
                let h = this_of(rt, t, "setDrawAffineTranslateMatrix")?;
                let mut m = [0.; 6];
                for i in 0..6 {
                    let v = a[i].to_real()?;
                    if !v.is_finite() {
                        return Err(err("nonfinite affine"));
                    }
                    m[i] = v as f32
                }
                player_mut(rt, h)?.affine = m;
                Ok(Variant::Void)
            }),
            ("draw", draw),
            ("clear", |rt, _, a| {
                if int(&a, 1, 0)? != 0 {
                    return Err(unsupported(
                        "Motion.Player.clear color",
                        "nontransparent puppet clear",
                    ));
                }
                let target = target_arg(rt, &a)?;
                refit(rt, target)?;
                let l = rt.host().layer(target)?;
                let size = [l.image_width.max(1) as f32, l.image_height.max(1) as f32];
                rt.host_mut().motion.clears.insert(target.0, size);
                Ok(Variant::Void)
            }),
            ("setColor", |_, _, a| {
                if int(&a, 0, 4286611584)? == 4286611584 && real(&a, 1, 0.)? == 0. {
                    Ok(Variant::Void)
                } else {
                    Err(unsupported(
                        "Motion.Player.setColor",
                        "shared puppet color adjustment",
                    ))
                }
            }),
            ("finalize", |rt, t, _| {
                let h = this_of(rt, t, "Player.finalize")?;
                rt.host_mut().motion.players.remove(&h.0);
                Ok(Variant::Void)
            }),
        ],
    );
    for key in ["onSync", "onAction", "onFindMotion", "tags"] {
        rt.set_object_member(c, key, Variant::Void);
    }
}
fn progress(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Motion.Player.progress")?;
    let finished = player_mut(rt, h)?
        .clock
        .advance_ms(real(&a, 0, 0.)?)
        .map_err(err)?;
    if finished {
        let callback = rt.object_member(h, "onSync");
        if !matches!(callback, Variant::Void | Variant::Null) {
            rt.call_function(callback, vec![])?;
        }
    }
    Ok(Variant::Void)
}
fn target_arg(rt: &Rt, a: &[Variant]) -> Result<ObjectHandle> {
    let h = a
        .first()
        .and_then(object_arg)
        .ok_or_else(|| err("draw target Layer required"))?;
    let h = rt.bound_this(h).unwrap_or(h);
    rt.host().layer(h)?;
    Ok(h)
}
fn refit(rt: &mut Rt, target: ObjectHandle) -> Result<()> {
    if let Some(owner) = rt.host().motion.adaptors.get(&target.0).copied() {
        let parent = rt.host().layer(ObjectHandle(owner))?.clone();
        let size = [parent.width.max(1), parent.height.max(1)];
        let old = rt.host().layer(target)?;
        if [old.image_width, old.image_height] != size {
            rt.host_mut()
                .reset_surface(target, size[0], size[1], [0; 4])?;
        }
        let l = rt.host_mut().layer_mut(target)?;
        l.width = size[0];
        l.height = size[1];
        l.visible = true;
        l.dirty = true;
    }
    Ok(())
}
fn draw(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Motion.Player.draw")?;
    let target = target_arg(rt, &a)?;
    refit(rt, target)?;
    let layer = rt.host().layer(target)?;
    let size = [
        layer.image_width.max(1) as f32,
        layer.image_height.max(1) as f32,
    ];
    let p = player(rt, h)?;
    let resource = p
        .resource
        .clone()
        .ok_or_else(|| err("draw called before play"))?;
    let sample = MotionSample {
        model: resource.model.clone(),
        object: p.object.clone(),
        animation: p.animation.clone(),
        tick: p.clock.tick,
        variables: p.variables.clone(),
        affine: p.affine,
        size,
        opacity: if p.visible { p.opacity } else { 0. },
        // Filled when the job launches, from the player's latest step.
        previous: None,
    };
    if std::env::var_os("CRYSIRROR_KRKR_TRACE_MOTION").is_some() {
        eprintln!(
            "[motion] sample layer={} {}/{} tick={:.1} dur={:.1} loop={:?} playing={} opacity={} vars={:?} affine={:?} size={:?}",
            target.0, sample.object, sample.animation, sample.tick, p.clock.timing.duration,
            p.clock.timing.loop_start, p.clock.playing, sample.opacity, sample.variables, sample.affine, sample.size
        );
    }
    rt.host_mut().motion.clears.remove(&target.0);
    rt.host_mut()
        .motion
        .requested
        .insert(target.0, (target, h.0, resource, sample));
    let l = rt.host_mut().layer_mut(target)?;
    l.has_image = true;
    l.blank = false;
    l.full_alpha = None;
    l.dirty = true;
    Ok(Variant::Void)
}
/// Owner completion boundary. CPU workers never mutate a VM or engine layers.
pub fn poll(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<bool> {
    anyhow::ensure!(
        !rt.host().motion.cancelled_load,
        "motion load owner invalidated before publication"
    );
    let mut loading = false;
    if let Some(mut pending) = rt.host_mut().motion.loading.take() {
        match pending.job.try_take()? {
            None => {
                rt.host_mut().motion.loading = Some(pending);
                loading = true;
            }
            Some(model) => {
                anyhow::ensure!(
                    rt.host().motion.managers.contains_key(&pending.manager),
                    "motion manager invalidated during load"
                );
                let bytes = model
                    .textures()
                    .iter()
                    .map(|t| t.rgba.len() as u64)
                    .sum::<u64>();
                let ledger = rt.host().motion.textures.clone();
                let count = model.textures().len() as u64;
                {
                    use std::sync::atomic::Ordering::Acquire;
                    anyhow::ensure!(
                        ledger.bytes.load(Acquire) + bytes <= 512 * 1024 * 1024,
                        "motion client texture budget exceeded"
                    );
                    anyhow::ensure!(
                        ledger.count.load(Acquire) + count <= 4096,
                        "motion client texture count exceeded"
                    );
                }
                ledger.bytes.fetch_add(bytes, std::sync::atomic::Ordering::AcqRel);
                ledger.count.fetch_add(count, std::sync::atomic::Ordering::AcqRel);
                let lease = TextureLease { ledger, bytes, count };
                let mut textures = Vec::new();
                for texture in model.textures() {
                    let counter = rt.host().motion.next_texture;
                    let slot = 0x5000_0000u32
                        .checked_add(counter)
                        .ok_or_else(|| anyhow::anyhow!("motion resource slots exhausted"))?;
                    rt.host_mut().motion.next_texture = counter
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("motion texture counter overflow"))?;
                    textures.push(
                        host.upload_rgba(
                            client,
                            slot,
                            texture.width,
                            texture.height,
                            &texture.rgba,
                        )?
                        .relative()
                        .to_owned(),
                    );
                }
                if let Some(dir) = std::env::var_os("CRYSIRROR_KRKR_DUMP_MOTION") {
                    fn json(v: &PsbValue) -> serde_json::Value {
                        match v {
                            PsbValue::Object(o) => o.iter().map(|(k, v)| (k.clone(), json(v))).collect::<serde_json::Map<_, _>>().into(),
                            PsbValue::List(l) => l.iter().map(json).collect::<Vec<_>>().into(),
                            PsbValue::Int(i) => (*i).into(),
                            PsbValue::Float(f) => (*f).into(),
                            PsbValue::Double(f) => (*f).into(),
                            PsbValue::Bool(b) => (*b).into(),
                            PsbValue::String(s) => s.clone().into(),
                            other => format!("{other:?}").chars().take(64).collect::<String>().into(),
                        }
                    }
                    let name = pending.name.replace(['/', '>'], "_");
                    let path = std::path::Path::new(&dir).join(format!("model-{name}.json"));
                    let _ = std::fs::write(path, serde_json::to_vec(&json(&model.psb().root))?);
                }
                let metadata = model
                    .psb()
                    .root
                    .field("metadata")
                    .map(|m| value(rt, m))
                    .unwrap_or(Variant::Void);
                rt.set_object_member(pending.result, "metadata", metadata);
                rt.host_mut()
                    .motion
                    .managers
                    .get_mut(&pending.manager)
                    .unwrap()
                    .insert(
                        pending.name,
                        Loaded {
                            refs: 1,
                            resource: Arc::new(Resource { model, textures, _budget: lease }),
                        },
                    );
            }
        }
    }
    let clears = std::mem::take(&mut rt.host_mut().motion.clears);
    for (target, size) in clears {
        rt.host_mut().motion.drawing.remove(&target);
        rt.host_mut().motion.requested.remove(&target);
        if let Ok(layer) = rt.host().layer(ObjectHandle(target)) {
            host.present_puppet_frame(
                client,
                layer.presentation(),
                &serde_json::to_vec(&PuppetFrame {
                    size,
                    ..Default::default()
                })?,
            )?;
        }
    }
    let targets: Vec<usize> = rt.host().motion.drawing.keys().copied().collect();
    for target in targets {
        let mut job = rt.host_mut().motion.drawing.remove(&target).unwrap();
        if !rt.host().layers.contains_key(&target) {
            continue;
        }
        if let Some(step) = job.job.try_take()? {
            if let Some(p) = rt.host_mut().motion.players.get_mut(&job.player) {
                p.step = Some((job.key.clone(), step.scene));
            }
            let mut frame = step.frame;
            frame.textures = job.resource.textures.clone();
            frame.validate()?;
            let layer = rt
                .host()
                .layer(job.target)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            if std::env::var_os("CRYSIRROR_KRKR_TRACE_MOTION").is_some() {
                eprintln!(
                    "[motion] publish layer={target} attached={} parts={} size={:?}",
                    layer.attached,
                    frame.parts.len(),
                    frame.size
                );
            }
            // Diagnostic: CRYSIRROR_KRKR_DUMP_MOTION=<dir> keeps the latest
            // published frame of each target layer as JSON.
            if let Some(dir) = std::env::var_os("CRYSIRROR_KRKR_DUMP_MOTION") {
                let path = std::path::Path::new(&dir).join(format!("motion-{target}.json"));
                let _ = std::fs::write(path, serde_json::to_vec(&frame)?);
            }
            if layer.attached {
                host.present_puppet_frame(
                    client,
                    layer.presentation(),
                    &serde_json::to_vec(&frame)?,
                )?;
                rt.host_mut().motion.published.insert(target);
            }
        } else {
            rt.host_mut().motion.drawing.insert(target, job);
        }
    }
    let targets: Vec<usize> = rt.host().motion.requested.keys().copied().collect();
    for target in targets {
        if rt.host().motion.drawing.contains_key(&target) {
            continue;
        }
        let (handle, player, resource, mut sample) =
            rt.host_mut().motion.requested.remove(&target).unwrap();
        let key = step_key(&sample.model, &sample.object, &sample.animation);
        sample.previous = rt
            .host()
            .motion
            .players
            .get(&player)
            .and_then(|p| p.step.as_ref())
            .filter(|(k, _)| *k == key)
            .map(|(_, scene)| scene.clone());
        let job = rt
            .host_mut()
            .motion
            .workers()
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .sample(sample)?;
        rt.host_mut().motion.drawing.insert(
            target,
            DrawJob {
                target: handle,
                player,
                key,
                resource,
                job,
            },
        );
    }
    Ok(!loading)
}

impl State {
    /// Managers, players, adaptors and in-flight loads/draws.
    pub(crate) fn gc_roots(&self, out: &mut Vec<ObjectHandle>) {
        out.extend(self.managers.keys().map(|k| ObjectHandle(*k)));
        out.extend(self.players.iter().flat_map(|(k, p)| [ObjectHandle(*k), ObjectHandle(p.manager)]));
        out.extend(self.adaptors.iter().flat_map(|(k, v)| [ObjectHandle(*k), ObjectHandle(*v)]));
        if let Some(l) = &self.loading {
            out.extend([ObjectHandle(l.manager), l.result]);
        }
        out.extend(self.requested.iter().flat_map(|(k, (h, ..))| [ObjectHandle(*k), *h]));
        out.extend(self.drawing.iter().flat_map(|(k, j)| [ObjectHandle(*k), j.target]));
        out.extend(self.published.iter().map(|k| ObjectHandle(*k)));
        out.extend(self.clears.keys().map(|k| ObjectHandle(*k)));
    }
}
