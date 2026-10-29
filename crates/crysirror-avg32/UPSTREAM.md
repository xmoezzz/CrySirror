# Vendored original AVG32 interpreter

- Source: https://github.com/xmoezzz/siglus_rs @ 5f358ef1360b0439e92aed2b749e8f25d8721954,
  `crates/avg32/src`. License: MPL-2.0 (`LICENSE`). Inspected in full at that
  commit; per-file upstream and current SHA-256 are in `src/vm/SOURCE.json`.
- Replaces the earlier git dependency on the `codex/avg32-portable-vm` branch
  (256c398), whose portable build compiled out every graphics, effect, menu
  and save path.

## Where each part runs

| Part | Implementation |
|---|---|
| Scenario bytecode, flags, Gameexe.ini, menus, save layout, meswin layout | `src/vm` (original logic) |
| PDT buffers, copies, masks, fades, stretches, all transitions | `crysirror_image::avg32` (shared raster device, moved out of the VM) |
| PDT10/11 decoding | `crysirror_image::pdt` |
| Presented screen | one engine dynamic surface + Layer |
| Message, choice, menu, backlog and name-entry text | engine GlyphRun layers (`vm/text.rs`, `menu::Overlay`) |
| Menu/backlog panels | engine solid layers |
| BGM (CD tracks, DirectSound WAV), WAV, SE, KOE | engine audio channels (`sound::AudioCommand`) |
| Game files and `SAVE.INI` | client capability storage (`vm/fs.rs`), saves on the save mount |

## Port changes

See `seam_changes` in `src/vm/SOURCE.json` (complete list). Summary: file
access, clock, audio device, font rasterisation, frame composition and movie
decoding are replaced by engine services or visible failures; interpreter
semantics are unchanged.

## Known gaps (fail visibly or are recorded as warnings)

- Text drawn into PDT buffers other than 0 (`0x66` into a work buffer).
- Cinepak AVI movies (no shared decoder yet).
- `NVL_SYSTEM` `FN.DAT` and `＊００` external glyphs still blit game-supplied
  bitmap glyphs into buffer 0 (pixels, not engine glyphs).
- Glyph visibility follows buffer-0 write damage, not the native present
  timing; buffer-0 copies taken while text is shown do not carry the text.
- Framework snapshots are unsupported; the original `SAVE.INI` slots work.
