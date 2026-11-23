# KiriKiri 2 / KiriKiri Z / KAG3

[TJS2 opcodes, native APIs and KAG control tags](native-api/krkr.md).

```sh
./target/release/crysirror play "/path/to/game"
```

Keep XP3 archives, patches, the game's executable and companion files in their
original layout. CrySirror reads them in place and starts the original TJS/KAG
scripts. Supported resource protection includes HXV4 and supported cxdec-family
filters; additional game-specific filters may require their accompanying files.
For HXV4 installations the main archive supplies the executable's bootstrap.
Indexed sibling XP3 archives are available by container name. The game's
compound-storage script chooses which archives to activate and in what order.
The Windows player recognizes KiriKiri Z's embedded `TEXT/139` configuration
resource, including language defaults used during startup.

For the HXV4 bootstrap, `System.checkSignature(file[, signature])` returns a
boolean verification result. An absent target or signature and a malformed
signature yield false. An omitted or empty signature name selects `file + ".sig"`.
The supported profile verifies RSA-PSS/SHA-256 against the bootstrap public key.
The calling TJS script decides how a failed check affects startup.
[Bootstrap callback and verifier reference](https://github.com/hktkqj/cxdec-hxv4-static-analysis/blob/3c6dee001df011fa7ed557c7463635c6f67698f6/docs/core/FileSignatureCheck.md).

## Game features

- TJS scripts and KAG3 scenarios, labels, macros, calls, choices and message waits.
- Layer positioning, scaling, rotation, clipping, blending, image filters and
  custom mouse cursors.
- Text windows, font selection, ruby, typewriter text and the TextRender extension.
- Music, voice, sound effects, Vorbis/Opus audio, voice filters (`WaveSoundBuffer.filters`, including `PhaseVocoder` pitch and time scaling), ordinary movies and AJPM alpha movies.
- Menu bars, layer movies, PSB images and M2 motion playback.
- Managed windows expose the read-only integer `Window.HWND`. Its value is
  zero because CrySirror's script windows have no native plug-in handle.
- Crossfade, rule-image transitions, and Kaicho blur/dim transitions.

For outline fonts, KiriKiri's absolute `Font.height` specifies the em size in
pixels. Glyph advances and drawing use that size and the selected face's
ascender. Games can install a script property over a layer's `font` and access
the backing font through `(Layer incontextof layer).font`. Native drawing
uses the backing font, including faces registered from the game's own archives.
Pre-rendered bitmap fonts retain their stored metrics.

An image reference of the form `psb://file/key` can open its named PSB from
the game's search paths even when no script object has loaded it yet.
Existence checks and image loading look up the same resource entry. The
shared image service decodes it. Small UI members and PNG overlays share the
8 MiB, 256-entry per-session limit. The most recent PNG with 4–8 MiB of decoded
pixels can retain them within the existing 256 MiB PSB retention budget. CPU-only
entries receive a normal temporary Canvas publication on each use. Cache
entries identify the loaded PSB instance and member index.
For a PNG member, the game receives the
signed `offs_x`, `offs_y` and `offs_unit` metadata used to position expression
overlays. Layer slicing can read
TLG5 and TLG6 dimensions, including TLG0-wrapped images, and copy rows to
the destination layers supplied by the script. The PackinOne data-pack reader
keeps the previous 64 KiB of LZ4 output when decoding linked blocks.

Re-enabling a layer image allocates a bitmap for its current rectangle.
On an alpha draw face, `operateRect` with an alpha source and alpha blend
updates both color and coverage even when `holdAlpha` is set. This lets
scripts assemble a portrait from slices over previously transparent pixels.

For games using mounted compound storage, repeated asset name searches reuse
both successful and unsuccessful results. The index is bounded per game and
reset when the script changes its search paths or archive routing. Save lookups
and mutable proxy mappings use their current storage state. KiriKiri Z also
caches hits and misses in its native placed-path search.
[KiriKiri Z storage implementation](https://github.com/krkrz/krkrz/blob/master/base/StorageIntf.cpp).

`System.getKeyState(key)` reads the latest logical key level, including when
an older script callback resumes after an asset wait. The optional `false`
argument consumes a press observed since the last such query for that key.
Queued key-down and key-up calls retain their own event order. Bitmap calls
in a resumable script can yield before their next operation during a long
redraw, allowing the host to collect input between portrait parts.
[Windows KiriKiri Z key-state contract](https://github.com/krkrz/krkrz/blob/master/base/win32/SystemImpl.cpp#L776-L789).

While a focused Windows player believes Ctrl is held, it also checks the
current Windows key level before passing input to the game. A missed
window key-up can then clear the held state. Ctrl auto-repeat notifications
do not create new presses. Games still own the policy for ending skip at
a dialogue wait or at a later scenario label.

Pointer hits test visible candidates from front to back. A completed hit
stops the search, including when a disabled layer blocks the pointer.
When coverage is required, the pending input retains its position in that
search until the selected layer's alpha data are ready.

Plug-in names below describe the interfaces built into CrySirror. They do not
imply every export of a similarly named DLL is available. The corresponding
`.tpm` names use the same built-in interfaces. TPM modules in the game,
`system` and native-bitness `plugin`/`plugin64` directories register automatically
before startup scripts, in filename order.

## Plug-in compatibility

### Scenarios and script data

| Plug-in | Available features | Restrictions |
| --- | --- | --- |
| `KAGParser.dll` | KAG scenario parsing, labels, macros and scenario execution. | Game-specific tags still need a matching handler. |
| `KAGParserEx.dll` | Extended KAG parser and tag-list access. | Supports the tag-list interface used by the scenario parser. |
| `ExtKAGParser.dll` | Extended scenarios with `while`, local dictionaries, multiline commands and parameter macros. | Parser-state saves and fuzzy parsing modes are unavailable. |
| `ScriptsEx.dll` | Script reflection and structure helpers. | — |
| `json.dll` | JSON parsing, serialization, loading and saving; Japanese game-script extensions. | Plain values, arrays and dictionaries; callable/cyclic objects are not serializable. |
| `toml.dll` | Loading and decoding TOML documents. | `Scripts.tomlDecodeFromStorage`. |
| `csvparser.dll` | CSV document parsing. | — |
| `lineparser.dll` | Line-by-line text parsing. | — |
| `dirlist.dll` | Directory listings and file metadata. | Access is limited to the game's resource namespace. |
| `varfile.dll` | In-memory binary files and image loading from those files. | — |
| `saveStruct.dll` | Structured data storage. | — |
| `fstat.dll` | File information queries. | — |

### Graphics and text

| Plug-in | Available features | Restrictions |
| --- | --- | --- |
| `layerExImage.dll` | Brightness/contrast, colorization, HSL adjustment, noise, white noise and Gaussian blur. | Only these image operations are available. |
| `layerExRaster.dll` | Sine-wave row copying with `copyRaster`. | Overlapping in-place self-copy is unavailable. |
| `layerExDraw.dll` | Solid-colour paths, rectangles, polygons and connected lines, with transforms. | Pens, gradients, curves, ellipses, GDI text and metafiles are unavailable. |
| `layerExBTOA.dll` | `clipAlphaRect`, `copyRightBlueToLeftAlpha`, `copyBottomBlueToTopAlpha`, `fillAlpha`, `overwrapRect`, `copyAlphaToProvince`, `fillByProvince` and `fillToProvince`. | — |
| `shrinkCopy.dll` | `shrinkCopy` area-average reduction and `shrinkCopyFast` integer reduction. | — |
| `layerStwCopy.dll` | `stitchWrappedCopy` wrapped row copies. | — |
| `GlitchEffect.dll` | `glitchCopy` and `doGlitch` block/line displacement with channel splitting. The `glitch` transition (rows switch to the new image as it progresses), `fadeglitch` (one image glitches while fading to or from `color`; `nofade` keeps colours) and `loopglitch` (finishes at its first update without drawing, as natively). | Without `seed`, and in transitions, the pattern differs from run to run, as natively. `fadeglitch` with `fadein` needs a source layer. Transition sources must match the destination size. |
| `PackinOne.dll` | The bundled members listed for the plug-ins above, data-pack and octet storage functions, `Layer.fetchImageSize`, `System.waitForAppLock` and `System.setDpiAwareness`. `SliceLayer` and its loader. `System.commandExecute` runs nothing and reports `status` `"failed"`. | Temporary-file helpers are unavailable. |
| `textrender.dll` | `TextRenderBase`, styled text layout, ruby, width measurement and rendering callbacks. | — |
| `addFont.dll` | Registering fonts supplied with the game. | — |
| `msdfrender.dll` | `makeGlyphSDF`, `makeGlyphMSDF`, `makeBitmapFromProvince`, `operateGlyphToProvince`, `getAscentHeight`, `copyGlyphByColorTable` and `Scripts.makeColorTable`. | None known. |
| `psbfile.dll` | PSBFile/PSBValue access and `psb://` image resources. | Does not enable every animation format that uses a PSB container. |
| `psd.dll` | `PSD` documents: layer pixels, masks, names, types, blend modes, groups, guides, slices and layer comps; `psd://` layer images for `Layer.loadImages`. | Adjustment layers, layer effects and smart objects load as their stored pixels, not re-rendered. |

### Movies, animation and sound

| Plug-in | Available features | Restrictions |
| --- | --- | --- |
| `fftgraph.dll` | Fire and LCD spectrum graphs from playing audio, with peak hold and decay. | Source decoding and rasterization run on shared workers. |
| `krmovie.dll` | VideoOverlay movie playback. | Playback depends on the movie's supported container and codecs. |
| `getSample.dll` | Play-cursor amplitude for voice lip sync, including compressed voices supported by the shared audio decoder. | Reads a bounded PCM window on a worker. The first query can return zero while awaiting the decoded samples. |
| `DrawDeviceD2Dm.dll` | Presents an additional primary layer through `viewState`. Supports destination rectangles, window zoom, visibility, opacity and layer-manager order. | Rectangle mode 0. The shared renderer controls texture filtering. Exchanging an additional view root during a transition is unsupported. |
| `AlphaMovie.dll` | AJPM alpha-video playback. | Other alpha-video layouts are not implied. |
| `layerExMovie.dll` | RGB movies rendered into layers, looping and playback status. | Side-by-side alpha and resized-image modes are unavailable. |
| `motionplayer.dll`, `motionplayer_nod3d.dll`, `emoteplayer.dll` (M2 Motion) | M2 motion resources, timeline playback and layer attachment. | Single-timeline playback; advanced masks, actions and physics are unavailable. |
| `wuvorbis.dll` | Vorbis playback and seeking. | Native gain, ReplayGain and PCM-format command-line overrides are unavailable. |
| `wumsadp.dll` | Microsoft ADPCM WAVE playback through the shared decoder. | Other ADPCM formats are not implied. |
| `kropus.dll` | Opus playback and seeking. | Native gain and PCM-format command-line overrides are unavailable. |
| `wvdecoder.dll` | `WaveSoundData.F32` and `WaveSoundData.S16`: `open`, `close`, `getSample`, `setSample`, `channels`, `frequency`, `samples`, `totalTime`, `format`, `foreach`, `getLipSync`, `makeThumbnail`, `save` and `rawdata` (an opaque token for `OscLoop`). | Sounds longer than 16 Mi frames do not open. |
| `wumultitrack.dll` | `.mtrk` (`dir/a|b|ID.mtrk` track files, or a TAB list of `name, path, flag`) and `.mchx` (`channel`, `source`, `track`, `volume` over one multichannel source) multi-track music, each track an independently gained stem of one voice. `setMultiTrackMask(id, tracks, gain, time)` fades tracks linearly; `setDelayMultiTrackMask(id, positions, [tracks, gain, …], time)` waits for the first listed sample position; `rebindMultiTrackID`, `multiTrackOpenCallbacks` (called with id, storage, track count, names and aliases) and the gain-callback functions. | `.mchx` output is mono or stereo, and Opus sources must be mono or stereo. WaveSoundBuffer `filters` cannot be combined with multi-track music. |
| `gamepad.dll` | `GamepadPort`. | No controllers are reported. |
| `wfBasicEffect.dll` | `WaveSoundBuffer.DelayEffect`, `StkFreeVerb`, `GraphicEqualizer`, `GainLimit`, `MathAbs`/`MathConstAdd`/`MathConstMul`, `RingModulation` and `Vocoder` filters with the `Osc*` carrier generators (sine, noise, Blit/BlitSaw/BlitSquare, modulate, mix, filter, const), including the `extend` tail after the sound ends (the voice effects of `voiceeffect.tjs`). | Filters apply when the sound opens; changing a sound's filters after `open` and before `play` is an error. `OscLoop` needs `wvdecoder.dll`. Unseeded `OscNoise` uses a fixed seed. |
| `wfTypicalDSP.dll` | `WaveSoundBuffer.WaveDSPFilter`: RBJ, Butterworth, Chebyshev I/II, Elliptic, Bessel, Legendre and custom pole/zero designs, all four filter structures, `setParams` and `getParamInfo`. | As above; parameter changes while a sound plays take effect at its next `open`, without the gradual 1024-sample change. |

### Transitions and windows

| Plug-in | Available features | Restrictions |
| --- | --- | --- |
| `menu.dll` | Menu bars and menu items. | Synchronous modal popup calls are unavailable. |
| `win32dialog.dll` | Message boxes and modal text-edit templates, including initialization, command callbacks, edit text, OK and Cancel. The script receives the chosen button ID. | The supported template has one editable field, a caption or label, OK and Cancel, and optionally a third push button. |
| `extrans.dll`, `extNagano.dll` | `crossfade` and `universal` rule-image transitions. | Other named transition methods are unavailable. |
| `kaichoTrans.dll` | `blur` and `dim` transitions. | — |
| `k2compat.dll` | `Window.TouchMouse` forwarding of touch handlers to mouse handlers and a hidden `System.ModelessOwnerWindow`. | The engine itself raises no touch events. |
| `windowEx.dll` | Window geometry, game-window cursor-position queries, constants and script helpers. | Win32 message hooks and HWND-only window actions are unavailable. |

An unavailable plug-in function produces an error naming the interface or method.
A DLL being present in the game folder does not add that functionality.

## Launch options

The player uses the game's normal startup scripts and disables the optional
Windows developer console by default. You do not need to extract XP3 archives
or write a bootstrap project.

For installed-game launches, the selected game directory is published as virtual
media labelled with the executable's stem. `Storages.searchCD` can find this
label; its returned drive routes back into that scoped game directory. This
mapping does not require an optical drive. Unregistered labels remain absent.

For desktop launches from a named EXE, detection keeps that selection. When a
KiriKiri wrapper references an adjacent `KrkrPatch.json`, the adapter reads
the declared `gameExecutableFile` within the game directory and reports both
names. The launch plan also mounts the listed plain XP3 overlays. For
`arc://`, `archive://` and `psb://` requests enabled in the manifest, the first
archive containing the full member name supplies the read. `patchNoProtocol`
enables the same lookup for relative names. A missing member falls through to
the game's normal lookup. The player logs the first selected override, and
`CRYSIRROR_KRKR_TRACE_STORAGE=1` logs each selection.

This applies to EXEs recognized as manifest launchers. The current scope is
XP3 sources and exact member paths. Folder overlays and additional media
types require their own compatibility work.

Fonts, save locations, window behaviour and SMB/NFS game paths are described in
the [desktop guide](DESKTOP_API.md). See [all supported engines](NATIVE_ADAPTERS.md)
for other game formats.

PackinOne bookmarks can include a captured layer as a JPEG or PNG preview.
The desktop player reads previews from the game's Save mount, including JPEG
bookmarks with APP11 records and PNG bookmarks with a tjSs record. A slot
written without an image retains its saved state; a subsequent in-game save
with thumbnails enabled records the current scene.

For bookmark comments, KiriKiri edit dialogs honor the script's
`EM_SETSEL` selection in UTF-16 code units. A game can preselect the
existing note so the next text commit replaces it. Clicking places the
caret, dragging selects a range, and Shift with the arrow keys extends
the selection. Confirm passes the edited string back to the game.
Cancel discards the edit.

`System.confirm` presents a shared modal dialog with Yes and No and resumes
the running script with the chosen result. No is initially focused and
Escape selects No. KiriKiri Z `Rect` supplies signed edges, sizing, offsets,
clipping, union and containment for game scripts and font ink bounds.
