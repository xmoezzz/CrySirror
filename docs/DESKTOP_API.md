# Desktop project and command guide

## Schema 2 minimum

```json
{
  "schema": 2,
  "name": "My CrySirror Project",
  "content_revision": "v1",
  "canvas": [960, 540],
  "assets": "assets",
  "clients": [{
    "name": "story",
    "adapter": "crysirror-script-v1",
    "game": "my_game",
    "asset_prefix": "story",
    "program": [
      {"op": "sprite", "id": 0, "asset": "background.png", "rect": [0, 0, 960, 540], "z": 0},
      {"op": "wait_input"},
      {"op": "halt"}
    ]
  }]
}
```

This needs an actual assets/story/background.png. The shipped desktop fixtures
supply their own original tiny images. Paths are logical relative paths, not
ambient drive letters, parent traversal or arbitrary host absolute paths.

The project describes data, resources and client instances. Each adapter owns
its instruction semantics and private state. The data-program syntax below is
not a compatibility format for the old engines.

## Built-in data-program actions

| op | Required values | Optional values |
|---|---|---|
| sprite | id, asset, rect [x,y,w,h], z | tint RGBA8 |
| text | id, text, font alias, size, rect, z | color RGBA8, layout |
| remove | id | none |
| play | channel, asset | looping, gain_milli 0..1000 |
| stop | channel | none |
| wait | ms | none |
| wait_input | none | none |
| halt | none | none |

Only simple-horizontal-v1 is a supported desktop text layout. An arbitrary
native profile name is rejected by the renderer/packager, not guessed.
A text project's fonts object maps aliases to files under the project asset
root, not the per-client prefix. Font files must be provided/licensed by the user.

Limits include project size 4 MiB, canvas dimensions 1..8192, at most 64 clients,
100,000 actions per client, files 64 MiB, cache budgets 256 MiB, text surfaces
four million pixels and static 3D at most one million expanded vertices. GPU
limits may be lower and are checked separately.

## CLI

- run: windowed loop, optional --data-dir, --font, --no-audio, --frames, --capture.
- render: actual offscreen GPU PNG, --output, --frames, --step-ms, --font, --data-dir.
- trace: VM-only ticks, --frames, --step-ms, repeatable --press-at frame indices.
- validate: schema/data checks, not native compatibility or complete asset checks.
- inspect: project and registered adapters, explicitly reporting absent natives.
- gpu-info: actual adapter feature flags and separately implemented renderer paths.

run and render accept --renderer raster (default), compute-raytrace or
hardware-raytrace. Compute ray tracing executes on the GPU and preserves 2D
overlays; hardware-raytrace currently fails explicitly. See ADR 0003 for limits.
Default desktop builds include the siglus-resources image plugin. G00 files with
multiple cuts or native pivot/crop metadata require explicit native processing
and fail in the generic sprite decoder. The original Siglus VM remains unlinked.

The shared CPU image service bounds each decoded surface to 64 MiB and dimensions
8192. The existing GPU cache budget remains 256 MiB.

Run the Python validator with --assets for explicit asset inventory checks.
Audio/device execution requires run; render and trace never emit sound.

## State location and portability

--data-dir overrides the selected state root. Otherwise the host uses the OS
user-data location plus a project fingerprint. Asset semantics changes require
a content_revision bump. Moving the whole project directory does not change its
logical asset references. Absolute --font overrides are not part of a portable
asset package or a snapshot identity.

F5/F9 quick saves are CrySirror envelopes. They are not original RFVP/Siglus
save files. GPU/audio handles are reconstructed, not serialized. Audio restarts.
