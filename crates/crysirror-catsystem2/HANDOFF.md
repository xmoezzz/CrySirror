# CatSystem2 continuation — 2026-11-01

Read repository AGENTS.md, STATUS.md, docs/ARCHITECTURE.md,
docs/NATIVE_ADAPTERS.md, this file and SOURCE.json before editing.
Native integration is first-class VM plus thin semantic adapter. Generic image,
text, UI, input, layer, effects and audio work belongs reusable shared APIs.
Never execute the original EXE/DLL, host another engine, upload a legacy composed
screen, ACK unknown operations, distribute owner data/fonts, commit/push or add CI.
Only release cargo validation is authorized. Append own STATUS sections only.

## Current actual frontier

Owner resources: `/Users/xmoe/Downloads/game-nekopara-vol3/data`; startup chooses
original `kcs.int/main.kcs`, virtual1280x720, heap1048576. Module SHA256:
`f8a1fb4fed8a0c64c3fbc0bcf009f63e8f3181cfbc1a0dbd60f9ddefc8e25b3d`.
This module is an ABI static-evidence guard, not a generic game hash lock.

Latest completed behavior: `target/verification/catsystem2/native66-fes-owner.log`
and`native449-canonical-runtime-owner.log`, eachrelease1/1,20,227instructions.
Pendingnative447PC0x1926,args[8,406],text`flow`,nextPC0x1931. Class34 constructor
and449areadmitted;FesTask8hasimageList6/rect[0,0,3072,4096]/upload[0,0],
raw-stringbankowner3,cursorUnknown,playerunloaded,firstticktrue/lowbuttonNone.
Native71 executed twice and yielded/resumed false on subsequent owner ticks;
256,260,262,449executedonceeach;66executes4times. Four Core journal click-down/up and Ctrl-down/up
probes retain the same original PC/instruction count/pending native and exact
leased delivery across repeated polls; callbacks remain explicitly unsupported.
No successful input, Cat GPU/title/gameplay/text/audio/FES/CST claim.

Actual shared resources: native image-list6 owns a real3072x4096 transparent
atlas,50,331,648bytes, page256x256/count192. Native internal basePlane7 has a
real1280x720 shared Image payload, hidden, nativeZ/shareddepth1000. Task parent6
is distinct from render graph ancestry; transparent1x1 group never becomes the
native empty rectangle. Atlas/base paths are engine publications. The owner test
uses an actual helper Runtime for ImageMetadata/process/publication; outer
CPU UI/layers/router are separate test state, not a desktop GPU parity result.

## Implemented semantic boundaries

- Original KCS v2 first-class bounded interpreter, typed stack/value/address,
  call/return/native pending frames; unknown opcodes remain at sourcePC.
-393 retained engine MENU104 item119 disable;465 reviewed absent debug flag;
  native taskID68/virtualcanvas8; byte string operations13-22,36,41-42; shared
  XML590-593 handles, original transparent wrapper/default/close semantics.
-400 real17-entry BGM LoopPoint import, shared client AudioLoopCatalog; original
 44100reference frame and duplicate-start-only update policy. Playback unmapped.
-359/789 exact original constructor/startup/gamedef/clientSave setup overlays.
  Mode3 loose-before-archive matters. Real flag count2048. No Windows settings
  import; malformed XML fails rather than claiming permissive source fallback.
-209 live/backup flag banks/parser/256 command slots and original integer
  expression behavior210-213. Backup remains uninitialized until source copy.
  Unsupported tokens/types fail; command execution beyond reviewed subset partial.
- Shared Text byte_split preserves native SJIS lead/trail boundaries, delimiter
  collapse, NUL output/capacity/cancel; VM41 only maps owned buffer ABI.
- Type2 Plane, type6 click controller, type15 image-list constructors only.
  Plane79 real Solid payload;88 native global flat floatZ maps shared Scene
  depth state on payload only,0<=Z<1e6; capture upperbound65536 unsupported.
 92 imperative47/48 child messages, not permanent inherited visibility.
-938 genuine shared soundconf XML AST/found(5groups/6parts), not an audio mixer.
-247 scoped Save CSS header/tag/records + sharedzlib; isolated owner save absent
  genuinely returns0. Present-save apply/write/version behavior unimplemented.
-276/277 original numeric raw-byte string variables/dirty;278$str unmapped.
-104/105/848/849 actual shared InputRouting bind/enable/hover/drag flags.
  Full RECT8CDAD0=[-100000,-100000,100000,100000], not empty/global sentinel;
  event_mode5 is distinct from fixed input source bits7. Bind targetmain3 is
  distinct from INPUT service1. No fake focused keyboard Button.
-71/65A180→877390 event wait yields after validating16byte caller slot; actual
  Main tick5E84E0→877450→8762A0 next owner step pushes0, clears original flag,
  leaves slot untouched.5E8270 is destructor. Atomic Machine.resume_native_event
  supports tested message copy+true result, but adapter callbacks/keyslot/result
  mapping remains unsupported; do not wire synthetic key bits as native slots.
-256 actual first image-list initialization uses shared ImageJobs to allocate
  zero32bpp atlas/base, then owner publishes actual engine images/layers before
  native ACK. `68E6F0` is empty bitmap wrapper, not a directory scanner;
  `686E50` is two RECTs(page,dirty), not packing. `742660` tile dimensions derive
  startup TEXTURE size via offsets with1048byte native manager header. Source
  requires page definitions; first init only, reinit fails. Scratch remains empty.
-260/743E10 stores original uploadXY(task112/116,manager4/5), no rendering.
  Uninitialized coords are None until setter.262/msg286 writes124/120=value,
  nonzero sets140=1, zero leaves140 unchanged. Neither composes/shows the image.
  Native image entries/composition/dirty uploads/capture/resource replacement,
  auxiliary image/timer/sound tasks remain unsupported. Never copy743600 raster
  compositor into Cat; use reusable shared Image/Canvas/Layer operations.

## Shared engine work and ownership

Shared stable float depth: Scene layer/depth_order.rs canonical side-map,
LayerCommand::SetDepth{id,depth:Option<f32>}, ordinary before depth within major,
pure frame calculation plus accepted owner commit_frame_order; snapshots freeze
effective order. Runtime hook and public Layers.setDepth were added by root.
No automatic task parentZ inheritance for Cat flat native draw order.

Shared InputRouting: client receivers, signed stable priority, source filters,
half-open region/shared mask, press capture/release/repeat, per-receiver hover
marker, explicit Drag origin broadcasts, single leased delivery, strict ACK,
monotonic decimal-string receipt fields, restore fresh epoch/held baseline.
Automatic hardware drag detection and actual controller observation unfinished.
Core queries InputRouteDelivery/Receiver/Stats; Scene ServiceBatch.input_routes;
Runtime submit/query/restore; Engine lifecycle owns raw-input isolation before
behaviors and through async audio_frame/finish_frame. Mixed interactive UI/routes
explicitly fails before journal/clock advance; registering mid-on_update never
reoffers already observed edges. Root owns reflected InputRoutes facade/docs.

Other shared extraction: Text byte_split and Resource zlib per-chunk/prepublish
cancellation. Do not move these algorithms back into this VM.

## Reviewed class34 and next native447

5DD580 switch decimal34/0x22→627E50 constructor. Complete vtable8D2A90:
[628360,62DAA0,629850,5DC9B0,5DD130,5DD210,62AD10,62BED0]. Full static
Hex-Rays results and ignored scripts `ida_task34.py`, `ida_task34_parts.py` are
under target/verification/catsystem2. This is a native FES task, not Plane.
Constructor has empty job/object lists, buffers, playerNone, once flag1100=1;
attach62DAA0 and initial tick629850 query root msg177; message26 copies caller
buffer into448. On first tick629850 creates type6 `low button`, binds actual
virtual screen rectangle with depth-derived priority and enables it, even before
FES player is loaded. Later handler331 constructs755600 player and obtains
INPUT/SOUND via34/35; full FES bytecode/resource policy remains to review.
627E50's125 target416 is indeterminate, but COMPLETE routing resolved this:
5DC340 sets parent0/emptychildmap; before62DAA0 attaches,5DD130→5DD210
cannotreachINPUT. Self-target125 isunhandledby62BED0/base,flag0/nokids.
No targetbits can invoke7106F0 here;cursorstaysUnknown. Later actualINPUT125
canhaveheld-keyreleaseeffects andmustnotbeignored. Native66/26post-attach
setsmessage_parameter1192=0,removinganyself-queryscratchuncertainty.
Class34 constructor/attach/26 and449nowimplemented;release23/23library and
originalowner1/1pass(native66-fes-tests.log/native66-fes-owner.log). Newfes_task.rs retainsrealnative
FESstate,bankowner,params,rect,childrefs,unknowncursor,initgate andselected/exit
values;notanemptytaskorprivateengine.449bindsactualmanagerresource.size and
nativeimage-listuploadXY;manager743E00returnsfield2wrapper and68F5A0→68CAC0
readsitsbitmapwidth/height. No draw occurs. Main-parentprofileonly.
Firsttick629850 lowbutton isregisteredthroughsharedInputRouting onlybefore
KCSresumefromactual71;quota exhaustion/resourcepollsnevertriggerit. Source
5DE8A0 resets existingcontrollerresults first;newlowbuttonretainsbindresult1
untilnextnativetick.447genuineFESplayer/loadremainsexplicitunsupported.
Actualnewownerlogpasses20,227/native447PC1926,[8,406],textflow.
447requiresreal755600player/localbank512DWORD+9names,755D10loadstages and
331/332callbacks(755C20→62D130 mode3 loose-before-archive,not767240default
plainfilecallback). First flow has noMACRO/KEYBLOCK/DEFINE/OBJECT declarations;
SUBCOMMAND aliasmetadata andflowsections are genuine. ItsSTART beginsfourflag
assignments thensteam_isenable1002. Do notfakeplatformresult0orACKemptyplayer.
Source7692C0 has131command tableat8F4A58/functions8F4A78;steam_isenable785920
observes nullableB9BD9C manager andwritesactualavailability. Shared platform
backend/status isnotyetmapped. NativeFES sourcepositions/sectionrowmustremain
pending onunsupportedoperation;no skiptoLOGO/TITLE.


## Validation ledger

- Cat lib release23/23: native66-fes-tests.log.
- Actual original adapter release1/1: native66-fes-owner.log;earlierbootstrap-owner.log1/1.
- Actual canonical Runtime release1/1: native449-canonical-runtime-owner.log. All31INTs
  mountedinoneRuntime,sourcepublication0instructions/frame/time,then20,227PC1926.
  PublicAssets.surface resolvesactualatlasandforeignclientfails;canonicalLayer
  basehidden/1280x720/Z1000;Runtime.tick fourmouse/Ctrledgesretaincallbackerror,
  pending/instructionsandsameleasedreceipt. NohelperRuntimeusedinthisnewtest.
- Unified24actualheadless exits1atthesameclass34PC18eb beforefinalcapture;
  player24-original.log. NoPNGorGPU-pixel/titleclaim.
- KCS VM release12/12: native71-vm-tests.log (new atomic delayed event resume).
- Cat runtime-feature releasecheck: native256-check.log.
- Shared router release7/7: shared-input-routing-drag-tests.log.
- Engine lifecycle isolation release3/3: shared-input-routing-lifecycle-tests.log.
- Shared inflate cancellation release2/2: shared-inflate-cancel-tests.log.
- Shared depth4/4 and byte split3/3 previously passed; see STATUS.
- Python3.11 tooling201/201: python-tooling-311.log. Default system Python
  lackedtomllib; initial6importfailures retainedseparately,notcountedaspass.
- Root/NNN generated API docs8/8, fingerprint bd71afbaa37a19da; independently
  recorded. No new engine facade is required for256/260/262 native state.

Other immutable player artifacts:23 at
`target/verification/krkr-ctrl-exits/player23`, SHA256
`2c7a0ac497d068d207b846ddbaa63c0b4f173342a2d74a0077e28762c557caf8`.
It stops before new256 support. Immutable24 now contains256/260/262 at
`target/verification/krkr-ctrl-exits/player24`, SHA256
`f6cc7fc86de4ed35650c510ecdda07cc2007900f0876500727295da611ef8d98`.
24isfixed;25alsofixed(SHA4241833a6b0bfdfd9185df9277a7450264588403f7f52e2291da47b07106cab5).
CatproductionnowfrozenforKRKRunique26untilitsSHAisfixed. New Cat preparation_pending/program-publication
yield was added afterward; Runtime authored Cat+peer release test1/1passed
(bootstrap-preparation-tests.log). It proves0instructions/PC/frame/time during
source preparation andfirstnormalframeCtrl reachespeer. Actual owner regression
for this hook passed1/1inbootstrap-owner.log:prepared0instructions/PC,then
same20,218/class34frontier andfourretainedinputfailureprobes.
Coordinate GPU with root/KRKR/NNN; do not duplicate full LTO/player builds.

31 original INT mounted namespaces were previously validated, including26
scene.int duplicate CSTL rows: scoped INT aliases are accepted only when entire
stored payloads agree with known same archive key. Preserve actual ArchiveMount
coverage, not only raw index parsing. No user game bytes were added to the repo.

## Unwired FES VM draft — not validated/accepted production

After the23/23+owner447 checkpoint, began fes_program.rs/fes_loading.rs for a
bounded ASCII/no-object orchestration VM using shared Text split/Storage/XML/
zlib, not another renderer. They are intentionally NOT referenced by lib.rs
while unified26 builds. New Interpreter local-bank methods are backed up in
ignored target/verification/catsystem2/fes-player-bank-methods.rs.txt and removed
from production until26SHA. Reinsert block beforeInterpreter.diagnostics and
restoremod declarations only afterunfreeze;then release-check/unit-test before
wiring447. Currentqueuedfes-flow-vm-tests.logdoesNOTvalidateunwiredmodules.
Draftloader/VMneedsreview:SourceAliasdefaultcount9requirescolon<=19;store
nativealiasmetadatawithoutclaimingcallableeffects;explicitlyrejectMACRO/
KEYBLOCK/DEFINE/OBJECT/stringtemplates/plainFES/groupedtokenprofiles untilmapped.
SourceParser767050uses6A5910 ratherthan41's6A5AB0;the draft'sASCII/noquote space
profiledelegatessplittingtosharedText. Program.step EOF/budget mustnotbeconflated
withsuccessfulforcedtick:source7563A0sectionend restartsrow1withwait1 and_FRAME_
isbasedonfirstgame tick. SourceNative448(lookup table)→333 forces629850 tick,
so actualflowexecutionneedsconstructalllocalbank/namedstate,lowbuttonshared
registrationandcorrectscopedchildticks beforeACK,then preservefirstunknownFES
operation. Neverreturnemptyplayer/skipsteamquerymerelytopushnextscript.
Read complete ignored decompilations755600,755D10,765A50,766CE0,7676C0,
75AAC0,758C60,7597B0,7589E0,758F70,7690E0,7692C0,769F30,76A680,7563A0,
62D130,73FF40 and6A7510. Draftbankmethodscreate512locals keyedplayeridentity;
7286D0 creates/selects,existingkeyreturnsfalsewithoutselect;728800failedselect
leavespriorselection;728B70 namedvalues do notchangeglobalbackupinitialized.
