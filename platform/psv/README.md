# PS Vita backend

Status: **not implemented or tested in CrySirror**.

Source candidates to inspect after fetching RFVP:
`crates/rfvp-psv` and `platform/vita`. Keep target-specific integration here;
do not spread SDK conditionals through VM instructions or common service state.

Required integration: input/touch, monotonic clock, memory/texture policy,
2D/text render submission, audio, VFS/save handling and a reproducible build and
package recipe. Measure budgets on the actual target; the reference host limits
are not hardware specifications.

No SDK binaries, signing material, game content, fonts, VPK or eboot artifact are
included. No claim of successful device or emulator execution is made.
