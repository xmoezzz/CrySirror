# KiriKiri ordinary TLG decode reuse on Windows

2026-11-23. Both offline Release desktop probes used the same branch state
with the preceding PSB changes. The candidate adds ordinary TLG sharing.
Both used the same HF installation, copies of the same save seed, the same
input events and 2 ms offscreen pacing. The RTX 4060 Laptop GPU rendered via
Vulkan. Audio output was disabled.

## Implementation and compilation

Metadata preparation, prefetch and image lookup now join the same scoped
prepared-image ticket. The worker derives dimensions, opacity and the encoded
BLAKE3 fingerprint from the same decode that supplies pixels to the renderer.
Mount refresh cancels pending tickets and discards prepared pixels and facts
for the replaced client. The existing prepared and scratch budgets cover
the shared results and worker allocations respectively.

Both desktop probes built successfully. The Release `async_assets` suite
with `arc-formats` passed all 11 cases. The added case exercised metadata-first
and prefetch-first orders with visible image lookup, one source read per
generation, size, alpha, fingerprint and replacement of the backing storage.

## HF front-to-side portrait replay

Rapid inputs sometimes reached different expressions, so timing used a paced
track with the final click at frame 12440. The track paused there and captured
the stable scene at frame 12660. Three runs per variant reached the same
final 1920 × 1080 RGBA hash. Click latency ran from mouse-down to the first
GPU-complete offscreen frame whose 48 sampled pixels matched the stable pose.

| Measurement | Baseline | Shared decode |
| --- | ---: | ---: |
| Ordinary TLG decodes for the 11 portrait resources | 22 | 11 |
| Target side-pose TLG decodes | 2 | 1 |
| Click latency, median [min, max] | 653.0 [638.4, 677.4] ms | 708.2 [699.4, 714.2] ms |
| Process peak working set, maximum across runs | 782.2 MiB | 859.2 MiB |
| Prepared asset residency at the final frame | 61.84 MiB | 62.62 MiB |

The interval between the first and last portrait decode starts had a median
of 178.7 ms before the change and 146.5 ms after it. These intervals include
work between calls. The sampled pose appeared at frame 12463 in the baseline
and at frames 12463–12464 in the candidate. This workload showed fewer decodes
with higher end-to-end latency and peak process working set. The working-set
counter covers the whole process over the replay, while prepared residency
is a separate final-frame snapshot. The input tracks, detailed timing logs,
captures and game data remain local.

## Follow-up on input latency and memory

Two additional runs of each original binary sampled the process every 15 ms.
From the click to the first matching frame, sampled peak working sets were
714.4 and 769.0 MiB for the baseline and 749.0 and 710.8 MiB for shared
decoding. The largest sampled working set in each run occurred after the final
audited frame, during screenshot capture and runtime-state export. Use the
click window for scene memory comparisons. The original 77 MiB difference
between whole-run maxima did not reproduce consistently.

The detailed Release probe kept the same inputs and added CPU spans before
the main render submission. In two runs with shared decoding, the initial
subtree composite took 11.6–65.6 ms of CPU time. The longest observed portrait
TLG decode took 16.1 ms and the longest queue delay for those decode jobs was
0.24 ms. The presentation frame also spent 64.9–69.2 ms in TJS and about
24.5 ms preparing ordinary layers. These measurements locate separate sources
of latency in the replay. A single overloaded frame can shift the first
matching pose by one frame as well.

### Renderer follow-up

Subtree compositing reuses its parent's crossfade, puppet and screen-transition
pipelines on the same GPU and output format. Subtree texture caches, masks and
frame history stay scoped to the individual renderer. A screen wipe reserves
its full-size output when it first draws at that size. This saves one potential
1920 × 1080 RGBA target, about 7.9 MiB of texture payload per renderer running
only ordinary presentation at that size.

The Windows Release replay used the same stage probes on both builds, with
TLG sharing enabled throughout. The later comparison includes two runs before
the renderer change and three after it. Click latency is measured to the first
GPU-complete frame matching the 48 pose samples. The earlier detailed runs are
retained for diagnosis because their overall script and render timings were
substantially longer.

| Measurement | Shared TLG | Shared TLG and renderer follow-up |
| --- | ---: | ---: |
| Ordinary portrait TLG decodes | 11 | 11 |
| Initial subtree composite, CPU time | 8.3–8.6 ms | 1.9–2.4 ms |
| Click latency, median [min, max] | 547.2 [521.5, 572.9] ms | 520.5 [456.2, 568.3] ms |
| Working set during switch, sampled peak range | 711.6–770.3 MiB | 710.0–749.1 MiB |
| Whole-run peak working set range | 765.4–854.4 MiB | 762.6–802.2 MiB |
| Prepared CPU image residency during switch | 62.62 MiB | 62.62 MiB |

The end-to-end latency and working-set ranges overlap across builds. The
isolated preparation stage improved in every follow-up run. All five captures
match in the portrait and scene. Four captures match across the full RGBA
frame. The remaining capture differs only in the 54 × 52 bounding box of the
animated advance indicator at the lower right.

The follow-up desktop probe built in Release. On the Windows Vulkan RTX 4060
Laptop adapter, the Release wgpu unit, canvas snapshot, ordered dither,
subtree crossfade and subtree effects suites completed with no failures.
Sixty-one cases exercised their assertions, including deferred wipe allocation
and nested renderer pipeline sharing. One canvas text fixture returned early
because it looks for a macOS-only font path. The glyph suites ran with the
Windows Arial fixture supplied through `CRYSIRROR_TEST_FONT`.
