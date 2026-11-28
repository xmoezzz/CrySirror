# ExHibit v1 opening CG: oversized mode-9 source rectangle

The stock `F00A1101_2.rld+0x130` requests CG 1239 in mode 9 with a 500 ms
transition and source rectangle `[0,0,1600,1200]`. The actual
`res/g/ev/12/1239.gyu` is 800 × 600. No external seed table, palette or
separate alpha plane is used by this 24-bit file.

On the original x86/MMX path, `cmdDraw` at RVA 0x6da70 dispatches to
`UxAdvSystem::draw` at 0x37780. With no rotation/filter flags and both
authored extents exceeding the canvas, it chooses the full-destination
helper at 0x8cab0. Its 0x8cb70 callback intersects the requested source
with the actual source bounds and the destination with its own bounds.
The resulting two 800 × 600 rectangles use the equal-size direct copy.
There is no nominal coordinate division or sampling outside the texture.
The unretained backup is released; 0x32580 separately records the authored
rectangle for scene-state bookkeeping. The non-MMX fallback and other
oversized or off-origin forms are not covered by this implementation.

The exact branch uses existing CrySirror layer 1 with full UVs and the
existing 500 ms, 16-step crossfade. Unit tests verify the dimensions, UVs,
stable layer identity and that nearby variants still fail. The ordinary
stock file decodes through the current shared Rust registry; its top-down
RGBA exactly matches an independent split-MT/LZSS/stride reader:

| Item | SHA-256 |
| --- | --- |
| Stock compressed asset | `349534508f357d2066b233b0b077a2f16edcde14d1b524e9c807d04d8ee7e26e` |
| 1,920,000 decoded RGBA bytes | `4fcd1d79b56f883dc3ce9b445d208b07152da1ac981f82498d03b376e73eddf6` |

No proprietary pixels are included here. These checks did not launch the
game, compare GPU blend pixels or validate the subsequent native save or
dialogue. Record and opcode admission counts are unchanged: the exact
opening tuple already reached its dispatch arm before the geometry fix.

## First-chapter crop and scale

The `F11A1101.rld+0x5bd` record selects a 400 × 300 window at (0,840)
on a 1600 × 1200 opaque GYU. In the 0x8c580 branch, each axis takes
the integer pixel at the current and next output coordinate. Weights
come from the current coordinate in 8.8 precision and sum to 255 on
each axis. RGB uses one final right shift of 16 bits. The successor
tap can cross the crop boundary while it remains inside the full source.
At the source edge, the corresponding tap repeats the current pixel.
The original helper has a different path when either extent equals
its destination or is at most a quarter of it.

The adapter reads bounded metadata from the original asset, then uses
the shared `rect_resample_byte255` image job and submits its immutable
output to the existing mode-9 crossfade. Preparation preserves the
previous layer and the instruction cursor. The fade completes before
the VM commits geometry, CG experience and the next instruction.
Subsequent replacement retires the generated resource through the host,
including its retained transition references. Existing image commands
and other image-effect variants retain their parameter and execution
contracts. Exhaustive Rust matches on `ImageEffect` need the new variant.
