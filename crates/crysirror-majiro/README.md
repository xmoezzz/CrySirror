# Majiro native adapter

This first-class crate owns original MJO parsing/control flow and native handle,
argument, resource and wait policy. It contains no Majiro renderer, private
bitmap/text algorithm or mixer and executes no original EXE/DLL.

The bounded X/V1.000 reader validates the entire bytecode, instruction and jump
boundaries, function hashes and budgets. MJO XOR is the fixed CRC-table stream;
it is separate from RCT picture-password handling. Original arrays, branch
selectors, exceptional math, full call/return/value compatibility and saves are
not completed. Unknown execution stays at its original instruction.

Native syscall events retain arguments and PC until the host acknowledges an
actual successful operation. Arguments use the original stack-top-first ABI.
The current thin runtime facade maps true-colour `page_create` to shared
Canvas resources. The examined native module allocates **24-bit BGR** pages,
zero-filled and opaque; the public wiki's 32-bit description is not applied.
Indexed pages fail rather than losing palette semantics. `auto_mode_config`
is native policy state; `is_testplay` follows explicit `run_mode != 1`.
An optional module path is only read and SHA-guarded against the examined ABI.
Without it the caller selects this bounded native profile explicitly; this is
not evidence of a reviewed arbitrary game module.

Program reads/parsing, the original ENV and initial portable settings load run
on bounded capability workers. `settings_profile=portable_client` stores named
native decimal string values in the client's Save namespace; it never claims
to import or inspect the original Windows registry. Missing portable values
use reviewed ENV defaults. Setters retain native PC/current values until the
shared atomic-write receipt completes. Strict original registry import is
explicitly unsupported. This preferences profile is distinct from game saves.

Native random integers call the existing shared CRT sequence kernel. Startup
captures original Unix-second seeding policy once; the VM retains explicit
seed/accumulator/draw-count data. This does not replace native modulo sampling
with uniform floats or fixed return values.

Native `fast_mode_disable` retains the reviewed nesting depth and zero result.
Its default explicit `keyboard_lock_policy=engine_logical` clears only this
client's shared logical CapsLock, preserving NumLock and ScrollLock. It does
not import or toggle the physical OS keyboard. `physical_windows` fails at
configuration admission. Shared owner completion precedes native acknowledgement.

Program reads/parsing run on bounded capability workers. Owner publication
precedes VM execution. Shared Canvas owns all resulting rendering state.
Message buffer/layout, image/audio syscalls, cross-module library linking,
original save/restore and whole-game compatibility remain unfinished and fail
visibly. Registration or successful format parsing is not completed integration.

Desktop feature `majiro` enables original archive/ENV boot without extraction.
Detection reads `majiro.env` from fastdata/update and `start.mjo` through common
`majiro/arc` indexing; original files remain intact. Conflicting patch providers
fail until their native precedence policy is mapped.

Local release validation and owner-corpus evidence are recorded in STATUS.md,
separately from implementation claims. No owner game/font/module is distributed.
