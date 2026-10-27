# ADR 0078: shared gain groups, playback age and Siglus PCM/SE

Date: 2026-10-27. Status: implemented bounded profile.
Follows ADR 0050 and 0067–0077. Actual validation is recorded in STATUS.md.

## Research and the reusable gap

Fixed master/music/sfx/voice/ambient/ui buses cannot express separate PCM and SE
category gains while retaining independent voice envelopes. Putting that routing
or gain interpolation into a native player would duplicate engine capabilities.
Unreal's [Sound Classes](https://dev.epicgames.com/documentation/en-us/unreal-engine/sound-classes-in-unreal-engine)
provide sound grouping and inherited volume control; Kira's
[track documentation](https://docs.rs/kira/0.12.5/kira/track/index.html) describes
hierarchical volume and independent sends/effects. These are design references,
not imported implementations or a claim of feature parity.

CrySirror adds a bounded logical gain hierarchy over its existing mixer. It is
not a submix DSP graph: arbitrary group effects, inherited pitch/filter/distance,
per-group concurrency, virtualization and sidechain graphs remain future work.
Existing FunDSP late reverb, acoustic/HRTF/echo resources, decoder workers and
source/bus/master ownership remain in their shared engine modules.

## Group ownership and evaluation

Core adds plain AudioMixGroupSettings/Info and SetMixGroup/SetMixGroupGain audio
requests, without external/device dependencies. AudioParameters and VoiceSpec
add optional mix_group. Names belong to the initiating client, require 1–128
bytes without control characters or surrounding whitespace, and resolve against
that mixer's namespace. A voice caches a dense index at owner publication.

The mixer admits 64 groups/client and 256/mixer, with depth at most eight.
Parents must already exist in the same client. Validation compiles a candidate
parent-before-child order and checks descendants, cycles, missing parents,
names, gains, fades and capacity before mutation. Failed changes preserve live
settings and playback. Removed leaf slots may be reused; live voices/effect
instances and child groups prevent removal. Queued playback resolves its logical
name again at publication and fails if the required group has disappeared.

Gain interpolation shares the existing engine output-frame Ramp, extracted into
one internal module. Each output frame evaluates dense parent indices without
name lookups, allocations or graph traversal. A group's own fade advances during
silence, paused sources, dependency holds and decoder starvation. Voice/source
clocks and independent envelopes retain their existing semantics. Parent changes
reroute immediately; a fade interpolates the group's own level, not a crossfade
between old and new graph topologies.

Own gain/mute and ancestor gains multiply once with voice envelope/gain and bus
controls. The resulting dry and early-echo signal feeds the existing reverb send
before Master; Master applies once at dry/return outputs. Previously committed
late-reverb history remains on its shared return bus even after the source group
is muted or removed. Group mute is not a resource suspension/virtualization rule.
Dialogue duck admission evaluates the current group level per sample; a muted
voice group stops triggering ducking at the correct output-frame boundary,
independently of render block partition.

Group settings/ramp state are host controls, like existing bus settings. Playback
clear/restore retains them; they are not automatically serialized as project/VM
DSP state. Native volume updates use SetMixGroupGain, preserving host-assigned
parents and mute. Native script volume owns that group's gain; host volume
sliders can use its parent or fixed bus. Groups do not require a window/device.

## Public and asynchronous boundaries

Audio.setMixGroup partially updates parent/gain/muted and validates both pooled
and native/media graphs before either publication. Parent null explicitly
removes a parent; omitted fields retain their prior settings. Null/nonfinite
numeric controls, invalid mute values and unknown fields fail visibly. mixGroup
and nativeMixGroup expose each namespace's AudioMixGroupInfo. removeMixGroup
checks both namespaces before removal; setVoiceMixGroup reroutes a live pooled
voice without replacing its source/clock. Rust Mixer/ChannelAudio expose the same
capabilities independently of a VM or App.

Control mutations are bounded synchronous owner work; no I/O or worker wait is
hidden in a setter. PCM decoding/resource preparation stays on bounded workers,
completion publishes on the owner, and gain processing belongs to the final
mixer's output clock. Moving final DSP/mixing/output and teardown to a complete
async execution pipeline remains an ADR 0050 requirement, not accomplished by
making control methods return futures.

AudioPlaybackInfo adds age_frames: output frames since voice publication,
including pause/preparation holds, starvation and effect tails. Seek changes the
decoded cursor without resetting age. Rust, pooled and native queries expose the
same distinct source-position/output-age units. The counter is an engine
capability, not a timer reconstructed by the native adapter.

## Original PCM/global SE semantic adapter

Audited siglus_rs at existing pin 5f358ef1360b0439e92aed2b749e8f25d8721954:
complete SFX engine, PCM/SE forms and immutable Kira hub, plus complete SE table
loader/count definitions. Previous BGM audit covers original argument/wait/save
paths; sound save/load ordering was checked again. Exact hashes/scopes are in
[siglus-sfx-source-audit.json](../siglus-sfx-source-audit.json). No native player,
clock, decoder, sample cache, envelope or Kira mixer is copied/linked by the hook.

Global PCM PLAY/STOP maps its independent native slot to engine channel 1;
global SE uses channel 2, while existing BGM remains channel 0. Client namespaces
prevent another VM's channel collision. PCMCH and other native slots are not
aliases for these global players. The new adapter stores only native volume,
source identity, observed logical playing state and ephemeral ordered operations.

Mapped SE forms include filename/SE-number/voice-number plays, stop/fade,
volume/extremes/getter, CHECK and WAIT/WAIT_KEY. Numbered SE uses decoded Gameexe
SE entries with original count default16/min8/max256. Canonical resources aliases
are mandatory. Voice numbers use explicit CrySirror aliases `koe/{number}`;
this is a project mapping policy, not native filename/pack discovery or KOE
speech/JITAN integration. Missing tables/aliases, malformed arguments, unknown
forms and unsupported property fallbacks fail instead of reporting fake success.

Native category gains map onto `siglus.pcm` / `siglus.se` groups. Worker resource
preparation measures unknown source lengths through ADR 0077, then independent
voice preparation captures a lease. One-shot resource lookup handles are released
right after voice admission; existing/admitted instances retain their own bytes.
The original PC/stacks stay uncommitted until successful voice publication.
Failure releases receipts/resources and preserves the previous live source;
repair can retry the original instruction. Pending snapshots serialize the old
instruction, never worker tickets or prepared bytes.

The upstream SFX duration predicate truncates decoded WAV duration to whole
milliseconds. The bridge compares that native value with shared playback age;
it can release WAIT while the last fractional-millisecond PCM samples still play.
Stop/fade and volume are different: a category fade changes gain without stopping
the source; native STOP clears logical playing while the engine may finish a
fade. SE WAIT_KEY preserves its original zero return and does not stop playback.
This is an owner-observed predicate: polling follows the engine's current
VM-before-mix frame phase, and is not an exact wall-clock/device timing match to
the original timer backend, especially under starvation. Final audio scheduling
and broad game validation remain open.

Ephemeral operation stamps merge BGM/PCM/SE actions in original call order.
This matters for original file import: PCM/SE stop before a subsequent BGM
preparation can block. The bounded queue contains semantic requests, not a
second audio execution engine, and stamps never enter saves.

## Saves and remaining coverage

Siglus runtime payload schema changes **2 -> 3** because PCM/SE logical state
changed. Schema-2 snapshots require migration and fail through the existing
version gate. Active runtime declarations reconstruct resources after candidate
commit at authored source start; stopped/ended declarations do not start voices.
Host parent/mute controls persist, while native category gain is restored.
Exact sample/decoder/effect/age histories are not saved. Halted-VM snapshots can
retain their last observed declaration, as described in ADR 0077.

Original file codecs/layouts are unchanged. PCM/global SE playback is not
persistent in that format: import stops it and restores category volumes through
shared controls. The isolated codec test host now uses real ChannelAudio in a
dev dependency; product VM dependencies still exclude an audio device/mixer.

PCMCH/PCM events, general KOE speech/JITAN/mouth tables, native UI sound slots,
automatic archives/append discovery, encrypted Gameexe, exact native audio timing,
all save variants and whole-game compatibility remain unsupported or unproven.
Dynamic acoustic geometry, Doppler, higher/diffuse propagation and full async
mix/output remain broader active goals.
