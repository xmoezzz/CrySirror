// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! Dataset selection and arbitrary-block bridge over sofar's partitioned convolution.
//! File reads, interpolation, FFT planning and filter transforms belong to workers.
use anyhow::{Context, Result, ensure};
use glam::Vec3;
use serde::{Deserialize, Serialize};
pub use sofar::render::{FilterPublisher, PreparedFilter};
use sofar::{
    filter::Filter,
    reader::{OpenOptions, Sofar},
    render::{FilterTransition, RealtimeRenderer, Renderer, RendererPlan},
};
/// SOFA receiver-record mapping to output ears. Explicit overrides handle
/// datasets whose position metadata differs from their authored channel order.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HrtfEarOrder {
    /// Preserve the conventional record-zero left, record-one right order.
    #[default]
    LeftRight,
    /// Swap the two receiver records and their delays.
    RightLeft,
    /// Determine ears from positive/negative SOFA Y receiver coordinates.
    ReceiverPositions,
}
/// Dataset preparation and per-source convolution policy.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HrtfOptions {
    /// SOFA ear mapping; defaults to authored left/right order. JSON has explicit ear fields.
    pub ear_order: HrtfEarOrder,
    /// FFT partition size, one of 32/64/128/256 samples; adds this many frames of bridge latency.
    pub partition_frames: u32,
    /// Filter-change crossfade in milliseconds, 1..200; applies to pose updates.
    pub crossfade_ms: f32,
    /// Maximum per-ear dataset delay in seconds, 0..0.25. Delay lines allocate on workers.
    pub max_delay_seconds: f32,
    /// Apply sofar's normalization to SOFA files; JSON coefficients retain authored gain.
    pub normalized: bool,
    /// SOFA lookup radius in metres; null uses the first measurement's radius.
    pub reference_distance: Option<f32>,
}
impl Default for HrtfOptions {
    fn default() -> Self {
        Self {
            ear_order: HrtfEarOrder::LeftRight,
            partition_frames: 64,
            crossfade_ms: 20.,
            max_delay_seconds: 0.05,
            normalized: false,
            reference_distance: None,
        }
    }
}
impl HrtfOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [32, 64, 128, 256].contains(&self.partition_frames),
            "HRTF partition size"
        );
        ensure!(
            self.crossfade_ms.is_finite() && (1.0..=200.).contains(&self.crossfade_ms),
            "HRTF crossfade duration"
        );
        ensure!(
            self.max_delay_seconds.is_finite() && (0.0..=0.25).contains(&self.max_delay_seconds),
            "HRTF delay capacity"
        );
        ensure!(
            self.reference_distance
                .is_none_or(|v| v.is_finite() && (0.01..=100.).contains(&v)),
            "HRTF reference distance"
        );
        Ok(())
    }
}
/// Portable prepared HRIR data; useful for authoring, tests and offline imports.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HrirAsset {
    /// Exact output sample rate. JSON requires a matching mixer rate; SOFA resamples through sofar.
    pub sample_rate: u32,
    /// 1..8192 measurements, with matching nonempty FIR lengths and at most 8388608 total taps.
    pub measurements: Vec<HrirMeasurement>,
}
/// One measurement in the listener's CrySirror local axes: +X right, +Y up, -Z front.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HrirMeasurement {
    /// Nonzero direction; magnitude is normalized at preparation.
    pub direction: [f32; 3],
    /// Left-ear FIR, at most 4096 finite taps.
    pub left: Vec<f32>,
    /// Right-ear FIR, same length as left and all other measurements.
    pub right: Vec<f32>,
    /// Additional left/right delay, in seconds; defaults to zero.
    #[serde(default)]
    pub delays_seconds: [f32; 2],
}
enum Data {
    Sofa {
        reader: Sofar,
        radius: f32,
        swap_ears: bool,
        global_delays: Option<[f32; 2]>,
    },
    Json(Vec<HrirMeasurement>),
}
/// Immutable dataset and FFT plan. Share between voice/filter worker jobs.
pub struct HrtfDataset {
    data: Data,
    plan: RendererPlan,
    options: HrtfOptions,
    measurements: usize,
    resident: usize,
    delay_frames: usize,
}
impl HrtfDataset {
    pub fn from_bytes(bytes: &[u8], sample_rate: u32, options: HrtfOptions) -> Result<Self> {
        options.validate()?;
        ensure!((8000..=192000).contains(&sample_rate), "HRTF sample rate");
        ensure!(bytes.len() <= 32 * 1024 * 1024, "HRTF input limit");
        let (data, len, measurements, resident) = if bytes.starts_with(b"\x89HDF\r\n\x1a\n") {
            let has_global_delays = preflight_sofa(bytes, sample_rate)?;
            let reader = OpenOptions::new()
                .sample_rate(sample_rate as f32)
                .normalized(options.normalized)
                .open_data(bytes)?;
            let h = reader.hrtf();
            let dims = h.dimensions();
            ensure!(
                h.get_attribute("SOFAConventions") == Some("SimpleFreeFieldHRIR")
                    && h.get_attribute("DataType") == Some("FIR")
                    && dims.r == 2
                    && dims.e == 1
                    && dims.i == 1
                    && dims.c == 3,
                "unsupported SOFA convention/dimensions"
            );
            let count = dims.m as usize;
            let len = dims.n as usize;
            ensure!(
                (1..=8192).contains(&count)
                    && (1..=4096).contains(&len)
                    && count * 2 * len <= 8388608,
                "SOFA measurement/tap limits"
            );
            ensure!(
                h.data_ir.values.len() == count * 2 * len
                    && h.source_position.values.len() == count * 3,
                "SOFA array dimensions"
            );
            ensure!(
                h.source_position
                    .get_attribute("Type")
                    .is_none_or(|v| v == "cartesian")
                    && h.source_position
                        .get_attribute("Units")
                        .is_none_or(|v| matches!(v, "meter" | "metre" | "meters" | "metres")),
                "unsupported SOFA source coordinates/units"
            );
            ensure!(
                h.data_ir
                    .values
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 16.),
                "invalid SOFA FIR"
            );
            ensure!(
                h.source_position
                    .values
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 100.),
                "invalid SOFA positions"
            );
            ensure!(
                h.data_delay
                    .values
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.)
                    && reader.max_delay() <= options.max_delay_seconds,
                "SOFA delay exceeds capacity"
            );
            ensure!(
                h.data_delay.values.is_empty()
                    || [count, count * 2, 2].contains(&h.data_delay.values.len()),
                "SOFA delay dimensions"
            );
            // The upstream reader warns for unsupported listener poses. Enforce our supported frame.
            for (values, expected) in [
                (&h.listener_view.values, [1., 0., 0.]),
                (&h.listener_up.values, [0., 0., 1.]),
                (&h.listener_position.values, [0.; 3]),
                (&h.emitter_position.values, [0.; 3]),
            ] {
                ensure!(
                    values.is_empty()
                        || (values.len() == 3
                            && values
                                .iter()
                                .zip(expected)
                                .all(|(v, e)| v.is_finite() && (v - e).abs() < 1e-4)),
                    "unsupported SOFA listener/emitter frame"
                );
            }
            let ears = &h.receiver_position.values;
            ensure!(
                ears.len() == 6
                    && ears.iter().all(|v| v.is_finite() && v.abs() <= 1.)
                    && ((ears[1] > 0. && ears[4] < 0.) || (ears[1] < 0. && ears[4] > 0.)),
                "unsupported SOFA receiver positions"
            );
            let swap_ears = match options.ear_order {
                HrtfEarOrder::LeftRight => false,
                HrtfEarOrder::RightLeft => true,
                HrtfEarOrder::ReceiverPositions => ears[1] < ears[4],
            };
            let global_delays = has_global_delays.then(|| {
                [
                    h.data_delay.values[0] / reader.sample_rate(),
                    h.data_delay.values[1] / reader.sample_rate(),
                ]
            });
            let radius = options
                .reference_distance
                .unwrap_or_else(|| Vec3::from_slice(&h.source_position.values[..3]).length());
            ensure!(
                radius.is_finite() && (0.01..=100.).contains(&radius),
                "SOFA lookup radius"
            );
            let resident = (h.data_ir.values.len()
                + h.source_position.values.len()
                + h.data_delay.values.len())
                * 4
                + count * 256;
            (
                Data::Sofa {
                    reader,
                    radius,
                    swap_ears,
                    global_delays,
                },
                len,
                count,
                resident,
            )
        } else {
            let mut asset: HrirAsset = serde_json::from_slice(bytes)?;
            ensure!(
                asset.sample_rate == sample_rate,
                "JSON HRIR sample rate must match the mixer; import/resample offline or use SOFA"
            );
            ensure!(
                (1..=8192).contains(&asset.measurements.len()),
                "HRIR measurement count"
            );
            let len = asset.measurements[0].left.len();
            ensure!(
                (1..=4096).contains(&len) && asset.measurements.len() * len * 2 <= 8388608,
                "HRIR tap limit"
            );
            let mut seen = std::collections::BTreeSet::new();
            for m in &mut asset.measurements {
                let dir = unit(Vec3::from(m.direction))?;
                m.direction = dir.to_array();
                let id = dir
                    .to_array()
                    .map(|v| if v == 0. { 0 } else { v.to_bits() });
                ensure!(seen.insert(id), "duplicate HRIR direction");
                ensure!(
                    m.left.len() == len
                        && m.right.len() == len
                        && m.left
                            .iter()
                            .chain(&m.right)
                            .all(|v| v.is_finite() && v.abs() <= 16.),
                    "HRIR FIR lengths/values"
                );
                ensure!(
                    m.delays_seconds
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=options.max_delay_seconds).contains(v)),
                    "HRIR delay exceeds capacity"
                );
            }
            let count = asset.measurements.len();
            (
                Data::Json(asset.measurements),
                len,
                count,
                count * (len * 8 + 128),
            )
        };
        let max_delay = match &data {
            Data::Sofa { reader, .. } => reader.max_delay(),
            Data::Json(rows) => rows
                .iter()
                .flat_map(|m| m.delays_seconds)
                .fold(0., f32::max),
        };
        let delay_frames = (max_delay * sample_rate as f32).ceil() as usize;
        let plan = RendererPlan::builder(len)
            .with_sample_rate(sample_rate as f32)
            .with_partition_len(options.partition_frames as usize)
            .build()?;
        Ok(Self {
            data,
            plan,
            options,
            measurements,
            resident,
            delay_frames,
        })
    }
    pub fn metadata(&self) -> serde_json::Value {
        serde_json::json!({"ear_order":if matches!(self.data,Data::Sofa{..}){Some(self.options.ear_order)}else{None},"format":if matches!(self.data,Data::Sofa{..}){"sofa"}else{"hrir_json"},"sample_rate":self.plan.sample_rate(),"measurements":self.measurements,"taps":self.plan.filter_len(),"latency_frames":self.plan.partition_len(),"estimated_bytes":self.resident})
    }
    pub fn estimated_bytes(&self) -> usize {
        self.resident
    }
    /// Worker operation: interpolate the current direction, then transform partitions to spectra.
    pub fn prepare_filter(&self, direction: [f32; 3]) -> Result<PreparedFilter> {
        let dir = unit(Vec3::from(direction))?;
        let mut filter = Filter::new(self.plan.filter_len());
        match &self.data {
            Data::Sofa {
                reader,
                radius,
                swap_ears,
                global_delays,
            } => {
                reader.filter(
                    -dir.z * radius,
                    -dir.x * radius,
                    dir.y * radius,
                    &mut filter,
                );
                if let Some(delays) = global_delays {
                    filter.ldelay = delays[0];
                    filter.rdelay = delays[1];
                }
                if *swap_ears {
                    std::mem::swap(&mut filter.left, &mut filter.right);
                    std::mem::swap(&mut filter.ldelay, &mut filter.rdelay);
                }
            }
            Data::Json(measurements) => {
                // Directional chord-distance interpolation for the portable prepared format.
                // Native SOFA keeps the library's neighborhood interpolation instead.
                let mut nearest = [(f32::INFINITY, 0usize); 3];
                for (i, m) in measurements.iter().enumerate() {
                    let d = dir.distance_squared(Vec3::from(m.direction));
                    for at in 0..3 {
                        if d < nearest[at].0 {
                            for j in (at + 1..3).rev() {
                                nearest[j] = nearest[j - 1];
                            }
                            nearest[at] = (d, i);
                            break;
                        }
                    }
                }
                let exact = nearest[0].0 < 1e-10;
                let weights = nearest.map(|(d, _)| {
                    if !d.is_finite() {
                        0.
                    } else {
                        1. / d.max(1e-10).sqrt()
                    }
                });
                let sum = if exact {
                    weights[0]
                } else {
                    weights.iter().sum()
                };
                for (n, ((_, i), weight)) in nearest.into_iter().zip(weights).enumerate() {
                    if weight == 0. || (exact && n > 0) {
                        continue;
                    }
                    let m = &measurements[i];
                    let w = weight / sum;
                    for ((left, right), (l, r)) in filter
                        .left
                        .iter_mut()
                        .zip(&mut filter.right)
                        .zip(m.left.iter().zip(&m.right))
                    {
                        *left += l * w;
                        *right += r * w;
                    }
                    filter.ldelay += m.delays_seconds[0] * w;
                    filter.rdelay += m.delays_seconds[1] * w;
                }
            }
        }
        ensure!(
            filter
                .left
                .iter()
                .chain(&filter.right)
                .all(|v| v.is_finite()),
            "invalid interpolated HRTF"
        );
        ensure!(
            [filter.ldelay, filter.rdelay]
                .iter()
                .all(|v| v.is_finite() && (0.0..=self.options.max_delay_seconds).contains(v)),
            "invalid interpolated HRTF delay"
        );
        Ok(self.plan.filter_transform().prepare(&filter)?)
    }
    /// Worker operation: allocate per-voice state and install its initial filter.
    pub fn voice(&self, direction: [f32; 3]) -> Result<(FilterPublisher, BinauralVoice)> {
        let filter = self.prepare_filter(direction)?;
        let mut renderer = Renderer::builder(&self.plan)
            .with_filter_transition(FilterTransition::crossfade(
                (self.plan.sample_rate() * self.options.crossfade_ms / 1000.).ceil() as usize,
            ))
            .with_max_delay(self.options.max_delay_seconds)
            .build()?;
        let _ = renderer
            .set_prepared_filter(filter)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let (publisher, renderer) = renderer.into_realtime();
        let n = self.plan.partition_len();
        Ok((
            publisher,
            BinauralVoice {
                renderer,
                input: vec![0.; n],
                left: vec![0.; n],
                right: vec![0.; n],
                at: 0,
                lease: None,
                tail: n + self.plan.filter_len() - 1 + self.delay_frames,
            },
        ))
    }
}
fn unit(v: Vec3) -> Result<Vec3> {
    ensure!(
        v.is_finite() && v.length_squared().is_finite() && v.length_squared() > 1e-12,
        "invalid HRTF direction"
    );
    Ok(v.normalize())
}
/// Convert a world-space pose to the listener's canonical CrySirror local direction.
/// Coincident sources retain the forward direction rather than sampling a zero vector.
pub fn listener_direction(
    source: [f32; 3],
    position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
) -> Result<[f32; 3]> {
    let source = Vec3::from(source);
    let position = Vec3::from(position);
    ensure!(
        source.is_finite() && position.is_finite(),
        "invalid binaural source/listener"
    );
    let f = unit(Vec3::from(forward))?;
    let r = unit(f.cross(Vec3::from(up)))?;
    let u = r.cross(f);
    let d = source - position;
    if d.length_squared() < 1e-12 {
        return Ok([0., 0., -1.]);
    }
    let d = unit(d)?;
    Ok([d.dot(r), d.dot(u), -d.dot(f)])
}
/// Per-voice streaming bridge. Adds one partition of latency, retaining state
/// across any caller block size. Destruction remains an owner/control operation.
pub struct BinauralVoice {
    renderer: RealtimeRenderer,
    input: Vec<f32>,
    left: Vec<f32>,
    right: Vec<f32>,
    at: usize,
    tail: usize,
    lease: Option<std::sync::Arc<dyn Send + Sync>>,
}
impl BinauralVoice {
    /// Retain a host reservation for as long as this stream owns its buffers.
    pub fn retain_storage(&mut self, lease: std::sync::Arc<dyn Send + Sync>) {
        self.lease = Some(lease);
    }

    pub fn sample_rate(&self) -> u32 {
        self.renderer.plan().sample_rate() as u32
    }
    pub fn latency_frames(&self) -> usize {
        self.input.len()
    }
    pub fn tail_frames(&self) -> usize {
        self.tail
    }
    pub fn reset(&mut self) {
        self.renderer.reset();
        self.input.fill(0.);
        self.left.fill(0.);
        self.right.fill(0.);
        self.at = 0;
    }
    pub fn process(&mut self, input: f32) -> Result<[f32; 2]> {
        let out = [self.left[self.at], self.right[self.at]];
        self.input[self.at] = input;
        self.at += 1;
        if self.at == self.input.len() {
            self.renderer
                .process_block(&self.input, &mut self.left, &mut self.right)
                .context("binaural convolution")?;
            self.at = 0;
        }
        Ok(out)
    }
}

// The upstream reader defaults missing rate data and assumes little-endian
// numeric arrays. Reject unsupported headers before it can resample guesses.
fn preflight_sofa(bytes: &[u8], target_rate: u32) -> Result<bool> {
    let parsed = sofar::hdf::parse_with_children(bytes)
        .map_err(|e| anyhow::anyhow!("SOFA header: {e:?}"))?;
    let mut rate = None;
    let mut ir_dimensions = None;
    let mut global_delays = false;
    for name in [
        "Data.SamplingRate",
        "Data.IR",
        "SourcePosition",
        "ReceiverPosition",
        "Data.Delay",
        "ListenerPosition",
        "ListenerView",
        "ListenerUp",
        "EmitterPosition",
    ] {
        let Some(child) = parsed.get_child(name) else {
            ensure!(
                !matches!(
                    name,
                    "Data.SamplingRate" | "Data.IR" | "SourcePosition" | "ReceiverPosition"
                ),
                "missing SOFA {name}"
            );
            continue;
        };
        let object = child.map_err(|e| anyhow::anyhow!("SOFA {name}: {e:?}"))?;
        let size = object.dt.size as usize;
        let class = object.dt.class_and_version & 15;
        ensure!(
            object.dt.class_bit_field & 1 == 0
                && [4, 8].contains(&size)
                && [0, 1].contains(&class)
                && object.data.len().is_multiple_of(size),
            "unsupported SOFA numeric encoding in {name}"
        );
        ensure!(
            matches!(object.dt.data_fmt,Some(sofar::hdf::DataFormat::Float{bit_offset:0,bit_precision,..}|sofar::hdf::DataFormat::Fixed{bit_offset:0,bit_precision}) if bit_precision as usize==size*8),
            "unsupported SOFA numeric bit layout in {name}"
        );
        if name == "Data.Delay" {
            let dims = object.ds.dimension_size.as_slice();
            let (m, _) = ir_dimensions.context("SOFA IR dimensions missing")?;
            ensure!(
                dims == [1, 2] || dims == [m, 2],
                "unsupported SOFA delay axes"
            );
            global_delays = dims == [1, 2];
        }
        let elements = object
            .ds
            .dimension_size
            .iter()
            .try_fold(1u64, |n, v| n.checked_mul(*v))
            .context("SOFA array dimensions overflow")?;
        ensure!(
            elements <= 8388608 && elements * size as u64 == object.data.len() as u64,
            "SOFA array byte size in {name}"
        );
        if name == "Data.SamplingRate" {
            ensure!(object.data.len() == size, "SOFA requires one sample rate");
            let value = match (class, size) {
                (1, 4) => f32::from_le_bytes(object.data[..4].try_into()?) as f64,
                (1, 8) => f64::from_le_bytes(object.data[..8].try_into()?),
                (0, 4) => i32::from_le_bytes(object.data[..4].try_into()?) as f64,
                (0, 8) => i64::from_le_bytes(object.data[..8].try_into()?) as f64,
                _ => unreachable!(),
            };
            ensure!(
                value.is_finite() && (8000.0..=192000.).contains(&value) && value.fract() == 0.,
                "unsupported SOFA sample rate"
            );
            rate = Some(value);
        }
        if name == "Data.IR" {
            let dims = &object.ds.dimension_size;
            ensure!(
                dims.len() == 3
                    && dims[1] == 2
                    && (1..=8192).contains(&dims[0])
                    && (1..=4096).contains(&dims[2]),
                "SOFA original IR dimensions"
            );
            ensure!(
                dims[0] * dims[1] * dims[2] <= 8388608
                    && dims[0] * dims[1] * dims[2] * size as u64 == object.data.len() as u64,
                "SOFA original IR size"
            );
            ir_dimensions = Some((dims[0], dims[2]));
        }
        if matches!(
            name,
            "SourcePosition"
                | "ReceiverPosition"
                | "ListenerPosition"
                | "ListenerView"
                | "ListenerUp"
                | "EmitterPosition"
        ) {
            let attr = |key: &str| {
                object
                    .parsed_attributes
                    .iter()
                    .find(|a| a.name == key)
                    .and_then(|a| a.value.as_deref())
            };
            if matches!(name, "SourcePosition" | "ReceiverPosition") {
                ensure!(
                    attr("Type").is_some() && attr("Units").is_some(),
                    "SOFA {name} Type/Units required"
                );
            }
            let units = attr("Units");
            match attr("Type") {
                Some("spherical") => {
                    let units = units
                        .context("SOFA spherical units required")?
                        .split(',')
                        .map(str::trim)
                        .collect::<Vec<_>>();
                    ensure!(
                        units.len() == 3
                            && matches!(units[0], "degree" | "degrees")
                            && matches!(units[1], "degree" | "degrees")
                            && matches!(units[2], "meter" | "metre" | "meters" | "metres"),
                        "unsupported SOFA spherical units"
                    );
                }
                Some("cartesian") | None => ensure!(
                    units.is_none_or(|v| matches!(v, "meter" | "metre" | "meters" | "metres")),
                    "unsupported SOFA cartesian units"
                ),
                _ => anyhow::bail!("unsupported SOFA coordinate type"),
            }
        }
    }
    let (m, n) = ir_dimensions.context("SOFA IR missing")?;
    let n =
        (n as f64 * target_rate as f64 / rate.context("SOFA sample rate missing")?).ceil() as u64;
    ensure!(
        (1..=4096).contains(&n) && m * 2 * n <= 8388608,
        "resampled SOFA exceeds tap budget"
    );
    Ok(global_delays)
}
