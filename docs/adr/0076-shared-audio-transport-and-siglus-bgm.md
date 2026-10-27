# ADR 0076: shared audio transport and original Siglus BGM

Date: 2026-10-27. Status: implemented bounded BGM profile.
Follows ADR 0050, 0067–0075. Actual validation is recorded in STATUS.md.

## Source and ownership

The original Siglus SceneVm is retained. The prior BgmEngine placeholder did
not bind native BGM forms to engine audio. New bgm_bridge and adapter/audio are
semantic/commit adapters over CrySirror HostServices, not copied native audio
implementations. No native BGM player, Kira hub, codec, sample buffer, fader,
renderer or output device is imported by this bridge.

Reviewed siglus_rs at the existing immutable pin
5f358ef1360b0439e92aed2b749e8f25d8721954: complete BGM engine/form/table/container
files, audio module/hub, and selected complete resource/wait/VM argument/save call
paths. Exact immutable Git hashes and review scopes are in
[siglus-bgm-source-audit.json](../siglus-bgm-source-audit.json). The source tree
contains 2519 tracked files; that inventory is not whole-engine semantic coverage.
The existing reference hub has earlier opt-in-device port changes, explicitly
separate from the original source hashes. No source file is copied for this hook.

## Shared capabilities added first

Core AudioParameters adds exact start_frame (overrides start_ms), paused,
replace_fade_ms and clamp_range. Defaults preserve existing callers: None, false,
zero and false. Positions/loops use decoded-source frames. The optional bounded
range policy clamps start/repeat to the prepared source end, disables an empty
loop span, and uses the repeat point when the initial position reaches the end
of a valid loop. Empty non-loop ranges fail. AudioAsset.frame_limit already
bounds the source end. These operations also work with prepared PCM or joined
intro/tail sources, independently of Siglus.

Channel workers seek/prewarm once. Owner publication sets the prepared logical
cursor without launching a redundant seek or consuming a sample. Initial paused
sources retain their exact position. Immediate replacement reuses the exact
channel voice, restoring its old state on validation failure rather than
stealing another channel. Crossfade publication requires a spare voice and
publishes the new source before fading the old; capacity failure preserves the
old resource. Ended/stolen channel controls remain idempotent and cannot touch
another generation.

Core AudioPlaybackInfo/HostServices.audio_info expose source-playing, pause,
actual loop mode, decoded position/rate/known length, output rate and remaining
stop/pause fade and scheduled-resume delay frames. Derived tails remain separate
from source completion. Mixer/ChannelAudio offer the same Rust query; modern
Audio.playbackInfo and nativeChannelInfo expose it through the class registry.
Native/media channels are distinct from canonical SceneState channelStatus.

AudioRequest::Resume/Mixer.schedule_resume/Audio.scheduleVoiceResume freeze the
source, count a bounded output-clock delay, then start a fresh fade-in. Delay
and fade counters use output frames, not decoded-source frames or worker read-
ahead. Pause, stop or another resume replaces the pending schedule. Explicit
stop/pause fades continue across paused/preparing/starved input while source
position and ordinary fade-in remain frozen. They cannot leave a native fade
wait stuck behind a decoder worker. This does not make final mixing asynchronous.

## Original VM translation and commit boundary

Mapped BGM operations: PLAY, PLAY_ONESHOT, PLAY_WAIT, READY, READY_ONESHOT, STOP,
PAUSE, RESUME/RESUME_WAIT, WAIT/WAIT_KEY, WAIT_FADE/WAIT_FADE_KEY, volume getters/
setters/extremes, registered name, CHECK and GET_PLAY_POS. Original positional/
named flags, wait return form, case/whitespace normalization and same-looping-name
idempotence are retained. Named wait forces one-shot exactly as the native form.
RESUME requires a still-retained source. After EOF/immediate stop it fails
visibly: a stable engine-owned prepared-source resource is still needed to match
native retained-data replay. Reopening an asset path is not used as a fake
equivalent, since its bytes may have changed or disappeared. Explicit PLAY is
ordinary new preparation. The original wait machinery still handles DECIDE release versus natural return
and does not stop a loop when WAIT_KEY skips.

BGM CHECK codes remain native values 0 free, 1 play, 2 fade-out, 3 pause. These
are translations of shared source/fade/pause observations, not a private audio
clock. Normal wait and fade wait have different predicates. Core source-playing
can include paused input; the native playing predicate excludes pause/fade-out.
EOF waits do not grow with engine HRTF/echo tails. Position queries inspect the
last committed mixer cursor; they do not extrapolate worker decoding or wall time.

A native command first produces logical operations. For resource work, the
adapter retains the post-command VM separately and keeps the visible VM at its
original instruction/argument stacks. begin_audio admits work; audio_preparation
Ready commits the VM and releases its receipt. Pending work returns cooperatively
without blocking the frame or following instructions. Failed work releases its
receipt, leaves the original command, and can retry after resource repair.
Unexpected status-query errors retain the same live ticket instead of starting
another job. Controls similarly pass through scoped HostServices operations.
Other clients keep progressing. No unsafe borrowed host pointer is retained.

## Resource and table profile

Each native BGM file requires an explicit Config.resources alias to an engine
asset under the client root. Optional decoded UTF-8 Gameexe BGM rows provide
registered/file names and start/end/repeat samples. At most 4096 rows/declared
entries are accepted; malformed range metadata fails visibly. There is no implicit
append-directory scan, absolute native filesystem access, encrypted Gameexe
conversion or native archive-entry selection. Such formats can be mounted/
converted through shared resource APIs and explicitly aliased when supported.

BGMTABLE count/listened/set/all operations retain native logical gallery flags;
successful registered playback marks them in the original VM save state. No
shared decoder or transport algorithm lives in that table. Unknown BGM/table
opcodes and old property fallbacks without a supported meaning fail visibly.
Other Siglus audio forms (PCM/PCMCH/SE/voice) remain unsupported by this phase.

## Saves and explicit remaining boundaries

Siglus runtime payload schema is now 2 because its logical BGM state changed;
schema-1 payloads need a migration and are rejected by the existing envelope.
Pending checkpoints serialize the original uncommitted instruction, never
worker tickets. Active snapshots retain logical declarations, and restore queues
derived BGM preparation only after candidate validation/commit. Like existing
engine audio restores, playback restarts at authored range start, not an exact
audio-device/sample-history checkpoint. Echo/HRTF/delay DSP history is not saved.

Original Siglus file codecs/header/layouts remain unchanged. Native BGM name,
volume, loop, pause and delay predicates use their existing codec fields.
Original import restores persistent declarations through shared preparation;
its existing cloned-state rollback now advertises transactional import to Runtime.
During an in-flight native resource command, direct original-file export/import
fails visibly rather than mutating the separately retained VM. This bounded
profile does not establish every native save variant or game compatibility.

Native boot/scene/Gameexe reads and bounded metadata lookup remain synchronous
owner work. Final DSP/mixing/output, control allocation/disposal and teardown
remain synchronous boundaries. Retained-source replay after EOF/stop, dynamic acoustic geometry, Doppler, higher/diffuse
reflections, full Siglus audio/layout coverage, KiriKiri WaveSoundBuffer and
commercial-game/device/other-platform validation remain open. ADR 0050 and the
broader modern engine/VN objective are not complete.
