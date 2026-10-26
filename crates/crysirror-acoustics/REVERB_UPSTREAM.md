# Late-reverb source and limits

CrySirror depends on the standalone DSP library
[FunDSP 0.23.0](https://github.com/SamiPerttu/fundsp/tree/8479a70d813d539c67a908f88e65f6b0b344122e),
original revision `8479a70d813d539c67a908f88e65f6b0b344122e` from the published
crate's `.cargo_vcs_info.json`. Cargo pins the crate exactly and its checksum is
recorded in Cargo.lock. Only the `std` feature is selected; optional file loading
and FFT convolution are disabled. It imports no game engine or output backend.
Original Sami Perttu / FunDSP contributor MIT or Apache-2.0 terms remain intact.
The engine wrappers do not relicense the upstream dependency.

A complete pinned reference checkout is under ignored local/references/fundsp.
reverb-upstream-files.json inventories all 70 Rust files in that checkout and
all 52 published registry source files. All registry sources match after CRLF
normalization: four raw line-ending differences are listed with both hashes.
No executable source difference was accepted. The inventory establishes source
identity, not blanket API coverage or DSP quality certification.

Reviewed hooks: `prelude::reverb_stereo`, `fdn`, `fir3`, stereo split/pan/sum;
Feedback/FrameHadamard state and tick, Delay allocation/reset/sample-rate setup,
FIR state, AudioNode/AudioUnit allocation and stereo filtering. Construction,
rate changes and allocate run on workers. Sample filtering uses the already
allocated 32-channel delay/FIR/normalized Hadamard network; the algorithm remains
inside the library. Upstream reverb generation/fitness and alternate reverbs are
not invoked. No engine-private copy of those algorithms is introduced.

This is an authored parametric late reverb. The room-size value scales delays;
its time is an approximate upstream -60 dB parameter, not a measured scene RT60.
It does not trace geometry, estimate materials or enclosure, render discrete
specular/diffuse paths, perform diffraction, rotate an Ambisonic return or
implement multi-band physical reverberation. BVH direct sound and HRTF remain
independent. Input sends currently follow the existing spatial/direct/HRTF voice
processing; there is no separately simulated unoccluded indirect source path.

Steam Audio's public integration and reflection-effect documentation provide a
reference for separating source/bus routing, simulation publication, convolution
and parametric late reverberation:
- https://valvesoftware.github.io/steam-audio/doc/capi/integration.html
- https://valvesoftware.github.io/steam-audio/doc/capi/reflections-effect.html

Steam Audio is not linked by this implementation. Those architecture references
do not establish equivalent behavior or feature parity.
