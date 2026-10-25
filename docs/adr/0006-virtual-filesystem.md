# ADR 0006: virtual filesystem providers and mount tables

Status: implemented (partial) on 2026-10-24.

## Requirement

R8: the filesystem is abstract. Native engines such as siglus_rs keep their
own filesystem semantics (archives, name rules, Gameexe/Scene.pck lookups) on
top of it.

## Decision

- `crysirror_core::vfs::VfsProvider` (no_std, read-only: `read`, `exists`,
  bounded `list`). It sees only canonical paths relative to its mount point and
  never sees a host path or a client identity. `MemoryProvider` is the first
  implementation.
- `DiskVfs::mount_provider(client, mount_point, provider)` overlays a provider
  on **one client's** asset namespace. Newer mounts shadow older mounts and the
  capability disk root. `NotFound` falls through, and any other error stops
  the lookup and is returned. Mount points are matched per component. Save and
  cache mounts stay disk-backed with atomic writes. The limit is 64 mounts per
  client.
- `Runtime::mount_provider` exposes this to hosts and plugins. Native VMs read
  through `HostServices::read`, so they see mounts without changing their own
  path or archive decoding. Engine scripts reach the same namespace through
  the `Fs` class.

A format-specific archive (for example an RFVP pack or a Siglus resource
archive) is implemented as a provider **inside its compatibility crate**,
after its format is audited at the pinned commit. The shared VFS does not
decode native archive formats.

## Evidence

- `vfs::tests::provider_mounts_shadow_disk_per_client_only` (runtime).
- `provider_tests::siglus_reads_its_scene_through_a_mounted_provider`: the
  original Siglus SceneVm loaded its `.scn` from a memory mount, with no
  file on disk.
- Not done: async/streaming reads, provider-backed `Assets` (renderer images
  still read the disk root, while Siglus uploads decoded pixels), archive
  providers.
