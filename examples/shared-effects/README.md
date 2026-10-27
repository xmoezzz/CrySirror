# Shared image pipeline showcase

All pixels and geometry are generated through CrySirror's public Bitmap/Layer
APIs. No game files, native VM or plugin is required. `showcase.ts` awaits shared
worker operations before publishing ordinary image layers.

Build the normal desktop host, then run with an explicitly supplied licensed
system font:

```sh
cargo build --release -p crysirror-desktop
target/release/crysirror render examples/shared-effects/project.json \
  --script examples/shared-effects/showcase.ts --script-client game \
  --font /absolute/path/to/system-font.ttc \
  --output /tmp/shared-effects.png --frames 300 \
  --data-dir /tmp/crysirror-effects-state
```

Panels demonstrate original pixels, per-channel gamma, Gaussian blur, vector
fill/stroke, perspective, row distortion, mask transition and blur transition.
The last panel is intentionally uniform at the midpoint: complementary checker
images blend to gray. These are CPU resource operations, independent of renderer
GPU shader effects. Native full-image `copyRaster` has its own preserved-edge
SineRowCopy definition and is independently tested.

For shared YM2608 synthesis and worker offline mixing:

```sh
cargo run --release -p crysirror-audio --example chiptune -- /tmp/crysirror-chiptune
```

This generates a self-authored `lead.opna` score and a two-second stereo
`lead.wav`; no Yamaha ROM or sample bank is needed. These outputs belong outside
the repository. See ADR 0089 and 0090 for limits and supported native profiles.
