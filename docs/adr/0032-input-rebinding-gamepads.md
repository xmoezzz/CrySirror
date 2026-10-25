# 0032: Runtime action rebinding and desktop gamepads

Date: 2026-10-25. Status: implemented; validation results recorded below.

## Scope and ownership

The owner authorized an input task in parallel with the audio and text work.
The engine owns checked physical-input events, logical action bindings and
prompt descriptors. The desktop owns the optional GilRs device context and
polls it on its event-loop thread. No GPU/scene/GoW importer or native VM
instruction/save layout is changed. The existing core `InputFrame` is unchanged.

New implementation files are `crysirror-engine/src/input_api.rs` and
`crysirror-desktop/src/gamepad.rs`. Existing engine class registration/context/
frame hooks and desktop feature/event-loop integration are parent-owned seams.

## Engine contract

`ActionMap` is per selected engine client/session. Bindings have at most 256
named actions and 16 bindings per action. Unknown enum variants/fields, physical
keys, non-finite values, invalid thresholds/dead zones and oversized definitions
fail before mutation. Complete profile imports validate before replacing live
bindings. Native gamepads must connect before sending state; at most 16 devices
are held. The most recently active device supplies gameplay values. Switching
active pads clears the other pad's pending button presses, and disconnects remove
its held state.

Bindings are checked data:

```json
{
  "bindings": [
    {"kind": "key", "key": "space"},
    {"kind": "button", "button": "south"}
  ],
  "threshold": 0.5
}
```

Axis bindings use `{kind:"axis", axis:"left_x", scale:1, dead_zone:0.15}`.
The engine applies an axial dead zone, rescales the remainder to full range,
sums bindings and clamps the result to -1..1. Axis signs follow GilRs (+Y up);
negative scale reverses an axis. `isDown` tests absolute magnitude against the
action threshold. `value` exposes signed analog values; `axis(negative,positive)`
returns the difference of the two action magnitudes. Stick movement therefore
need not become digital buttons.

`begin_frame` exposes keyboard/button press pulses plus analog threshold
crossings; `end_frame` records held values and pointer position. `pressed` and
`released` are frame queries intended for lifecycle callbacks. Direct rebinds
and capture suppress action edges until the end of that frame, preventing the
binding-confirmation key from also activating gameplay.

The `Input` class retains `bind`, `isDown`, `pressed`, `axis`, `pointer`,
`pointerDelta` and `keys`, and adds:

- `rebind(action, spec)`, `bindings(action)`, `value(action)`, `released(action)`.
- `startRebind(action)`, `cancelRebind()`, `pendingRebind`, `takeRebind()`.
  Capture accepts the next pressed key/button or a fresh axis threshold crossing
  at magnitude 0.65. It stores a physical binding rather than a device handle.
- `exportBindings()` / `importBindings(profile)`: schema-1 logical profiles.
  Applications can persist them through the existing client-scoped Fs save API.
  Raw held buttons, hardware IDs, device pointers and the capture session are
  not serialized. Runtime world/VM snapshots are not changed.
- `prompt(action)`, `lastDevice`, `setPromptFamily(family|null)`.

A prompt is `{device,family,control,label,icon}`. `icon` is a stable semantic
asset token such as `input/playstation/south`; `label` is `Cross` for that case,
`A` for Xbox south, `B` for Nintendo south. Keyboard/mouse prompts return names
and tokens such as `input/keyboard/space`. The engine does not ship controller
artwork, embed a proprietary font, or claim that the renderer draws an icon
without an application-supplied asset. Meaningful device activity selects the
preferred binding; applications can override label family independently.

The legacy `bindings` keyboard-mask field remains for the unregistered old
Input class implementation's source compatibility. The registered class uses
validated action definitions. Keyboard names remain the existing 19 shared
core names; arbitrary scan-code binding is outside this slice.

## Desktop backend

Optional feature `gamepad` uses **GilRs 0.11.2**, whose installed crate source
and [official API documentation](https://docs.rs/gilrs/0.11.2/gilrs/) were
inspected. `Gilrs::new`, `gamepads`, `next_event`, `inc`, `GamepadId` conversion
and typed Button/Axis mappings are used as provided. No FFI or native handle is
exposed to the engine. GilRs' normalized/default-filtered values then pass
through the configured action dead zone. A desktop poll drains at most 4,096
events per frame to bound work. Initialization errors are reported by the host;
keyboard/mouse operation can continue when no backend is available.

Device family inference uses the device name (DualSense/DualShock/Sony,
Xbox/XInput/Microsoft, Nintendo/Joy-Con/Switch Pro); unidentified pads stay
`generic`. This is a label heuristic, not hardware certification. South/East/
West/North remain physical positions regardless of their printed labels.

`Backend::poll` returns plain `GamepadEvent`s for engine injection. Separately,
`navigation_frame` merges south→Enter and D-pad→arrow button bits into the
legacy core frame for retained UI and native VM navigation. Held and pressed
bits stay distinct; a press/release within one poll still produces one press
pulse. D-pad axes also navigate. Gameplay rebinding does not silently rewrite
original VM key semantics. Other native buttons use the advanced action path.

## Implementation, compilation and behavior

Implementation includes the engine action map, reflective methods, semantic
prompts, checked profile replacement, physical capture, optional native backend,
and synthetic event tests. Parent-owned integration connects the engine class
and frame hooks plus desktop event polling.

Validation commands:

```sh
cargo test -p crysirror-engine --test input_rebind
cargo test -p crysirror-desktop --features gamepad --bin crysirror gamepad::tests
cargo check -p crysirror-desktop --features gamepad
```

The input tests cover keyboard hold/press/release, analog dead-zone rescaling,
signed axis values, pressure-sensitive button thresholds and short pulses, runtime capture suppression, logical profile roundtrip and
atomic invalid-profile rejection, family-aware prompts/device switching,
active-pad isolation, disconnects, default navigation pulses, the real App class
registry and generated TypeScript declarations. Desktop mapping tests use pure
mapping functions and do not open a device. Actual local results on 2026-10-25:

- `cargo check -p crysirror-engine`: passed.
- `cargo test -p crysirror-engine --test input_rebind`: **5 passed / 0 failed**.
- `cargo test -p crysirror-desktop --features gamepad --bin crysirror gamepad::tests`:
  **1 passed / 0 failed**; the real backend module compiled against GilRs 0.11.2.
- Logs: `/tmp/crysirror-input-check.log`, `/tmp/crysirror-input-tests.log`,
  `/tmp/crysirror-gamepad-tests.log`. The parent records aggregate validation
  separately. One intermediate test run was blocked by in-progress text-module
  syntax edits; it was rerun successfully and is not counted as a pass.

No gamepad hardware was used or certified. No rumble, player/device assignment,
chord bindings, radial paired-stick dead zones, arbitrary keyboard scan codes,
OS-level rebinding UI or controller artwork is implemented. Multiple devices
can connect, but gameplay intentionally uses one last-active pad rather than
independent multiplayer action maps.
