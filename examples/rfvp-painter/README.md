# RFVP Win95 Painter on CrySirror

Fetch the game files first (they are not committed; repository policy keeps
`.hcb` and fonts out of the tree):

    python3 tools/fetch_demo_assets.py rfvp-painter   # -> ~/.cache/crysirror/demos/rfvp-painter/assets

`assets/game/painter.hcb` and `assets/game/graph/*` come from rfvp's
`win95_painter_demo` (MPL-2.0, commit 062fec82033b554b9eb3b5da069518c0c96a354e);
`painter.luax`/`painter.yaml` are its source. They run unmodified in the
vendored original RFVP engine through the `rfvp` adapter (ADR 0008).

No font is distributed. Assemble a local project outside the tree and put
any TrueType/OpenType font at `assets/font/host-font.ttf` (it replaces the
original's MS Gothic face), e.g.

    D=$HOME/.cache/crysirror/demos/rfvp-painter
    cp examples/rfvp-painter/project.json $D/
    mkdir -p $D/assets/font && cp "/System/Library/Fonts/Supplemental/Arial.ttf" $D/assets/font/host-font.ttf
    cargo run -p crysirror-desktop -- run $D/project.json
