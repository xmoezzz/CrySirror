# ADR 0091: Authenticated HXV4 resources and explicit executable bootstrap

Date: 2026-10-28
Status: Accepted for the bounded static resource profile below

## Source and ownership

The owner's xp3-brute revision c82cb14302138125640478556723705aa720007b
provides the reviewed Special/index, hash, static bootstrap/KDF and reconstructed
content-filter algorithms. Adapt those pure Rust pieces into the existing shared
Xp3 ArchiveFormat. Preserve their MIT notice and source hashes in
hxv4-source-audit.json. Do not depend on the full recovery application or import
its CLI, Win32 host, Unicorn, GPU solver, renderer, codecs or ambient filesystem
search. No native PE/DLL is executed. The small FilterManager interpreter is a
bounded content-transform algorithm, not a second game runtime.

All access uses DecodeContext and explicit capability-relative companion names.
The public Resources service can prepare/index on existing bounded CPU workers,
mount after retained completion and expose readers to any client API. No HXV4
algorithm, keys, image composition or output buffer is private to KiriKiri.
The decoder is still partial: ordinary/chained/other protection boundaries from
ADR 0065 remain explicit. A source project claiming broad support is not proof
that every variant is implemented here.

## Resource profile

An explicit resource profile contains, for example:

```json
{
  "limits": {"max_entry_size": 268435456},
  "parameters": {
    "kirikiri/xp3": {
      "hxv4_executable": "game.exe",
      "hxv4_startup_target": "__bootstrap/startup.tjs"
    }
  }
}
```

The companion executable is read only through the same scoped VFS as the archive.
Static PE resources supply bootstrap inputs; candidates are accepted only after
XChaCha20-Poly1305 authenticates the Special payload and strict object parsing
succeeds. No title-specific secret is embedded in code or stored in project/save
JSON. Parameters select source/target names, not native function pointers.

The Hxv4 descriptor, offsets, flags, object counts/depth, lengths, directory record
shapes, duplicate IDs and hashed paths are checked. Limits include a 32 MiB
companion, 16 MiB Special/bootstrap, at most 64 PE/prefix/seed candidates, 262144
object nodes and depth 64. Caller index limits also bound Special inflation.
Multi-archive lookup slots, packed-executable normalization/emulation and unknown
static families remain unsupported.

Protected entry contents are reconstructed by the existing raw/zlib segment
pipeline. A bounded 64 KiB pass checks original and decoded Adler values before
publishing a reader. The protected flag is advisory: a plaintext bootstrap whose
original checksum matches is kept intact. Otherwise authenticated per-entry
state drives the reconstructed FilterManager; missing state or checksum mismatch
fails. Returned transformed readers apply the same state at logical read offsets,
without a second whole-entry copy. Zlib source buffers retain their existing
DecodeContext charges. A full verification pass is CPU work and belongs on the
resource-opening worker; this is not a constant-time cold-open claim.

Stored synthetic names remain opaque metadata. Archive::resolve optionally maps
requested names to authenticated directory/name hashes; ArchiveMount validates
canonical paths and calls the same reader. It does not invent a complete plaintext
filename dictionary. KiriKiri storage naming falls back to provider existence
within its existing capability root. Names/paths do not confer OS authority.

## Embedded startup and actual game probes

The tested local archive's ordinary startup.tjs is a plaintext corruption sentinel;
executing it produces an informational error and no game layers. That is not a
successful game boot or evidence that the owner's executable is corrupt.

When explicitly requested by hxv4_startup_target, the shared archive exposes the
exact statically decoded PE RCDATA/STARTUP.TJS bytes as a separate derived resource,
labelled with their origin. It never overwrites the archive sentinel. A project
can select that resource as its native startup. Subsequent native bootstrap/bres
operations must acquire real shared capabilities; no no-op bootstrap is installed
to silence errors. Native bootstrap DLL execution is not introduced.

The tools/probe_krkr.py offline probe uses public Resources mounting and a temporary
project beside the authorized game folder. The normal project containment check
remains enabled. Logs, state and optional captures go to the requested output
folder; the project is removed even on native failure. A zero process exit or
blank frame is explicitly not a whole-game compatibility assertion. The offline
mount is synchronous; realtime callers use beginDecode/decodeStatus/mountDecode
and must order native startup after publication. Automatic native archive/preload
startup scheduling is not added by this phase.

## Shared VFS additions

ClientStorage::open_read supports bounded-before-allocation consumers in asset,
save and cache namespaces; its default rejects before reading, while Runtime
provides actual capability streams. JSON
facades use that contract. VfsProvider::list_dir and ArchiveMount's bounded
directory index avoid repeatedly cloning/scanning a full archive when native
storage builds its index. The directory index has a 16 MiB admission cap and
preserves mount-prefix ownership. These services work without a native VM.

Container origin metadata is exposed by VfsProvider::container_source and the
client-scoped ClientStorage::container_mount query. The latter only finds an
already published provider; it never auto-opens an archive. Native file-media
current-directory/auto-path naming can preserve `file://./data.xp3>...` while
resolving through that explicit engine mount. Unknown/nested media, unmounted
containers, traversal, foreign roots and save-media default directories fail.
The native current URI is logical state, not an OS process working directory.
Worker archive publication forwards requested-name lookup through the same
SharedArchive wrapper; an independent public Resources/Image test covers this
path and container origin without a VM.

The actual embedded startup now passes its archive-directory request and reaches
Plugins.linkZ, a separate compressed-PE/bootstrap registration protocol. It is not
aliased to Plugins.link or replaced by a success stub. See
krkr-bootstrap-gap.md for signature and CompoundStorage requirements; neither
resource authentication nor a rendered sentinel is a completed native boot.

Compilation, unit/contract behavior, external 32-sample validation and actual
native startup outcomes are recorded separately in STATUS.md. No game data,
executables, DLLs, keys, fonts or generated captures enter tracked assets.
