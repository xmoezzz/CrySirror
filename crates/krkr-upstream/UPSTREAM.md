# Vendored Kirakira crates

- Source: https://github.com/liulifox233/Kirakira @ c451918023477620ff9055a2eb8a6714b947d9c1
- Crates: `krkr-tjs2` (TJS2 frontend, compiler, bytecode loader/VM, builtins)
  and `krkr-kag` (KAG scenario parser). License: AGPL-3.0-or-later (`LICENSE`).
- `SOURCE.json` records the SHA-256 of every vendored source file; they are
  byte-identical to the pinned commit (checked by `tests/test_krkr_policy.py`).

Not vendored: Kirakira's engine, renderer, audio, font, video, E-mote,
plugin, asset/XP3 and app crates. CrySirror's `crysirror-krkr` adapter
translates KiriKiri native classes onto CrySirror engine services instead
(ADR 0053); E-mote uses CrySirror's own puppet pipeline (ADR 0029).

## Port changes (complete list)

1. Manifests are self-contained: edition 2024, `publish = false`, automatic
   test/example/bench targets and lib tests are off, upstream warnings are
   allowed. `krkr-tjs2` keeps its only dependency, `regex = "1"`.
2. No source file is modified.

## Licence boundary

These crates and `crysirror-krkr` are the only AGPL code in the workspace.
They are excluded from the default desktop build; enabling the desktop
`krkr` feature produces a binary distributed under AGPL-3.0-or-later.
