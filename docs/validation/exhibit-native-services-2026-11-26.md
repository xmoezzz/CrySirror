# ExHibit native services and views, 2026-11-26

## Behavior covered

| Profile / instruction | Release regression and observed result |
| --- | --- |
| V1 0x29/2a/2b/2e and decimal service 36 | `retail_view_definitions_route_to_independent_owners_and_override_only_the_load_page` executes generated RLD. Distinct save and load definitions keep independent page coordinates. The load override moves only the addressed page. The test checks signed bounds, extended versus default font words, paired log fonts and selection geometry. |
| V1 decimal service 30 | `entry_assignment_retains_cp932_and_final_remainder` checks the five original defaults, byte-preserving replacement, missing arguments and invalid indices. The scenario test also reaches the handler from RLD. |
| V1 decimal services 40/41 and 0x5d | `gallery_gate_requires_any_positive_counter_in_the_selected_bank` checks the enabled/disabled decision against actual experience counters and invalid layer/item IDs. `binding_two_buttons_publishes_idle_pixels_with_original_parent_alpha` tests the shared image-worker output, disabled atlas cell, range/group removal, surviving hit target, RGB clear, original alpha and stale-image retirement. The gate is covered by its native decision and the underlying disable/erase paths have pixel assertions. |
| 2011 0x2e | `selection_2011_keeps_extra_image_ids_and_replaces_geometry_and_mode_properties` executes all three layouts. It checks eight independently retained image slots, the exact -1 sentinel, normalized negative bounds, unsigned mode bits, binary font fields and replacement by the basic layout. |
| 2011 0x29/2a/2b and decimal service 36 | `slot_and_log_2011_resolve_layer_aliases_and_keep_ordinal_page_overrides` checks explicit and compact layer IDs, independent page positions, log creation argument, binary font data and rollback of an unsupported live alias. |
| 2011 decimal services 23/44 and actions 186–191, 340–342, 348 | `action_flag_families_resolve_live_registers_and_leave_unrelated_bits_untouched` executes 13 RLD records, checking local versus system bits, enabled and disabled states, toggle, ruby-line fade nibble, out-of-range bit handling and action result writeback. The action parser now resolves supported scalar R/S/L/F/M/C reads from live state; native function and arithmetic programs remain separate. |
| Both numeric ABIs | `installed_crt_overflow_and_prefixes_are_profile_specific` compares wraparound/saturation, binary and hexadecimal prefixes and the 2011 unsigned conversion. The installed original CRT code was inspected statically. V1 decimal uses wrapping EAX arithmetic. The 2011 decimal path uses strtol, while its unsigned binary-prefixed string is passed whole to strtoul and stops at the letter b. |

The four view opcodes define the original state used by later calls. This pass
does not execute an original save/load, log or choice screen. No retail game
or vendor executable was started. Tests use generated RLD and in-memory host
resources. The optional read-only data tests access the installed scripts,
media headers and specified source images without writing to either game.

## Installed corpus and native dispatch

| Profile | Structural data | Dispatch admission using the current parser | Native nested dispatch |
| --- | --- | --- | --- |
| V1 Famisupi | 357 scripts / 57,255 records | 51/91 used IDs admit at least one record; 11,602 admits, 45,653 rejects | 12 of 23 0x0b services have a scoped route. The sample uses 18 and admits 11, or 220 of 426 call records. |
| V3 2011 Midori | 379 scripts / 51,797 records | 36/97 used IDs admit at least one record; 8,581 admits, 43,216 rejects | 6 of 49 original 0x0b service selectors and 14 of 232 explicitly assigned non-default 0xbf action slots have a scoped route; the sample admits 7/20 service records and 373/388 action records. |
| Later v3 volume 1 | 65 RLD headers | Ordinary payload/VM behavior unverified | 51 non-default services; 296 explicit non-default action slots. |
| Later v3 volume 2 | 76 RLD headers | Ordinary payload/VM behavior unverified | Separate resident has 51 non-default services and 296 explicit non-default action slots. |

The 2011 corpus test previously required 71 records in the outer def.rld.
Its installed header declares 82; an independent record walk ends precisely at
byte 8452. The first debug/checkpoint, OS branch, settings program and final
nested transfer offsets all match. The corrected test retains those assertions.

## Execution boundaries

- V1 still rejects all 30,462 ordinary dialogue records (0x1c). Original history insertion, voice completion and hit-wait must agree at one execution boundary. The current title control renderer handles the unmasked indexed kind; the original hover/callback, masked hit maps, typed controls and nested modal paths have outstanding work.
- V1 sampled native service selectors still without an admitted record are 2, 3, 24, 25, 32, 35 and 42. They cover system interactions and restart/load requests, original confirmation and URL handling, work surfaces, UI sound and an additional draw dispatcher. Unsampled services 0, 1, 31 and 34 also remain. In particular 0/1 load a replacement command table without resetting the interpreter's already-incremented ordinal; adapting them to the ordinary transfer would change script state.
- Opening a native save/load dialog and opcode 0x0c need the original serializer, file-time/memo indexes, restore and transactional state. V1 projected character/body definitions, some motion/CG and DLT shapes leave most onstage actor, printer, effect and camera paths open.
- The 2011 direct entry remains at the taken `start.rld+0x114` checkpoint. Original camera/viewport, live layer/order, positioned actors and printer/control state are not all available. The original launcher first runs def.rld, initializes camera and controls, then invokes start.rld. Static `0x2e` admissions in def.rld are independent of that lifecycle.
- The remaining sampled 2011 service selectors are 25, 27, 28 and 51. These need the original URL/modal, toolbar callbacks including voice cancellation/update, and the DLT proxy/layer operation. Outstanding sampled actions are 151, 164, 166, 250, 265, 345 and 350. They depend on typed save/load and auto/skip controls; the complete RNF program with A/@ directives; action265's modes and native parameters; temporary layer state including motion/fade restart; and extension/inner-call state, respectively.
- 2011 view layers 1000 and above need live character/actor mappings. Unsupported compound expressions, restore, printer output and further native visual/audio command forms remain explicit gaps. Both later volumes have separate initializer and live-state prerequisites. Neither imports the 2011 dispatcher.

The reproducible native tables and relevant RVAs are in the
[ABI reference](../../crates/crysirror-exhibit/NATIVE_ABI.md).
The full used-ID denominators are in the [v1 census](../../crates/crysirror-exhibit/V1_RETAIL_CENSUS.md)
and [2011 census](../../crates/crysirror-exhibit/MIDORI_2011_CENSUS.md).

## Reproduction

```sh
CRYSIRROR_EXHIBIT_CENSUS_ROOT="$HOME/Downloads/game-ExHIBIT-famisupi" \
CRYSIRROR_EXHIBIT_MIDORI_ROOT="$HOME/Downloads/game-ExHIBIT-midori-no-umi" \
cargo test --offline --locked --release -p crysirror-exhibit --lib -- --include-ignored --nocapture
```

Local logs are under `target/exhibit-proof/v1-services-1126/`. The default
release run passed 83 tests and skipped three installed-data tests (`final-test.log`).
With the installed sample paths above, all 86 tests passed, including both
script censuses and the stock Vorbis loop test (`final-all.log`). Numeric
per-shape totals and native-constructor evidence
are in `census-v1.log`, `census-2011.log` and `dispatch-totals.log`.
