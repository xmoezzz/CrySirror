# Ren'Py 6.99.12.4 bring-up

2026-11-26 JST, macOS on Apple M4 Max with the Metal renderer.

The Python and Ren'Py release suites passed 37 tests. Runtime cases cover
the decision and visible output of a shared
screen button, dialogue rendered by the authored screen, a timed dissolve,
ATL position and transparency during a dialogue wait, and a fresh engine
reading explicitly saved persistent data. A nested screen action transfers
control to the parent label and restores context-local values. The rollback
fixture restores a mutable object in place, including references shared with a `NoRollback`
object, while data reachable solely through the excluded object survives.
A Python 2 regression covers the unparenthesized tuple iterable used in
the original spline module and checks that Python 3 rejects that syntax.

The separately run installed-data test mounted DDLC's four original RPA
files and decoded 35 game and 44 common compiled scripts. DDLC executed
past the console, graphics-test, IAP, NVL and spline initialization code.
Startup then hit the `import tarfile` at `renpy/common/00updater.rpy:27`.
The installed library supplies `lib/pythonlib2.7/tarfile.pyo`. The workspace
interpreter currently runs Python source and its own code objects. It has
no CPython 2.7 bytecode loader. This run did not reach the DDLC title or a
playable scene. The [adapter README](../../crates/crysirror-renpy/README.md)
records the implemented behavior and remaining runtime families.

The desktop release linked successfully. The normal `play --headless`
entry point used a separate data directory for each run. With its
12-frame input track, the source fixture displayed `game/room.png` and
`Right choice` at line 14. The shared audio mixer rendered 8,448 frames
with one active channel, a peak of 0.1831 and zero underruns. The desktop
player also reproduced the installed DDLC `tarfile` error. Neither run
opened a window or audio device.

```sh
cargo test --offline --locked --release -p crysirror-python -p crysirror-renpy
cargo build --offline --locked --release -p crysirror-desktop --bin crysirror
CRYSIRROR_RENPY_DDLC="$HOME/Downloads/game-renpy-DDLC" \
  cargo test --offline --locked --release -p crysirror-renpy \
  --test execution ddlc_native_script_index -- --ignored --nocapture
CRYSIRROR_RENPY_DDLC="$HOME/Downloads/game-renpy-DDLC" \
  CRYSIRROR_RENPY_CHECK_INIT=1 \
  cargo test --offline --locked --release -p crysirror-renpy \
  --test execution ddlc_native_script_index -- --ignored --nocapture
```

## Native runtime, release matrix (2026-11-28 JST)

macOS on Apple M4 Max, release build, `crysirror play --headless` with a
fresh data directory per run and a 300-frame budget, no input track.

Fixtures are the MIT-licensed `the_question` and `tutorial` games with
`renpy/common`, `renpy/__init__.py` and `vc_version.py` exported from each
reference tag (`git archive`). For 6.18.3.761, 6.99.12.4.2187, 7.0.0.196,
7.3.5.606, 7.4.11.2266, 7.5.3.22090809, 7.6.3.23091805, 7.8.7.25031702,
8.0.3.22090809, 8.1.3.23091805, 8.2.3.24061702, 8.3.7.25031702,
8.4.1.25072401, 8.5.3.26051504 and 8.6.1.26111705, both games ran the release's bootstrap,
init phase and start label and yielded `WaitForInput` at their first
interaction with no Ren'Py exception in the native log. The only log
lines besides timings were upstream's own messages that Steam is not
available. The 6.18, 7.4, 8.3 and 8.6 runs were repeated after the last
runtime change with the same result. Captures of the 6.18, 7.4 and 8.6 main
menus show the release's menu layout. The 8.6 menu text is drawn from the
WOFF2 fonts that release ships.

This records boot-to-first-interaction only. It does not establish
dialogue, menus, saves, transitions or rendering fidelity for these
releases; the 8.3.7 dialogue/menu playthrough of 2026-11-28 in the adapter
history is the deepest interaction evidence. No third-party game was run.

The module surface was compared with every reference release by the
ignored `api_surface_matches_reference_releases` test (all importable
upstream modules and their top-level names): missing names per release
went from 330–712 to 200–468 during this work.
