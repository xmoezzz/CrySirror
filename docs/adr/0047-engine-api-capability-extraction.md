# ADR 0047: engine API capability extraction, no hosted engines

Date: 2026-10-26. Status: accepted; hosted implementations removed.

The purpose of studying external engines is to expand CrySirror's reusable
capabilities. A VM should need a thin semantic/syscall mapping to shared APIs,
not carry its original rendering, text, UI and audio engine with it.

The RFVP/BGI/RealLive hosting implementations were removed. Preserving some
shared helper calls while flattening a native scene into an uploaded frame
does not satisfy this requirement. Running a game or matching screenshots
cannot be used as the acceptance criterion for capability extraction.

Generic mechanisms belong to engine crates and must have VM-independent API
coverage. Adapters preserve original instruction/value/layout/resource-policy/
wait/save semantics and translate their operations into those APIs and shared
logical state. Missing APIs must be implemented in CrySirror before integration.
