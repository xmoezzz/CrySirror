# 0068: Engine HRTF resources and worker-prepared binaural voices

Date: 2026-10-27. Status: implemented SOFA/prepared-HRIR convolution profile;
validation and remaining limits are recorded separately in STATUS.md.

## Decision

Extend the existing engine mixer with HRTF, using a standalone Rust DSP library.
`sofar 0.4.0` supplies SOFA lookup/resampling, shared FFT plans, partitioned
convolution, per-ear delay, crossfades and a prepared-filter publisher. It does
not own CrySirror voices, devices, buses, resource roots, world state or tasks.
The dependency is pinned at `303532bd609db2d74c1b9e7f7483666dad1210fa`
in vendor/sofar, with full original licenses and an audited one-line HDF5 v3
version-range fix. SOURCE.json records original hashes and manifest changes.
No legacy engine or composed output is hosted. Source audit and semantic
adaptations are described in crysirror-acoustics/HRTF_UPSTREAM.md.

HrtfDataset is an immutable engine resource. Inputs are SOFA SimpleFreeFieldHRIR
FIR assets, or portable HrirAsset JSON with explicit left/right FIRs and local
directions. JSON requires the mixer sample rate; SOFA resamples through sofar.
The prepared JSON profile blends the closest three normalized directions by
inverse chord distance. Native SOFA retains the library's neighborhood lookup.
This authoring representation does not pretend synthetic impulses are measured
human HRIR. Only authored synthetic SOFA fixtures are distributed in the repo.

SOFA's forward/left/up coordinates map from engine listener-local right/up/
backward as (-z,-x,y). HrtfEarOrder preserves authored left/right record order by default, with
explicit swaps or receiver-position mapping for other datasets. Historical
metadata can disagree with conventional channel order; callers choose the
mapping rather than inheriting a silent flip. The engine uses an explicit lookup radius
(default: the first measurement's radius); this is a fixed-reference HRTF,
not dynamic near-field propagation. Existing engine distance curves remain
separate. Missing sample rates, unsupported coordinates/units/numeric encodings,
invalid axes/sizes, excessive delays and unsupported conventions fail visibly.
The dependency's ambiguous M=R=2 global-delay heuristic is overridden using
actual Data.Delay dimensions. Constant per-ear IR delays remain distinct.

## Async ownership and completion (ADR 0050)

- Audio.beginHrtf(asset, options) captures a client-scoped storage capability.
  CPU workers read at most 32 MiB, validate, decode/resample and plan FFTs.
  Cancellation is checked between phases; dependency parsing/resampling/FFT
  planning is a bounded non-preemptible phase.
  Owner publication atomically replaces a profile revision; failed/canceled
  replacements keep the old profile. hrtfStatus and cancelHrtfJob are scoped to
  client and restore epoch. Release of a ready ticket retains its resource.
- prepareHrtfAsync wraps this lifecycle with a cancellable Promise and releases
  its ticket. It completes when the resource is installed, before any later
  voice/filter request. It does not claim an audio-device fence.
- playSfx({...,spatial/follow,binaural:true}) or setVoiceHrtf(handle,true)
  captures source/listener poses and requests per-voice worker preparation.
  FFT planning, lookup, filter transforms and voice buffer allocation stay out
  of sample processing. Source/envelope clocks wait for initial preparation,
  preserving short one-shots. Initial failures stay muted and observable;
  update failures retain the last ready filter until explicit retry/rebind.
- A voice retains its dataset snapshot/revision and reservation while workers
  and DSP hold buffers. Latest poses coalesce in a bounded round-robin queue;
  unchanged directions submit no new work. At most 16 voice/filter jobs run,
  with limits of 32 binaural voices per client and 64 per host. Saturated
  submission retains work for a retry without waiting on the owner.
- Workers return immutable prepared filters. The owner publishes them through
  sofar's mailbox before rendering. DSP adopts at partition boundaries and
  crossfades updates; retired filter owners reclaim outside normal processing.
  hrtfVoice reports **published_direction** and pending CPU preparation, not
  completion of audible filter adoption/crossfade.
- Dataset operations retain at most eight results per client and 32 per host.
  Admission reserves 256 MiB per dataset build and 2 MiB per voice under a
  1 GiB host accounting budget; ready datasets shrink to an estimate. Canceled
  jobs retain their reservations until captured allocations actually release.
  These are admission estimates; dependency HDF expansion limits still apply,
  and the process is not sandboxed to this exact memory usage.
- clearHrtf removes the owner's profile/build/filter work and restores ordinary
  stereo pan, independently of direct-path acoustic filtering. Successful profile
  replacement re-prepares enabled voices and can interrupt audio; only ordinary
  pose updates have seamless filter crossfades. Restore invalidates all derived
  datasets, voices and jobs. No dataset/device/FFT pointer enters save JSON.

## Streaming and mix behavior

The per-voice bridge accepts arbitrary mixer block sizes while processing whole
FFT partitions. It adds exactly one partition of latency: 32/64/128/256 frames,
64 by default (1.33 ms at 48 kHz), plus the dataset's per-ear delay. FIR history
and delay lines survive block boundaries. Natural EOF drains pending samples,
FIR tails and actual dataset delays; source pitch/rate does not rescale this
output-rate tail. Pause freezes the stream, and seek resets histories.

HRTF replaces equal-power spatial panning. Acoustic bands/occlusion, distance,
voice envelopes, buses, gain and ducking feed the same engine output; explicit
voice pan is a post-HRTF balance. No secondary mixer is introduced.

## Limits and next work

Only the declared convention/axes/size profile is accepted. Legacy HDF5 object
header v1 and some generic HDF5 group layouts are unsupported by this loader.
HRTF quality depends on the supplied measurement data and listener; subjective
headphone localization is not proved by an impulse test or a rendered WAV.
No measured HRIR data, native SDK or audio driver is bundled.

The bridge's sample processing allocates no buffers, but the existing mixer
still allocates scratch vectors and drops ended voices during render. Source
opening, decoder reads, mix/output and teardown need further async/real-time
boundaries. No all-engine async or hard real-time deadline claim is made.
Acoustic diffraction, early reflections, late reverb and dynamic geometry
remain separate work. This is not a new completed native VN integration.
