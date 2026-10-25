# Siglus resource port

Source: https://github.com/xmoezzz/siglus_rs
Commit: `5f358ef1360b0439e92aed2b749e8f25d8721954` (actual clean checkout).
License: MPL-2.0; full text is in the monorepo root LICENSE.
Source hashes are in upstream-files.json.

The complete runtime assets/g00.rs and assets/mod.rs files were read before
porting. The format crate's complete g00.rs was also inspected to distinguish
its parser from the runtime decoder. Only its fixed type-3 JPEG XOR transform
was carried into jpeg.rs. That table is a public format algorithm constant,
not a game-specific key or user credential. No fonts, games, keys, executables,
platform patches or SDK files were imported.

The runtime decoder is the reference for palette alpha, forward overlapping
LZSS matches, display cropping, image centers, original sizes, missing PATNO
slots and negative-length reused cuts. These policies stay in this crate.

Changes from upstream:

- Checked little-endian reads use crysirror-core::binary.
- RGBA size checks, red/blue conversion and bounded JPEG decoding use the shared
  crysirror-image service. Window/GPU/audio SDK dependencies are absent.
- Added 64 MiB input/decompression/surface/total-frame budgets, 8192 dimension
  and 4096 cut limits; invalid ranges and coordinate overflows return errors.
  Files exceeding this implementation profile fail visibly.
- The generic image plugin accepts only one frame without native pivot/crop
  metadata. Native objects must explicitly select frames and apply metadata;
  the shared renderer never guesses these semantics.

The original SceneVm, CommandContext, waits, forms/syscalls and original save
codecs are not replaced by a new interpreter or by the data VM. Their integration
is still pending; this port is an executable native-resource slice only.

CrySirror now also owns standalone crysirror-audio and crysirror-text services.
Those are engine implementations, not copied Siglus subsystems. Future native
integration maps audited native policies to them; see docs/ENGINE_SYSTEMS.md.
