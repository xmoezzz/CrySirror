# KiriKiri Windows startup across Loca-Love editions

2026-11-23. Windows x86-64, locked offline Release build, 16 codegen units,
LTO disabled. The player reads the original installed XP3 files. Test saves
and logs use separate local directories.

## Implementation

- Legacy XP3 name records select by the logical filename token within an
  Adler bucket. The resolved entry keeps its native token alias and shadows
  a conflicting unmapped placeholder.
- CXDEC candidate selection validates entries that extend into different
  filter regions when short entries leave a boundary ambiguity. Stream
  validation remains bounded by source bytes and transform work.
- Outline and shaping faces share the encoded font backing. The dynamic font
  budget reserves the transient read-to-publication copy and accounts for one
  shared source after conversion. The last consumer releases the allocation.
- Horizontal TextRender ruby accepts uneven reading and base lengths. A
  reading is apportioned on character boundaries while the base glyph
  positions and reveal timing still follow the existing layout.

## Scope exercised

| Installation | Evidence |
| --- | --- |
| Loca-Love My Cute Roommate | All 10 archives mounted. The first-run language dialog accepted input. From the title, START advanced beyond the earlier uneven-ruby error and rendered a dialogue wait. |
| Loca-Love My Commuting Crush | All 6 archives mounted. The first-run language dialog accepted input. From the title, START reached and rendered a dialogue wait. |
| ろけらぶ 神社×先輩 | Initially blocked before filename recovery. See the [Nana startup follow-up](krkr-nana-shrine-windows.md) for archive, startup, save and bilingual rendering evidence. |

Archive tests include a synthetic equal-Adler fixture with a shadowed
startup entry, a larger entry that distinguishes two CXDEC candidates,
and opt-in checksum-verified samples from both Loca-Love profiles.
Font tests cover read overlap, live sharing, and retirement of source leases.
TextRender tests cover uneven readings, supplementary Unicode characters,
and base reveal timing. Playback coverage is offscreen, without audio output.

## Trilogy PR candidate

A fresh Windows x86-64 Release build from the scoped PR worktree was replayed
with separate game state and audio output disabled. Each run completed with
its mode-specific target state and no runtime error.

| Game | Replay result |
| --- | --- |
| My Cute Roommate (`localove1.exe`) | Ten XP3 archives mounted. A fresh state accepted language selection and advanced from START to the prologue dialogue at `start.ks`, `*envplay`, line 25. |
| My Commuting Crush (`localove2.exe`) | Six XP3 archives mounted. A fresh state accepted language selection and advanced from START to the prologue dialogue at the same script label. |
| 神社×先輩 (`localove3.exe`) | Six XP3 archives mounted with the matching local Nana profile. No.003 loaded with Simplified Chinese primary text and English secondary text. The capture shows both lines during reveal. |

In a separate Shrine replay, Quick Save created a new 137 × 77 BMP preview
and moved the previous Quick 01 to Quick 02 byte for byte. Canceling Quick
Load kept the advanced line. Confirming Quick Load restored the saved
dialogue pixels, and the next click reproduced the advanced line.
A separate 16,300-frame Shrine replay loaded No.003 and held Ctrl past the
previous TextRender color error. It completed without a VM or asset error.

## Windows library and movie audio follow-up

The desktop library passed a named entry's executable to the player and
opened the Sickly Days introduction in a real window. Directory discovery
picked a different executable for that installation. The Shrine replay
used the matching Nana companion files in an isolated installation copy.
A scripted mouse press on Play wrote the selected entry ID to the launcher
receipt. The developer library root also contained a directory without a
supported game, which the explicit acceptance list omits.

The original Commuting opening movie reproduced the player exit in a
bounded Windows window run with audio enabled. Its ASF audio uses format
tag 0x0163. VideoOverlay selected the game's matching MPEG companion with
MP2 audio. The updated player presented a movie frame and completed the
same windowed check. The runtime trace records the selected storage,
decoder working state and 1024 × 576 sequence dimensions. The fallback
does not change playback of a WMV with a supported audio track.
Two VideoOverlay and three incremental ASF tests passed in Release.
One owner-media test remains opt-in.

The updated player also replayed fresh starts to dialogue in Commuting and
Roommate, loaded Shrine No.003 with Chinese primary and English secondary
text, and reached the Sickly Days title using the explicit executable.
These game replays used isolated saves without an audio device.

After Episode 2 restored the No.001 save and reached its dialogue wait.

AMBITIOUS MISSION main still reaches the image composition byte limit
during the long Ctrl route. After Episode 1 fails when its save page
requests a snapshot of an active composite. These failures block acceptance
of the broader KiriKiri batch.
