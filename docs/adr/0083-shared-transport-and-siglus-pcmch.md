# ADR 0083: shared transport clocks and original Siglus PCMCH

Date: 2026-10-27

Status: bounded implementation; actual validation is recorded in STATUS.md.

## Context and source

PCMCH was unbound and its original-save volume getter returned a fictitious
1000. The pinned original separates global PCM from the PCMCH list. Channels
can retain READY/paused input, schedule resume while a pause fade continues,
wait for stop-fade completion after input EOF and save their independent volume.
Private VM timers or a copied SFX engine would violate the ownership rule.

Reference: siglus_rs `5f358ef1360b0439e92aed2b749e8f25d8721954`.
[siglus-pcmch-source-audit.json](../siglus-pcmch-source-audit.json) records exact
Git blob hashes and review scope. Complete original PCMCH and SFX source and
selected routing/wait/save functions were inspected. No original mixer, clock,
resource decoder, path finder or envelope implementation is copied/linked.

## Shared engine capabilities

- AudioPlaybackInfo.transport_frames counts output frames while input transport
  is running. Fully paused input and a freezing resume delay do not count;
  pause-fade frames do. Decoder starvation, pending seek and initial effect
  dependencies count intent-to-run time, independently of decoded position,
  loops/rate and publication age. It is an output transport clock, not measured
  media/device consumption. Unknown source EOF can count its discovery frame.
- AudioResumePolicy::Freeze retains the old Resume/schedule_resume semantics.
  Preserve keeps the current running/paused/fading state before the deadline.
  AudioRequest::ResumeWithOptions and Mixer::schedule_resume_with_settings expose
  the plain scoped contract. Audio.scheduleVoiceResumeWithOptions takes a
  fourth options map: policy is "freeze" (default) or "preserve". The original
  three-argument scheduleVoiceResume retains its signature and freeze policy,
  including TypeScript
  declarations. Unknown policies/options fail. AudioResumeSettings.reset_gain
  restarts a gain ramp from zero to the latest requested target at the deadline,
  opening the transport envelope. Default false retains the independent gain
  ramp and applies the fade to transport. Native PCMCH selects reset_gain=true;
  PLAY initial fade also uses shared gain, allowing SET_VOLUME to replace it.
- A delayed action commits after its last waiting output frame. The next frame
  starts input with a fresh fade-in, regardless of chunk boundaries. Later
  pause/stop/resume replaces it. Input EOF cancels the action even while an HRTF
  or echo tail is retained; scheduling on already-ended input is a no-op that
  leaves the independent tail alone. Completed stop also cancels the action.
  Running preserved dialogue continues to contribute to shared music ducking.
- Stop completion retains a silent voice when input EOF arrives before its
  stop-fade deadline. source_playing stays false, transport_frames stops, while
  stopping/fade_out_frames remain observable. is_playing/Audio.voice.playing
  includes this completion as well as effect tails. pausing distinguishes a
  pause fade from a stop fade. A stop deadline is independent of later pause
  envelopes, so a shorter/longer pause cannot shorten/extend WAIT_FADE.
  Processing tails still do not prolong input waits.

Each retained stop consumes an ordinary voice/decoder lease until its deadline;
existing 600000 ms duration and pool limits apply. No unbounded hidden timer or
new background playback resource is introduced. Frames and counters saturate.

## Execution, dependencies and ownership

The owner admits plain controls only outside a pending final DSP frame. Existing
resource workers own immutable asset snapshots and decoder preparation; the
owner publishes ready voices separately. The existing MixFrame/MixJob boundary
moves mixer state to a CPU worker and publishes PCM/transport atomically. Queries
remain at the last published frame while work runs. Pause, stop, scheduled
resume and EOF progress in that same worker-owned sample loop. They add no owner
wait, VM thread, callback lock or wall-clock deadline. PcmOutput/device readiness
remains a separate boundary. Offline synchronous mixing remains available.

## Original native semantic adapter

PCMCH[ch] maps to scoped shared channel 4+ch. Channels 0/1/2/3 remain BGM/global
PCM/global SE/global KOE. The native default list length is 16, bounded by decoded
Gameexe PCMCH.CNT up to 256. CrySirror honors this declared bound (the pinned
original Rust playback backend only allocates 16 PCMCH slots). Larger lists still
consume the existing shared voice/receipt/memory budgets and fail on exhaustion;
there is no reservation guaranteeing 256 simultaneous voices plus other audio.
The list owns only native cue/mode/volume/delay declarations and derived engine
observations. Its queues are per-slot bounded at 128, then globally serial-sorted
with other sound forms. No sample buffers, faders, decoders or independent clocks
are held by the VM.

PLAY/PLAY_LOOP/PLAY_WAIT, READY/READY_LOOP, STOP/PAUSE/RESUME/RESUME_WAIT,
CHECK, WAIT/WAIT_KEY/WAIT_FADE/WAIT_FADE_KEY and volume operations translate onto
shared prepared resources, channels, gain, transport inspection and original
wait predicates. PLAY_WAIT/RESUME_WAIT do not synthesize a return value. DECIDE
key waits preserve down/up and 1/0 completion semantics. Pause fades make native
CHECK false immediately but are not WAIT_FADE stop fades. Global PCM STOP only
stops the global slot; original sound load stops every PCM slot.

Source selection is original PCM name, BGM name, KOE number, then SE number.
Every result requires an explicit canonical Config.resources alias. Gameexe BGM
registration maps only its filename: music intro/range markers do not apply to
PCMCH. Sparse BGM registration indices are bounded at 4096 before lookup.
KOE uses explicit koe/{number}, no JITAN transform; SE uses the decoded
Gameexe SE table and always plays once without a fade, preserving original
persistent requested loop metadata. Missing/unknown sources, layouts, opcodes,
parameters and resource failures are visible, with following instructions uncommitted.

This phase accepts default PCM category/system routing, chara_no<0 and no BGM
fade source/target flags. Other routing fails explicitly before sound admission;
full native user/category/character/dual-BGM duck state is not emulated by an
inert property store. Per-slot 8-bit gain lives on the shared voice, and native
PCM master volume uses the existing client siglus.pcm gain group.

Native one-shot completion compares measured integer-ms source duration against
shared transport_frames, excluding READY/pause/delay time. This retains the
original predicate within CrySirror's frame publication phases, independently
of physical EOF and effect tails. Exact wall-clock/device timing and arbitrary
source-rate changes applied externally are not established as original behavior.

## Saves and migration

Runtime adapter payload schema changes **4 -> 5** with no old-schema migration.
Checkpoints serialize logical declarations; worker tickets, leases, clocks and
DSP are derived. Active cues reprepare at source start, READY/fully paused cues
remain paused, delayed resumes reconstruct their remaining integer-ms delay.
A pending stop reconstructs as stopped. A pause fade reconstructs fully paused;
exact envelope/sample/seek/effect history is not saved. Source files are needed.

Original Siglus file codecs/layouts are unchanged. The false slot-volume default
1000 is corrected to native 255. Original load restores every volume, restarting
only looped, READY or delayed declarations. Delay is a restore predicate, not a
new scheduled action. Ambiguous records retain original PCM-first then independent
BGM/KOE/SE order. STOP only clears persistent loop_flag; READY metadata remains as
in the original, including its effect on a later original file load. Framework
snapshot fidelity and original file semantics are deliberately different contracts.

## Validation and remaining work

Independent mixer/channel/public Rust/JS/TS cases inspect actual PCM and clocks,
including pause fades, resumed envelopes, preserved/frozen delay, starvation,
short-source stop completion, scope and worker publication. Authored original
Scene.pck cases test resource admission, slot independence, named sources, waits,
error boundaries, PCM output and both save paths. The standalone transport_timeline
example exports a sample-asserted WAVE without any VM or device. No game/font or
proprietary SDK files are included. See STATUS.md for measured results.

PCMEVENT, native discovery/append directories, full routing/MWND/character voice
semantics and whole Siglus games remain open. This does not establish acoustic
native opcodes, Doppler/diffraction, audio listening quality, performance ranking,
hardware-RT/cross-platform results or completion of engine-wide ADR 0050.
