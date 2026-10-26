# KiriKiri API examples

The optional `krkr` feature uses the original pinned Kirakira TJS2 VM.
Native Layer/Window/Timer calls issue CrySirror Layer/Canvas/clock operations.
These are self-authored scripts, not complete KAG3 game support.

```sh
cargo run -p crysirror-desktop --features krkr -- render \
  examples/krkr-demo/metrics.json --font /path/to/licensed/system-font.ttf \
  --output target/verification/krkr-metrics.png
```

`metrics.tjs` measures a label with `Layer.font.getTextWidth`, uses the result
to draw a background rectangle, then draws the text through engine Canvas.
The host and renderer use the same configured font resource. Fonts are loaded
at runtime; no fonts are included. The font height API returns nominal absolute
height, rather than glyph ink height. Unmapped styles throw an exception.

`project.json` is the image/Timer/click example and does not require a font.

Without a VM, the same measurement is available as
`text.measureSimple("default", 36, "CrySirror font API")` in JS/TS, or
`TextSystem::measure_simple` in Rust. No rasterizer is held by the adapter.
