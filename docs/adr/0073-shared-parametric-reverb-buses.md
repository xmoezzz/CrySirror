# ADR 0073: shared worker-prepared late-reverb buses

Date: 2026-10-27. Status: implemented parametric profile; results in STATUS.md.
Follows ADR 0030, 0050, 0067–0072.

## Scope and source

Use standalone FunDSP 0.23.0's 32-channel FDN for authored late reverberation,
behind CrySirror `LateReverb`, `PreparedReverb` and bus routing. Exact original
revision, complete source inventory, registry comparison, reviewed hooks and
unchanged upstream licenses are in crysirror-acoustics/REVERB_UPSTREAM.md.
Only std is selected: its optional file loader/convolution features are disabled.
No VM, legacy mixer, engine renderer or audio-device backend is imported.

This fills the shared parametric late-reverb capability. Geometry-derived early
reflections, material-dependent impulse responses, diffuse multi-bounce/pathing,
diffraction and measured physical RT60 remain separate requirements. Room size
scales FDN delays and the upstream decay value is approximate. AcousticMaterial
absorption/scattering are not consumed by this effect. It is not a geometry
simulation or full Steam Audio equivalent.

## Engine routing and API

ReverbSpec validates room scale 1..50 metres, approximate decay 0.1..20 seconds
and damping 0..1. ReverbSettings adds up to five unique non-Master bus sends,
return bus and wet gain 0..1. Defaults send Sfx at 0.2 to Ambient at unity wet
return. A prepared effect is useful without any VM, scene or output device.

For each client, accumulate post-source-bus, gain/envelope, ducking, pan/spatial,
direct-filter and HRTF stereo samples before Master. Process one FDN and apply
its wet gain, return-bus gain/mute/ducking and Master exactly once. The wet output
never feeds the bus send again. Dry output remains present. Input bus mute blocks
new sends; return mute silences the existing wet tail while processing continues.
Master mute also continues effect history. Returning to the same source bus
applies that bus's fader at both the send and return; Ambient is the default to
keep those controls separate. Wet Voice returns do not become dialogue sources.
There is no independent geometric indirect path behind the direct occlusion.

Tail history belongs to the effect and survives voice EOF/Stop/stealing. Explicit
clear/replacement removes that history; there is no automatic truncation/sleep,
seamless profile crossfade or per-voice reverb delay line. Installed idle effects
continue processing silence. Mixer::clear removes derived effects as well as
voices, so old room history cannot leak through restore.

Audio.beginReverb(settings), reverbStatus(ticket), cancelReverbJob(ticket),
clearReverb() and reverbSettings() expose the resource. prepareReverbAsync wraps
owner-polled publication in the existing bounded Promise system and releases
its ticket. Releasing a published receipt keeps the effect. Rust Mixer::set_reverb
accepts prepared data and returns old history for caller-owned retirement.

## Async boundaries and native/media audio

Delay allocation, graph construction, sample-rate setup and preallocation happen
on bounded CPU workers. Deferred task admission keeps frame owners responsive
when the shared pool is full. Publication moves a prepared object without scanning
samples or allocating delay buffers. Old effects remain until successful
replacement. Malformed settings reject before admission; later errors appear in
status/Promise rejection. Cancellation/supersession prevent stale publication.
Tickets are client/restore scoped. Derived effects/jobs never enter VM saves.

App currently mixes pooled/canonical and native/media channels in two existing
mixers. Prepare one matched graph for each on a worker and publish both, rolling
back the first installation if the second fails. This extends the same shared
effect to original native/Media AudioRequest calls without adapter algorithms.
The return is linear and no limiter/nonlinear processing is inserted. Publication
affects subsequent processing; it cannot rewrite a native PCM block rendered
earlier in the same frame. The plain Rust ChannelAudio host can install the same
prepared effect through its public Mixer.

Operation tables allow 8 retained jobs/client and 32/host. Reserve 64 MiB per
prepared/installed pair against an estimated 512 MiB host budget. Leases follow
both actual DSP owners and canceled workers; reset does not zero live counters.
These are conservative admission estimates, not a process-memory limit.

Final sample processing/mixing remains on the current owner. Old effect disposal
on replacement/clear/restore, control/scratch allocations, device queues and final
pool destruction still have synchronous boundaries. This ADR does not complete
the engine-wide async migration or claim hard realtime execution.

## Validation

Independent impulse tests exercise decay, source EOF tails, block invariance,
Master scaling, input/return mute, client isolation, native PCM channels and
actual storage lifetime. Engine tests exercise publication in both mixers,
full-pool cancellation/supersession, scoped receipts, invalid replacement and
restore. JS/TS tests execute the Promise helper and receipt release.

The room_reverb example produces authored dry/room/hall clicks without game data
or devices, and prints offline mix time, peak and post-source tail energy. This
is a reproducible single-effect CPU sample, not an audio-device latency or
full-engine performance benchmark. Actual commands/results are in STATUS.md.
