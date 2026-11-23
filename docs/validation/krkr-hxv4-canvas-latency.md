# KiriKiri Canvas rebuild and HXV4 stream reuse

## Implementation

- The existing CPU profiler reports KiriKiri callbacks, script calls and
  resumptions, layer synchronization and commit, and transition preparation
  as nested spans on the owner frame. Canvas materialization and transition
  history updates have separate spans before snapshot scheduling.
- A Canvas presentation rebuild validates active crossfades once at the end of
  the rebuild. Per-layer checks still run during each update. The enclosing
  layer transaction restores the original roots and transition state on error.
- An HXV4 catalog shares verified single-segment readers of immutable bytes
  within that mount. It limits retained handles to 128 and checks caller byte
  limits, cancellation, and transform selection before reuse.

## Compilation and behavior tested

Release tests passed for all 18 Canvas cases, five crossfade cases, seven HXV4
cases, two CPU profiler cases, and the Windows XP3 sharing case. The new Canvas
case resizes both endpoints of an existing crossfade, then verifies that an
invalid one-sided resize rolls back Canvas and Layer state.

The PC edition of AMBITIOUS MISSION was replayed on Windows using a Release
build and Vulkan offscreen rendering on an RTX 4060 Laptop GPU. Both binaries
used the original HXV4 archives and the same test-only startup compatibility
fixture for `Rect`. The dialogue workload sent 105 clicks and measured 3,670
consecutive GPU probe intervals in each run.

| Measurement | Before | After |
| --- | ---: | ---: |
| P95 frame interval | 47.51 ms | 38.58 ms |
| Frame intervals over 50 ms | 166 | 96 |
| Longest frame interval | 419.66 ms | 95.88 ms |
| Frame 5441 interval, matching probes | 419.66 ms | 89.88 ms |
| KiriKiri layer commit in frame 5441 | 342.80 ms | 41.44 ms |
| Total time in 183 selected archive opens | 72.02 ms | 29.29 ms |
| Median of 111 repeated entry opens | 0.124 ms | 0.002 ms |

The separate batch trace at frame 5441 recorded 219 Canvas presentations and
19 hides in a single commit. In the timed baseline, script resumption took
8.52 ms and transition preparation took 0.08 ms on that frame. The frame 5441
comparison matched the current and preceding pixel probes. Across all 3,387
intervals that matched both probes, P95 changed from 47.05 ms to 37.06 ms.

The school double-portrait shot matched outside the animated advance marker.
The prologue reached the same CG and script position. Its initial timed
captures differed by up to two RGB levels because the transition phases
differed. For that CG the second entry open dropped from 1.528 ms to 0.002 ms.
The rapid run's last captures differed in dialogue reveal and the advance marker.
These are offline playback and CPU tracing measurements on one local game and
machine. Presentation through a desktop window and other storage providers
were outside this replay.
