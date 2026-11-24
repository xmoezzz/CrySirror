# KiriKiri Nana archive and startup validation on Windows

2026-11-24. Tested `localove3.exe` from ろけらぶ 神社×先輩 with Windows x86-64 Release builds. The native process and CrySirror ran against a separate validation copy. CrySirror used a local Nana profile matching that installation.

## Archive results

- [xp3-brute PR #3](https://github.com/xmoezzz/xp3-brute/pull/3) validated Nana filename and content decoding against `fgimage.xp3`. With the matching profile, the upstream inspection command still rejected `data.xp3` because it expects a filename record for every `File` entry.
- `data.xp3` contains 1,725 `hnfn` name records and 1,727 XP3 file entries. All 1,725 token and Adler-32 pairs matched. The two other entries are literal `$` and `startup.tjs`. Both pass their stored Adler checks. `$` is the 2,020-byte bootstrap. The 316-byte plain-name script only reports a corrupt executable. CrySirror exposes `$` under both `$` and `startup.tjs`.
- All six original XP3 files mounted. Forty-four XP3 content samples were read successfully, with zero failures. The archive library ran 50 passing unit tests and one ignored test, including the mixed filename and startup fixture. The owner-local archive integration test passed separately.

## Startup results

The first implementation mounted all archives but executed `$` into `起動処理に失敗しました: failed to startup.`. The capture stayed black. The native executable opened the game's window with the same installed files.

The bootstrap derives the `.key` filename from `System.exeName`, checks for `Storages.setupArchiveData`, and passes the file's first line before calling `system/Initialize.tjs`. The installation and decoded names contain no physical key file. The input observed in the normally launched isolated native process reproduced the complete 4,096-byte table, all six name-state words, and both content-filter parameters. The equivalent key companion was staged only in the validation copy. The original script and six XP3 files were used unchanged.

CrySirror verified and registered the existing native callback, rendered the first-run language selector, accepted its OK button, and reached `title.ks` at `*wait` with the title menu visible in an 8,800-frame Windows offscreen run. A separate 12,200-frame run clicked START and reached `start.ks`, `*envplay`, line 25. A synthetic startup fixture with independent derivation executed `Array.load`, `Storages.setupArchiveData` and the dependent initialization script through `play`. A different first line rejected the fixture during detection.

With the same Release binary, My Commuting Crush mounted six archives and My Cute Roommate mounted ten. Both reached the first-run options script's wait in offscreen regression runs. Audio output, save recovery and later scenes were outside this startup check.

## Simplified Chinese fonts and Quick Save follow-up

The original test player exited in `uimain.tjs` during Quick Save rotation.
The `copyFile` fix already tested on AMBITIOUS MISSION was carried over from
the local performance worktree. A Windows Release replay
of the same Shrine route reproduced the old player error. The updated player
wrote a new Quick 01. Quick 02 matched the previous Quick 01 byte for byte.
Both runs wrote only to copies of the user's test state.

The game's `PreRenderFontEx` copies each Font property's native accessor into a
dictionary. The accessor retains its original Font as receiver. Native text
operations read the state on that Font. If a mapped bitmap font lacks a glyph,
measurement and drawing use the outline font underneath it for that glyph.
The earlier capture used the launcher's default font for the Chinese dialogue.
The updated capture of the same line used `SourceHanSansSC-Bold.otf` from the
game archive, as confirmed by the Canvas paint alias. The 41 focused Windows
Release tests passed, with two opt-in diagnostics ignored. An independent
English first-run probe still hit the unsupported `width_time_scale` option.

## Quick Load after Quick Save

The earlier Windows Release build entered the Quick Load confirmation flow
but left the dialog invisible. The game's dialog definition uses a postfix
TJS eval to choose the localized PIMG name. When the storage probe yielded,
the VM discarded the evaluated expression's return value and the dialog
received an empty resource name.

The evaluator now uses a normal VM call continuation and delivers the result
to the operand register when the evaluated script finishes. The regression
reproduced the empty result on the earlier implementation and verified the
resolved name and a single evaluation on the updated implementation.

On a copy of the user's test data, a Windows Release offscreen replay wrote a
new Quick 01 and advanced one line. Quick Load displayed the Chinese
confirmation. Clicking NO kept the advanced line. Reopening Quick Load and
clicking YES restored the saved line. A further click advanced to the same
next line. All three dialogue captures matched the reference dialogue pixels.
The previous Quick 01 was rotated into Quick 02 without changing its bytes.
The TJS2 suite passed 436 tests, and the KiriKiri storage and TJS semantics
suites passed 12. The replay used virtual input and disabled audio output.

## Dialogue outlines and BMP save previews

The user's No.001 bookmark reproduced a long gradient-colored line with
missing black outlines. Each gradient draw uses its own coverage snapshot.
Compaction could inline a gradient
snapshot containing keep-alpha draws into a canvas that already had the
outline. The gradient then recolored that outline as well. Keeping the
snapshot isolated over an existing draw preserved the gradient and the
black outline in the Windows Release GPU capture of the same loaded line.

No.001, No.002 and No.006 had valid 137×77 BMP headers and pixels followed by
native save data. The old preview reader accepted JPEG and PNG only. The
updated reader uses the BMP size field to pass the leading image to the
shared decoder. All three existing thumbnails were visible in the same
Release replay. The replay used a copy of the test save directory.

A new Quick Save in the final Release build produced a valid 137×77 BMP
preview. Quick 02 matched the prior Quick 01 byte for byte. The adapter test
round-trips a BMP preview and the appended native slot data alongside the
JPEG and PNG cases.

## Fast-forward and English secondary text

The previous Windows player reached the empty TextRender color command
during fast-forward from No.003. Selecting English secondary text from
the Text Language menu reproduced a separate unsupported layout option.
The updated TextRender resets the appropriate default color and accounts
for wide glyphs in the reveal intervals.

The same bilingual line redraws gradient glyphs already on screen as new
characters appear. The Windows presenter can keep compacted canvases from
a paused redraw as budgeted immutable RGBA images. This avoids PNG
round-trips on the frame thread and uses the existing PNG path if the
resident image budget is full. The text service also reuses its simple
placement loop without measuring the same run first.

With the same No.003 save and input track, both Release builds completed
12,510 virtual frames with Chinese primary text and English secondary
text selected. The baseline included the TextRender compatibility fixes.
After the rendering changes, the sampled Canvas compaction P95 went
from 24.5 ms to 21.6 ms and the engine frame P95 from 32.2 ms to 29.7 ms.
These are Windows offscreen measurements with audio output disabled.
The final build also completed a 16,300-frame fast-forward replay from
No.003, passing the point of the original empty-color error.

## Windowed bilingual reveal from No.003

The original Windows executable and both CrySirror players opened copies
of the same No.003 save with Simplified Chinese primary text and English
secondary text. A 120 Hz capture sampled the subtitle region in the real
window. Each comparison matched the settled dialogue in the capture and
allowed the previous line to finish before advancing it. Fixed click
counts alone could land on different lines.

The original game's bilingual path redraws its visible text with gradient
glyphs as the prefix grows. The CrySirror trace showed repeated native
bitmap checkpoints during these redraws. An extended checkpoint budget
made the longest callbacks more expensive and gave unstable windowed
results. The shipped path uses the original scheduling budget.

The renderer reuses the same-size temporary attachment for successive
snapshot draws, keeps a budgeted cache of small immutable snapshots, and
reads a completed snapshot by its existing render key. Canvas compaction
also skips generations that have no remaining replaceable draw. On the
12,510-frame replay the recorded compaction P95 was 9.4 ms, down from
21.6 ms in the previous player. The new spans measured 8.5 ms P95 for
snapshot preparation, 3.1 ms for readback, and 0.45 ms for publishing.
The retained offscreen trace recorded 225 small snapshot reuses during
compaction.

During a parked Windows bitmap redraw, the desktop host keeps the last
already presented 2D picture while it advances script and input. Surface
reconfiguration, restore and changed viewport settings invalidate reuse.

For the same narration line beginning with “或是，哪怕现在还不是恋人”,
the real-window measurements were as follows. The reveal span runs from the
first visible pixel change to the last recorded change in the subtitle
region. The intervals include pauses between partial draws.

| Player | Visible update gap P95 | Longest gap | Reveal span |
| --- | ---: | ---: | ---: |
| Original Windows executable | 28 ms | 34 ms | 1.35 s |
| Previous CrySirror Windows build | 546 ms | 786 ms | 4.39 s |
| Final CrySirror Windows build | 312 ms | 389 ms | 3.43 s |

The final Windows run recorded held-frame reuse and displayed the complete
bilingual line. Three Canvas ownership tests, five real-GPU snapshot tests
and two KiriKiri bitmap-copy regression tests passed in Release.
