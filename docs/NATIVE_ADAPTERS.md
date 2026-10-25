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

The desktop and default Runtime register `crysirror-script-v1` and `siglus`.
The latter is the independent subset in crysirror-siglus-vm; it is not a full
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
