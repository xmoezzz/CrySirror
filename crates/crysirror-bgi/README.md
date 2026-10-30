# Native BGI BP profile

`crysirror-bgi-vm` executes native BP bytes and retains the recovered dispatch
ABI/opcode metadata from the owner's ethornell-rs (see SOURCE.json). This is a
bounded, one-module/one-thread interpreter slice, not a completed BGI runtime.
No `ethornell-app`, renderer, decoder, text painter or mixer is linked.

Config is `{ "script": "main._bp", "root": "", "font": "main" }`.
`font` is optional unless plain native text is executed. Images are shared engine
resources; native bitmaps become Canvas objects and sprites/backgrounds ordinary
Layer presentations. Plain Graph92:9C text is shared grid/GlyphRun state. BGM
load waits for shared audio-resource preparation before acknowledging the native
call; playback, pause/resume and mixing use shared channels. Native timers,
cooperative yield and supported physical-key queries use engine time/input.

The resource worker owns the capability reader and immutable BP image. The VM
owner alone publishes prepared state. Native calls retain their operands/PC until
a successful engine call. Image metadata admission is still synchronous through
the existing engine image-size API; full async image preparation remains open.

Native opcode coverage is partial. Scenario/Buriko script files, packed resource
search, heap/module/thread scheduling, original saves, native text markup/ruby/
kinsoku/shadows/vertical output, most graphics/filter modes and complete sound
control are not implemented. Unknown modes/calls fail visibly. No real BGI game
has been verified. Synthetic BP and source tables are not whole-game acceptance.

An authored scene exercising bitmap/image/sprite/text/audio/time/input can be
created locally with:

```
cargo run --release -p crysirror-bgi --example fixture -- TARGET_DIR SYSTEM_FONT
```

The font is read at runtime only. The fixture creates its own PNG/WAVE/BP assets,
waits for Enter after the timer, and never distributes game content or fonts.
