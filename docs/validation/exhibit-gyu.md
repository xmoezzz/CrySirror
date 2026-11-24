# ExHibit image resource validation

The 2026-11-22 ExHibit work follows the adopt model. CrySirror's shared image
registry owns the decoder. The runtime installs that registry for ordinary
client assets. That revision covered image resources. The [RLD adapter](../../crates/crysirror-exhibit/README.md) records subsequent work.

## Format evidence

The format reference is GARbro at
[b09ee457, ImageGYU.cs](https://github.com/morkt/GARbro/blob/b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0/ArcFormats/ExHibit/ImageGYU.cs).
Its [palette reader](https://github.com/morkt/GARbro/blob/b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0/GameRes/Image.cs)
specifies BGRX entries. This independently written Rust decoder uses the
on-disk layout described by those readers. The evidence does not establish
an original executable version or that any one title uses this profile.

The registry recognizes the four-byte GYU signature regardless of filename.
The initially tested header profile uses mode 0x0100 and key 0xffffffff. Its color
plane contains bottom-up rows padded to four bytes, with indexed 8-bit, 24-bit
BGR or opaque 32-bit BGRX pixels. Indexed and BGR24 images may supply a
separate stored alpha plane with its own four-byte row alignment. Flag 3
stores byte alpha directly. Other flags multiply the stored sample by 16 and
saturate at 255. Coverage limits and closure criteria are in
[SATD-image-exhibit-gyu](../../crates/crysirror-image/SATD.md#satd-image-exhibit-gyu).

## Local behavior checks

The [image registry tests](../../crates/crysirror-image/tests/exhibit_gyu.rs)
construct synthetic inputs for content detection, palette selection, ignored
BGRX padding bytes, alpha expansion and malformed or unsupported profiles.
The [runtime test](../../crates/crysirror-runtime/tests/exhibit_gyu.rs)
writes an original synthetic 2 × 2 image into a temporary client asset
directory, reads it through scoped storage and checks the dimensions and all
16 output bytes. Both optional runtime format packs are enabled in this check
to exercise normal format selection.

```sh
cargo test --release --locked -p crysirror-image --test exhibit_gyu
cargo test --release --locked -p crysirror-runtime --features arc-formats,blackcyc-formats --test exhibit_gyu
```

On 2026-11-22, release builds in the isolated macOS checkout passed all
three GYU image tests and the runtime storage test. The image library unit
suite passed 240 tests with three marked ignored.

On 2026-11-23, header inspection of three original Famisupi startup assets
confirmed 800×600 indexed and BGR24 images with mode 0x0400 and inline
shuffle seeds. Final player execution and pixel comparisons remain pending.

## 2026-11-23 static resource and script inspection

| Resource | Evidence | Limit |
| --- | --- | --- |
| GYU | 7,328 headers in four installations. Encountered modes 0x0100, 0x0200, 0x0400 and 0x0800. The new mode-0x0800 reader interleaves literals and backwards references using MSB-first control bits. | Counts are header inspections. Final RGBA checks, runtime upload and ingame blending remain pending. |
| AiFS v5 voice | In three v3 installations, 13,030, 3,179 and 2,594 nonempty numeric entries fit within their respective res2001.grp companion volumes and begin with the OggS capture pattern. Both index header layouts occur. | Archive indexing and bounds checked. Individual stream decode, voice selection and playback remain pending. |
| Loose .v voice | All 11,221 Famisupi files consumed exactly the declared pages and ended at EOF. Full pages included 1,137,313 mode-4 interpolation pages and 254,398 zero pages. All 11,221 final pages had tag zero. | Structural inspection and native mixer analysis. Final sample comparison and playback remain pending. |
| RLD | Independent full record walks consumed all 357 v1 and 379 Midori v3 files. The two later volumes contributed 65 and 76 header observations. | Script record boundaries alone do not establish execution. See the [ABI inventory](../../crates/crysirror-exhibit/NATIVE_ABI.md). |

The v1 native voice dispatch selects the .v page reader by extension (resident.dll RVAs 0xbb5b0, 0xbb780). The reader initializes from the two 16-bit header fields at offsets 4 and 6 (0xbcfe0), restarts at byte 8 (0xbd030), and produces 2,048-frame stereo pages (0xbd060). Full mode-4 pages insert a midpoint between adjacent mono samples and restart interpolation on each page. Tail pages use a zero-initialized mixer buffer. Silence resets the IMA predictor (0xbd050). Compressed pages decode high nibbles first (0xbd500). The implementation has synthetic output checks for PCM interpolation and tail padding.

The new Rust tests and final player were not run during this inspection.

## 2026-11-23 release playback on Apple Silicon

The Rust 2024 desktop binary linked with the ordinary player features. The adapter passed
18 release tests. Three shared resource tests selected by `-p crysirror-resource --lib exhibit`
passed, including voice page decoding and archive access. All five image tests in
`crysirror-image --test exhibit_gyu` passed. The unfiltered resource suite also
reported a separate capability-lookup failure: its test expects 65,537 entries to exceed a
budget now set to 2,097,152.

Each game was opened through `crysirror play` with `--headless --no-audio` and a private
`--data-dir`. Famisupi used a 300,000-frame ceiling to let its asynchronous logo decoder finish.
The title ran its save-index query, started effect 64, and panned CG 1246. At the first unsupported
record, title.rld+0x202 (0x5e), the layer dump had source UVs (0, 0.43636, 1, 0.98182)
and a separate retouch.gyu layer. The mixer had rendered 25,344 frames. This records
execution and an intermediate crop, not an interactive menu or a full playthrough.

Midori selected the neutral ID 152 resource and decoded start.rld. It stopped at +0x114,
where the taken 0x05 block requires the native layer, camera and printer checkpoint.
Both Stray Sheep volumes stopped at their unrecognized v3 initializer before scenario execution.
The opcode census and the scope of these implemented forms are in the
[ABI inventory](../../crates/crysirror-exhibit/NATIVE_ABI.md).

## 2026-11-23 RLD v1 title panels and select-item attachment

The Famisupi resident module examined locally has SHA-256
`dc1add582bf76754e5f38d6ab47e69f2921e4a900c244062c72149e6a519d808`.
The four `0x5e` records at title.rld+0x202, +0x28c, +0x317 and +0x3a1 use
the path overload of `cmdBillboard`. The native bindings use image dimensions
and permit negative or off-canvas placement. Effect 519 creates a zero-level
surface and 255 keeps the supplied level. All six `0x5b` records starting at
+0x42a attach a 648 × 29 atlas to slot 42. All six 256-entry palettes match
the parent image. The original bind callback copies the left 216 × 29 cell
into the indexed parent bitmap while retaining the parent's separate alpha
plane. The implementation uses the shared image worker's RGB-only copy.

Across 357 original v1 scripts, `0x5e` appears 1,033 times, including 21 binary
records. An independent scan found 35 records in 16 files meeting the numeric
and text guards of the two implemented image effects. Of 243 `0x5b` records,
206 in five files met the new operand guards. These counts omit runtime
requirements such as a live parent and a supported indexed surface.

An intermediate release replay reached +0x42a. Its retained scene contained
all four new native slots. Slot 40 was (600, 578, 184, 15), slot 41 was
(−800, 460, 800, 100), slot 42 was (800, 460, 454, 100) and slot 43 was
(16, 375, 330, 220). The two zero-level panels and the two off-canvas panels
still required the subsequent native effects to appear on screen.

The final scoped release test suite passed 20 tests. A two-button fixture uses
the shared runtime image worker with two different palettes, and asserts
all 16 destination RGBA bytes after both registrations. The second atlas
deliberately assigns different colours to the copied indices. The resulting
parent still has its own palette colours, its four original alpha values and
both button images. The stock title assets exercise the matching-palette path.

The standard desktop player linked in a separate local target directory. Each
installed game ran through `play --headless --no-audio` with its own fresh
`--data-dir`. The four runs ended with explicit unsupported-operation errors.

| Installed profile | Final execution evidence |
| --- | --- |
| Famisupi v1 | At frame 75,450, title.rld+0xfff called native 29 with parent 42 and item 13. The preceding six attachments replaced the published bitmap for slot 42 six times. The scene pointed to generation 6 and retained one 454 × 100 RGBA image (181,600 bytes), zero retired images and no pending reservations. Slots 40–43 reported opacity 0, 255, 255 and 0. The title could not yet accept input. |
| Midori v3 | start.rld+0x114 still requires the camera, layer and printer state checkpoint. |
| Stray Sheep v3, volumes 1 and 2 | Both stopped at the unsupported initializer before an ordinary scenario ran. |

Native call 29 resolves character aliases through `cmdLid2lid`, looks up a control
on the resolved parent and decrements its enable count. On the transition to
−1, the visible kind-1 button redraws with the third atlas cell. The relevant
v1 RVAs are 0x68b20, 0x87690, 0x994a0, 0xb1420 and 0xb1ab0. This call and the
subsequent fade, motion and input path are outstanding. Native pixel comparison,
a selectable menu, and story playback still need separate verification.

## 2026-11-23 v1 title controls and entrance animation

The original resident module is the v1 binary identified above. The clean-data
release runs used the installed Famisupi files, the standard desktop player,
headless Metal on Apple M4 Max and disabled audio output. Frame numbers vary
with worker scheduling during the starting movie. The script offsets and layer
state identify the checkpoints.

| Release build | First unsupported operation | Observed layer state |
| --- | --- | --- |
| Button disabling | title.rld+0x6aa, 0x69, frame 63,439 | Generation 9 of the 454 × 100 panel after the six attachments and three disable redraws. Slots 41/42 are offscreen at x = −800/+800. |
| Panel animation | title.rld+0x816, 0x8b, frame 76,494 | Slots 40–43 all have opacity 255. Slots 41/42 are at (0,460)/(338,460). The published panel image is unchanged. |
| Input blocking and completed clip release | title.rld+0x870, 0x5c, frame 76,131 | Same four layers and destination positions. The CG crop reached UV [0,0,1,0.54545456]. The retained image has 181,600 bytes, with no retired image or reservation. |

The final build also replayed Midori to start.rld+0x114 and both Stray Sheep
volumes to their explicit initialization failures. No physical input or native
Windows pixel comparison was performed. The v1 select wait asks group 0 to
write to R1300 with event mask 23. Original cmdExecSelectItem (0x66570)
distinguishes item, right-click, keyboard and wheel results. Recovering the
event producer and connecting hover, entry/exit and press/release remains
necessary before the title can select an action.

All 22 adapter release tests passed. The real image-worker test checks distinct
idle/disabled cells with independent parent alpha, a repeated disable without
redrawing, and half-time shared-layer opacity 128 and rect [40,20,4,1] while
retaining the same composed image. Separate assertions cover the original
mode-2 logarithmic move rounding and the 127 per-slot input counters. The two
broad SATD records remain open. Local logs are in `target/exhibit-proof`
under `{disable,animated,gated}-famisupi.log`, `gated-desktop.log` and
`gated-state-test.log`. Reproduce from the checkout with:

```sh
CARGO_TARGET_DIR=target/exhibit-proof/cargo-local cargo build --offline --locked --release -p crysirror-desktop --bin crysirror
cargo test --offline --locked --release -p crysirror-exhibit
target/exhibit-proof/cargo-local/release/crysirror play ~/Downloads/game-ExHIBIT-famisupi --headless --no-audio --data-dir "$(mktemp -d)" --frames 300000
```

An additional paced run (`CRYSIRROR_OFFSCREEN_PACING_MS=3`, 1,248 frames)
wrote the 800 × 600 `target/exhibit-proof/gated-title-late.png` locally. Its
`gated-final-capture.log` confirms slots 41 and 42 at their final positions and
all four title layers at opacity 255. The capture ends before input dispatch.


### Famisupi primary title input (2026-11-24 JST)

The release player on M4 Max/Metal accepts a primary press and release on
最初から始める at canvas pixel (420,475), commits item 10 to the v1 script, and
takes the START branch rather than waiting in opcode 0x5c. With a press at
frame 1601 and release at 1605, the real script reaches its exit fade at
title.rld+0x10e1. The first input-only build stopped there because the fade
includes unbound slot 44. That run did not present a scene or dialogue. In a separate
1,800-frame run, dragging off START and clicking the disabled Extra Mode
item at (650,505) left the same title wait active, with no item selected.

The standard release desktop build and all 22 adapter release tests pass.
The tests cover accepted click, disabled item, pointer loss, release outside
and blocked layer. Real replay used the original installation only as assets.
No vendor code ran. To reproduce the positive input, supply this JSON with
`--input-track` and use a private writable `--data-dir`:

```json
[
  {"frame":1600,"pointer":[420,475]},
  {"frame":1601,"down":["mouse_left"],"pressed":["mouse_left"]},
  {"frame":1605,"down":[]}
]
```

Use `--headless --no-audio --frames 1800` and
`CRYSIRROR_OFFSCREEN_PACING_MS=3` on a host with an accessible GPU. Local
logs, both tracks and the negative-run capture are under
`target/exhibit-proof/primary-select-play.*`.


The bounded title-exit build replays this same track on Metal through the
synchronized fade, group exit (0x5d), and the four zero-time removals (0x5f).
An inactive zero fade is admitted only when a full-opacity bound lane shares
the same synchronized channel and duration. This preserves the original
completion without conflating a native descriptor with a drawable layer.
The adapter does not write opacity to the inactive slot. The release test
also rejects asynchronous, unmatched-duration and nonzero-target forms,
and checks the expected 500-ms fade's quantized endpoint at 469 ms.

The real replay stops at frame 1,671 at `title.rld+0xf91`, opcode 0x4e
(scene fade-out). No menu sprites or retained composed images remain.
The original handler coordinates music fade, scene drawing and cleanup of
renderer, character and message state. The following message frame (0x48)
and scenario path have not been run. Local evidence is under
`target/exhibit-proof/exit-stage-play.*`: `exit.log` and
`exit-001660.png` show the frontier and intermediate fade. The latter is not a
capture of the opening scene.
