# Native execution and ownership audit

Reference VM: xsystem4 `a85467d78eb5c7fb6e581d7fe5a6fdaec42b9ef7`.
Format/type definitions: libsys4 `b14839f23af0963abbdb17f7cac57a36fd8e9bd2`.
The source-file hashes and licence notices are retained in SOURCE.json.

`vm.c`'s `_function_call`/`function_call` move arguments in original order into
local pages, while callee return values stay on the operand stack. References
retain their original page/index pair and ownership. AIN v5 methods do not add
the later v6.1 `this` reference. Native frame cleanup calls destructors before
removing the frame, then releases page members in reverse order. The Rust
continuation stack retains this order even if a destructor reaches a pending
HLL call. It does not execute a nested C interpreter or callback pointer.

`vm_execute_ain` allocates every global structure before ordinary initializers,
resolves reference initializers afterwards, runs function `0` if declared, then
runs recursive global constructors and main. `page.c` allocates struct members
recursively and invokes child constructors before the parent. Arrays are trees
of pages with native dimension order, not a flattened generic collection.
Heap slots use original LIFO reuse and independent allocation sequences. Null
page and null reference remain distinct. Intrinsic VM scalar defaults are
implemented only for reviewed types; no unknown operation gets a zero result.

The adapter's bootstrap and execution batches own the full mutable Machine on a
shared TaskPool worker. Admission is reserved before ownership moves. The owner
observes a completion receipt before reading/mutating that state or calling a
host service. A pending HLL or Sleep never advances its original PC. Limits:
64 MiB AIN, 1,000,000 heap slots, 8,000,000 accounted heap words, 65,536 operand
words, 4,096 call frames, 64 nested structs/copies and 32 array dimensions.
Errors remain native locations; public serialized snapshots/original saves are
unsupported. This is a bounded VM subset, not a full System4 implementation.

Static original OutputLog evidence takes priority over compatibility stubs:
Create RVA0x1000 obtains string bytes, checks a cached connection and calls
GetModuleHandleA("DPLogViewer.ade"), GetProcAddress("GetInterface") and a GUID
interface request. It returns -1 if any lookup fails. Only a real interface can
return a log handle. Disconnected Output/Clear return without dispatch; Save,
EnableAutoSave and DisableAutoSave return false. SHA validation restricts those
branches to the reviewed DLL. No Windows DLL is loaded or executed. The pinned
xsystem4 code creates its own incrementing handle table instead; that mechanism
and its unimplemented persistence macros were deliberately not adopted.

Next shared requirement: a bounded, client-scoped HostServices log operation
must append to the **existing** EngineContext engine log state (including its
line/entry limits), with explicit failure/ownership/completion semantics. It
must be usable independently of System4. CALLSYS6 translates the original byte
string to the chosen original encoding, emits through that shared entry point,
and leaves the native caller responsible for the following S_POP. The System4
adapter must retain the call if admission/completion fails. A connected future
OutputLog HLL may keep name/handle *policy* here, while actual named buffers,
append, clear, save and auto-save belong to shared engine services. This round
stops at CALLSYS6 because shared production source is frozen.
