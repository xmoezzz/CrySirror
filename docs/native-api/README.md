# Native engine developer references

Each reference contains the native ABI, dispatch/return and wait contracts, shared-service mappings, implementation tables, subselectors and dynamic routing rules.

| Engine | Reference | Opcode / API entries | Selector / conditional rules |
| --- | --- | ---: | ---: |
| Circus | [circus](circus.md) | 753 | 1357 |
| AliceSoft System 4 | [system4](system4.md) | 616 | 471 |
| RealLive | [reallive](reallive.md) | 1504 | 318 |
| AVG32 | [avg32](avg32.md) | 297 | 167 |
| CatSystem2 | [catsystem2](catsystem2.md) | 732 | 868 |
| Majiro | [majiro](majiro.md) | 458 | 193 |
| NScripter | [nscripter](nscripter.md) | 341 | 96 |
| QLIE | [qlie](qlie.md) | 683 | 660 |
| System-NNN | [system-nnn](system-nnn.md) | 120 | 630 |
| SugarPot | [sugarpot](sugarpot.md) | 184 | 451 |
| WillPlus / AdvHD | [willplus](willplus.md) | 519 | 74 |
| KiriKiri / TJS2 / KAG | [krkr](krkr.md) | 1025 | 479 |
| N-System | [nsystem](nsystem.md) | 134 | 128 |
| codeX RScript | [rscript](rscript.md) | 138 | 0 |

## Status

| Label | Meaning |
| --- | --- |
| Handled | Executable source branch present. Parameter restrictions remain in the row and linked handler. |
| Partial | The branch includes an explicit unsupported/unimplemented path. |
| Unsupported | The branch explicitly rejects execution. |
| Original no-op | Confirmed empty entry in the recorded native dispatch catalogue. |
| Empty handler | Empty source body without original no-op evidence. |
| Decode only | An operand/name catalogue entry with no extracted execution route. |
| Unreviewed | Known native catalogue entry whose execution route needs review. |
| Continuation | Bytecode continuation record, not a separate VM operation. |
| Parameterized | Native registration accepts a caller-supplied name; this is an open name pattern, not one fixed builtin. |
| Guard / allowlist | A native dispatch predicate; execution belongs to the caller. |
| Return only | The branch returns a boolean/optional result; its caller defines pending/forward semantics. |

Counts are table entries, not an upstream completeness percentage. Modules, class receivers, profiles and subcommands have separate namespaces. Unknown native totals remain unknown.

Siglus, BGI, SoftPAL and RFVP use the [upstream-version and local-change table](../INTEGRATED_ENGINES.md).

Malie / MalieSystem has a [native coverage and host contract reference](../../crates/crysirror-malie/README.md#coverage-against-the-exe), with separate [game validation records](../validation/malie-ragnarok.md).

Regenerate after dispatch changes:

```sh
python3 -m pip install -r tools/requirements-native-api.txt
python3 tools/native_api_inventory.py
python3 tools/native_api_inventory.py --check
```

## Maintenance and coverage

The text above the generated marker in each reference is maintained by developers. The generator rebuilds explicit dispatch tables and selector/conditional rules from production Rust source, including default branches. Update the ABI and service contract when changing a dispatcher.

Coverage includes matched opcode/module/export tables, native selector expressions and conditional name/opcode routes. Serialized save-value kinds, opcode layout catalogues already listed in the opcode tables, decompilers and test modules are excluded from selector counts. Rust pattern counts and aliases are not native operation totals. Parameterized registrations retain their receiver expression; game-defined macros/functions have no fixed built-in denominator.

`Handled` reports a source implementation, not a semantic-parity certificate. `—` means no explicit rejection was found in that branch. Validation is recorded separately in each reference; neither table generation nor source inspection is a game or platform execution result.
