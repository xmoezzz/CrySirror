# ExHibit DLT: ordinary indexed clips across four installations

## Famisupi: DLT 0x0101 opening asset

Validated the stock `res/g/ev/12/1239.dlt` from the pinned local v1
installation. No assets or frames are included in the repository.

- Container: 10,659,750 bytes; SHA-256
  `3f92bcd7e21c9169dd43175d119792e232526393341e7b6a12eb006a7e75a72c`.
- 800 × 600 pixels, 80 ms period, loop flag, 49 frames.
- A separate Python implementation and the Rust decoder produced the same
  SHA-256 over the 49 concatenated top-down RGBA frames:
  `4e8890840c634b958bfa0c87bcc32710c7f2776eb00921142d1c8a4ecf917462`.

The palette contains BGR0 entries. The base image is a bottom-up indexed
plane. Each subsequent frame LZSS-decodes a top-down rectangle and replaces
only its nonzero indices. The loop wraps to the first delta without
reapplying the raw base; unit coverage verifies it preserves the last
canvas. The v1 opening adapter rejects other versions before using the
shared decoder.

Re-run with a user-supplied local copy, without launching the game:

```sh
CRYSIRROR_EXHIBIT_DLT_FIXTURE=/path/to/1239.dlt cargo test --release \
  -p crysirror-resource official_opening_asset_matches_independent_pixel_hashes \
  -- --ignored
```

## Famisupi numbered clips in F11A1101

At `+0x609`, opcode `0x76` prepares CG 1180 on layer 40 at (200, 50).
Its v1 header declares 400 × 400, 25 images, an 80 ms interval and repeat.
At `+0x631`, `0x77` starts the decoder and fades to level 16 over 800 ms.
Four dialogue records follow. At `+0x7cb`, `0x75` fades to zero over
800 ms and retires the layer and its image. The earlier 1239 clip uses
the same adapter on layer 4 at the origin.

The numbered filename, layer geometry and CG experience follow resident
RVAs `0x3c580` and `0x3c650`. The playback and exit paths are at
`0x3c7a0` and `0x3c900`. RVA `0x0e830` selects the interval and caps
long-gap catchup. A non-looping clip ends its frame updates while the
layer retains the final image. The tested adapter admits numbered,
channel-zero fresh layers and script-controlled opacity and fade times.
A live channel's completion wait, exit with retained layer, replacement
of an occupied layer, and skip input during fades remain outside coverage.

For DLT 0x0101 pixel profiles 0x0802 and 0x0803, RVA `0x4a150` reads
the separate mask byte length. A 0x4f02 full image has two padded,
bottom-up raw planes. A 0x4200 rectangle contains separate LZSS color
and mask streams. At `0x4b140` a zero color index preserves the previous
index. At `0x4b1c0` a mask byte of 0xfe preserves coverage and a zero
clears it. RVA `0x4a710` selects the 16-level mask. The decoder finishes
both streams before it updates either retained plane.

Stock 1180.dlt SHA-256 is
`fe17d25044ef8851f6487b31f516844e49ae4c2cd369ed6c2c255afb4a9ba83e`.
The independent decoder and the release Rust fixture test produced the same SHA-256 over all 25 top-down RGBA images,
`0d6aea2ba35be3f915b3dbd343162a2a17ca976ead05fa789df2c8b22e7dd52b`.
The fixture can be checked without booting the game:

```sh
CRYSIRROR_EXHIBIT_DLT_MASKED=/path/to/1180.dlt cargo test --release \
  -p crysirror-resource official_masked_clip_matches_independent_rgba_hash \
  -- --ignored
```

## Midori and Stray Sheep: DLT 0x0102 and 0x0103 media only

The Midori frame decoder, rectangle compositor and repeat logic are at
resident RVAs `0x7b1f0`, `0x7a920` and `0x7a850`. Both later resident
builds have the same ordinary indexed record path, independently located
at RVAs `0x242cd0` (Vol. 1) and `0x243250` (Vol. 2). The latter two
residents are distinct. This asset compatibility does not establish that
their VM dispatch or initialization is interchangeable.

| Version | Indexed no-alpha layout |
| --- | --- |
| 0x0101 | Padded raw bottom-up base (0x0f02), top-down 0x0200 delta, count excludes terminator. |
| 0x0102 | Same raw padded base; 0x10200 deltas are tightly packed bottom-up. Count excludes terminator. Native looping skips palette and base, preserving the previous indices. |
| 0x0103 | Nonlooping profile (0x0800), DLT marker at header +28, LZSS-compressed tightly packed bottom-up base (0x0d02), bottom-up 0x10200 delta, count includes terminator. |

Zero in a delta retains the previous index. An 0x10004 record in 0x0102 or
0x0103 consumes one display step without changing pixels. All palette,
canvas and LZSS spans are bounds-checked. An invalid compressed base or
delta cannot advance the cursor or partially update the canvas.

A separate decoder stored the native DIB upside down and only flipped for
output. SHA-256 of its complete top-down RGBA sequence matched the shared
Rust decoder for the pinned fixtures:

| Fixture | Dimensions | Frames (holds) | Period | Concatenated RGBA SHA-256 |
| --- | --- | --- | --- | --- |
| Midori 1209.dlt | 1024×576 | 49 (1) | 80 ms | `96b785cb83fa56afb136d3c192d2c7d2876a528c8bf3020dc2d927c69bbeb58a` |
| Stray Sheep Vol. 1 transition 0001.dlt | 1920×1080 | 64 (0) | 33 ms | `ad095683af5cd9b8efdf52b4f1c1488a400dc5e7a3055aad49f02e1385e879e7` |
| Stray Sheep Vol. 2 effect tisibuki.dlt | 1920×1080 | 8 (2) | 80 ms | `9bc9aa7b61befbff823ccac98f4be993dee06918d18ec8527c42d8fa630f1145` |

`CRYSIRROR_EXHIBIT_DLT_MIDORI`, `CRYSIRROR_EXHIBIT_DLT_STRAY_VOL1` and
`CRYSIRROR_EXHIBIT_DLT_STRAY_VOL2` name optional local input files for
the ignored resource tests. No original executables, game sessions or
user saves are involved.

Both Stray Sheep installations identify RLD v3 in their loose headers and
declare `start` and 1920×1080 in their launch metadata. The two directories
contain 65/76 RLD headers, 7/5 DLT clips and 1/2 ASF/WMV files respectively.
Only 2/3 of the DLT files have the now-admitted non-alpha header. The other
5/2 declare a separate alpha plane and use different record formats; they
still reject. The decoder is an ordinary standalone asset reader. No
later script operation or initialization route is enabled by it.
The next scenario intake evidence would be a reproducible ordinary late-v3
fixture with known record boundaries and an independently established native
initialization contract. A matching version-3 header or shared media codec
does not qualify the 2011 initializer for either later profile.
