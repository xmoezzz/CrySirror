# Native plugin facade over shared vector resources

A self-authored TJS scene uses layerExDraw's admitted solid-fill profile. The
original TJS VM calls thin Appearance/Path/transform mappings; CrySirror owns the
geometry, bounds, worker raster products and retained-layer renderer. No game,
DLL, source image or font is included. This is not a commercial-game startup test.

```sh
cargo build --release -p crysirror-desktop --features krkr
target/release/crysirror render examples/krkr-vector/project.json \
  --frames 600 --font /absolute/path/to/licensed-font.ttf \
  --data-dir /tmp/crysirror-vector-state --output /tmp/crysirror-vector.png
```

The first panel shows ordered brushes, offsets and a polygon; the second uses
an even-odd path and Append matrices. See ADR 0093 for the explicit profile and
unsupported pen/gradient/text/metafile/GDI edge-coverage semantics. The optional
krkr build retains its existing AGPL distribution boundary (ADR 0053).
