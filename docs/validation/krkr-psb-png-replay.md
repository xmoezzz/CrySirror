# KiriKiri PSB PNG revisit on Windows

2026-11-23. The local change was compared against `a821e97` in two locked,
offline Release desktop builds with LTO disabled and 16 codegen units.
Both used the existing HF installation,
independent copies of the same save seed, and the same 5,720-frame input track.
The replay opened `eva01_01` at frame 4190, closed it at 4700, and reopened it
at 5290. The RTX 4060 Laptop GPU rendered offscreen via Vulkan with 2 ms wall
pacing and no audio output.

## Implementation and compilation

The PSB adapter retains the most recent large PNG as a CPU image lease within
the existing package retention budget. It reuses the existing member budget
for PNG overlays, alongside small UI images. The large lease is dropped when
replaced, when its package is replaced, or when its bytes plus the retained
packages would exceed the package budget. Cached pixels follow normal Canvas
publication and retirement. Both desktop probes built successfully.
`cargo test -p crysirror-krkr --lib --offline --release --locked` passed all
22 tests, including a full-size PNG, overlay, UI lease and package replacement
regression.

## Behavior

Each variant ran five times in alternating order. Click latency is the wall
time from tracked mouse-down until the first GPU-complete offscreen frame
whose 16 sampled pixels match the open CG. Full-frame parity was checked in
separate replays to keep PNG capture out of the timed runs.

| Measurement | Baseline | PNG reuse |
| --- | ---: | ---: |
| First-open latency, median [min, max] | 217.4 [166.2, 265.6] ms | 236.0 [153.1, 311.8] ms |
| Reopen latency, median [min, max] | 235.6 [175.9, 288.8] ms | 184.0 [153.6, 219.7] ms |
| CG PNG decodes, first open / reopen | 2 / 1 | 2 / 0 |
| Maximum process peak working set | 855.3 MiB | 864.0 MiB |
| Maximum Windows peak pagefile counter | 1335.2 MiB | 1344.4 MiB |

The decode count covers the PNG base and overlay referenced by the selected
CG. Reopen recorded a hit for both entries in the instrumented replay, and
all five timed runs recorded zero decodes for them. The CPU lease for the
full-size base held 8,294,400 bytes. The existing member cache peaked at
8,388,168 bytes within its 8,388,608-byte cap. GPU memory was outside the
measurement scope.

The full 1920 × 1080 RGBA frame matched across builds after the first open,
after closing to the gallery and after reopening. Every timed run also matched
the final frame hash. Timing describes this single offscreen replay on the
tested hardware. The test scripts, logs and game data remain local.
