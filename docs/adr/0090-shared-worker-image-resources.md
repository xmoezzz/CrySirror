# ADR 0090: Shared worker image resources and native effect facades

Date: 2026-10-27
Status: Accepted for the bounded CPU-resource profile below

## Decision

Image algorithms belong to crysirror-image, not to a VM. Add reusable gamma,
grayscale, invert, flips, deterministic noise, separable Gaussian blur, sine row
sampling, vector fill/stroke, perspective projection and transitions. tiny-skia
0.12.0 provides the Rust vector rasterizer; CrySirror validates path/transform
budgets and converts through premultiplied pixels. Blur and projective bilinear
sampling preserve alpha correctly. Paths support line/quadratic/cubic contours,
affine transforms, winding and strokes. This is not a GDI/font/metafile emulator.
Mask and midpoint-blur transitions have an explicit engine definition, not a
claim of complete KaichoTrans compatibility. These operations produce CPU image
resources; they do not silently become renderer GPU shader passes.

Core exposes device-free ImageResource leases, ImageInput and owner-polled
ImageProcess completion through ImageMetadata::begin_process. Runtime reserves
its existing asset CPU worker pool and a shared 512 MiB processing budget before
admission. Inputs are immutable, scoped by client, and at most 4194304 pixels;
chains contain 1–16 validated operations and at most 64 KiB JSON. Encoded assets
are capped at 64 MiB each. Conservative scratch/output charges remain live until
worker/result release. Admission failure leaves published state unchanged.
Cancellation is checked between rows/passes and vector raster calls; a single
tiny-skia call or legacy image decode is not preemptible. Polling never joins a
worker. Ready/error results are retained. Dropping a task requests cancellation.

Asset inputs use capability-relative paths, expected dimensions and an optional
observed encoded-content fingerprint. Native loadImages records that fingerprint;
worker processing rejects a changed asset instead of mutating a later version.
Published resource inputs retain exact pixel snapshots. Metadata/discovery still
have synchronous boundaries; this is one stage of ADR 0050, not full-engine async.

Public Bitmap.beginEffect/beginTransition/beginRowCopy return bounded scoped job
tickets. effectStatus publishes only when client, runtime epoch and destination
revision still match. cancelEffect releases jobs. JavaScript awaits the same jobs
through applyEffectAsync/transitionAsync/rowCopyAsync, never a second implementation.
Transition/source/mask inputs remain snapshots if their original slots change.

## Native mappings

The optional KiriKiri adapter serializes native parameters to this shared service,
parks the original TJS stack at a resumable boundary and resumes only after owner
publication. adjustGamma, doGrayScale, flipLR/flipUD admit ordinary straight-alpha
or opaque loaded, processed or uniform images. Flips also call Core RegionMap.flip
for interaction provinces, preserving overlap order. Composed Canvas/text readback,
unsupported native blend/clip operations and nonresumable callbacks fail visibly.

layerExRaster.dll is now a real native facade for copyRaster. It preserves the
reference full-image sine displacement/truncation/phase and uncovered destination
pixels, through shared SineRowCopy. Inputs are shared image descriptors, never raw
native buffers. Native size mismatch and zero lines/cycle are no-ops. Overlapping
self-copy and partial clip profiles fail instead of claiming snapshot equivalence.
No C++ plugin, alternate renderer or legacy composed screen is loaded. Other
plugins can map only when shared capabilities and exact native semantics exist.

Individual processed images are published through existing HostServices surfaces
and ordinary Canvas Image paints. This is a reusable image operation, not hosting
a legacy framebuffer. Derived bitmap resources/job receipts are not original VM
saves, and no save-compatibility extension is claimed.

## Verification

Independent image pixel tests and public API tests require no native VM. Native
bridge tests exercise a busy shared pool, retained TJS continuation, gamma/gray/
flip output and actual Plugins.link/copyRaster execution. The self-authored
examples/shared-effects scene uses public asynchronous APIs and ordinary layers.
Release compilation and actual Metal capture results are recorded in STATUS.md,
separately from implementation. No game data, fonts, ROMs or rendered assets enter
the repository.
