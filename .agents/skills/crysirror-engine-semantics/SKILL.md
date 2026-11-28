---
name: crysirror-engine-semantics
description: Maintain native engine ABI profiles, opcode and syscall semantics, and readable implementation tables before mapping behavior to CrySirror capabilities. Use for VM parsing/dispatch work, native version compatibility and assessing whether shared API changes break existing consumers.
---

# Native semantics before capability mapping

Read the repository's `AGENTS.md`, [onboarding contracts](../../../docs/ENGINE_ONBOARDING.md)
and the engine's native reference, source record and dispatch catalogues.
Continue its mode and matching findings. Use
[research](../crysirror-engine-research/SKILL.md) or
[original analysis](../crysirror-reverse-engineering/SKILL.md) for missing evidence.
Matching official material is preferred; without it, original-program reverse
evidence governs. Unofficial implementations, including our own, remain fallible.

Start by choosing and recording the supported ABI profiles. Reverse and
document the full native dispatch surface for each profile before game
behavior acceptance. Use documented native behavior to choose or extend
CrySirror capabilities. Reverse and implement reviewed families in batches
with synthetic tests when useful.

## 1. Identify native ABI/profile boundaries

Record stable profile IDs, architecture, engine revision, script/resource
versions, and the pinned source or original-binary evidence. Record the
evidence used for runtime selection separately from provenance. Use structural
ABI checks for compatible releases and require evidence for any exact-binary
constraint. Native ABI, CrySirror API and project/save schema versions are
distinct identities.

| Compare across profiles | Record |
| --- | --- |
| Instruction encoding | Opcode widths/numbering/remaps, operand order/width, signedness, endianness, strings/values and PC/jump units. |
| Execution/value ABI | Stack effects, calls/returns, ownership, arithmetic, callbacks, destructors and faults. |
| Native calls | Module/receiver, ID/name, overload/signature, arguments/results, subselectors, ranges and dynamic registration. |
| Continuations | Wait completion, pending PC/arguments, input, clocks and retry side effects. |
| Persistence/resources | Original-save versions, identities and restoration. Resource format compatibility does not establish VM ABI compatibility. |

Choose the smallest evidenced distinction: share a proven compatible contract,
add explicit profile guards/remaps for real differences, or separate decoders/
dispatchers where encoding or semantics diverge. Avoid per-title forks for
equivalent ABIs and guessed common handlers for incompatible ones. Record
selection evidence and supported/parse-only/unknown profiles. Ambiguous profiles
need explicit selection or a visible error; do not retry a different decoder
after partially executing instructions.

## 2. Maintain opcode and syscall inventories

Inspect complete dispatchers, registration and called helpers. Qualify native
identities by ABI/profile and module/receiver. Equal numeric IDs or names in
different namespaces do not identify one operation.

For each operation record operands/guards, stack/result effects, ownership,
state writes, branches/returns, errors, cleanup, waits and resume behavior.
Follow constructors/destructors and indirect calls that affect it. Preserve
original explicit no-ops with evidence. Empty implementations, error callbacks,
allowlists and operand-only decoders are not complete native operations.

Use the full native dispatcher and registration tables as the implementation
denominator. Include operations unused by any sample and record their valid
operand forms, subselectors and native failure/default branches. Count
aliases, confirmed no-ops, reserved selectors and independently executable
operations according to the native dispatch evidence. Use script scans and
replays to measure exercise coverage separately.

Connect a syscall to the active dispatcher only after implementing its full
contract for that ABI. Keep unfinished work isolated, keep the syscall visibly
unsupported, and record the gap in the inventory. Do not publish a partial syscall
handler or classify it as `Partial`. A native no-op or rejection is complete
only when the original behavior is established and reproduced.

For `integrated` runtimes, inspect and retain native dispatch catalogues inside
their complete maintained modules; do not replace the runtime or change modes.

## 3. Document the contract before binding it

Follow [native developer references](../../../docs/native-api/README.md), using
[Circus](../../../docs/native-api/circus.md) and
[System 4](../../../docs/native-api/system4.md) as layout examples.

| Section | Required content |
| --- | --- |
| ABI/profile matrix | Identity/evidence, selection, compatible regions and version-specific differences. |
| Opcode table | Profile + ID/name, purpose, operand/value contract, execution route, restrictions and gaps. |
| Syscall/subselector tables | Qualified identity/signature, purpose, ownership, completion/errors, native implementation and remaining branches. |
| Capability mapping | Native family, shared API or integrated host seam, connected behavior and missing capability. |
| Evidence | Source/original identities separately from implementation, compilation and exercised game/platform paths. |

Maintain ABI/service prose above generated markers and regenerate affected
source-driven tables after dispatch changes. Branch counts are not native
completeness percentages. Preserve full guards and dynamic namespaces; generic
handler names and absence of rejection text do not establish reviewed semantics.

Reuse maintained runtime catalogues for integrated engines, linking their exact
revision and keeping upstream/local-change records current. Avoid a second
contradictory opcode specification. Native findings belong beside code and in
its developer reference; STATUS.md is not the specification and raw dumps are
not readable API documentation.

## 4. Map semantics to existing capabilities

For native save/load operations and versioned serialized layouts, continue with
[native saves](../crysirror-native-save/SKILL.md) after documenting their ABI.
Parsing the file alone does not implement its resource-dependent restoration.

After recording the native contract, inspect a shared service's actual inputs,
results/errors, ownership, async completion and limits. Classify the mapping as
an exact match, thin semantic translation, missing capability or unsupported
native case. Similar names alone are insufficient; review rounding/blending,
input, clocks and resource/save lifetime where relevant.

For `adopt`, keep native VM/value/resource/wait/save policy in the adapter and
reusable mechanisms in shared services. For `integrated`, use the documented
host/output seam without extracting or rewriting its complete runtime by
default. Use [archive onboarding](../crysirror-archive-onboarding/SKILL.md) for
shared formats. Missing behavior must not become a silent approximation or
successful acknowledgement before native completion.

## 5. Assess API compatibility before changing it

Follow [compatibility assessment](../../../docs/api/README.md#compatibility-assessment).
Review Rust, JS/TS, JSON commands, adapters and affected platform consumers.
Compare signatures and behavior: defaults, errors, scheduling, ownership and
serialization can break callers without renaming a method. Added fields and
enum variants are not automatically compatible.

Prefer a compatible extension or explicit profile/versioned operation that
preserves the old contract. If a break is necessary, record before/after
behavior, affected consumers, migration and versioning before applying it;
update consumers in scope. Do not conceal a break as a parser fix or invent an
extra approval requirement just because compatibility needs review.

Regenerate reflective artifacts/fingerprints when declarations change. Document
Rust-only host contracts at their actual declarations. Review native ABI,
public API and save/schema migrations separately. Use scoped release checks
when authorized; a skill/documentation-only change needs no player build or
whole-workspace regression run.

Continue with [engine onboarding](../crysirror-engine-onboarding/SKILL.md) for
factory/launch, lifecycle, output and platform wiring. Report native inventory,
documented semantics, connected capability and observed execution separately.
