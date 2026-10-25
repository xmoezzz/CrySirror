# Native adapters: engine capability extraction

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

The desktop default registers `crysirror-script-v1`, `siglus` and `reallive`.
The bare Runtime registers the first two; hosts explicitly register other VMs.
`siglus` is the independent subset in crysirror-siglus-vm; it is not a full
Siglus integration. CD_TEXT/CD_NAME fail explicitly. AVG32 is not registered.

## Acceptance

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
