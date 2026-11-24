# ExHibit v1 first opening printer stage

A numeric-only scan of 357 installed v1 scripts found 30,462 character-say
records (`0x1c`, flags 2, seven words, two strings). One has the pinned tuple
`[5,1,1,1,0,0,0]`: `F00A1101_2.rld+0x1eb`. The two stock strings
are well-formed CP932 with 2 and 23 scalars respectively and no embedded
controls. The first string equals the native narration sentinel at RVA
0x107a04, so the first opening body has no speaker row. No original
dialogue is stored, printed or rendered by the test.

Native `liteExec` (0x70c10, case 28) calls `RetouchAdvCharacter::say` (0x288a0).
The character path may replace the current voice and apply mute/character
eligibility, gain and optional lip animation. The stock resource for this
voice ID exists, but existence alone does not identify the original user
settings or prove voice playback. Then `printEngine` (0x54d50) derives clear,
name and wait flags before `messageEngine` (0x54da0) runs the print transaction.
For the first opening shape native execution substitutes an empty speaker and
places the body in the first row. The same staged helper can also plan the
separate native branch with explicit inline name followed by LF when inline
names are enabled and no separate name rectangle is active. Delay passed
to the printer is zero. A separate native
hit-wait follows; no synthetic reveal clock or implicit advancement is used.

Native message setup (0x755c0, 0x76220) uses text slot 52 at the frame
origin plus its text inset, yielding `[32,288,728,404]` for area 5.
The authored family is proportional MS P Gothic with a 26 px glyph request
and 30 px line step. The printer centers fitting lines, reserves four pixels
for width fitting and draws the 1 px border at four cardinal offsets
(0xaba50). Scoped layout uses the existing retained glyph service and only an
authorized alias of the authored face. It rejects missing/invalid glyphs,
out-of-range geometry, control sequences and wrapping rather than replacing
them. Exact Windows GDI metrics/pixels, native wrap and inline-control
treatment remain unverified on the shared backend.

Synthetic Japanese unit tests exercise CP932, centered glyphs, explicit line
breaks, slot identity, authorized-font failures and unchanged PC on the
unsupported composite operation. No gameplay or original save ran. The
consolidated implementation gate remains: no `0x1c` record is admitted by
the VM, and the earlier `0x0c` autosave has not been bypassed.

## Linked operation follow-up

The calling definition is now traced without reading any dialogue. Both
authored `def.rld` branches transfer to `defChara.rld`; its `0x30` record at
`+0x147` declares character 5 with 24 variants and voice enabled. The native
character constructor starts voice-disabled (0x26290), and
`createCharacter` (0x63410) stores the third definition field as the enable
byte. The printer preflight predated support for those character definitions;
the narrowly scoped follow-up below installs them when `defChara` executes. Loose voice existence alone
is not sufficient to replace that initialization.

For an eligible unpositioned character the voice path starts with a stereo
Q8 gain of 256 and no pan or lip attachment. Native mute/gain use S53/S54
for character 5 when S30 selects per-character settings, otherwise S40
and S41/S42 (0x35bf0, 0x35b40). The fresh defaults are S53=0 and S54=128.
This is not the effect-voice S38 path. The loose source is
`res/s/v/00/00001.v`. No audio was played.

The native printer initializes a 256-entry rolling history
(0x54230, 0x2b970), storing ordinal, character, name and body in scan order
before setting the corresponding seen bit (0x2d8b0, 0x30840). The subsequent
hit-wait enters and removes its own indicator, and clears its input latch
and stops voice before returning (0x5d950). The stock indicator is layer 55
and uses the indexed 140 × 28 `res/g/gn/hit.gyu` animation; its retained
frame selection/transparency and cadence have not been verified. The
adapter's existing aggregate selection wait does not implement this
cursor, ordinary advance event ownership, or native history/menu handling.
No timed or synthetic auto-advance was added.

A retained glyph command can be planned, but the linked voice/history
transaction and authored wait indicator/input lifecycle still need integration
before `0x1c` can be admitted. The printer-stage desktop release link passed.
No gameplay or save was used.

## Ordinary character definitions and voice preparation

The `0x30` handler retains the fourteen-field authored descriptor: numeric
character ID, body-variant capacity and voice-enable bit, followed by the
CP932 expression and ten name/address fields. The descriptor binds eight
registers from `100+8*id` without clearing their values. This only defines
unplaced characters during `defChara`; it does not invent body contents,
or run a partial definition bootstrap at title start. The body-definition
follow-up is recorded separately in the v1 census.

A separate unpositioned voice helper sends eligible ordinary loose assets to
the existing shared PCM reader/preparation with the native S30-selected
mute/gain registers. Mute, zero Q8 gain, and a missing asset are distinct.
Immediate owner checks guard the resident voice job. No native ordinary
printer invokes the helper yet, and no devices play during the synthetic
fixture: its host checks the prepared PCM bytes and captured channel gain.

In the corpus, all 32 `0x30` records now parse. All 30,462 `0x1c` records
still fail, including the first narration. No title, actual save, or
commercial dialogue was run.
