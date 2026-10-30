# ADR 0098: Client-scoped GPU mesh frame history

Date: 2026-10-30
Status: Implemented; validation evidence belongs in STATUS.md

## Problem

Native RFVP `Sprt(-1)` references the previously presented logical game frame.
Its original GPU renderer copies a render target. The first CrySirror adapter
instead captured a shared CPU mesh snapshot and uploaded the resulting pixels on
each feedback frame. This preserved behavior but added a full-frame CPU loop.
An adapter-private framebuffer or a hosted original renderer is not acceptable.

## Decision

The core declares `MeshFrameHistory` and its provider as client-scoped host
capabilities. Reserving a layer returns a logical engine texture path; no GPU
object, pointer or executable crosses the VM boundary. Runtime publication checks
that each referenced slot was reserved by that same client.

The wgpu presenter owns target/history textures, copies and queue submissions.
All referenced sources are frozen before rendering any new target. This works
for mutual references and for unchanged/hidden source layers. An undrawn source
starts transparent; explicit readback has no completed picture yet and fails.
Normal feedback has no buffer mapping or device completion wait. Declaration
identity plus ordinary resource revisions prevent repeated extraction/capture
from recursively advancing feedback, while mutable image updates remain visible.
A new native publication uses a new immutable declaration.

The desktop owner advances registered layers at each logical tick, including
headless ticks. Final capture reuses that renderer. Completion is queue ordered;
only successfully submitted mesh draws publish the completed output identity.
Dropping the last client capability releases its slots. Admission bounds are
eight slots per client, 64 clients and 128 MiB of logical target/history storage;
existing renderer cache and transient target budgets still apply.

RFVP keeps only native texture IDs and `Sprt`/wait/save policy. Hosts without this
capability retain the shared device-free CPU reference implementation. The native
VM has no dependency on wgpu, winit or an audio device. Engine-only GPU tests can
exercise feedback without RFVP or game resources.

## Limits and remaining async work

Explicit native save/thumbnail readback remains a blocking, bounded GPU completion
fence. Its pixels are engine-owned; it is not the normal feedback path. A future
asynchronous native capture-completion API must preserve the waiting instruction
and save state, rather than treating pending pixels as blank or successful.
Movie warm-up currently uses the shared CPU reference path. No window/presentation
rate, other GPU backend or hardware performance is inferred from headless tick
throughput. Provider capabilities and GPU state are never serialized as saves.
