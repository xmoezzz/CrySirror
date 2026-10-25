# 0040: Level sequences

Date: 2026-10-25. Status: implemented and validated. ADR numbers 0030–0039
belong to the parallel Codex work, so this one continues after them.

## Context

The owner asked for the GoW III level to move. The level's WAD has 332
ANM-related records, for example `ANM_chainl1`, `ANM_gearplatupcd`,
`ANM_sparkleflame1` and `ANM_screenshaker` (kind `0x00010034`). Inspection
of `ANM_sparkleflame1` shows:
- a 1/30 s frame step (`0x3d088889`);
- half-float channel data;
- channel descriptor tables;
- the level-space position (155.4, −24.8, −314.8).

The binding of these records to mesh nodes and the compression are not
decoded (ADR 0017 P0), and guessing them is not allowed. The engine still
lacked the generic tool a level needs to move: a keyed timeline over the
camera, transforms and lights, like UE Level Sequences or Unity Timeline.

## Decision

- **`crysirror-scene::sequence`.** A `SequenceSpec` has a duration,
  looping, a speed of 0..16, an optional camera track, entity tracks and
  events.
  - Camera keys hold eye, target and an optional fov, with optional shake
    noise.
  - Track properties are translation, rotation, scale, light intensity,
    light colour, light range and visible.
  - Interpolation is linear, step, smooth, or a spline: cubic Hermite with
    Catmull-Rom tangents for uneven key spacing.
  - Optional smooth value noise (amplitude, frequency, seed) adds flicker,
    sway or shake.
  - Events fire when their time is crossed, including over loop wraps.
- **World state.**
  - `WorldCommand::PlaySequence {name, sequence, start_ms}` and
    `StopSequence`.
  - Up to 32 sequences per client, stored in `ClientWorld`.
  - Sequences advance on the fixed 16 ms step, hold while the client is
    paused, and are saved and validated.
  - Tracks write entity transforms and light fields. Camera tracks write
    the shared view camera, so playing a camera sequence requires a camera
    to be set.
  - Light tracks on entities without lights, and missing entities, fail at
    submit.
- **API.** `World.playSequence`, `stopSequence`, `sequenceStatus` and
  `sequenceEvents` (events are consumed when read). Mirrored in
  `tools/engine_schema.py`.
- **Desktop.** `render --movie-frames N --movie-dir D` writes N consecutive
  frames with one renderer, so caches and history carry over as in a
  running game.

## Validation (2026-10-25, Apple M5 / Metal)

- Unit tests:
  - All interpolation modes hit their keys, and they differ between keys.
  - Noise is bounded, continuous and deterministic.
  - Playback fires events, loops twice and stops at the end; validation
    rejects wrong value counts and keys outside the sequence.
- Engine `tests/sequences.rs`:
  - A camera sequence without a camera, a light track on a non-light and a
    missing entity are all rejected.
  - At 800 ms of a 1600 ms sequence, the gear is at exactly 180° and the
    camera is exactly halfway.
  - The torch intensity flickers within 10 ± 4.
  - The event fires once and is consumed.
  - The saved world validates.
  - Pausing holds the time; looping counts one loop; stop removes the
    sequence.
- Python: the schema mirror accepts valid specs and rejects six invalid
  ones.
- **OLY100 hall.** `hall-sequence.ts` sets up a 6 s looping spline dolly
  over three camera keys. It also gives each of the 102 warm, non-ambient
  imported lights (red > 1.3 × blue) intensity flicker (±35%, 6 Hz) and an
  additive ember emitter.
  - 188 frames at 32 ms render in 45.7 s headless, including load and TAA
    warm-up per frame.
  - The movie is
    `~/.cache/crysirror/gow3/olympus-hd/preview/hall-sequence.mp4` and is
    derived from user data, so it is not in the repository.
  - This is engine-authored motion, not native GoW animation.

## Gaps

- Decode GoW ANM: the channel layout, compression and node binding. Then
  map native animations onto sequences or skeletal clips.
- More track kinds: material parameters, particles on/off, audio cues,
  sub-sequences and blending between sequences.
- An editor or curve tools. Sequences are data only.
