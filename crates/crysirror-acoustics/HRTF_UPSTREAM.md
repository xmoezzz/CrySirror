# HRTF source and data boundary

CrySirror uses the standalone `sofar = 0.4.0` Rust library, original revision
`303532bd609db2d74c1b9e7f7483666dad1210fa`, from
[andreiltd/sofar](https://github.com/andreiltd/sofar). The complete pinned source
checkout is the reference. vendor/sofar/SOURCE.json inventories original Rust
sources and the local HDF5 v3 inclusive-range fix. Cargo uses this audited local
patch; no second engine, mixer or device backend is imported.
The library remains under its upstream MIT/Apache terms and acknowledgments
(Tomasz Andrzejak; the libmysofa work by Christian Hoene / Symonics GmbH).
The standalone library is vendored with original notices. Only our synthesized
SOFA impulses are bundled; no measured human HRIR is distributed.

Reviewed integration boundaries: the SOFA public reader/options and its
coordinate, validation, resampling and interpolation behavior; renderer plans,
filter transforms, convolution/delay/crossfade state, latest-wins publisher and
retirement queues. The HDF parser is part of the dependency, not replaced
by a CrySirror format parser. Its internal expansion/allocation limits still
apply; engine input/admission limits are not a hard process-memory sandbox.
The optional upstream libmysofa-sys workspace member is not linked.

`HrtfDataset` adds resource validation, a portable prepared JSON input and the
engine coordinate convention. Native SOFA keeps sofar's interpolation and
resampling. JSON has explicit matching sample rate and chord-distance weighting
of the closest three directions; it is suitable for offline conversion and
synthetic tests, not a claim that synthetic impulses model a human head.
Both paths use the same sofar convolution, delay and crossfade implementation.

Sofar's reader logs warnings for some invalid/unsupported SOFA profiles and can
return a silent filter when lookup fails. CrySirror validates convention, axes,
array sizes, finite coefficients/delays and supported units before exposing a
prepared dataset. Only the documented SimpleFreeFieldHRIR/FIR profile is
accepted. It queries a fixed reference radius; distance attenuation stays in
the existing engine mixer. This is not a moving near-field propagation model.

Coordinate mapping: CrySirror listener-local `(right, up, backward)` maps to
SOFA `(forward, left, up)` as `(-z,-x,y)`. Listener rotations affect lookup;
source directivity and occlusion remain independent direct-path operations.

Global per-receiver delays use Data.Delay axes rather than the upstream
length heuristic when M=R=2. HrtfEarOrder defaults to conventional left/right record order, with explicit
right/left and receiver-position mapping. The tested CIPIC profile has receiver
coordinates opposite to its conventional response-channel order, so position
metadata is not used to silently flip the default.
Preflight rejects missing rate, unsupported endian/bit layouts and units, and
checks dimensions before resampling. Legacy object-header v1 and some generic
HDF5 group layouts remain unsupported.

Dataset decode, lookup, FFT planning, filter preparation and voice allocations
run as bounded workers. The mixer owns one stream per voice. A fixed partition
bridge handles arbitrary caller block sizes, adds explicit latency, and drains
natural EOF through FIR/ear-delay tails. Prepared filters pass through sofar's
publisher outside sample processing; retired filters reclaim outside render.
The stream retains its host reservation until buffers are actually dropped.
Existing mixer render/decoder/output scheduling has additional synchronous and
allocation boundaries; this does not claim the whole engine is real-time safe.

Validation uses authored impulses, analytic convolution and offline engine PCM.
The ignored SOFA integration test reads an explicit runtime path; running it
on a local measured profile is recorded separately. Measured SOFA data is not
bundled in tests, examples, project resources or generated API documentation.
The small fixtures under tests/fixtures are generated from authored constants
with tools/make_hrtf_fixtures.py (netCDF4/h5py, generation only).
