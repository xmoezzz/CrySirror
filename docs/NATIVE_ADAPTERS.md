# Native adapters: engine capability extraction

## Installed-game launch

Use `crysirror play <game-directory-or-file>`. The program detects the native
profile and constructs its project and resource mounts internally. It does
not require a hand-written project JSON, external bootstrap script or bulk
archive extraction. `--detect-only` prints one JSON plan without starting a
VM/window. Recognizing an engine is separate from completing that game's API
coverage; unsupported commands still fail at their native location.

Detection runs on a named boot worker. Its JSON plan includes bounded content
evidence from the shared pure-Rust magic classifier, with no C libmagic or
runtime rule installation. Native layouts without reliable magic signatures
still require their own structural parser; an unknown MIME is not a supported
engine or decoder. A renamed image cannot be accepted as a text-script source.

`play <directory> --headless --frames 1200 --capture /tmp/game.png` uses the
same detection, resource bootstrap and adapter with the offscreen renderer.
`--input-track track.json` supplies the existing deterministic input format.
This path creates no window/audio device. Its capture contains the game and
engine UI, not an OS menu bar or title bar; it cannot validate physical native
menu clicks. Ordinary `play` opens a desktop window. On macOS its menu bar
belongs at the top of the screen, on Windows it belongs to the HWND, and Linux
currently retains the visible engine menu fallback. Windowed games keep their
OS title bar unless `--allow-borderless` is explicit; fullscreen is independent.

NScripter launch reads the original source and mounts SAR/NSA containers on
engine resource workers under private `__nscript/` prefixes. Native lookup
checks loose files first, then configured containers. The mixed SAR/NSA profile
uses higher-numbered NSA patches before the base NSA and SAR fallback; it is
explicit and not a claim about every historical ONScripter variant. A local
`default.ttf`/`default.otf` supplies the default font before a system fallback.
Siglus still requires caller-owned decode material where applicable. Its original
Windows optical-host check follows the explicit portable-host policy below;
unknown native operations still fail at their actual command.

Automatic KiriKiri player launches supply the native `-debugwin=no` startup
argument before game scripts run. This selects the game's own production
branch instead of opening optional Windows-only developer console windows.
Explicit adapter `arguments` override executable defaults; raw adapters retain
their empty default. This does not implement or acknowledge unsupported
WIN32Dialog modeless windows. Startup arguments do not relocate the automatic
player's existing save directory; snapshot configuration still includes them.

Owner requirement, 2026-10-26: reference engines are capability requirements
and test inputs. The goal is a reusable CrySirror engine on which different
VMs can run through thin semantic/syscall adapters.

## Removed hosting paths

RFVP, BGI and RealLive hosted adapters and their dedicated engine packages
have been deleted, including desktop features, factories and native project
configuration schemas. Whole-frame hosting is prohibited. Earlier successful
screenshots/builds document the rejected approach, not accepted integrations.
No fallback aliases to the data VM are provided.

## Current registry

The desktop default registers `crysirror-script-v1`, `siglus`, `reallive` and
`avg32`.
The non-default desktop feature `krkr` also registers the AGPL KiriKiri
adapter (ADR 0053).
The bare Runtime registers the first two; hosts explicitly register other VMs.
`siglus` is the independent subset in crysirror-siglus-vm; it is not a full
Siglus integration. Explicit `message_window` configuration enables the plain
TEXT/NAME profile described below. Unconfigured or unmapped text fails visibly.
`avg32` runs an installed AVG32 game with the vendored original interpreter
(ADR 0095); raster work is the shared `crysirror_image::avg32` device.

## Acceptance

KiriKiri's menu.dll now names a facade over CrySirror MenuSpec/Ui operations.
Explicit registration-only plugin shells are allowed through Config.plugin_shells;
they export no fake functionality and report their status. Shell registration
is not capability extraction or game compatibility. Native synchronous popup
waits and unmapped plugin methods continue to fail visibly. See
[ADR 0066](adr/0066-awaitable-services-and-native-plugin-facades.md).

1. Add missing generic image/text/UI/layer/effect/audio/video/resource abilities
   to CrySirror APIs and engine-owned state, usable without any native VM.
2. Keep only opcode/value/resource-policy/layout/wait/save semantics and handle
   translation in the compatibility layer, preserving original VM behavior.
3. Native operations must call these shared APIs; do not maintain a duplicate
   native renderer or generic engine implementation inside an adapter.
4. Verify the shared capability independently, then verify real native calls,
   including inputs, state inspection, unsupported operations and saves.
5. Matching pixels or starting a native game alone is not acceptance.

Shared CPU image/composition/glyph, PCM mixing, incremental MPEG, scoped
resource decoding and native format plugins remain engine capabilities.
CBG resource decoding lives in crysirror-image::cbg without any BGI runtime.
Pinned upstream source is reference material; it is not a product host.

## RealLive replacement API bridge (2026-10-26)

`crysirror-reallive` is a new pure-VM/semantic adapter after removal of the old
hosted implementation. The desktop includes its `reallive` feature by default;
minimal builds can enable that feature alone or
register its factory explicitly; the default Runtime registry remains data VM
plus the Siglus subset. This does not reinstate RFVP/BGI hosting.

Original parser/bank/control flow remains in the VM, whose external Command/Text
events stay pending until successful CrySirror calls. Native operations target
engine Canvas/Layer/GlyphRun/UI, shared channel audio, resource capabilities and
Media services. The adapter owns no screen pixels or private generic painter.
Canvas presentations expand logical draws into shared retained Layer nodes;
media textures represent only the relevant movie resource. Shared services are
also directly available to Rust/JS/TS clients without this VM.

The source/routing audit verifies the selected upstream hashes and 90-file
reference inventory, not blanket mapping of those source files. Native opening
profile evidence is recorded separately by the integration run; the independent
full RUSUR media-service test does not establish native script/game completion.
Native save-file import/export, whole-adapter snapshots, DLL emulation and all
native layout/effect/resource variants remain unsupported or unproven.

See [REALLIVE.md](REALLIVE.md),
[ADR 0048](adr/0048-reallive-vm-engine-apis.md) and
[reallive-source-audit.json](reallive-source-audit.json).

## KiriKiri TJS2 adapter (2026-10-26)

`crysirror-krkr` runs original TJS2 scripts on the vendored Kirakira VM and
translates `Layer`, `Window`, `Timer`, `System`, `Storages` and `Scripts` onto
engine Layer/Canvas services, the engine clock and capability storage. It
added two shared capabilities first: canvas presentations attached inside a
layer tree, and a thread-safe image metadata handle. It owns no pixels,
renderer or mixer. Members without an equivalent engine operation throw a
TJS exception naming the missing capability. Font width queries use a
client-authorized engine text measurement handle, sharing the configured font
alias and simple-layout metrics with Canvas drawing. Nominal height preserves
the native absolute-height rule; unsupported font styles fail visibly
([ADR 0063](adr/0063-native-font-measurement.md)). The initial ADR 0053
limits are extended by [ADR 0088](adr/0088-kag3-semantic-adapter-and-shared-prerequisites.md):
bounded stock KAG3 execution, shared audio, bold/unblurred shadow text and native
atomic saveStruct are now mapped. XP3 auto-mounts, font-face selection, video,
transitions and complete native saves remain open.

## Siglus plain message profile (2026-10-26)

The original SceneVm's native PCK/SCN TEXT/NAME instructions can issue engine
GlyphRun resources using shared fixed-cell layout and engine-owned reveal
state. The adapter queries reveal completion through client-scoped engine
state; it does not rasterize glyphs or maintain a second typewriter clock.
The bounded horizontal profile targets `FRONT.MWND[0]`. Explicit mappings or
Gameexe grid/position/size/palette/name-spacing configuration select shared
cell layout; WAKU images and filters become engine-owned layers. OPEN/CLOSE,
PRINT/NAME, newline, deferred CLEAR, WAIT_MSG/PP/R/PAGE and read-flag commits
retain native boundaries. Window animation types 0 through 5 use shared
ScalarTween/Easing state and engine layer transforms/opacity. Native VM and
engine snapshots restore together (Siglus payload schema 9).

Other targets, rich markup, vertical/novel profiles, font decorations, WAKU
UI buttons, backlog and original MWND save files remain unsupported.
Supplying a different font does not establish matching native glyph metrics.
See [ADR 0061](adr/0061-siglus-plain-message-engine-apis.md).

Gameexe MOUSE_CURSOR.DEFAULT selects the native custom index, including zero.
G00 cuts/centers/SPEED become shared CursorSpec frames, hotspots and steps.
SCRIPT cursor selection and display commands call engine UI; resource caches
rebuild after restore. Rewrite HF/Plus each contain a validated 20-frame 32x32
default cursor. Auto-hide and exact physical host animation phase remain
unverified; this cursor validation does not get either game past disk checking.

## AVG32 original VM library (2026-10-26)

`crysirror-avg32` depends on `siglus_rs` commit
`256c398e06fbcbae8e956eb046c831b3d6c8a593`, with `portable-vm` enabled and
default features disabled. The original Scenario/parser/control-flow source
is compiled there; no interpreter source was copied into CrySirror. The
portable boundary was added in the original repository on a separate branch.
Its compiled dependency tree excludes the old renderer, painter and mixer.

The bounded `plain-adv-v1` adapter translates image loads into engine Canvas,
text into shared grid/GlyphRun state, choices into engine UI, and BGM/WAV/voice
into shared channel audio. Original waits, pending native events and hidden
choice ordinals are retained. Resource aliases stay inside client roots.
Only opaque full-size image loads and the documented plain text profile are
mapped initially. Other drawing/effect/layout/movie operations and both native
saves and framework snapshots fail visibly. No commercial AVG32 game is
claimed complete. See [ADR 0062](adr/0062-avg32-portable-vm-engine-apis.md).

## AVG32 installed games (2026-10-30)

Superseding the runtime above: `crysirror-avg32` vendors the original
interpreter from siglus_rs 5f358ef (`src/vm`, `SOURCE.json`) and runs an
installed game given `{root, font, encoding?}`. PDT buffers, copies and all
transitions are `crysirror_image::avg32`; the presented screen is one engine
surface layer; text that the original rasterised onto buffer 0 is a retained
GlyphRun layer that drops where buffer 0 is next overwritten; menus, backlog
and name entry are engine solid/GlyphRun layers; audio uses engine channels;
`SAVE.INI` is read/written in the original layout on the client save mount.
Text into other PDT buffers and Cinepak movies are reported as warnings in
diagnostics; framework snapshots fail. See
[ADR 0095](adr/0095-avg32-installed-games-shared-raster.md).

## Shared modular effects (2026-10-26)

`EntitySpec.vfx` / `WorldCommand::SetVfx` now declare the same reusable effect
systems as the JS/TS `Vfx` and `Entity` APIs. The safe core owns compilation,
CPU simulation/events and GPU tick plans; runtime workers and renderer-owned
GPU resources implement execution. A native particle/effect instruction should
translate its scheduling/resource/coordinate semantics into these declarations
or add a missing shared module first. It must not copy a private VM particle
simulator or render a legacy engine framebuffer. This capability addition does
not itself establish new native Siglus/AVG32/RealLive opcode coverage. See
[ADR 0064](adr/0064-modular-vfx-cpu-gpu.md) for supported modules and explicit
backend gaps.

## Native resource codecs (2026-10-26)

The complete named arc_unpacker inventory is now exposed by CrySirror resource
registries: archives become client VFS providers, pictures use the shared image
registry, and encoded files/audio become engine-owned readers. A VM adapter
requests these APIs and translates native names/parameters; it must not run an
unpacker or retain a private media engine. `Resources.beginDecode` and ordinary
async Assets loading provide the task/completion boundary. Keys are host aliases
and companion paths stay under the same client capability. Codec availability
alone does not establish VM/opcode, game, save or renderer compatibility. See
[ADR 0065](adr/0065-native-resource-codecs-and-async-decode.md) and the explicit
[resource](arc-resource-support.json)/[image](arc-image-support.json) ledgers.


## Native audio readiness (2026-10-27)

Mapped RealLive BGM/WAV calls and AVG32 music/wave/SE/voice events now submit
AudioAsset paths through HostServices::begin_audio and retain their original
pending events until audio_preparation reports owner publication. Opening,
probing, intro/loop/startup preparation and streaming use shared ChannelAudio
and BufferedSource workers. Admission is explicitly distinct from readiness.
Failures do not acknowledge following native instructions. Offline fixtures
retain immediate preparation behavior; original save/layout/opcode gaps remain.
Resource discovery and boot reads are still synchronous. See [ADR 0070](adr/0070-native-channel-audio-readiness.md).

## Shared late-reverb buses (ADR 0073)

Authored late reverberation is an engine resource over the standalone FunDSP
FDN, not an algorithm in a native adapter. Audio.prepareReverbAsync publishes
matched resources in ordinary and native/media channel mixers. Original native
play/wait handlers still target shared channel APIs and observe source completion;
the bus owns remaining tails independently. Adapter source/opcodes/save semantics
are unchanged. Original RealLive/AVG32 sound fixtures exercise that boundary.
This parametric capability does not establish geometry reflections, a physical
room model or additional whole-game compatibility.

## Captured early reflections and native sound completion (ADR 0074)

Finite one-bounce image-source paths, band transport, sparse delay histories and
reflection preparation/publication are shared engine capabilities. Rust channel
services can install those resources without adapter algorithms. Modern App
clients also use reflection probes and prepareVoiceReflectionsAsync. Source
completion is distinct from retained echo/HRTF tails: original RealLive/AVG32
sound waits still advance at source EOF while engine echo state drains. Native
adapter implementations/opcodes/save layouts are unchanged. This is a static
pose profile; dynamic tracking and higher-order/diffuse propagation remain open.


## Shared moving reflection plans (2026-10-27)

ADR 0075 adds reusable ReflectionPlan and persistent EarlyReflections history,
with Mixer/ChannelAudio publication and source-completion semantics independent
of App/VM/device. Modern engine Audio can automatically coalesce entity/listener
updates; native adapters can translate a spatial operation onto those shared
resources without private echo algorithms. This phase adds no native opcode or
save mapping. Original RealLive/AVG32 bridge fixtures still verify source waits
while engine-owned effects drain; that bounded evidence does not establish full
game support or native automatic spatialization.


## Original Siglus BGM through shared transport (ADR 0076)

BGM forms now translate native arguments, state codes and wait predicates onto
HostServices begin_audio/audio_preparation/submit_audio/audio_info. The adapter
owns no decoder/mixer/fader. Resource admission keeps the VM at its original
command until owner publication; failures do not acknowledge following arithmetic.
READY/pause/delayed resume, source/fade waits, WAIT_KEY return values, registered
name/position/volume and same-loop-name semantics have authored native fixtures.
BGMTABLE flags remain private native logical/save state.

An explicit resources alias can map each native music file to a client engine
asset. The ordinary empty-append profile also resolves native music names under
bgm/ using shared capability directory lookup and the fixed native sound extension
order. Decoded Gameexe BGM rows supply start/end/repeat frames, bounded at4096
entries. Additional append directories and native sound-pack entry selection
remain explicit unsupported resource policies. Unsupported opcodes/property fallbacks fail visibly.
ADR 0078 adds global PCM and SE below; ADR 0081 adds the bounded global KOE
profile. ADR 0083 adds the bounded PCMCH profile below.
ADR 0077 adds immutable
CrySirror resource handles: native PLAY/READY commits after resource and voice
preparation; RESUME after EOF/stop replays the retained resource without reopening
its original path. Stopped runtime snapshots reconstruct resources without hidden
playback. Handles are ephemeral; rebuilding a snapshot still needs the assets.

ADR 0078 used runtime Siglus payload schema 3; ADR 0081 advances it to 4
without automatic schema-2/3 migration.
Pending checkpoints replay uncommitted native
commands; active restores reprepare declarations at authored start. Original file
codecs remain intact, with BGM persistence translated through shared preparation.
No whole-game, every-save-variant, device or other-platform claim follows. See
[siglus-bgm-source-audit.json](siglus-bgm-source-audit.json) and ADR 0076.

## Original PCM/global SE and shared gain groups (ADR 0078)

Global PCM PLAY/STOP and SE filename/number/explicit voice-alias play, volume,
stop/fade, checks and waits now use shared audio resource/channel/group APIs.
`siglus.pcm` and `siglus.se` are client-owned gain groups; the adapter holds no
mixer, source buffer, decoder cache or private timer. Resource/voice completion
keeps original instructions pending; one-shot handles release at admission while
actual decoder leases retain bytes. Ordered semantic actions preserve cross-
category stop-before-BGM-prepare behavior on original file import.

Original integer-ms SFX end predicates use shared publication age, distinct from
physical source/effect EOF. Polling follows the current engine frame phase;
exact timer/device timing under starvation is not established. Decoded Gameexe
SE table count follows native 16/default, 8/min and 256/max policy. Every native
source needs a canonical resources alias; `koe/{number}` is an explicit CrySirror
mapping for SE voice-number calls, without automatic pack/filename discovery.
Missing/unknown operations fail visibly.

PCM/SE declarations entered runtime payload schema 3; schema 4 adds
KOE declarations (ADR 0081). Active snapshots
reconstruct after commit, without exact sample/age/DSP history. Original file
layouts remain unchanged; import stops nonpersistent PCM/SE and restores their
category volume while preserving host group parents/mute. PCM events, full PCMCH routing,
full KOE/MWND/mouth tables, native UI sound slots and full game/save coverage
remain open. Source audit: [siglus-sfx-source-audit.json](siglus-sfx-source-audit.json).

## Final audio worker dependency (ADR 0079)

Native semantic adapters continue using the same shared resources, transport and
channel completion contracts. Live App/desktop final DSP now uses shared CPU
workers; observations remain frozen at the previous owner publication while that
frame is pending. The host collects it before another VM/world tick. Backpressure
must not rerun the native instruction or consume another input edge. Restore
invalidates the obsolete frame and drops its PCM. This adds no legacy mixer or
new VM hooks, and does not establish exact native wall-clock timing or an
independent device clock. See [ADR 0079](adr/0079-worker-final-audio-frames.md).

## Host audio output ownership (ADR 0080)

Native channel resources, opcode waits and original save declarations continue
using CrySirror APIs. The output device is now owned by an independent host
worker, with separate readiness/failure/close observations and bounded stereo
PCM transfer. Native waits still observe logical source/DSP publication, not
hardware playback counters. No device handle is given to a native VM, and no
legacy engine audio runtime is introduced. See [ADR 0080](adr/0080-async-output-worker-and-bounded-pcm-transfer.md).


## Siglus global KOE/EXKOE (2026-10-27, ADR 0081)

Global voice operations now translate to shared prepared sources, Voice bus,
siglus.koe gain group and channel 3. The JITAN numeric processing belongs to
crysirror-audio and is also available to public Rust/JS/TS clients without this
VM. Configure resources explicitly, for example {"koe/12":"voice.wav"}. The
24-entry number/rate cache contains engine handles, with source bytes and
decoders owned by the shared bank. It is not a private native sound engine.

Historical no-trailing-operand KOE establishes message identity; newer trailing
read-flag operands fail before audio admission because MWND/read-skip bookkeeping
is missing. EXKOE named flags preserve the
message voice while controlling playback/wait/JITAN. The selected JITAN SYSCOM
config commands are mapped; replay switches alone do not implement backlog
replay. Native integer-ms completion and DECIDE down-up waits retain original
semantics within the shared frame timing model. Replacement stops the old slot
before preparing the new source; failure remains visible at the original command.

ADR 0081 introduced runtime adapter schema 4, restarting active logical cues through
engine preparation; prior schema 3 snapshots are not migrated. Original Siglus
save-file codecs/layouts are unchanged and restore KOE volume with playback
stopped. Native pack/path discovery, append-directory policy, MWND/KOE_ST,
backlog/menu replay, mouth tables/emotes and full character/system/BGM duck
routing remain unimplemented. This is a bounded global profile, not full-game
compatibility. Source scope and hashes: siglus-koe-source-audit.json.


## Shared moving acoustic geometry (ADR 0082)

Rust/JS/TS clients can now commit acoustic mesh poses/enabled flags or bind meshes
to owned World entities. Local BVHs, world surfaces, worker snapshots and sound
query/effect completion stay in engine crates. A native spatial/scene instruction
can translate onto these shared capabilities without private geometry tracing.
Mapping a native opcode also requires its explicit HostServices facade. This
phase adds no native opcode, resource-discovery or save mapping. The active
Siglus/RealLive/AVG32 profiles retain their earlier acceptance limits.


## Original Siglus PCMCH (2026-10-27, ADR 0083)

The original PCMCH command chain now targets independent scoped engine channels,
prepared sources and shared gain controls. CrySirror owns decoder leases, sample
fades, pause, delayed actions, EOF and completion. The VM holds native semantic
cue/volume/wait/save declarations and derived observations. Global PCM is separate;
unknown layouts/operations and nondefault native system/character/BGM duck routing
fail visibly. The ordinary default-system-volume PCM/SE category2/3 profile
resolves WAV-directory cue names through bounded shared ASCII case lookup, with
explicit aliases retaining precedence. Independent PCMCH script gain does not
inherit GLOBAL.PCM script volume. Character groups, BGM ducking, nondefault
native system gains and implicit dual PCM/BGM source reconstruction remain
explicit unsupported profiles. Numeric voice-pack sources still need aliases.

Shared transport_frames excludes fully paused/delayed input; preserve resume keeps
an ongoing pause fade. Stop-fade completion survives short input EOF without
prolonging source waits. Both capabilities are independently available to Rust and
public Audio clients. Runtime payload schema is now 5; prior schema 4 snapshots are
not migrated. Original sound file layouts stay unchanged, with true default slot
volume 255 and native loop/READY/delay restore predicates/order. No PCMEVENT, full
routing/discovery, sample-perfect runtime snapshots or whole-game result is claimed.
Source scope: [siglus-pcmch-source-audit.json](siglus-pcmch-source-audit.json).


## Shared direct Doppler (ADR 0084)

Spatial voices can opt into bounded radial Doppler, explicit velocities or App
world/camera inference, all in engine audio/motion APIs. Mixer workers own source
resampling and pitch progression; no VM has a private Doppler/resampling engine.
This phase adds no native spatial opcode or HostServices spatial facade. Existing
native two-dimensional audio does not opt in automatically and retains its
resource, wait and original-save policies. Per-path reflected Doppler and direct
propagation delay remain unimplemented; see ADR 0084.

## PCMEVENT shared prerequisites (ADR 0085)

The complete pinned PCMEVENT scheduler and selected native state/wait/save/source
functions have been reviewed. CrySirror now provides a shared owner-clock cue bank
through Audio/HostServices: ordered/weighted emission, interval/duration policy,
original modulo RNG option and async dependency receipts. No scheduler algorithm
is added privately to the VM. At ADR 0085 this was a reusable prerequisite, with native dispatch, RNG/source,
wait and save mapping still open. The following ADR 0086 section supersedes that
historical boundary for the stated mapped profile. See
[siglus-pcmevent-source-audit.json](siglus-pcmevent-source-audit.json).

## Original PCMEVENT mapped profile (ADR 0086)

The ADR 0085 prerequisite is now used by the original Siglus adapter. Native event
START/STOP/CHECK/WAIT/WAIT_KEY translate through shared HostServices cue handles
and prepared global PCM/PCMCH resources. The VM retains native values, save/wait
policy and portable frame continuation data. Scheduling/RNG/interval algorithms
remain in CrySirror and public Rust/JS/TS clients can capture/restore the same data.

Runtime payload schema is 6: pending preparation rewinds to a pre-selection cursor
and restores the already-advanced native frame once. Original files still restart
LOOP/RANDOM declarations and discard ONESHOT. No source/decoder/DSP history or
engine handle enters the original file. Explicit aliases/default PCMCH routing,
current unscaled native game-clock behavior and broader adapter limits remain;
this does not establish a whole-game result. See ADR 0086 and the updated source
scope/hash audit for exact acceptance.

## Siglus buffered input during audio admission (ADR 0087)

The ADR 0086 pending-input gap is addressed for the existing framed VK/DECIDE
profile. Incoming InputFrame data enters the shared Core/Scene InputInbox before
audio worker polling. Routing and acknowledgement happen once at a fresh owner
pass; resource completion first resumes the interrupted script. Runtime payload
7 preserves queued logical intents and that continuation; original file import
discards old backlog. Existing original file layouts stay unchanged. This adds
no raw OS/IME/analog input, extra native key mapping or frame-action ordering
acceptance. Other adapters' input admission remains further work.

## KAG3 over shared engine services (ADR 0088)

The optional `krkr` adapter now admits the bounded stock KAG3 profile described in
[ADR 0088](adr/0088-kag3-semantic-adapter-and-shared-prerequisites.md). The pinned
original TJS2 VM and KAG parser are unchanged. KAGParser.dll is an engine facade
with native callbacks/macros/control flow, not an empty plugin registration or a
hosted legacy engine. Stock system scripts are external inputs, never vendored.

Reusable additions are Core DeferredDispatch and RegionMap, shared simple glyph
placement, requested window policy/fullscreen, decoder opacity metadata and
worker-backed atomic capability writes. Native text uses ordinary retained
GlyphRun/Canvas operations; wait-glyph atlas cells use existing engine snapshots.
WaveSoundBuffer uses shared prepared sources/channels/mix groups. Input ingress
uses Core's journal and a resumable owner phase prevents replay during I/O waits.
Native positive hit tests call the original TJS onHitTest work-flag protocol;
disabled hits block fallthrough. These kernels/services have independent tests
without any VM.

The profile does not claim arbitrary KAG games, full native save stacks or VM
snapshots, XP3 auto-path mounting, nonuniform pixel-mask queries, focus/modal
input, video playback, nonidentity gamma, grayscale/blur/flips or every blend/
copy operation. Unloaded VideoOverlay declarations and identity gamma admission
are explicitly not completed media/effect integrations. See the source review
pins/hashes in [kag3-source-audit.json](kag3-source-audit.json) and separate actual
compilation/behavior records in STATUS.md.

## Shared plugin graphics and UK2-derived audio capabilities (2026-10-27)

ADR 0089 extracts the pinned UK2 pure OPNA/PMD DSP into shared crysirror-audio.
Any adapter may prepare .opna/.pmd/.mmm sources through existing HostServices;
no UK2 VM integration is added by this phase. No Kira/device/ROM is imported.

ADR 0090 adds shared image processing and native KiriKiri adjustGamma,
doGrayScale and flips on loaded/processed/uniform images. LR/UD flips also
reflect Core province regions. Operations execute on shared workers; original
TJS resumes only after individual image-resource publication. Nested callbacks
without a resumable boundary, unsupported blend profiles and composed Canvas/
text readback fail. Legacy script reads and image discovery remain synchronous.

Plugins.link('layerExRaster.dll') now installs the copyRaster engine facade,
using shared sine-row copying with full clip boxes and immutable source images.
Uncovered destination pixels are retained. Partial clipping and overlapping
self-copy remain unsupported. The modern vector/perspective/transition APIs are
available independently; this phase does not advertise arbitrary third-party
DLLs, complete layerExDraw/GDI or KaichoTrans compatibility. Native errors and
remaining save/media/clip limitations from ADR 0088 still apply.

## HXV4, JSON and bounded LayerExDraw (2026-10-28)

ADR 0091 adds shared authenticated HXV4 resource parsing/filtering and optional
labelled PE RCDATA startup resources. ArchiveMount owns requested-name hash lookup;
KiriKiri storage adds only a scoped naming fallback. No binary plugin or second
engine is executed. An archive's corruption-sentinel startup remains distinct
from its executable's actual startup. tools/probe_krkr.py reproduces the offline
resource/native probe without editing original game files. Native bootstrap/bres
operations remain real compatibility requirements and cannot be skipped.

ADR 0092 maps json.dll Scripts methods through shared bounded document values,
read streams and atomic writes. ADR 0093 maps a solid-brush linear/rectangular
LayerExDraw profile through shared vector geometry/bounds and image workers.
Appearance/Path/transform state is native semantic data; the adapter has no private
rasterizer. Pen/gradient/font/metafile and unsupported blending/AA semantics fail.
The local 39-DLL inventory is not an executed dependency list or a claim that
those DLLs are all implemented; see docs/krkr-plugin-audit.md.

## game-kr continuation profiles (2026-10-28)

The KiriKiri Motion ResourceManager/Player/LayerAdaptor profile maps `.mtn`
resources to shared `crysirror-puppet-eluna` decoding, clocks and worker sampling.
Immutable textures and validated PuppetFrames are published to engine-owned
layers and drawn by the existing PuppetRenderer. Rectangle mesh clipping and
UV/color interpolation are public puppet capabilities. Complex masks, action
tags, feedback, timeline mixing and physics remain unsupported.

`Array.save2` uses shared text-line encoding and asynchronous atomic writes;
`fstat` read/query methods use scoped VFS metadata. Missing archive timestamps
are absent rather than invented; unimplemented filesystem mutations throw.
Native `crossfade` with children uses the engine isolated-subtree API (ADR 0094),
then performs original-object exchange and one synchronous completion callback.
Unsupported transition handlers and clocks fail explicitly. Engine GPU tests and
headless native tests are recorded separately in STATUS.md; neither establishes
that a complete game or every DLL works.

Shared MP4 AVC/HEVC/AV1 video decoding is independent of this native VM and uses
existing Media workers. It does not imply complete krmovie protocol or MP4 audio
support. Codec versions, profile boundaries and sources are in crysirror-video.

## First-class RFVP and installed Siglus resources (2026-10-30)

The RFVP portable package is retired: original bytecode/syscall/save policy is
maintained in `crysirror-rfvp-vm`, called by optional `crysirror-rfvp`. Its process
GLOBAL is removed; opcode globals, global/slot saves, text speed and volatile
palette color belong to the owning VM. Shared `crysirror-text` resources provide
font/glyph layout, ruby/inline bitmap tokens, outline/shadow and reveal surfaces;
shared image/resource APIs provide NVSG decode, channel conversion, tone, borrowed
blits, coverage effects, output-history LZSS and cyclic masks. The original software
rasterizer/mixer and unsafe private pixel-storage implementation are removed.
Native rendering submits engine images and individual mesh draw declarations.
Native audio uses scoped groups/channels and observes engine source EOF. These
capabilities have standalone tests; a title screenshot alone is not acceptance.

This is still a bounded integration. Previous-frame and mask-wipe snapshots now
use the shared `crysirror-puppet::cpu_snapshot::MeshSnapshot` API, with immutable
engine textures and a CPU reference for the engine's indexed mesh/blend contract.
It does not host a legacy renderer. GPU capture parity, live asynchronous capture,
native movies, PCM streams, complete font selection, framework snapshots and
whole-game/save coverage remain unsupported or unproven. Native legacy picker,
menu-shell, cached-title and exit-dialog UI have no backend and fail explicitly. RFVP text,
scenario/image admission and native audio preparation do not yet provide the full
ADR 0050 async dependency boundary. Source provenance/patches are recorded in the
first-class VM SOURCE.json and shared-native-capability-sources.json.

Siglus config can name a private capability-relative `key_file` containing
caller-owned executable/scene/Gameexe decode material. Gameexe.dat is decoded
through the shared bounded LZSS/cyclic-mask kernels; START_SCENE/label and screen
metadata choose boot behavior. No user key tables or game bytes are shipped.
The original Windows `system.check_dummy_file_once` has a void three-argument
ABI. On the owner-selected CrySirror native host it is explicitly inapplicable,
as described below: no DVD is required and no successful authentication result
is fabricated. Current Rewrite probes continue into `sys10_ginit00`; subsequent
unmapped native operations remain real failures, not completed game support.
Their earlier utility-scene/blank-frame exit is not a successful game boot.

The original Siglus interpreter remains in `crysirror-siglus-vm`; its existence
is required, not a hosting violation. Shared Runtime/Image/Audio/Video/Puppet
services no longer depend on the old `siglus_scene_vm` or `siglus_assets`
packages. Grayscale wipe generation, NWA decoding, OMV plane conversion,
MPEG probing and the E-mote player helpers belong to the corresponding engine
crates. The transition shader belongs to the shared wgpu transition compositor.
The five pure codec libraries are first-class `crysirror-codec-*` crates;
standalone players and device dependencies are excluded. This extraction does
not imply that Siglus's still-missing movie, E-mote, wipe, complex MWND or
system-form adapters are implemented. Origin hashes remain recorded in
`shared-native-capability-sources.json` and each codec's `SOURCE.json`.


## Native Media and additional first-class profiles (2026-10-30)

Optional desktop features `rfvp`, `bgi`, `nscripter` register first-class native
adapters. They have no hosted legacy renderer. Their registration and config
metadata are not claims of full native-game support.

RFVP Movie declares shared Media playback and observes leased engine pictures
and EOF, retaining its own native movie slot/layer semantics. The shared
ASF/WMV/WMA and OMV/Theora/Vorbis paths decode on existing engine workers.
Save/load failures propagate; a missing thumbnail backend or a header-only
save is not acknowledged as a restored state. Real testcase title, NewGame
movie/CG, slot-1 write/thumbnail and cold-load results are recorded in STATUS.
Legacy shell/dialog UI, full font/PCM profiles, live Movie save reconstruction
and full async native resource preparation remain incomplete or unproven.

Siglus GLOBAL.MOV PLAY/WAIT/WAIT_KEY/STOP uses those same Media services,
with engine Image layers and native EOF/DECIDE/CANCEL returns 0/1/2. It has
no second movie clock or decoder. Its authored SCN/WMV/WMA fixture runs
through the desktop. Rewrite HF/Plus now continue past the explicit optical-host
compatibility policy. Native OBJECT.CREATE_MOVIE/CREATE_MOVIE_LOOP declarations
and pause/resume/auto-free policy now bind the same shared Media service. The
current ordinary FRONT object/CHILD profile waits for validated decoder metadata
before subsequent GET_SIZE queries, publishes only actual engine picture
receipts as Image layers, and releases retired/auto-free resources with Remove.
The engine owns decoding, audio and playback clocks; original VM state only
mirrors metadata/position/EOF. Shared metadata duration remains optional. Native
ordinary-stage movies select samples from existing local_game_time or
local_real_time through Media.setClock. Ctrl/read skip accelerates only the native
game domain by32; ordinary stage time-stop and the object's own pause are combined
as shared Media pause/resume policy. No private playback timer is advanced by the
VM. Shared external-clock playback currently requires a silent source: a decoded
audio track fails before PCM or the first picture until audio-master clock mapping
is implemented. Both installed Rewrite title-logo sets were structurally checked
as Theora-only Ogg streams. Native MWND/EXCALL movie locations, movie waits/seek/
end-loop changes and live Movie save reconstruction remain explicit unsupported
profiles. This implementation does not itself establish that a real Rewrite title
is working.

BGI is a bounded native-bp-v1 executor plus a semantic adapter, derived from
the owner's ethornell-rs `0cfff1d274ecc4554d0af8af1c7b1c7908595e96`.
Native BP uses shared Canvas/Layer/GlyphRun, worker-backed resource/audio
admission and engine time/input. It is not the old complete BGI VM/App moved
under another name. Scenario/Buriko, archive search, complete heap/scheduling,
original saves and numerous native calls are unsupported. No real BGI game
has been verified in this batch; its independent authored BP fixture is
separate evidence. See the two crate SOURCE.json files and adapter README.

NScripter plain-nscript-v1 interprets original text scripts and nscript.dat
through a first-class Rust semantic layer. Control flow, variables, input,
plain text/reveal, basic image/sprite drawing, immediate print, selection,
caption and audio call engine services. Source files are decoded/parsed on
shared workers; XOR uses the common Resource transform. Unknown commands
fail at the original PC. Complex windows/markup/transitions, archive mounting,
numbered script concatenation, skippable delay and original saves remain
unsupported. Framework snapshots are explicitly separate. The reference is
ONScripter `848a60892e4d721459e8d1051e1b8fe45164c3e9`, GPL-2.0-or-later.
No real NScripter game has been verified in this batch.

## Circus bounded native execution (2026-10-31)

DC4PHDL contains Circus offset2 MES version 0xabb6, CRXG images and XPCM
sound/music resources. `crysirror-circus::Program` validates the native header,
operand widths, signed words, Shift-JIS/shifted strings and 0x77 argument
continuations. It preserves instruction boundaries and label metadata, with
strict byte/string/instruction budgets. The first-class native interpreter is
registered by desktop feature `circus` for ABI0xabb6; ABI0xaab6 remains parse-only
and fails explicitly at execution. Detection mounts the original game directory
and selects its native1280x720 profile without unpacking or bootstrap files.

The default engine image registry recognizes CRXG compression 1/2/3, returns
straight RGBA and exposes native origin metadata independently. Shared audio
recognizes XPCM mode 0 PCM and mode 5 Ogg using a seekable payload view and
its existing bounded packet decoder. XPCM spectral modes 1/3 are unsupported.
Local tests parsed 536 original scripts, decoded 1339 original CRXG images,
and opened/read/seeked 148 PCM and 52 Ogg XPCM resources. No game data is
shipped. Native execution additionally preserves byte/value banks, original
labels/conditionals, resource and audio waits, names/system preferences and
read/save availability policy. Shared Image/Layer/Canvas/Text/UI/Audio/VFX
services own pixels, controls, transitions, sound and falling sprites; the VM
contains no private renderer, mixer, particle loop or pointer coverage sampler.
The native menu maps its20-tick movement and15-tick hover scale onto reusable
Layer/UI motion tracks, and its coverage onto immutable shared widget masks.
Actions wait for retained engine UI events and resolve real MES labels; a raw
click cannot skip process11 into the following bytecode. This menu mapping has
independent native/shared tests and requires installed-game verification before
a title claim. Unsupported navigation, native text/animation modes and original
slot restoration fail visibly. Current registration and earlier resource/startup
runs do not establish full game compatibility.


## Siglus portable-host policy and shared text coverage (2026-10-30)

The owner explicitly selected `disabled-for-native-host` for the original
Windows optical-disc environment check. Native SYSTEM form 92.2 still consumes
and validates filename/key/code, records its once identity, and has no return
value. CrySirror does not run SiglusEngine.exe, create dummy/env files or report
a successful DVD authentication. Diagnostics expose `optical_disc_check` with
`applicability: not-applicable` and `authentication_performed: false`. Framework
snapshot schema 11 stores and validates this host policy, its native once
state and suspended EXCALL caller waits. It preserves wait ownership across
async preparation/rollback; the active native continuation belongs to the VM.
Earlier STATUS failures document the superseded host policy and must not
be interpreted as a current DVD requirement. Original native-save compatibility
is a separate feature.

Real unchanged Rewrite HF/Plus scripts now pass `sys10_ginit00` and enter
`sys10_tt01` / the `$FaMain` callback. Their native
character voice groups use Gameexe CHRKOE membership/defaults and shared Audio
voice gain/stop operations, including overlapping groups and EXKOE on/off policy.
Game-defined extra switches/modes remain per-VM saved script policy. Explicitly
unsupported PCMCH character/category routing is not silently accepted.

Shared GlyphRun coverage adds the optional `blur.kernel` values
`weighted_blur` (backward-compatible default), `disk_outline` and `square_face`.
Text owns the bounded mask algorithms; wgpu owns individually cached glyph
resources. Outline/face require extent 0..16 and level 256. Native Siglus maps
font bold and the four none/shadow/fuchi/combined modes onto ordinary shared
GlyphRuns with native colors, placement/order and shared reveal state. No whole
message or screen renderer is hosted. `examples/desktop/glyph-effects.json`
exercises these APIs independently of any native VM and reads a runtime font.
The shared glyph GPU assertions and authored data/native decoration projects
have passed offscreen Metal validation; those captures are not Rewrite title
evidence. Native frame-action policy uses the existing original callback VM,
while its integer Counter and IntEvent timing mechanisms are shared Scene APIs.
Callbacks execute within the same capability I/O retry/rollback fence as normal
instructions, so resources requested during a callback cannot escape into a
process-global filesystem or consume a second input/timer tick on retry.

Native object-list allocation, bounded CHILD trees and PCT/RECT payload state
map to retained shared Image/Solid/Layer resources. Transparent groups represent
native child-transform origins separately from each payload's texture/pivot
center. The current tree profile is 2D, default/none child ordering with zero
child order offsets, and plain parents; inherited 3D/filter/blend state,
non-default tree sort scopes, button interactions and other payload types fail
explicitly. Original X/Y/Z/TR offset-list values remain native policy over the
shared IntEvent implementation. Ordinary property events (opacity, position,
scale, rotation and color targets) map native SET/LOOP/TURN/REAL/END/CHECK onto
that same shared curve type; layer publication samples its current totals,
and static construction initializes the same event backing. Event waits,
parameter slots and offset-event-list suboperations remain unsupported. Native
window-mode preferences publish shared
UI fullscreen requests, and CHECK_ACTIVE observes the host's actual optional
focus value; an offscreen host is explicitly active without claiming a window.

The bounded WAKU pointer-button profile now uses shared Widget interaction
and ordinary engine Image layers. Native Gameexe TYPE/CALL/ACTION/SE tables
select callback policy, image cuts, visual offsets/brightness and Audio requests.
The PCT backing rectangle is the stable pointer target; ACTION.PUSH visual
movement does not move the held gesture. Generic Ui WidgetSpec exposes
interaction_only and keyboard_focusable, and UiInteraction exposes a repeatable
activation_serial edge. No private hit tester, UI renderer or event interpreter
is embedded in Siglus. Rotated/alpha-tested/nested buttons and unsupported
native action profiles still fail visibly. Unsubmitted UI intents are retained
across async preparation; framework snapshots currently reject a pending intent
rather than discard or duplicate it.

Native EXCALL admission suspends its caller's VmWait and restores it at the
original VM process-pop boundary; these continuations are VM transaction data
in schema 11. Quota exhaustion resumes the same native SCRIPT proc, rather than
ticking frame callbacks and replaying input midway through an unfinished proc.
Flag-table transaction checkpoints share immutable table data and copy only on
write; the native record and pre-COW Vec wire shape remain compatible.

Real-game title completion still requires separate behavior evidence. Authored
native button/wait/snapshot callbacks and generic UI tests have passed, while
rich/vertical layout, wipes, other object profiles and original-save
reconstruction are not implied by that verified subset.

### CatSystem2 native KCS foundation (2026-10-31)

`crysirror-catsystem2` is a first-class original KCS v2 VM and thin semantic
adapter. It does not host an original engine or reuse a composed framebuffer.
The original configured KCS entry is parsed and stepped; substituting a CST
scenario for the KCS boot is not accepted. `SOURCE.json` records inspected public
source pins and the owner module SHA256 used only to guard reviewed native ABI
semantics. Unknown ABI, opcode, native call or native menu command fails visibly.

Shared `cat-system/int` archive loading can derive the encrypted archive profile
from capability-scoped PE V_CODE2 resources. Shared `resource::pe_menu` parses
bounded standard MENU v0 resources; the adapter translates original menu IDs to
engine-owned retained UI trees and maps enabled state through public UI commands.
Shared `resource::xml` owns bounded immutable XML element/attribute/text parsing
and path lookup. CatSystem2 retains only original numeric XML handles, native
format/address/default/close policy, and pending-call completion. Task admission
precedes XML source reads and CPU parsing; only the owner publishes completed Arc
resources and native handles. Original logical addresses never become pointers.

This is partial implementation. Parsing all original containers/scripts and
reaching a native frontier does not validate a title screen, text/image/audio
playback, CST/FES execution, original saves or complete game compatibility.
Observed behavior and compilation results are separately logged in STATUS.md.
