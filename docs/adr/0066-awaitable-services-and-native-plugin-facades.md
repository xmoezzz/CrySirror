# 0066: Awaitable services and native plugin facades

Date: 2026-10-27. Status: implemented bounded service/menu profile; full KAG3 and
engine-wide asynchronous execution remain incomplete.

## Decision

Native engines and plugins supply requirements for shared engine capabilities.
The KiriKiri MenuItem native surface now maps onto CrySirror's retained
MenuSpec/MenuItemSpec. Tree validation, popup/bar layout, radio groups, mouse and
keyboard interaction, paint primitives and save state live in crysirror-scene.
The VM adapter owns only native handles, properties, Windows caption mnemonics
and event callbacks. Ui APIs use the same state without a native VM.
Ui.drainEvents consumes client-scoped queued events once; the events property
only reads them. Menu events share the existing retained-last-256 policy.
Snapshot validation includes menu, queued event and focus client identities.

Plugin registration distinguishes `engine_facade`, `registration_only` and
`unsupported`. `menu.dll` names the implemented native-to-engine facade.
Explicit `Config.plugin_shells` entries can register a plugin name for scripts
that only require registration. They introduce no fake callable exports.
`Plugins.getStatus` reports the distinction; `getList` lists linked names.
Unknown modules fail visibly. No native DLL, external engine or compositor is
hosted. A live menu facade cannot unload its class while native objects depend
on it; unlink returns false. Shells can unlink normally.

Original native popup's synchronous return/wait semantics and accelerator
mapping are not yet implemented: MenuItem.popup and nonempty shortcut fail
explicitly. Applications can use the engine's asynchronous Ui.openMenu and
menu_activate events. This does not pretend to preserve the native popup wait.

## Async ownership and completion

The shared OperationTable admits bounded client/generation-scoped work, retains
ready/failed results until released and supports phased jobs and short owner
publication. A rejected dependent phase retains its predecessor for a retry;
workers never wait for another task on their own saturated executor.

- Resources probes run on I/O workers; native image source opening, companion
  access and decoding run on CPU workers. Immutable ready pixels publish
  without an owner-side pixel copy. Client profile and host image staging
  reservations retain their lifetime. Mounted lazy entries still require
  worker-backed consumers in live applications.
- Text captures immutable fonts, locale, layout options, icons and reveal
  inputs. Parsing, shaping, icon decode and raster run on CPU workers. Owner
  publication and reveal events are ordered together. Failed replacements keep
  the previous block; stale generations cannot publish into a restored runtime.
- Save captures logical state and native VM checkpoints at an ordered
  safepoint. CPU serialization, I/O and validation use bounded phased jobs.
  Ready loads are prepared only; commitLoad stages native reconstruction then
  atomically swaps live state. Snapshot format and native unsupported-save
  errors are preserved. Capture/reconstruction are not claimed to be async.
- JS/TS service awaits derive signatures from the live registry. The script
  host captures the initiating client and restore generation, pumps bounded
  Boa jobs, fences async init/start and prevents pending-hook reentry. Hook
  rejection faults the App. Shutdown has an explicit bounded drain/cancel
  boundary; ordinary lifecycle frames never join these service jobs.

All ticket lifetimes, limits, release rules and the offline-only legacy methods
are described in [ASYNC_SERVICES](../ASYNC_SERVICES.md). This is further
implementation of ADR 0050, not completion of its full subsystem table. Native
VM interpretation and storage callbacks still need resumable worker/continuation
boundaries. GPU pipelines/uploads/readback, remaining audio/media preparation,
editor/MCP live operations and other subsystem gaps retain their prior status.

## Source and validation

Inspected KiriKiri native registration/menu conventions in the complete pinned
Kirakira reference `c451918023477620ff9055a2eb8a6714b947d9c1`.
The vendored original TJS2/KAG libraries and their hash manifests are unchanged;
no general engine implementation is imported. Full original KAG3 startup is an
external diagnostic at `1f3ab309106d210e3169bbbe0fb4e066ae463b42`; loading
menu.dll alone does not establish KAGParser/KAG3/game compatibility.

Independent engine menu tests, native TJS tests and blocked-worker service tests
separate shared capability acceptance from native registration. System-font and
external-source tests require explicit runtime paths and are ignored by default.
Implementation, compilation and actual behavior results are recorded separately
in STATUS.md; a diagnostic failure or ignored test is never counted as a pass.

## Local build footprint

Owner-requested validation uses `--release`, including tools/verify.py.
Dev/test profiles disable incremental compilation and debug information, after
target/debug grew to about 313 GB (165 GB incremental, 145 GB dependencies).
Only rebuildable debug output was cleaned; release output, verification evidence
and local game/reference assets were preserved. No GitHub workflows or remote
build automation are introduced.

Edited sources use full license notices instead of SPDX-only headers. Existing
third-party authors and licenses remain attributed. This is not a blanket
relicensing of the original VMs or ported third-party algorithms.
